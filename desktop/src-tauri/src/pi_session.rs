use crate::attachments::PendingAttachment;
use crate::extension_ui::{
    parse_extension_ui_request, DialogId, DialogKind, ExtensionUiAnswer, ExtensionUiError,
    ExtensionUiResponse, ExtensionUiTracker,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use verity_core::json::{parse_json, to_json_string, JsonValue};
use verity_core::session::{transition_session, SessionActivity, SessionCommand, SessionError};

/// Branded request correlation ID for Pi JSONL RPC commands.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RequestId(String);

impl RequestId {
    pub fn parse(raw: impl Into<String>) -> Result<Self, PiSessionError> {
        let s = raw.into();
        if s.trim().is_empty() {
            return Err(PiSessionError::Protocol(
                "request id must not be empty".to_string(),
            ));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Configuration for spawning a long-lived Pi JSONL RPC subprocess.
#[derive(Debug, Clone)]
pub struct PiSessionConfig {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub command_timeout: Duration,
}

impl PiSessionConfig {
    /// Standard long-lived Pi RPC configuration (`pi --mode rpc --no-session`).
    pub fn default_pi(cwd: PathBuf) -> Self {
        Self {
            program: "pi".to_string(),
            args: vec![
                "--mode".to_string(),
                "rpc".to_string(),
                "--no-session".to_string(),
            ],
            cwd,
            command_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PiSessionError {
    SpawnFailed(String),
    Io(String),
    Session(SessionError),
    ExtensionUi(ExtensionUiError),
    RpcError {
        command: String,
        message: String,
    },
    Timeout {
        request_id: RequestId,
        command: String,
    },
    ProcessExited,
    AlreadyShutdown,
    Protocol(String),
}

impl fmt::Display for PiSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpawnFailed(m) => write!(f, "failed to spawn pi rpc process: {}", m),
            Self::Io(m) => write!(f, "pi rpc io error: {}", m),
            Self::Session(e) => write!(f, "session state error: {}", e),
            Self::ExtensionUi(e) => write!(f, "extension ui error: {}", e),
            Self::RpcError { command, message } => {
                write!(f, "pi rpc '{}' failed: {}", command, message)
            }
            Self::Timeout {
                request_id,
                command,
            } => write!(
                f,
                "pi rpc '{}' (id={}) timed out waiting for response",
                command, request_id
            ),
            Self::ProcessExited => write!(f, "pi rpc process exited unexpectedly"),
            Self::AlreadyShutdown => write!(f, "pi rpc supervisor has already shut down"),
            Self::Protocol(m) => write!(f, "pi rpc protocol error: {}", m),
        }
    }
}

impl std::error::Error for PiSessionError {}

impl From<SessionError> for PiSessionError {
    fn from(value: SessionError) -> Self {
        Self::Session(value)
    }
}

impl From<ExtensionUiError> for PiSessionError {
    fn from(value: ExtensionUiError) -> Self {
        Self::ExtensionUi(value)
    }
}

/// Disposition returned by `request_provider_reload`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadDisposition {
    ExecutedImmediately,
    DeferredUntilSettled,
}

/// Correlated RPC response returned by Pi for a stdin command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PiRpcResponse {
    pub id: RequestId,
    pub command: Option<String>,
    pub success: bool,
    pub data_json: Option<String>,
    pub error: Option<String>,
}

/// All 24 typed Pi JSONL RPC stdin commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PiRpcCommand {
    GetState,
    GetMessages,
    GetSessionStats,
    GetAvailableModels,
    SetModel {
        provider: String,
        model_id: String,
    },
    GetAvailableThinkingLevels,
    SetThinkingLevel {
        level: String,
    },
    GetCommands,
    NewSession {
        parent_session: Option<String>,
    },
    SwitchSession {
        session_path: String,
    },
    SetSessionName {
        name: String,
    },
    GetTree,
    GetForkMessages,
    Fork {
        entry_id: String,
    },
    Clone {
        entry_id: Option<String>,
    },
    ExportHtml {
        output_path: Option<String>,
    },
    Prompt {
        message: String,
        images: Vec<PendingAttachment>,
    },
    Compact {
        custom_instructions: Option<String>,
    },
    Steer {
        message: String,
        images: Vec<PendingAttachment>,
    },
    FollowUp {
        message: String,
        images: Vec<PendingAttachment>,
    },
    ClearQueue,
    Abort,
    AbortRetry,
    ExtensionUiResponse(ExtensionUiResponse),
}

impl PiRpcCommand {
    /// Returns the wire `type` string for this Pi RPC command.
    pub fn command_type(&self) -> &'static str {
        match self {
            Self::GetState => "get_state",
            Self::GetMessages => "get_messages",
            Self::GetSessionStats => "get_session_stats",
            Self::GetAvailableModels => "get_available_models",
            Self::SetModel { .. } => "set_model",
            Self::GetAvailableThinkingLevels => "get_available_thinking_levels",
            Self::SetThinkingLevel { .. } => "set_thinking_level",
            Self::GetCommands => "get_commands",
            Self::NewSession { .. } => "new_session",
            Self::SwitchSession { .. } => "switch_session",
            Self::SetSessionName { .. } => "set_session_name",
            Self::GetTree => "get_tree",
            Self::GetForkMessages => "get_fork_messages",
            Self::Fork { .. } => "fork",
            Self::Clone { .. } => "clone",
            Self::ExportHtml { .. } => "export_html",
            Self::Prompt { .. } => "prompt",
            Self::Compact { .. } => "compact",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::ClearQueue => "clear_queue",
            Self::Abort => "abort",
            Self::AbortRetry => "abort_retry",
            Self::ExtensionUiResponse(_) => "extension_ui_response",
        }
    }

    /// Serializes this command into a single-line JSONL payload with the given correlation ID.
    pub fn to_jsonl(&self, request_id: &RequestId) -> String {
        if let Self::ExtensionUiResponse(resp) = self {
            return resp.to_jsonl();
        }

        let mut map = BTreeMap::new();
        map.insert("id".to_string(), JsonValue::String(request_id.0.clone()));
        map.insert(
            "type".to_string(),
            JsonValue::String(self.command_type().to_string()),
        );

        match self {
            Self::GetState
            | Self::GetMessages
            | Self::GetSessionStats
            | Self::GetAvailableModels
            | Self::GetAvailableThinkingLevels
            | Self::GetCommands
            | Self::GetTree
            | Self::GetForkMessages
            | Self::ClearQueue
            | Self::Abort
            | Self::AbortRetry => {}

            Self::SetModel { provider, model_id } => {
                map.insert("provider".to_string(), JsonValue::String(provider.clone()));
                map.insert("modelId".to_string(), JsonValue::String(model_id.clone()));
            }
            Self::SetThinkingLevel { level } => {
                map.insert("level".to_string(), JsonValue::String(level.clone()));
            }
            Self::NewSession { parent_session } => {
                if let Some(parent) = parent_session {
                    map.insert(
                        "parentSession".to_string(),
                        JsonValue::String(parent.clone()),
                    );
                }
            }
            Self::SwitchSession { session_path } => {
                map.insert(
                    "sessionPath".to_string(),
                    JsonValue::String(session_path.clone()),
                );
            }
            Self::SetSessionName { name } => {
                map.insert("name".to_string(), JsonValue::String(name.clone()));
            }
            Self::Fork { entry_id } => {
                map.insert("entryId".to_string(), JsonValue::String(entry_id.clone()));
            }
            Self::Clone { entry_id } => {
                if let Some(id) = entry_id {
                    map.insert("entryId".to_string(), JsonValue::String(id.clone()));
                }
            }
            Self::ExportHtml { output_path } => {
                if let Some(path) = output_path {
                    map.insert("outputPath".to_string(), JsonValue::String(path.clone()));
                }
            }
            Self::Prompt { message, images }
            | Self::Steer { message, images }
            | Self::FollowUp { message, images } => {
                map.insert("message".to_string(), JsonValue::String(message.clone()));
                if !images.is_empty() {
                    map.insert("images".to_string(), attachments_to_json(images));
                }
            }
            Self::Compact {
                custom_instructions,
            } => {
                if let Some(instr) = custom_instructions {
                    map.insert(
                        "customInstructions".to_string(),
                        JsonValue::String(instr.clone()),
                    );
                }
            }
            Self::ExtensionUiResponse(_) => unreachable!("handled above"),
        }

        to_json_string(&JsonValue::Object(map))
    }
}

fn attachments_to_json(images: &[PendingAttachment]) -> JsonValue {
    let items = images
        .iter()
        .map(|img| {
            let mut obj = BTreeMap::new();
            obj.insert("type".to_string(), JsonValue::String("image".to_string()));
            obj.insert(
                "mimeType".to_string(),
                JsonValue::String(img.mime_type.clone()),
            );
            obj.insert(
                "data".to_string(),
                JsonValue::String(img.base64_data.clone()),
            );
            obj.insert(
                "filename".to_string(),
                JsonValue::String(img.filename.clone()),
            );
            JsonValue::Object(obj)
        })
        .collect();
    JsonValue::Array(items)
}

/// All 10 Pi JSONL stdout events forwarded to the Tauri UI via `app.emit("pi:event", ...)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiUiEvent {
    AgentSettled {
        summary: Option<String>,
        raw_json: String,
    },
    MessageStart {
        message_id: Option<String>,
        role: Option<String>,
        raw_json: String,
    },
    MessageUpdate {
        message_id: Option<String>,
        delta: Option<String>,
        raw_json: String,
    },
    MessageEnd {
        message_id: Option<String>,
        stop_reason: Option<String>,
        raw_json: String,
    },
    ToolExecutionStart {
        tool_call_id: Option<String>,
        tool_name: Option<String>,
        raw_json: String,
    },
    ToolExecutionEnd {
        tool_call_id: Option<String>,
        tool_name: Option<String>,
        is_error: bool,
        raw_json: String,
    },
    QueueUpdate {
        pending_count: Option<u64>,
        raw_json: String,
    },
    AutoRetryStart {
        attempt: Option<u64>,
        max_attempts: Option<u64>,
        delay_ms: Option<u64>,
        raw_json: String,
    },
    AutoRetryEnd {
        attempt: Option<u64>,
        success: Option<bool>,
        raw_json: String,
    },
    ExtensionUiRequest {
        id: String,
        kind: DialogKind,
        title: Option<String>,
        options: Vec<String>,
        timeout_ms: Option<u64>,
        raw_json: String,
    },
}

/// Pure boundary parser converting a Pi stdout JSONL object into a typed `PiUiEvent`.
pub fn parse_pi_ui_event(val: &JsonValue, raw_json: &str) -> Option<PiUiEvent> {
    let event_type = val.get("type").and_then(JsonValue::as_str)?;
    let raw_json = raw_json.to_string();

    match event_type {
        "agent_settled" => Some(PiUiEvent::AgentSettled {
            summary: opt_str(val, &["summary"]),
            raw_json,
        }),
        "message_start" => Some(PiUiEvent::MessageStart {
            message_id: opt_str(val, &["messageId", "message_id", "id"]),
            role: opt_str(val, &["role"]),
            raw_json,
        }),
        "message_update" => Some(PiUiEvent::MessageUpdate {
            message_id: opt_str(val, &["messageId", "message_id", "id"]),
            delta: opt_str(val, &["delta", "text"]),
            raw_json,
        }),
        "message_end" => Some(PiUiEvent::MessageEnd {
            message_id: opt_str(val, &["messageId", "message_id", "id"]),
            stop_reason: opt_str(val, &["stopReason", "stop_reason"]),
            raw_json,
        }),
        "tool_execution_start" => Some(PiUiEvent::ToolExecutionStart {
            tool_call_id: opt_str(val, &["toolCallId", "tool_call_id", "id"]),
            tool_name: opt_str(val, &["toolName", "tool_name", "name"]),
            raw_json,
        }),
        "tool_execution_end" => Some(PiUiEvent::ToolExecutionEnd {
            tool_call_id: opt_str(val, &["toolCallId", "tool_call_id", "id"]),
            tool_name: opt_str(val, &["toolName", "tool_name", "name"]),
            is_error: val
                .get("isError")
                .or_else(|| val.get("is_error"))
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            raw_json,
        }),
        "queue_update" => Some(PiUiEvent::QueueUpdate {
            pending_count: opt_u64(val, &["pendingCount", "pending_count", "count"]),
            raw_json,
        }),
        "auto_retry_start" => Some(PiUiEvent::AutoRetryStart {
            attempt: opt_u64(val, &["attempt"]),
            max_attempts: opt_u64(val, &["maxAttempts", "max_attempts"]),
            delay_ms: opt_u64(val, &["delayMs", "delay_ms"]),
            raw_json,
        }),
        "auto_retry_end" => Some(PiUiEvent::AutoRetryEnd {
            attempt: opt_u64(val, &["attempt"]),
            success: val.get("success").and_then(JsonValue::as_bool),
            raw_json,
        }),
        "extension_ui_request" => {
            let parsed = parse_extension_ui_request(val, Instant::now()).ok()?;
            let timeout_ms = opt_u64(val, &["timeout_ms", "timeoutMs", "timeout"]);
            Some(PiUiEvent::ExtensionUiRequest {
                id: parsed.id.as_str().to_string(),
                kind: parsed.kind,
                title: parsed.title,
                options: parsed.options,
                timeout_ms,
                raw_json,
            })
        }
        _ => None,
    }
}

fn opt_str(val: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| val.get(k).and_then(JsonValue::as_str))
        .map(ToString::to_string)
}

fn opt_u64(val: &JsonValue, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| match val.get(k) {
        Some(JsonValue::Number(n)) if *n >= 0.0 && n.is_finite() => Some(*n as u64),
        _ => None,
    })
}

fn parse_rpc_response(val: &JsonValue) -> Option<PiRpcResponse> {
    if val.get("type").and_then(JsonValue::as_str) != Some("response") {
        return None;
    }
    let id = val
        .get("id")
        .and_then(JsonValue::as_str)
        .and_then(|s| RequestId::parse(s).ok())?;
    let command = opt_str(val, &["command"]);
    let success = val
        .get("success")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    let data_json = val.get("data").map(to_json_string);
    let error = opt_str(val, &["error"]);
    Some(PiRpcResponse {
        id,
        command,
        success,
        data_json,
        error,
    })
}

/// Event callback sink so `main.rs` can wire `app.emit("pi:event", event)` without
/// coupling `pi_session` to Tauri runtime types.
pub type EventSink = Arc<dyn Fn(PiUiEvent) + Send + Sync>;

/// Callback invoked when a provider reload executes (either immediately while idle
/// or deferred upon `agent_settled`).
pub type ReloadHook = Arc<dyn Fn() + Send + Sync>;

type PendingMap = HashMap<RequestId, mpsc::Sender<Result<PiRpcResponse, PiSessionError>>>;

fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        let pid_str = pid.to_string();
        let _ = Command::new("pkill").args(["-9", "-P", &pid_str]).status();
        let _ = Command::new("kill").args(["-9", &pid_str]).status();
    }
}

/// Long-lived supervisor managing a `pi --mode rpc --no-session` child process.
pub struct PiSessionSupervisor {
    config: PiSessionConfig,
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    reader_handle: Mutex<Option<JoinHandle<()>>>,
    activity: Arc<Mutex<SessionActivity>>,
    pending: Arc<Mutex<PendingMap>>,
    extension_ui: Arc<Mutex<ExtensionUiTracker>>,
    reload_hook: ReloadHook,
    next_id: AtomicU64,
    shutdown_flag: Arc<AtomicBool>,
}

impl PiSessionSupervisor {
    /// Spawns the configured Pi JSONL RPC child process and starts its background stdout reader.
    pub fn spawn(
        config: PiSessionConfig,
        event_sink: EventSink,
        reload_hook: ReloadHook,
    ) -> Result<Self, PiSessionError> {
        let mut child = Command::new(&config.program)
            .args(&config.args)
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| PiSessionError::SpawnFailed(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PiSessionError::Io("missing child stdin pipe".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PiSessionError::Io("missing child stdout pipe".to_string()))?;

        let activity = Arc::new(Mutex::new(SessionActivity::Idle));
        let pending: Arc<Mutex<PendingMap>> = Arc::new(Mutex::new(HashMap::new()));
        let extension_ui = Arc::new(Mutex::new(ExtensionUiTracker::new()));
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        let r_activity = Arc::clone(&activity);
        let r_pending = Arc::clone(&pending);
        let r_ext_ui = Arc::clone(&extension_ui);
        let r_reload = Arc::clone(&reload_hook);
        let r_sink = Arc::clone(&event_sink);

        let reader_handle = std::thread::Builder::new()
            .name("pi-rpc-stdout".to_string())
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line_res in reader.lines() {
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

                    if let Some(resp) = parse_rpc_response(&val) {
                        let waiter = {
                            let mut map = r_pending.lock().expect("pending lock poisoned");
                            map.remove(&resp.id)
                        };
                        if let Some(tx) = waiter {
                            let _ = tx.send(Ok(resp));
                        }
                        continue;
                    }

                    if let Some(event) = parse_pi_ui_event(&val, trimmed) {
                        let mut trigger_reload = false;
                        match &event {
                            PiUiEvent::AgentSettled { .. } => {
                                let mut act = r_activity.lock().expect("activity lock poisoned");
                                if let Ok(tr) =
                                    transition_session(*act, SessionCommand::AgentSettled)
                                {
                                    *act = tr.activity;
                                    trigger_reload = tr.run_provider_reload;
                                }
                            }
                            PiUiEvent::ExtensionUiRequest { .. } => {
                                if let Ok(dlg) = parse_extension_ui_request(&val, Instant::now()) {
                                    let mut tracker =
                                        r_ext_ui.lock().expect("extension_ui lock poisoned");
                                    let _ = tracker.register(dlg);
                                }
                            }
                            _ => {}
                        }

                        r_sink(event);
                        if trigger_reload {
                            r_reload();
                        }
                    }
                }

                // Fail any still-waiting callers so nobody hangs when the child exits.
                let mut map = r_pending.lock().expect("pending lock poisoned");
                for (_, tx) in map.drain() {
                    let _ = tx.send(Err(PiSessionError::ProcessExited));
                }
            })
            .map_err(|e| {
                kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                PiSessionError::SpawnFailed(e.to_string())
            })?;

        Ok(Self {
            config,
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
            reader_handle: Mutex::new(Some(reader_handle)),
            activity,
            pending,
            extension_ui,
            reload_hook,
            next_id: AtomicU64::new(1),
            shutdown_flag,
        })
    }

    /// Returns the current `SessionActivity` snapshot.
    pub fn activity(&self) -> SessionActivity {
        *self.activity.lock().expect("activity lock poisoned")
    }

    /// Returns the number of currently open Extension UI dialogs.
    pub fn open_dialog_count(&self) -> usize {
        self.extension_ui
            .lock()
            .expect("extension_ui lock poisoned")
            .open_count()
    }

    /// Requests a provider reload: runs immediately if `Idle`, defers until `agent_settled`
    /// if `Streaming`, or returns `SessionError::Busy` while compacting/switching.
    pub fn request_provider_reload(&self) -> Result<ReloadDisposition, PiSessionError> {
        let run_now = {
            let mut act = self.activity.lock().expect("activity lock poisoned");
            let tr = transition_session(*act, SessionCommand::RequestProviderReload)?;
            *act = tr.activity;
            tr.run_provider_reload
        };

        if run_now {
            (self.reload_hook)();
            Ok(ReloadDisposition::ExecutedImmediately)
        } else {
            Ok(ReloadDisposition::DeferredUntilSettled)
        }
    }

    /// Purges expired Extension UI dialogs and sends `cancelled` responses to Pi.
    pub fn purge_expired_dialogs(&self) -> Result<Vec<ExtensionUiResponse>, PiSessionError> {
        let expired = {
            let mut tracker = self
                .extension_ui
                .lock()
                .expect("extension_ui lock poisoned");
            tracker.purge_expired()
        };
        for resp in &expired {
            self.write_jsonl(&resp.to_jsonl())?;
        }
        Ok(expired)
    }

    /// Sends any of the 24 typed Pi RPC commands, enforcing `verity_core::session::transition_session`
    /// and awaiting the correlated RPC response (except `extension_ui_response`, which is fire-and-forget).
    pub fn send_command(&self, cmd: PiRpcCommand) -> Result<Option<PiRpcResponse>, PiSessionError> {
        if self.shutdown_flag.load(Ordering::Acquire) {
            return Err(PiSessionError::AlreadyShutdown);
        }

        if let PiRpcCommand::ExtensionUiResponse(ref resp) = cmd {
            self.write_jsonl(&resp.to_jsonl())?;
            return Ok(None);
        }

        // Apply pre-send state machine transition where applicable.
        let pre_transition = match &cmd {
            PiRpcCommand::Prompt { .. } => Some(SessionCommand::SendPrompt),
            PiRpcCommand::Steer { .. } => Some(SessionCommand::Steer),
            PiRpcCommand::FollowUp { .. } => Some(SessionCommand::FollowUp),
            PiRpcCommand::ClearQueue => Some(SessionCommand::ClearQueue),
            PiRpcCommand::Compact { .. } => Some(SessionCommand::Compact),
            PiRpcCommand::SwitchSession { .. } => Some(SessionCommand::SwitchSession),
            _ => None,
        };

        let previous_activity = if let Some(scmd) = pre_transition {
            let mut act = self.activity.lock().expect("activity lock poisoned");
            let prev = *act;
            let tr = transition_session(prev, scmd)?;
            *act = tr.activity;
            Some(prev)
        } else {
            None
        };

        let request_id = self.allocate_request_id();
        let line = cmd.to_jsonl(&request_id);
        let (tx, rx) = mpsc::channel();

        {
            let mut map = self.pending.lock().expect("pending lock poisoned");
            map.insert(request_id.clone(), tx);
        }

        if let Err(err) = self.write_jsonl(&line) {
            self.pending
                .lock()
                .expect("pending lock poisoned")
                .remove(&request_id);
            if let Some(prev) = previous_activity {
                *self.activity.lock().expect("activity lock poisoned") = prev;
            }
            return Err(err);
        }

        let wait_result = rx.recv_timeout(self.config.command_timeout);

        // Complete synchronous exclusive transitions (Compacting / SwitchingSession) regardless of outcome.
        match &cmd {
            PiRpcCommand::Compact { .. } => {
                let mut act = self.activity.lock().expect("activity lock poisoned");
                if let Ok(tr) = transition_session(*act, SessionCommand::FinishCompacting) {
                    *act = tr.activity;
                }
            }
            PiRpcCommand::SwitchSession { .. } => {
                let mut act = self.activity.lock().expect("activity lock poisoned");
                if let Ok(tr) = transition_session(*act, SessionCommand::FinishSwitch) {
                    *act = tr.activity;
                }
            }
            _ => {}
        }

        match wait_result {
            Ok(Ok(resp)) => {
                if !resp.success {
                    if let (Some(prev), PiRpcCommand::Prompt { .. }) = (previous_activity, &cmd) {
                        *self.activity.lock().expect("activity lock poisoned") = prev;
                    }
                    return Err(PiSessionError::RpcError {
                        command: cmd.command_type().to_string(),
                        message: resp
                            .error
                            .unwrap_or_else(|| "unknown pi rpc error".to_string()),
                    });
                }
                Ok(Some(resp))
            }
            Ok(Err(e)) => Err(e),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending
                    .lock()
                    .expect("pending lock poisoned")
                    .remove(&request_id);
                Err(PiSessionError::Timeout {
                    request_id,
                    command: cmd.command_type().to_string(),
                })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(PiSessionError::ProcessExited),
        }
    }

    // --- Typed convenience methods for all 24 Pi RPC stdin commands ---

    pub fn get_state(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetState)
    }

    pub fn get_messages(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetMessages)
    }

    pub fn get_session_stats(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetSessionStats)
    }

    pub fn get_available_models(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetAvailableModels)
    }

    pub fn set_model(
        &self,
        provider: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::SetModel {
            provider: provider.into(),
            model_id: model_id.into(),
        })
    }

    pub fn get_available_thinking_levels(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetAvailableThinkingLevels)
    }

    pub fn set_thinking_level(
        &self,
        level: impl Into<String>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::SetThinkingLevel {
            level: level.into(),
        })
    }

    pub fn get_commands(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetCommands)
    }

    pub fn new_session(
        &self,
        parent_session: Option<String>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::NewSession { parent_session })
    }

    pub fn switch_session(
        &self,
        session_path: impl Into<String>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::SwitchSession {
            session_path: session_path.into(),
        })
    }

    pub fn set_session_name(
        &self,
        name: impl Into<String>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::SetSessionName { name: name.into() })
    }

    pub fn get_tree(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetTree)
    }

    pub fn get_fork_messages(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetForkMessages)
    }

    pub fn fork(&self, entry_id: impl Into<String>) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::Fork {
            entry_id: entry_id.into(),
        })
    }

    pub fn clone_session(&self, entry_id: Option<String>) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::Clone { entry_id })
    }

    pub fn export_html(
        &self,
        output_path: Option<String>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::ExportHtml { output_path })
    }

    pub fn prompt(
        &self,
        message: impl Into<String>,
        images: Vec<PendingAttachment>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::Prompt {
            message: message.into(),
            images,
        })
    }

    pub fn compact(
        &self,
        custom_instructions: Option<String>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::Compact {
            custom_instructions,
        })
    }

    pub fn steer(
        &self,
        message: impl Into<String>,
        images: Vec<PendingAttachment>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::Steer {
            message: message.into(),
            images,
        })
    }

    pub fn follow_up(
        &self,
        message: impl Into<String>,
        images: Vec<PendingAttachment>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::FollowUp {
            message: message.into(),
            images,
        })
    }

    pub fn clear_queue(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::ClearQueue)
    }

    pub fn abort(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::Abort)
    }

    pub fn abort_retry(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::AbortRetry)
    }

    /// Validates an answer against the tracked Extension UI dialog and writes
    /// `extension_ui_response` to Pi stdin.
    pub fn extension_ui_response(
        &self,
        dialog_id: &DialogId,
        answer: ExtensionUiAnswer,
    ) -> Result<ExtensionUiResponse, PiSessionError> {
        let resp = {
            let mut tracker = self
                .extension_ui
                .lock()
                .expect("extension_ui lock poisoned");
            tracker.resolve(dialog_id, answer)?
        };
        self.send_command(PiRpcCommand::ExtensionUiResponse(resp.clone()))?;
        Ok(resp)
    }

    /// Idempotently shuts down the Pi child process tree, closes stdin, drains waiters,
    /// and joins the background reader thread.
    pub fn shutdown(&self) {
        self.shutdown_flag.store(true, Ordering::Release);

        // Close stdin first to signal clean EOF.
        {
            let mut stdin_guard = self.stdin.lock().expect("stdin lock poisoned");
            stdin_guard.take();
        }

        // Terminate process tree and reap child if still present.
        {
            let mut child_guard = self.child.lock().expect("child lock poisoned");
            if let Some(mut child) = child_guard.take() {
                kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        // Drain any remaining waiters.
        {
            let mut map = self.pending.lock().expect("pending lock poisoned");
            for (_, tx) in map.drain() {
                let _ = tx.send(Err(PiSessionError::AlreadyShutdown));
            }
        }

        // Join stdout reader thread.
        {
            let mut handle_guard = self
                .reader_handle
                .lock()
                .expect("reader_handle lock poisoned");
            if let Some(handle) = handle_guard.take() {
                let _ = handle.join();
            }
        }
    }

    fn send_expect_response(&self, cmd: PiRpcCommand) -> Result<PiRpcResponse, PiSessionError> {
        self.send_command(cmd)?
            .ok_or_else(|| PiSessionError::Protocol("expected correlated rpc response".to_string()))
    }

    fn allocate_request_id(&self) -> RequestId {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        RequestId(format!("req-{}", n))
    }

    fn write_jsonl(&self, line: &str) -> Result<(), PiSessionError> {
        let mut stdin_guard = self.stdin.lock().expect("stdin lock poisoned");
        let stdin = stdin_guard
            .as_mut()
            .ok_or(PiSessionError::AlreadyShutdown)?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .and_then(|_| stdin.flush())
            .map_err(|e| PiSessionError::Io(e.to_string()))
    }
}

impl Drop for PiSessionSupervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{validate_image_bytes, ImageMimeType};
    use std::sync::atomic::AtomicUsize;

    fn mock_sh_config(script: &str) -> PiSessionConfig {
        PiSessionConfig {
            program: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            cwd: std::env::temp_dir(),
            command_timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn serializes_all_24_commands_with_exact_wire_types() {
        let img = validate_image_bytes("a.png", ImageMimeType::Png, b"abc").unwrap();
        let ext_resp = ExtensionUiResponse {
            id: DialogId::parse("dlg-1").unwrap(),
            answer: ExtensionUiAnswer::Confirmed(true),
        };

        let all_24: Vec<(PiRpcCommand, &str)> = vec![
            (PiRpcCommand::GetState, "get_state"),
            (PiRpcCommand::GetMessages, "get_messages"),
            (PiRpcCommand::GetSessionStats, "get_session_stats"),
            (PiRpcCommand::GetAvailableModels, "get_available_models"),
            (
                PiRpcCommand::SetModel {
                    provider: "anthropic".to_string(),
                    model_id: "claude-sonnet-4".to_string(),
                },
                "set_model",
            ),
            (
                PiRpcCommand::GetAvailableThinkingLevels,
                "get_available_thinking_levels",
            ),
            (
                PiRpcCommand::SetThinkingLevel {
                    level: "high".to_string(),
                },
                "set_thinking_level",
            ),
            (PiRpcCommand::GetCommands, "get_commands"),
            (
                PiRpcCommand::NewSession {
                    parent_session: None,
                },
                "new_session",
            ),
            (
                PiRpcCommand::SwitchSession {
                    session_path: "/tmp/s.jsonl".to_string(),
                },
                "switch_session",
            ),
            (
                PiRpcCommand::SetSessionName {
                    name: "Sprint".to_string(),
                },
                "set_session_name",
            ),
            (PiRpcCommand::GetTree, "get_tree"),
            (PiRpcCommand::GetForkMessages, "get_fork_messages"),
            (
                PiRpcCommand::Fork {
                    entry_id: "e1".to_string(),
                },
                "fork",
            ),
            (
                PiRpcCommand::Clone {
                    entry_id: Some("e1".to_string()),
                },
                "clone",
            ),
            (
                PiRpcCommand::ExportHtml {
                    output_path: Some("/tmp/out.html".to_string()),
                },
                "export_html",
            ),
            (
                PiRpcCommand::Prompt {
                    message: "hi".to_string(),
                    images: vec![img.clone()],
                },
                "prompt",
            ),
            (
                PiRpcCommand::Compact {
                    custom_instructions: Some("brief".to_string()),
                },
                "compact",
            ),
            (
                PiRpcCommand::Steer {
                    message: "pivot".to_string(),
                    images: vec![],
                },
                "steer",
            ),
            (
                PiRpcCommand::FollowUp {
                    message: "next".to_string(),
                    images: vec![],
                },
                "follow_up",
            ),
            (PiRpcCommand::ClearQueue, "clear_queue"),
            (PiRpcCommand::Abort, "abort"),
            (PiRpcCommand::AbortRetry, "abort_retry"),
            (
                PiRpcCommand::ExtensionUiResponse(ext_resp),
                "extension_ui_response",
            ),
        ];

        assert_eq!(all_24.len(), 24);
        let req_id = RequestId::parse("req-42").unwrap();
        for (cmd, expected_type) in all_24 {
            assert_eq!(cmd.command_type(), expected_type);
            let json_str = cmd.to_jsonl(&req_id);
            let parsed = parse_json(&json_str).expect("valid jsonl");
            assert_eq!(
                parsed.get("type").and_then(JsonValue::as_str),
                Some(expected_type)
            );
        }
    }

    #[test]
    fn parses_all_10_ui_events() {
        let samples = [
            r#"{"type":"agent_settled","summary":"done"}"#,
            r#"{"type":"message_start","messageId":"m1","role":"assistant"}"#,
            r#"{"type":"message_update","messageId":"m1","delta":"hel"}"#,
            r#"{"type":"message_end","messageId":"m1","stopReason":"end_turn"}"#,
            r#"{"type":"tool_execution_start","toolCallId":"t1","toolName":"read"}"#,
            r#"{"type":"tool_execution_end","toolCallId":"t1","toolName":"read","isError":false}"#,
            r#"{"type":"queue_update","pendingCount":2}"#,
            r#"{"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":500}"#,
            r#"{"type":"auto_retry_end","attempt":1,"success":true}"#,
            r#"{"type":"extension_ui_request","id":"ui-1","method":"confirm","title":"Sure?"}"#,
        ];

        assert_eq!(samples.len(), 10);
        for raw in samples {
            let val = parse_json(raw).unwrap();
            let ev = parse_pi_ui_event(&val, raw).expect("must parse known ui event");
            let _ = ev;
        }
    }

    #[test]
    fn mock_sh_prompt_streaming_steer_followup_deferred_reload_and_clean_shutdown() {
        // Mock Pi JSONL RPC subprocess implemented in POSIX /bin/sh:
        // - Responds to get_state, prompt, steer, follow_up, clear_queue, abort.
        // - On prompt: emits response, message_start, message_update, tool_execution_start/end,
        //   and waits for a follow_up command before emitting message_end + agent_settled.
        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_state)
      printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"model":"mock"}}\n' "$id"
      ;;
    prompt)
      printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
      printf '{"type":"message_start","messageId":"m1","role":"assistant"}\n'
      printf '{"type":"message_update","messageId":"m1","delta":"hello "}\n'
      printf '{"type":"tool_execution_start","toolCallId":"tc1","toolName":"read"}\n'
      printf '{"type":"tool_execution_end","toolCallId":"tc1","toolName":"read","isError":false}\n'
      ;;
    steer)
      printf '{"type":"response","id":"%s","command":"steer","success":true}\n' "$id"
      printf '{"type":"queue_update","pendingCount":1}\n'
      ;;
    follow_up)
      printf '{"type":"response","id":"%s","command":"follow_up","success":true}\n' "$id"
      printf '{"type":"queue_update","pendingCount":2}\n'
      printf '{"type":"message_update","messageId":"m1","delta":"world"}\n'
      printf '{"type":"message_end","messageId":"m1","stopReason":"end_turn"}\n'
      printf '{"type":"agent_settled","summary":"hello world"}\n'
      ;;
    clear_queue)
      printf '{"type":"response","id":"%s","command":"clear_queue","success":true}\n' "$id"
      ;;
    *)
      printf '{"type":"response","id":"%s","command":"%s","success":true}\n' "$id" "$typ"
      ;;
  esac
done
"#;

        let events: Arc<Mutex<Vec<PiUiEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let (settled_tx, settled_rx) = mpsc::channel();
        let events_clone = Arc::clone(&events);
        let sink: EventSink = Arc::new(move |ev| {
            let is_settled = matches!(ev, PiUiEvent::AgentSettled { .. });
            events_clone.lock().unwrap().push(ev);
            if is_settled {
                let _ = settled_tx.send(());
            }
        });

        let reloads = Arc::new(AtomicUsize::new(0));
        let reloads_clone = Arc::clone(&reloads);
        let reload_hook: ReloadHook = Arc::new(move || {
            reloads_clone.fetch_add(1, Ordering::SeqCst);
        });

        let sup = PiSessionSupervisor::spawn(mock_sh_config(script), sink, reload_hook)
            .expect("spawn mock pi");

        // Idle reload executes immediately.
        assert_eq!(
            sup.request_provider_reload().unwrap(),
            ReloadDisposition::ExecutedImmediately
        );
        assert_eq!(reloads.load(Ordering::SeqCst), 1);

        // Steer while Idle is rejected by verity_core::session::transition_session.
        assert_eq!(
            sup.steer("too early", vec![]),
            Err(PiSessionError::Session(SessionError::NotStreaming))
        );

        // Correlated get_state command succeeds.
        let state = sup.get_state().expect("get_state");
        assert!(state.success);
        assert_eq!(state.command.as_deref(), Some("get_state"));

        // Start prompt -> transitions to Streaming.
        let prompt_resp = sup.prompt("say hi", vec![]).expect("prompt");
        assert!(prompt_resp.success);
        assert!(sup.activity().is_streaming());

        // Single-flight exclusivity: second prompt while Streaming is rejected.
        assert_eq!(
            sup.prompt("overlap", vec![]),
            Err(PiSessionError::Session(SessionError::NotIdle))
        );

        // Request provider reload while Streaming -> deferred (does not run yet).
        assert_eq!(
            sup.request_provider_reload().unwrap(),
            ReloadDisposition::DeferredUntilSettled
        );
        assert_eq!(reloads.load(Ordering::SeqCst), 1);
        assert!(sup.activity().reload_deferred());

        // Steer while Streaming increments pending queue.
        let steer_resp = sup.steer("focus", vec![]).expect("steer");
        assert!(steer_resp.success);
        assert_eq!(sup.activity().pending_queue(), 1);

        // FollowUp while Streaming triggers agent_settled from mock script.
        let fu_resp = sup.follow_up("finish", vec![]).expect("follow_up");
        assert!(fu_resp.success);

        settled_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("should receive agent_settled");

        // Wait briefly for reload_hook right after sink(agent_settled).
        let deadline = Instant::now() + Duration::from_secs(2);
        while reloads.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(reloads.load(Ordering::SeqCst), 2);
        assert_eq!(sup.activity(), SessionActivity::Idle);

        let recorded = events.lock().unwrap();
        assert!(recorded
            .iter()
            .any(|e| matches!(e, PiUiEvent::MessageStart { .. })));
        assert!(recorded.iter().any(
            |e| matches!(e, PiUiEvent::MessageUpdate { delta: Some(d), .. } if d == "hello ")
        ));
        assert!(recorded
            .iter()
            .any(|e| matches!(e, PiUiEvent::ToolExecutionStart { .. })));
        assert!(recorded
            .iter()
            .any(|e| matches!(e, PiUiEvent::ToolExecutionEnd { .. })));
        assert!(recorded
            .iter()
            .any(|e| matches!(e, PiUiEvent::QueueUpdate { .. })));
        assert!(recorded
            .iter()
            .any(|e| matches!(e, PiUiEvent::MessageEnd { .. })));
        assert!(recorded
            .iter()
            .any(|e| matches!(e, PiUiEvent::AgentSettled { .. })));
        drop(recorded);

        // Idempotent shutdown + Drop.
        sup.shutdown();
        sup.shutdown();
        assert_eq!(sup.get_state(), Err(PiSessionError::AlreadyShutdown));
    }
}
