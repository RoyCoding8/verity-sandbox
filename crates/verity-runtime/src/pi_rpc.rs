use crate::sandbox::{
    default_offline_config, pi_sandbox_available, pi_sandboxed_command, SandboxConfig, SandboxError,
};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
use verity_core::json::{escape_json_string, parse_json, JsonValue};
use verity_core::pi_wire::{WireRole, WireStopReason};

const MAX_RPC_BYTES: usize = 16 * 1024 * 1024;
const MAX_RPC_DEPTH: usize = 128;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn apply_no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Resolves the Pi CLI executable name across platforms.
/// On Windows, npm global shims are `.cmd` files, so `"pi"` resolves to
/// `pi.exe` / `pi.cmd` on `PATH` or `%APPDATA%\npm\pi.cmd` before falling back to `"pi.cmd"`.
pub fn resolve_pi_program(program: &str) -> String {
    #[cfg(windows)]
    {
        if program == "pi" {
            if let Some(paths) = std::env::var_os("PATH") {
                for dir in std::env::split_paths(&paths) {
                    let exe = dir.join("pi.exe");
                    if exe.is_file() {
                        return exe.to_string_lossy().to_string();
                    }
                    let cmd = dir.join("pi.cmd");
                    if cmd.is_file() {
                        return cmd.to_string_lossy().to_string();
                    }
                }
            }
            if let Some(appdata) = std::env::var_os("APPDATA") {
                let npm_cmd = PathBuf::from(appdata).join("npm").join("pi.cmd");
                if npm_cmd.is_file() {
                    return npm_cmd.to_string_lossy().to_string();
                }
            }
            return "pi.cmd".to_string();
        }
    }
    program.to_string()
}

#[derive(Debug, Clone)]
pub struct PiRpcConfig {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    pub max_tool_calls: usize,
}

impl PiRpcConfig {
    pub fn default_pi(cwd: PathBuf, read_only: bool) -> Self {
        let mut args = vec![
            "--mode".to_string(),
            "rpc".to_string(),
            "--no-session".to_string(),
        ];
        if read_only {
            args.push("--no-extensions".to_string());
            args.push("--no-prompt-templates".to_string());
            args.push("--tools".to_string());
            args.push("read,grep,find,ls".to_string());
        }
        Self {
            program: resolve_pi_program("pi"),
            args,
            cwd,
            timeout: Duration::from_secs(300),
            max_tool_calls: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PiRpcError {
    Sandbox(SandboxError),
    SpawnFailed(String),
    Io(String),
    Timeout,
    ToolCallBudgetExceeded { used: usize, limit: usize },
    Protocol(String),
}

impl fmt::Display for PiRpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PiRpcError::Sandbox(error) => write!(f, "{error}"),
            PiRpcError::SpawnFailed(m) => write!(f, "failed to spawn pi rpc: {}", m),
            PiRpcError::Io(m) => write!(f, "pi rpc io error: {}", m),
            PiRpcError::Timeout => write!(f, "pi rpc worker exceeded wall-clock timeout"),
            PiRpcError::ToolCallBudgetExceeded { used, limit } => {
                write!(f, "tool call budget exceeded ({}/{})", used, limit)
            }
            PiRpcError::Protocol(m) => write!(f, "pi rpc protocol error: {}", m),
        }
    }
}

impl std::error::Error for PiRpcError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiPromptResult {
    pub final_text: String,
    pub tool_calls: usize,
    pub settled: bool,
}

fn kill_child(child: &mut Child) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
        }
        let _ = unsafe { kill(-(child.id() as std::ffi::c_int), 9) };
    }
    #[cfg(windows)]
    {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        apply_no_window(&mut cmd);
        let _ = cmd.status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Extracts text delta from either nested Pi `assistantMessageEvent` (`text_delta`)
/// or top-level `delta` fields on a `message_update` event.
fn extract_text_delta(val: &JsonValue) -> Option<&str> {
    if let Some(ame) = val.get("assistantMessageEvent") {
        return match ame.get("type").and_then(JsonValue::as_str) {
            Some("text_delta") => ame
                .get("delta")
                .or_else(|| ame.get("text"))
                .and_then(JsonValue::as_str),
            _ => None,
        };
    }
    val.get("delta").and_then(JsonValue::as_str)
}

/// Runs one RPC prompt against a trusted local worker command.
pub fn run_pi_prompt(
    cfg: &PiRpcConfig,
    prompt_id: &str,
    prompt_text: &str,
) -> Result<PiPromptResult, PiRpcError> {
    let mut cmd = Command::new(resolve_pi_program(&cfg.program));
    cmd.args(&cfg.args).current_dir(&cfg.cwd);
    run_turn(cmd, cfg, prompt_id, prompt_text, false)
}

/// Runs a worker with Linux bubblewrap, using the sandbox's working directory
/// and explicit environment. The host must support the required namespaces.
/// A Pi executable on the sandbox's PATH can use the system runtime paths.
/// Supply additional trusted installation paths through the sandbox config.
fn is_pi_cli_program(program: &str) -> bool {
    let base = std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .or_else(|| program.rsplit(['/', '\\']).next())
        .and_then(|name| name.rsplit('\\').next())
        .unwrap_or("");
    base.eq_ignore_ascii_case("pi")
        || base.eq_ignore_ascii_case("pi.cmd")
        || base.eq_ignore_ascii_case("pi.exe")
}

pub fn run_sandboxed_pi_prompt(
    cfg: &PiRpcConfig,
    sandbox: &SandboxConfig,
    prompt_id: &str,
    prompt_text: &str,
) -> Result<PiPromptResult, PiRpcError> {
    let mut args: Vec<_> = cfg.args.iter().map(String::as_str).collect();
    if is_pi_cli_program(&cfg.program) {
        if !args.contains(&"--no-extensions") {
            args.push("--no-extensions");
        }
        if !args.contains(&"--no-prompt-templates") {
            args.push("--no-prompt-templates");
        }
    }
    let cmd = pi_sandboxed_command(sandbox, &cfg.program, &args).map_err(PiRpcError::Sandbox)?;
    run_turn(cmd, cfg, prompt_id, prompt_text, true)
}

/// Runs a read-only Pi subagent against `cfg.cwd`.
/// On Linux when bubblewrap is available, mounts `cfg.cwd` read-only (`--ro-bind`)
/// with a writable tmpfs `HOME=/tmp` so even a prompt-injected worker cannot modify
/// repository files. On non-Linux (or when bubblewrap is unavailable), enforces
/// Pi's `--no-extensions --no-prompt-templates --tools read,grep,find,ls` flags.
pub fn run_readonly_pi_prompt(
    cfg: &PiRpcConfig,
    prompt_id: &str,
    prompt_text: &str,
) -> Result<PiPromptResult, PiRpcError> {
    let mut args: Vec<&str> = cfg.args.iter().map(String::as_str).collect();
    if is_pi_cli_program(&cfg.program) {
        if !args.contains(&"--no-extensions") {
            args.push("--no-extensions");
        }
        if !args.contains(&"--no-prompt-templates") {
            args.push("--no-prompt-templates");
        }
        if !args.contains(&"--tools") {
            args.push("--tools");
            args.push("read,grep,find,ls");
        }
    }

    if cfg!(target_os = "linux") && pi_sandbox_available() {
        let git_dir = crate::git::git_common_dir(&cfg.cwd).ok();
        let mut sandbox = default_offline_config(&cfg.cwd, git_dir);
        sandbox.readonly_worktree = true;
        sandbox.allow_network = true;
        sandbox.env_passthrough = [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "OPENROUTER_API_KEY",
        ]
        .into_iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_string(), value))
        })
        .collect();
        let cmd =
            pi_sandboxed_command(&sandbox, &cfg.program, &args).map_err(PiRpcError::Sandbox)?;
        return run_turn(cmd, cfg, prompt_id, prompt_text, true);
    }

    let mut cmd = Command::new(resolve_pi_program(&cfg.program));
    cmd.args(&args).current_dir(&cfg.cwd);
    run_turn(cmd, cfg, prompt_id, prompt_text, false)
}

fn run_turn(
    mut cmd: Command,
    cfg: &PiRpcConfig,
    prompt_id: &str,
    prompt_text: &str,
    sandboxed: bool,
) -> Result<PiPromptResult, PiRpcError> {
    let started = Instant::now();
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    apply_no_window(&mut cmd);

    let mut child = cmd.spawn().map_err(|error| {
        if sandboxed {
            PiRpcError::Sandbox(SandboxError::Io(error.to_string()))
        } else {
            PiRpcError::SpawnFailed(error.to_string())
        }
    })?;
    let outcome = drive_turn(&mut child, cfg, prompt_id, prompt_text, started, sandboxed);
    kill_child(&mut child);
    outcome
}

fn drive_turn(
    child: &mut Child,
    cfg: &PiRpcConfig,
    prompt_id: &str,
    prompt_text: &str,
    started: Instant,
    sandboxed: bool,
) -> Result<PiPromptResult, PiRpcError> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| PiRpcError::Io("missing child stdin".to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PiRpcError::Io("missing child stdout".to_string()))?;

    if prompt_id.len().saturating_add(prompt_text.len()) > MAX_RPC_BYTES {
        return Err(PiRpcError::Protocol("RPC prompt exceeds size limit".into()));
    }
    let payload = format!(
        "{{\"id\":{},\"type\":\"prompt\",\"message\":{}}}\n",
        escape_json_string(prompt_id),
        escape_json_string(prompt_text)
    );
    if payload.len() > MAX_RPC_BYTES {
        return Err(PiRpcError::Protocol("RPC prompt exceeds size limit".into()));
    }
    let (events, lines) = mpsc::sync_channel(1);
    let write_errors = events.clone();
    let (_keep_stdin_open, finished) = mpsc::channel::<()>();
    std::thread::Builder::new()
        .spawn(move || {
            if let Err(error) = stdin
                .write_all(payload.as_bytes())
                .and_then(|_| stdin.flush())
            {
                if error.kind() != std::io::ErrorKind::BrokenPipe {
                    let _ = write_errors.send(Err(PiRpcError::Io(error.to_string())));
                }
            } else {
                let _ = finished.recv();
            }
        })
        .map_err(|e| PiRpcError::Io(e.to_string()))?;
    std::thread::Builder::new()
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                let event = match reader
                    .by_ref()
                    .take((MAX_RPC_BYTES + 1) as u64)
                    .read_line(&mut line)
                {
                    Ok(0) => Ok(None),
                    Ok(size) if size > MAX_RPC_BYTES => {
                        Err(PiRpcError::Protocol("RPC record exceeds size limit".into()))
                    }
                    Ok(_) if !line.ends_with('\n') => {
                        Err(PiRpcError::Protocol("unterminated RPC record".into()))
                    }
                    Ok(_) => Ok(Some(line)),
                    Err(e) => Err(PiRpcError::Io(e.to_string())),
                };
                let terminal = !matches!(event, Ok(Some(_)));
                if events.send(event).is_err() || terminal {
                    return;
                }
            }
        })
        .map_err(|e| PiRpcError::Io(e.to_string()))?;

    let mut turn = PromptTurn::default();
    let mut received_event = false;
    loop {
        let remaining = cfg
            .timeout
            .checked_sub(started.elapsed())
            .ok_or(PiRpcError::Timeout)?;
        let line = lines.recv_timeout(remaining).map_err(|e| match e {
            RecvTimeoutError::Timeout => PiRpcError::Timeout,
            RecvTimeoutError::Disconnected => PiRpcError::Io("RPC pipe readers stopped".into()),
        })??;
        let line = line.ok_or_else(|| {
            if sandboxed && !received_event {
                PiRpcError::Sandbox(SandboxError::Io(
                    "Pi worker exited before starting RPC; check bubblewrap and the worker executable".into(),
                ))
            } else {
                PiRpcError::Protocol("worker closed stdout before settling".into())
            }
        })?;
        if line.trim().is_empty() {
            continue;
        }
        check_record_depth(&line)?;
        let event = parse_json(&line).map_err(|e| PiRpcError::Protocol(e.to_string()))?;
        received_event = true;
        let settled = turn.accept(&event, prompt_id, cfg.max_tool_calls)?;
        if started.elapsed() >= cfg.timeout {
            return Err(PiRpcError::Timeout);
        }
        if settled {
            return turn.finish();
        }
    }
}

fn check_record_depth(line: &str) -> Result<(), PiRpcError> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in line.bytes() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if in_string => escaped = true,
            b'"' => in_string = !in_string,
            b'[' | b'{' if !in_string => {
                depth += 1;
                if depth > MAX_RPC_DEPTH {
                    return Err(PiRpcError::Protocol(
                        "RPC record exceeds nesting limit".into(),
                    ));
                }
            }
            b']' | b'}' if !in_string => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

#[derive(Default)]
struct PromptTurn {
    text: String,
    tool_calls: usize,
    assistant_failure: Option<String>,
}

impl PromptTurn {
    fn accept(
        &mut self,
        event: &JsonValue,
        prompt_id: &str,
        tool_limit: usize,
    ) -> Result<bool, PiRpcError> {
        let event_type = event
            .get("type")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| PiRpcError::Protocol("missing RPC event type".into()))?;
        match event_type {
            "response" if event.get("id").and_then(JsonValue::as_str) == Some(prompt_id) => {
                if event.get("command").and_then(JsonValue::as_str) != Some("prompt") {
                    return Err(PiRpcError::Protocol(
                        "unexpected command in prompt response".into(),
                    ));
                }
                match event.get("success").and_then(JsonValue::as_bool) {
                    Some(true) => {
                        if event
                            .get("data")
                            .and_then(|data| data.get("disposition"))
                            .and_then(JsonValue::as_str)
                            == Some("handled")
                        {
                            return Err(PiRpcError::Protocol(
                                "worker handled the prompt without starting a run".into(),
                            ));
                        }
                    }
                    Some(false) => return Err(protocol_error(event, "error", "prompt rejected")),
                    None => {
                        return Err(PiRpcError::Protocol(
                            "missing prompt response status".into(),
                        ))
                    }
                }
            }
            "error" => return Err(protocol_error(event, "error", "worker error")),
            "agent_settled" if event.get("aborted").and_then(JsonValue::as_bool) == Some(true) => {
                return Err(PiRpcError::Protocol("prompt aborted".into()));
            }
            "agent_start" => {
                self.assistant_failure = None;
                self.text.clear();
            }
            "message_start" if event.get("message").is_some_and(is_assistant) => self.text.clear(),
            "message_update" => {
                if let Some(delta) = extract_text_delta(event) {
                    if self.text.len().saturating_add(delta.len()) > MAX_RPC_BYTES {
                        return Err(PiRpcError::Protocol("RPC answer exceeds size limit".into()));
                    }
                    self.text.push_str(delta);
                }
            }
            "message_end" => {
                if let Some(message) = event.get("message").filter(|message| is_assistant(message))
                {
                    self.record_message(message);
                }
            }
            "tool_call" | "tool_execution_start" => {
                self.tool_calls += 1;
                if self.tool_calls > tool_limit {
                    return Err(PiRpcError::ToolCallBudgetExceeded {
                        used: self.tool_calls,
                        limit: tool_limit,
                    });
                }
            }
            "auto_retry_end"
                if event.get("success").and_then(JsonValue::as_bool) == Some(false) =>
            {
                return Err(protocol_error(
                    event,
                    "finalError",
                    "automatic retry failed",
                ));
            }
            "agent_end" => {
                if let Some(message) = event
                    .get("messages")
                    .and_then(JsonValue::as_array)
                    .and_then(|messages| {
                        messages.iter().rev().find(|message| is_assistant(message))
                    })
                {
                    self.record_message(message);
                }
                match event.get("willRetry").and_then(JsonValue::as_bool) {
                    Some(true) => self.text.clear(),
                    Some(false) => {}
                    None => return Ok(self.settle(event)),
                }
            }
            "agent_settled" => return Ok(self.settle(event)),
            _ => {}
        }
        Ok(false)
    }

    fn record_message(&mut self, message: &JsonValue) {
        let reason = message
            .get("stopReason")
            .or_else(|| message.get("stop_reason"))
            .and_then(JsonValue::as_str)
            .map(WireStopReason::parse_str);
        let error = message
            .get("errorMessage")
            .and_then(JsonValue::as_str)
            .filter(|s| !s.is_empty());
        self.assistant_failure = match reason {
            Some(WireStopReason::Error) => Some(error.unwrap_or("assistant failed").into()),
            Some(WireStopReason::Aborted) => Some(error.unwrap_or("prompt aborted").into()),
            Some(WireStopReason::Length) => {
                Some(error.unwrap_or("assistant reached the token limit").into())
            }
            _ => error.map(str::to_string),
        };
        if let Some(parts) = message.get("content").and_then(JsonValue::as_array) {
            self.text = parts
                .iter()
                .filter(|part| part.get("type").and_then(JsonValue::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(JsonValue::as_str))
                .collect();
        }
    }

    fn settle(&mut self, event: &JsonValue) -> bool {
        if self.text.is_empty() {
            if let Some(summary) = event.get("summary").and_then(JsonValue::as_str) {
                self.text.push_str(summary);
            }
        }
        true
    }

    fn finish(self) -> Result<PiPromptResult, PiRpcError> {
        if let Some(error) = self.assistant_failure {
            return Err(PiRpcError::Protocol(error));
        }
        Ok(PiPromptResult {
            final_text: self.text,
            tool_calls: self.tool_calls,
            settled: true,
        })
    }
}

fn is_assistant(message: &JsonValue) -> bool {
    message
        .get("role")
        .and_then(JsonValue::as_str)
        .map(WireRole::parse_str)
        == Some(WireRole::Assistant)
}

fn protocol_error(event: &JsonValue, key: &str, fallback: &str) -> PiRpcError {
    PiRpcError::Protocol(
        event
            .get(key)
            .and_then(JsonValue::as_str)
            .unwrap_or(fallback)
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn shell_config(cwd: PathBuf, script: &str) -> PiRpcConfig {
        PiRpcConfig {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd,
            timeout: Duration::from_secs(2),
            max_tool_calls: 2,
        }
    }

    #[cfg(unix)]
    fn shell_prompt(script: &str) -> Result<PiPromptResult, PiRpcError> {
        run_pi_prompt(
            &shell_config(std::env::current_dir().unwrap(), script),
            "test-prompt",
            "test message",
        )
    }

    #[cfg(unix)]
    #[test]
    fn waits_for_settlement_after_retry() {
        let result = shell_prompt(
            r#"read -r prompt
printf '%s\n' '{"type":"message_update","delta":"discarded"}'
printf '%s\n' '{"type":"agent_end","willRetry":true}'
printf '%s\n' '{"type":"message_update","delta":"answer"}'
printf '%s\n' '{"type":"agent_end","willRetry":false}'
printf '%s\n' '{"type":"agent_settled","aborted":false}'"#,
        )
        .unwrap();
        assert_eq!(result.final_text, "answer");
    }

    #[cfg(unix)]
    #[test]
    fn propagates_matching_prompt_rejection() {
        let result = shell_prompt(
            r#"read -r prompt
printf '%s\n' '{"type":"response","id":"different","command":"prompt","success":false,"error":"ignored"}'
printf '%s\n' '{"type":"response","id":"test-prompt","command":"prompt","success":false,"error":"no model"}'
printf '%s\n' '{"type":"agent_end"}'"#,
        );
        assert_eq!(result, Err(PiRpcError::Protocol("no model".into())));
    }

    #[cfg(unix)]
    #[test]
    fn keeps_stdin_open_until_settled() {
        let result = shell_prompt(
            r#"read -r prompt
(sleep 0.15; printf '%s\n' '{"type":"agent_settled","summary":"done"}') &
read -r unexpected || printf '%s\n' '{"type":"error","error":"stdin closed early"}'"#,
        )
        .unwrap();
        assert_eq!(result.final_text, "done");
    }

    #[cfg(unix)]
    #[test]
    fn times_out_when_worker_does_not_read_prompt() {
        let mut cfg = shell_config(std::env::current_dir().unwrap(), "sleep 5");
        cfg.timeout = Duration::from_millis(150);
        let started = Instant::now();
        let result = run_pi_prompt(&cfg, "test-prompt", &"a".repeat(256 * 1024));
        assert_eq!(result, Err(PiRpcError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_malformed_output_and_close_before_settlement() {
        for script in [
            r#"read -r prompt; printf '%s\n' 'invalid JSON' '{"type":"agent_end"}'"#,
            "read -r prompt",
        ] {
            assert!(matches!(shell_prompt(script), Err(PiRpcError::Protocol(_))));
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_assistant_failure_at_settlement() {
        let result = shell_prompt(
            r#"read -r prompt
printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"error","errorMessage":"provider failed"}}'
printf '%s\n' '{"type":"agent_end","willRetry":false}'
printf '%s\n' '{"type":"agent_settled"}'"#,
        );
        assert_eq!(result, Err(PiRpcError::Protocol("provider failed".into())));
    }

    #[cfg(unix)]
    #[test]
    fn uses_final_assistant_message_when_no_text_was_streamed() {
        let result = shell_prompt(
            r#"read -r prompt
printf '%s\n' '{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"before tool"}]},{"role":"assistant","stopReason":"stop","content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"final "},{"type":"text","text":"answer"}]}]}'"#,
        )
        .unwrap();
        assert_eq!(result.final_text, "final answer");
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn sandbox_unavailable_fails_before_spawning_worker() {
        let cwd = std::env::current_dir().unwrap();
        let sandbox = crate::sandbox::default_offline_config(&cwd, None);
        let mut pi = PiRpcConfig::default_pi(cwd, false);
        pi.program = "definitely-not-an-installed-verity-worker".into();
        assert!(matches!(
            run_sandboxed_pi_prompt(&pi, &sandbox, "test-prompt", "test message"),
            Err(PiRpcError::Sandbox(SandboxError::NotAvailable(_)))
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_worker_can_write_worktree_and_cannot_write_host_or_git_files() {
        use crate::git::{create_worktree, git_common_dir, read_ref, run_git};

        if !crate::sandbox::pi_sandbox_available() {
            eprintln!("skipping native Pi worker test because bubblewrap is unavailable");
            return;
        }
        let root = PathBuf::from("/var/tmp").join(format!(
            "verity-pi-rpc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repo = root.join("repo");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&repo).unwrap();
        assert!(run_git(&repo, &["init"]).unwrap().status.success());
        assert!(run_git(
            &repo,
            &[
                "-c",
                "user.name=Verity test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-m",
                "fixture"
            ],
        )
        .unwrap()
        .status
        .success());
        let base = read_ref(&repo, "HEAD").unwrap().unwrap();
        let task_id = verity_core::TaskId::parse("test-prompt").unwrap();
        create_worktree(&repo, &worktree, &task_id, &base).unwrap();
        let git_dir = git_common_dir(&repo).unwrap();
        let git_marker = std::fs::read_to_string(worktree.join(".git")).unwrap();
        std::fs::write(git_dir.join("sentinel"), "git data").unwrap();
        std::fs::write(root.join("sentinel"), "host data").unwrap();

        let sandbox = crate::sandbox::default_offline_config(&worktree, Some(git_dir.clone()));
        let mut pi = shell_config(
            worktree.clone(),
            r#"read -r prompt
[ "$PWD" = "$HOME" ] && [ ! -e /run ] && [ ! -e "$1" ] || exit 1
printf '%s' 'worker output' > result || exit 1
printf '%s' 'private temp' > "$TMPDIR/verity-pi-temp" || exit 1
[ -r "$2" ] && git rev-parse --verify HEAD >/dev/null || exit 1
if (printf '%s' 'unexpected' > "$1") 2>/dev/null; then exit 1; fi
if (printf '%s' 'unexpected' > "$2") 2>/dev/null; then exit 1; fi
if (printf '%s' 'unexpected' > .git) 2>/dev/null; then exit 1; fi
/bin/sh -c 'printf "%s" ready > child-started; sleep 30; :' "$3" &
child=$!
while [ ! -f child-started ]; do sleep 0.01; done
kill -0 "$child" || exit 1
printf '%s\n' '{"type":"agent_settled","summary":"done"}'"#,
        );
        let child_tag = format!("verity-worker-child:{}", worktree.display());
        pi.args.extend([
            "worker".into(),
            root.join("sentinel").to_str().unwrap().into(),
            git_dir.join("sentinel").to_str().unwrap().into(),
            child_tag.clone(),
        ]);
        let selected = run_sandboxed_pi_prompt(&pi, &sandbox, "test-prompt", "test message");
        let result = std::fs::read_to_string(worktree.join("result"));
        let marker = std::fs::read_to_string(worktree.join(".git"));
        let git_data = std::fs::read_to_string(git_dir.join("sentinel"));
        let host_data = std::fs::read_to_string(root.join("sentinel"));
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(selected.unwrap().final_text, "done");
        assert_eq!(result.unwrap(), "worker output");
        assert_eq!(marker.unwrap(), git_marker);
        assert_eq!(git_data.unwrap(), "git data");
        assert_eq!(host_data.unwrap(), "host data");
        for _ in 0..20 {
            let child_alive = std::fs::read_dir("/proc")
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| {
                    std::fs::read(entry.path().join("cmdline")).is_ok_and(|argv| {
                        argv.split(|byte| *byte == 0)
                            .any(|arg| arg == child_tag.as_bytes())
                    })
                });
            if !child_alive {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("sandbox worker descendant remained alive");
    }

    #[test]
    fn limits_rpc_record_nesting() {
        assert!(check_record_depth(r#"{"summary":"[\"{]"}"#).is_ok());
        let deep = format!(
            "{}{}",
            "[".repeat(MAX_RPC_DEPTH + 1),
            "]".repeat(MAX_RPC_DEPTH + 1)
        );
        assert!(matches!(
            check_record_depth(&deep),
            Err(PiRpcError::Protocol(_))
        ));
    }

    #[test]
    fn extracts_nested_assistant_message_event_text_deltas_only() {
        let nested = parse_json(
            r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"hello"}}"#,
        )
        .unwrap();
        assert_eq!(extract_text_delta(&nested), Some("hello"));

        let thinking = parse_json(
            r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"secret"}}"#,
        )
        .unwrap();
        assert_eq!(extract_text_delta(&thinking), None);

        let flat = parse_json(r#"{"type":"message_update","delta":"world"}"#).unwrap();
        assert_eq!(extract_text_delta(&flat), Some("world"));

        let malformed =
            parse_json(r#"{"type":"message_update","delta":"bad","assistantMessageEvent":{}}"#)
                .unwrap();
        assert_eq!(extract_text_delta(&malformed), None);
    }

    #[test]
    fn detects_pi_cli_program_across_platforms_and_casing() {
        assert!(is_pi_cli_program("pi"));
        assert!(is_pi_cli_program("/usr/local/bin/pi"));
        assert!(is_pi_cli_program("pi.cmd"));
        assert!(is_pi_cli_program("PI.CMD"));
        assert!(is_pi_cli_program(
            r"C:\Users\verity\AppData\Roaming\npm\pi.cmd"
        ));
        assert!(is_pi_cli_program("pi.exe"));
        assert!(is_pi_cli_program(r"C:\Tools\PI.EXE"));
        assert!(!is_pi_cli_program("/bin/sh"));
        assert!(!is_pi_cli_program("pilot"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn readonly_worker_cannot_modify_repo_under_bwrap() {
        if !crate::sandbox::pi_sandbox_available() {
            eprintln!("skipping readonly bwrap test because bubblewrap is unavailable");
            return;
        }
        let repo = PathBuf::from("/var/tmp").join(format!(
            "verity-pi-readonly-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            repo.join("keep.txt"),
            "untouched
",
        )
        .unwrap();

        let pi = shell_config(
            repo.clone(),
            r#"read -r prompt
[ "$HOME" = "/tmp" ] || exit 1
printf 'tmp-ok' > "$HOME/scratch.txt" || exit 1
err=$(touch pwned.txt 2>&1)
rc=$?
if [ "$rc" -eq 0 ]; then
    printf '{"type":"error","error":"touch unexpectedly succeeded"}
'
    exit 0
fi
printf '{"type":"message_update","delta":"blocked:%s"}
' "$err"
printf '{"type":"agent_settled"}
'"#,
        );

        let res = run_readonly_pi_prompt(&pi, "ro-test", "try to touch pwned.txt").unwrap();
        let pwned_exists = repo.join("pwned.txt").exists();
        let kept = std::fs::read_to_string(repo.join("keep.txt")).unwrap();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(res.settled);
        assert!(
            res.final_text.to_ascii_lowercase().contains("read-only"),
            "expected read-only filesystem error, got: {}",
            res.final_text
        );
        assert!(!pwned_exists, "pwned.txt must not be created");
        assert_eq!(
            kept,
            "untouched
"
        );
    }
}
