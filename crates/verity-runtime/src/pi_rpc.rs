use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use verity_core::json::{escape_json_string, parse_json, JsonValue};

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
    SpawnFailed(String),
    Io(String),
    Timeout,
    ToolCallBudgetExceeded { used: usize, limit: usize },
    Protocol(String),
}

impl fmt::Display for PiRpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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

fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        let pgid_arg = format!("-{}", pid);
        let pid_str = pid.to_string();
        let _ = Command::new("kill")
            .args(["-9", "--", &pgid_arg])
            .status();
        let _ = Command::new("pkill").args(["-9", "-P", &pid_str]).status();
        let _ = Command::new("kill").args(["-9", &pid_str]).status();
    }
    #[cfg(windows)]
    {
        let pid_str = pid.to_string();
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid_str, "/T", "/F"]);
        apply_no_window(&mut cmd);
        let _ = cmd.status();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

fn kill_child(child: &mut Child) {
    kill_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

/// Extracts text delta from either nested Pi `assistantMessageEvent` (`text_delta`)
/// or top-level `delta` fields on a `message_update` event.
fn extract_text_delta(val: &JsonValue) -> Option<&str> {
    if let Some(ame) = val.get("assistantMessageEvent") {
        if let Some(ame_type) = ame.get("type").and_then(JsonValue::as_str) {
            return match ame_type {
                "text_delta" => ame
                    .get("delta")
                    .or_else(|| ame.get("text"))
                    .and_then(JsonValue::as_str),
                _ => None,
            };
        }
    }
    val.get("delta").and_then(JsonValue::as_str)
}

/// Runs a single prompt turn against a Pi JSONL RPC child process with hard
/// wall-clock timeout and tool-call budget enforcement, tearing down the child
/// and any descendant processes so no orphan workers survive.
pub fn run_pi_prompt(
    cfg: &PiRpcConfig,
    prompt_id: &str,
    prompt_text: &str,
) -> Result<PiPromptResult, PiRpcError> {
    let resolved_program = resolve_pi_program(&cfg.program);
    let mut cmd = Command::new(&resolved_program);
    cmd.args(&cfg.args)
        .current_dir(&cfg.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    apply_no_window(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| PiRpcError::SpawnFailed(e.to_string()))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| PiRpcError::Io("missing child stdin".to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PiRpcError::Io("missing child stdout".to_string()))?;

    let payload = format!(
        "{{\"id\":{},\"type\":\"prompt\",\"message\":{}}}\n",
        escape_json_string(prompt_id),
        escape_json_string(prompt_text)
    );
    if let Err(e) = stdin
        .write_all(payload.as_bytes())
        .and_then(|_| stdin.flush())
    {
        kill_child(&mut child);
        return Err(PiRpcError::Io(e.to_string()));
    }
    drop(stdin);

    let timed_out = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let child_pid = child.id();

    let t_out = Arc::clone(&timed_out);
    let t_done = Arc::clone(&done);
    let limit_dur = cfg.timeout;
    let watchdog = std::thread::spawn(move || {
        let start = Instant::now();
        while !t_done.load(Ordering::Relaxed) {
            if start.elapsed() >= limit_dur {
                t_out.store(true, Ordering::SeqCst);
                kill_tree(child_pid);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    let reader = BufReader::new(stdout);
    let mut final_text = String::new();
    let mut tool_calls = 0usize;
    let mut settled = false;
    let mut budget_err: Option<PiRpcError> = None;

    for line_res in reader.lines() {
        if timed_out.load(Ordering::SeqCst) {
            break;
        }
        let line = match line_res {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let val = match parse_json(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match val.get("type").and_then(|v| v.as_str()) {
            Some("message_update") => {
                if let Some(delta) = extract_text_delta(&val) {
                    final_text.push_str(delta);
                }
            }
            Some("tool_call") | Some("tool_execution_start") => {
                tool_calls += 1;
                if tool_calls > cfg.max_tool_calls {
                    budget_err = Some(PiRpcError::ToolCallBudgetExceeded {
                        used: tool_calls,
                        limit: cfg.max_tool_calls,
                    });
                    kill_child(&mut child);
                    break;
                }
            }
            Some("agent_settled") | Some("agent_end") => {
                settled = true;
                if let Some(summary) = val.get("summary").and_then(|v| v.as_str()) {
                    if final_text.is_empty() {
                        final_text.push_str(summary);
                    }
                }
                break;
            }
            _ => {}
        }
    }

    done.store(true, Ordering::SeqCst);
    kill_child(&mut child);
    let _ = watchdog.join();

    if timed_out.load(Ordering::SeqCst) {
        return Err(PiRpcError::Timeout);
    }
    if let Some(err) = budget_err {
        return Err(err);
    }

    Ok(PiPromptResult {
        final_text,
        tool_calls,
        settled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
