use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use verity_core::json::{escape_json_string, parse_json};

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
            program: "pi".to_string(),
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
        let pid_str = pid.to_string();
        let _ = Command::new("pkill").args(["-9", "-P", &pid_str]).status();
        let _ = Command::new("kill").args(["-9", &pid_str]).status();
    }
    #[cfg(windows)]
    {
        let pid_str = pid.to_string();
        let _ = Command::new("taskkill")
            .args(["/PID", &pid_str, "/T", "/F"])
            .status();
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

/// Runs a single prompt turn against a Pi JSONL RPC child process with hard
/// wall-clock timeout and tool-call budget enforcement, tearing down the child
/// and any descendant processes so no orphan workers survive.
pub fn run_pi_prompt(
    cfg: &PiRpcConfig,
    prompt_id: &str,
    prompt_text: &str,
) -> Result<PiPromptResult, PiRpcError> {
    let mut child = Command::new(&cfg.program)
        .args(&cfg.args)
        .current_dir(&cfg.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
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
                if let Some(delta) = val.get("delta").and_then(|v| v.as_str()) {
                    final_text.push_str(delta);
                }
            }
            Some("tool_call") => {
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
            Some("agent_settled") => {
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
