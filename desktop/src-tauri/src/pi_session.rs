use crate::attachments::PendingAttachment;
use crate::extension_ui::{
    parse_extension_ui_effect, parse_extension_ui_request, DialogId, DialogKind, ExtensionUiAnswer,
    ExtensionUiError, ExtensionUiResponse, ExtensionUiTracker,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use verity_core::json::{parse_json, to_json_string, JsonValue};
use verity_core::session::{transition_session, SessionActivity, SessionCommand, SessionError};

const MAX_STDERR_LINES: usize = 32;
const MAX_STDERR_LINE_BYTES: usize = 512;
const ABORT_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

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
    pub env: Vec<(String, String)>,
    pub command_timeout: Duration,
    pub no_session: bool,
}

/// Resolves the Pi CLI executable name across platforms.
/// On Windows, npm global shims are `.cmd` files, so the default `"pi"` program
/// resolves to `pi.exe` / `pi.cmd` on `PATH` or `%APPDATA%\npm\pi.cmd` before
/// falling back to `"pi.cmd"`.
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

impl PiSessionConfig {
    /// Standard long-lived Pi RPC configuration (`pi --mode rpc`), persisting sessions per project `cwd`.
    pub fn default_pi(cwd: PathBuf) -> Self {
        Self {
            program: resolve_pi_program("pi"),
            args: vec!["--mode".to_string(), "rpc".to_string()],
            cwd,
            env: Vec::new(),
            command_timeout: Duration::from_secs(30),
            no_session: false,
        }
    }

    /// Enables or disables ephemeral `--no-session` mode (useful for isolated unit tests).
    pub fn with_no_session(mut self, no_session: bool) -> Self {
        self.no_session = no_session;
        self
    }

    /// Appends `--extension <path>` to the Pi CLI arguments.
    pub fn with_extension(mut self, extension_path: &Path) -> Self {
        self.args.push("--extension".to_string());
        self.args.push(extension_path.to_string_lossy().to_string());
        self
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
        stderr_tail: Option<String>,
    },
    ProcessExited {
        stderr_tail: Option<String>,
    },
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
                stderr_tail,
            } => match stderr_tail {
                Some(tail) => write!(
                    f,
                    "pi rpc '{}' (id={}) timed out waiting for response; stderr: {}",
                    command, request_id, tail
                ),
                None => write!(
                    f,
                    "pi rpc '{}' (id={}) timed out waiting for response",
                    command, request_id
                ),
            },
            Self::ProcessExited { stderr_tail } => match stderr_tail {
                Some(tail) => write!(f, "pi rpc process exited unexpectedly; stderr: {}", tail),
                None => write!(f, "pi rpc process exited unexpectedly"),
            },
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

/// All 34 typed Pi JSONL RPC stdin commands.
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
    Bash {
        command: String,
    },
    AbortBash,
    SetAutoCompaction {
        enabled: bool,
    },
    SetAutoRetry {
        enabled: bool,
    },
    SetSteeringMode {
        mode: String,
    },
    SetFollowUpMode {
        mode: String,
    },
    CycleModel,
    CycleThinkingLevel,
    GetLastAssistantText,
    GetEntries,
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
            Self::Bash { .. } => "bash",
            Self::AbortBash => "abort_bash",
            Self::SetAutoCompaction { .. } => "set_auto_compaction",
            Self::SetAutoRetry { .. } => "set_auto_retry",
            Self::SetSteeringMode { .. } => "set_steering_mode",
            Self::SetFollowUpMode { .. } => "set_follow_up_mode",
            Self::CycleModel => "cycle_model",
            Self::CycleThinkingLevel => "cycle_thinking_level",
            Self::GetLastAssistantText => "get_last_assistant_text",
            Self::GetEntries => "get_entries",
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
            | Self::AbortRetry
            | Self::AbortBash
            | Self::CycleModel
            | Self::CycleThinkingLevel
            | Self::GetLastAssistantText
            | Self::GetEntries => {}

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
            Self::Bash { command } => {
                map.insert("command".to_string(), JsonValue::String(command.clone()));
            }
            Self::SetAutoCompaction { enabled } | Self::SetAutoRetry { enabled } => {
                map.insert("enabled".to_string(), JsonValue::Bool(*enabled));
            }
            Self::SetSteeringMode { mode } | Self::SetFollowUpMode { mode } => {
                map.insert("mode".to_string(), JsonValue::String(mode.clone()));
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

/// Pi JSONL stdout events forwarded to the Tauri UI via `app.emit("pi:event", ...)`.
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
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking_delta: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking_phase: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking_content: Option<String>,
        raw_json: String,
    },
    MessageEnd {
        message_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        stop_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking: Option<String>,
        raw_json: String,
    },
    ToolExecutionStart {
        tool_call_id: Option<String>,
        tool_name: Option<String>,
        raw_json: String,
    },
    ToolExecutionUpdate {
        tool_call_id: Option<String>,
        tool_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        partial_result: Option<String>,
        raw_json: String,
    },
    ToolExecutionEnd {
        tool_call_id: Option<String>,
        tool_name: Option<String>,
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        raw_json: String,
    },
    CompactionStart {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        raw_json: String,
    },
    CompactionEnd {
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        aborted: bool,
        will_retry: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        raw_json: String,
    },
    BashExecutionUpdate {
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        is_error: bool,
        raw_json: String,
    },
    SessionInfoChanged {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        raw_json: String,
    },
    ThinkingLevelChanged {
        #[serde(skip_serializing_if = "Option::is_none")]
        level: Option<String>,
        raw_json: String,
    },
    TurnStart {
        raw_json: String,
    },
    TurnEnd {
        raw_json: String,
    },
    AgentStart {
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
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        raw_json: String,
    },
    AutoRetryEnd {
        attempt: Option<u64>,
        success: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        final_error: Option<String>,
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
    #[serde(rename = "extension_ui_effect")]
    ExtensionUiEffect {
        method: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        notify_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status_key: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        widget_key: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        widget_lines: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        raw_json: String,
    },
    Error {
        message: String,
        raw_json: String,
    },
    ProcessExited {
        exit_code: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stderr_tail: Option<String>,
    },
}

/// Pure boundary parser converting a Pi stdout JSONL object into a typed `PiUiEvent`.
pub fn parse_pi_ui_event(val: &JsonValue, raw_json: &str) -> Option<PiUiEvent> {
    let event_type = val.get("type").and_then(JsonValue::as_str)?;
    let raw_json = raw_json.to_string();

    match event_type {
        "agent_settled" | "agent_end" => Some(PiUiEvent::AgentSettled {
            summary: opt_str(val, &["summary"]),
            raw_json,
        }),
        "message_start" => Some(PiUiEvent::MessageStart {
            message_id: opt_str_or_nested(val, "message", &["messageId", "message_id", "id"]),
            role: opt_str_or_nested(val, "message", &["role"]),
            raw_json,
        }),
        "message_update" | "content_block_delta" => {
            let ame = val
                .get("assistantMessageEvent")
                .or_else(|| val.get("assistant_message_event"));
            let ame_type = ame.and_then(|o| o.get("type")).and_then(JsonValue::as_str);

            if matches!(
                ame_type,
                Some(
                    "toolcall_start"
                        | "toolcall_delta"
                        | "toolcall_end"
                        | "text_start"
                        | "start"
                        | "done"
                )
            ) {
                return None;
            }

            let (thinking_phase, thinking_content) = match ame_type {
                Some("thinking_start") => (Some("start".to_string()), None),
                Some("thinking_end") => (
                    Some("end".to_string()),
                    ame.and_then(|o| {
                        opt_non_empty_str(
                            o,
                            &[
                                "content",
                                "thinking",
                                "reasoning",
                                "reasoning_content",
                                "text",
                            ],
                        )
                    }),
                ),
                _ => (None, None),
            };

            let (delta, thinking_delta) = parse_message_update_deltas(val);
            Some(PiUiEvent::MessageUpdate {
                message_id: opt_str_or_nested(val, "message", &["messageId", "message_id", "id"]),
                delta,
                thinking_delta,
                thinking_phase,
                thinking_content,
                raw_json,
            })
        }
        "message_end" => {
            let (text, thinking) = extract_message_end_content(val);
            Some(PiUiEvent::MessageEnd {
                message_id: opt_str_or_nested(val, "message", &["messageId", "message_id", "id"]),
                role: opt_str_or_nested(val, "message", &["role"]),
                stop_reason: opt_str_or_nested(val, "message", &["stopReason", "stop_reason"]),
                error_message: opt_str_or_nested(
                    val,
                    "message",
                    &["errorMessage", "error_message", "error"],
                ),
                text,
                thinking,
                raw_json,
            })
        }
        "tool_execution_start" => Some(PiUiEvent::ToolExecutionStart {
            tool_call_id: opt_str(val, &["toolCallId", "tool_call_id", "id"]),
            tool_name: opt_str(val, &["toolName", "tool_name", "name"]),
            raw_json,
        }),
        "tool_execution_update" => Some(PiUiEvent::ToolExecutionUpdate {
            tool_call_id: opt_str(val, &["toolCallId", "tool_call_id", "id"]),
            tool_name: opt_str(val, &["toolName", "tool_name", "name"]),
            partial_result: opt_tool_output_text(
                val,
                &["partialResult", "partial_result", "output", "delta"],
            ),
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
            result: opt_tool_output_text(
                val,
                &[
                    "result",
                    "output",
                    "content",
                    "partialResult",
                    "partial_result",
                ],
            ),
            raw_json,
        }),
        "compaction_start" => Some(PiUiEvent::CompactionStart {
            reason: opt_str(val, &["reason"]),
            raw_json,
        }),
        "compaction_end" => Some(PiUiEvent::CompactionEnd {
            result: opt_str_or_json(val, &["result", "summary"]),
            aborted: val
                .get("aborted")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            will_retry: val
                .get("willRetry")
                .or_else(|| val.get("will_retry"))
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            error_message: opt_str(val, &["errorMessage", "error_message", "error"]),
            raw_json,
        }),
        "bash_execution_update" => Some(PiUiEvent::BashExecutionUpdate {
            command: opt_str(val, &["command"]),
            output: opt_str(val, &["output", "chunk", "delta", "text"]),
            is_error: val
                .get("isError")
                .or_else(|| val.get("is_error"))
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            raw_json,
        }),
        "session_info_changed" => Some(PiUiEvent::SessionInfoChanged {
            name: opt_str(val, &["name", "sessionName", "session_name"]),
            session_id: opt_str(val, &["sessionId", "session_id", "id"]),
            raw_json,
        }),
        "thinking_level_changed" => Some(PiUiEvent::ThinkingLevelChanged {
            level: opt_str(val, &["level", "thinkingLevel", "thinking_level"]),
            raw_json,
        }),
        "turn_start" => Some(PiUiEvent::TurnStart { raw_json }),
        "turn_end" => Some(PiUiEvent::TurnEnd { raw_json }),
        "agent_start" => Some(PiUiEvent::AgentStart { raw_json }),
        "queue_update" => Some(PiUiEvent::QueueUpdate {
            pending_count: opt_u64(val, &["pendingCount", "pending_count", "count"]),
            raw_json,
        }),
        "auto_retry_start" => Some(PiUiEvent::AutoRetryStart {
            attempt: opt_u64(val, &["attempt"]),
            max_attempts: opt_u64(val, &["maxAttempts", "max_attempts"]),
            delay_ms: opt_u64(val, &["delayMs", "delay_ms"]),
            error_message: opt_str(val, &["errorMessage", "error_message", "error", "reason"]),
            raw_json,
        }),
        "auto_retry_end" => Some(PiUiEvent::AutoRetryEnd {
            attempt: opt_u64(val, &["attempt"]),
            success: val.get("success").and_then(JsonValue::as_bool),
            final_error: opt_str(
                val,
                &[
                    "finalError",
                    "final_error",
                    "errorMessage",
                    "error_message",
                    "error",
                ],
            ),
            raw_json,
        }),
        "extension_ui_request" => {
            if let Ok(parsed) = parse_extension_ui_request(val, Instant::now()) {
                let timeout_ms = opt_u64(val, &["timeout_ms", "timeoutMs", "timeout"]);
                return Some(PiUiEvent::ExtensionUiRequest {
                    id: parsed.id.as_str().to_string(),
                    kind: parsed.kind,
                    title: parsed.title,
                    options: parsed.options,
                    timeout_ms,
                    raw_json,
                });
            }
            let effect = parse_extension_ui_effect(val)?;
            Some(PiUiEvent::ExtensionUiEffect {
                method: effect.method,
                message: effect.message,
                notify_type: effect.notify_type,
                status_key: effect.status_key,
                status_text: effect.status_text,
                widget_key: effect.widget_key,
                widget_lines: effect.widget_lines,
                title: effect.title,
                text: effect.text,
                raw_json,
            })
        }
        "error" | "extension_error" => {
            let message = opt_str(
                val,
                &[
                    "error",
                    "message",
                    "errorMessage",
                    "error_message",
                    "reason",
                ],
            )
            .unwrap_or_else(|| "Pi reported an error".to_string());
            Some(PiUiEvent::Error { message, raw_json })
        }
        _ => None,
    }
}

fn opt_str(val: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| val.get(k).and_then(JsonValue::as_str))
        .map(ToString::to_string)
}

fn opt_str_or_json(val: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match val.get(k) {
        Some(JsonValue::String(s)) => Some(s.clone()),
        Some(JsonValue::Null) | None => None,
        Some(other) => Some(to_json_string(other)),
    })
}

/// Unwraps Pi/Anthropic tool result JSON envelopes (`{"content":[{"type":"text","text":"..."}]}`,
/// `{"content":[{"text":"..."}]}`, `{"output":"..."}`, `{"text":"..."}`) into clean plain text,
/// falling back to compact JSON serialization for arbitrary non-text payloads.
pub fn extract_tool_output_text(val: &JsonValue) -> Option<String> {
    match val {
        JsonValue::Null => None,
        JsonValue::String(s) => Some(s.clone()),
        JsonValue::Object(_) => {
            if let Some(content) = val.get("content") {
                match content {
                    JsonValue::String(s) => return Some(s.clone()),
                    JsonValue::Array(blocks) => {
                        let mut parts = Vec::new();
                        for block in blocks {
                            if let Some(s) = block.as_str() {
                                parts.push(s.to_string());
                            } else if block.as_object().is_some() {
                                let is_text_or_untyped = matches!(
                                    block.get("type").and_then(JsonValue::as_str),
                                    Some("text") | None
                                );
                                if is_text_or_untyped {
                                    if let Some(t) = opt_str(block, &["text", "content"]) {
                                        parts.push(t);
                                    }
                                }
                            }
                        }
                        if !parts.is_empty() {
                            return Some(parts.join("\n"));
                        }
                    }
                    _ => {}
                }
            }
            if let Some(s) = opt_str(val, &["output", "text"]) {
                return Some(s);
            }
            Some(to_json_string(val))
        }
        other => Some(to_json_string(other)),
    }
}

fn opt_tool_output_text(val: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| val.get(k).and_then(extract_tool_output_text))
}

fn opt_str_or_nested(val: &JsonValue, nested_key: &str, keys: &[&str]) -> Option<String> {
    opt_str(val, keys).or_else(|| val.get(nested_key).and_then(|obj| opt_str(obj, keys)))
}

fn opt_non_empty_str(val: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| {
            val.get(k)
                .and_then(JsonValue::as_str)
                .filter(|s| !s.is_empty())
        })
        .map(ToString::to_string)
}

fn serde_value_to_json_value(v: serde_json::Value) -> JsonValue {
    match v {
        serde_json::Value::Null => JsonValue::Null,
        serde_json::Value::Bool(b) => JsonValue::Bool(b),
        serde_json::Value::Number(n) => JsonValue::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => JsonValue::String(s),
        serde_json::Value::Array(arr) => {
            JsonValue::Array(arr.into_iter().map(serde_value_to_json_value).collect())
        }
        serde_json::Value::Object(map) => JsonValue::Object(
            map.into_iter()
                .map(|(k, v)| (k, serde_value_to_json_value(v)))
                .collect(),
        ),
    }
}

/// Parses a single JSONL line using `verity_core::json::parse_json`, falling back to
/// `serde_json` so no valid Pi streaming event is ever silently dropped.
pub fn parse_jsonl_line(raw: &str) -> Option<JsonValue> {
    let trimmed = raw.trim().trim_start_matches('\u{feff}');
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(v) = parse_json(trimmed) {
        return Some(v);
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .map(serde_value_to_json_value)
}

fn parse_nested_delta_object(delta_obj: &JsonValue) -> (Option<String>, Option<String>) {
    match delta_obj.get("type").and_then(JsonValue::as_str) {
        Some("text_delta") => (opt_non_empty_str(delta_obj, &["text", "delta"]), None),
        Some("thinking_delta") | Some("reasoning_delta") => (
            None,
            opt_non_empty_str(
                delta_obj,
                &[
                    "thinking",
                    "reasoning",
                    "reasoning_content",
                    "text",
                    "delta",
                ],
            ),
        ),
        None => (
            opt_non_empty_str(delta_obj, &["text", "delta"]),
            opt_non_empty_str(delta_obj, &["thinking", "reasoning", "reasoning_content"]),
        ),
        Some(_) => (None, None),
    }
}

fn parse_message_update_deltas(val: &JsonValue) -> (Option<String>, Option<String>) {
    if let Some(ame) = val
        .get("assistantMessageEvent")
        .or_else(|| val.get("assistant_message_event"))
    {
        if let Some(ame_type) = ame.get("type").and_then(JsonValue::as_str) {
            return match ame_type {
                "text_delta" => (opt_non_empty_str(ame, &["delta", "text"]), None),
                "thinking_delta" | "reasoning_delta" => (
                    None,
                    opt_non_empty_str(
                        ame,
                        &[
                            "delta",
                            "thinking",
                            "reasoning",
                            "reasoning_content",
                            "text",
                        ],
                    ),
                ),
                "content_block_delta" => {
                    if let Some(delta_obj) = ame.get("delta").filter(|d| d.as_object().is_some()) {
                        parse_nested_delta_object(delta_obj)
                    } else {
                        (
                            opt_non_empty_str(ame, &["delta", "text"]),
                            opt_non_empty_str(
                                ame,
                                &[
                                    "thinking",
                                    "thinking_delta",
                                    "thinkingDelta",
                                    "reasoning",
                                    "reasoning_content",
                                    "reasoning_delta",
                                    "reasoningDelta",
                                ],
                            ),
                        )
                    }
                }
                _ => (None, None),
            };
        }
    }

    if let Some(delta_obj) = val.get("delta").filter(|d| d.as_object().is_some()) {
        return parse_nested_delta_object(delta_obj);
    }

    let flat_delta = opt_non_empty_str(val, &["delta", "text"]);
    let flat_thinking = opt_non_empty_str(
        val,
        &[
            "thinking_delta",
            "thinkingDelta",
            "reasoning_delta",
            "reasoningDelta",
            "reasoning",
            "reasoning_content",
        ],
    );
    if flat_delta.is_some() || flat_thinking.is_some() {
        return (flat_delta, flat_thinking);
    }

    if let Some(msg) = val.get("message") {
        if let Some(delta_obj) = msg.get("delta").filter(|d| d.as_object().is_some()) {
            return parse_nested_delta_object(delta_obj);
        }
        let msg_delta = opt_non_empty_str(msg, &["delta", "text"]);
        let msg_thinking = opt_non_empty_str(
            msg,
            &[
                "thinking_delta",
                "thinkingDelta",
                "reasoning_delta",
                "reasoningDelta",
                "reasoning",
                "reasoning_content",
            ],
        );
        if msg_delta.is_some() || msg_thinking.is_some() {
            return (msg_delta, msg_thinking);
        }
    }

    (None, None)
}

fn extract_message_end_content(val: &JsonValue) -> (Option<String>, Option<String>) {
    let Some(content) = val
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| val.get("content"))
    else {
        return (None, None);
    };

    match content {
        JsonValue::String(s) => {
            if s.is_empty() {
                (None, None)
            } else {
                (Some(s.clone()), None)
            }
        }
        JsonValue::Array(blocks) => {
            let mut text_parts = Vec::new();
            let mut thinking_parts = Vec::new();

            for block in blocks {
                if let Some(s) = block.as_str() {
                    if !s.is_empty() {
                        text_parts.push(s.to_string());
                    }
                    continue;
                }
                match block.get("type").and_then(JsonValue::as_str) {
                    Some("thinking") | Some("reasoning") => {
                        if let Some(t) = opt_str(block, &["thinking", "text", "content"]) {
                            if !t.is_empty() {
                                thinking_parts.push(t);
                            }
                        }
                    }
                    Some("text") => {
                        if let Some(t) = opt_str(block, &["text", "content"]) {
                            if !t.is_empty() {
                                text_parts.push(t);
                            }
                        }
                    }
                    None => {
                        if let Some(t) = opt_str(block, &["thinking"]) {
                            if !t.is_empty() {
                                thinking_parts.push(t);
                            }
                        } else if let Some(t) = opt_str(block, &["text", "content"]) {
                            if !t.is_empty() {
                                text_parts.push(t);
                            }
                        }
                    }
                    Some(_) => {}
                }
            }

            let text = if text_parts.is_empty() {
                None
            } else {
                Some(text_parts.join("\n"))
            };
            let thinking = if thinking_parts.is_empty() {
                None
            } else {
                Some(thinking_parts.join("\n"))
            };
            (text, thinking)
        }
        _ => (None, None),
    }
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
    let error = opt_str(val, &["error", "errorMessage", "error_message"]);
    let success = val
        .get("success")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
        && error.is_none();
    let data_json = val.get("data").map(to_json_string);
    Some(PiRpcResponse {
        id,
        command,
        success,
        data_json,
        error,
    })
}

fn parse_idless_response_error(val: &JsonValue, raw_json: &str) -> Option<PiUiEvent> {
    if val.get("type").and_then(JsonValue::as_str) != Some("response") {
        return None;
    }
    let has_valid_id = val
        .get("id")
        .and_then(JsonValue::as_str)
        .and_then(|s| RequestId::parse(s).ok())
        .is_some();
    if has_valid_id {
        return None;
    }
    let error = opt_str(val, &["error", "errorMessage", "error_message", "message"]);
    let explicit_failure = val.get("success").and_then(JsonValue::as_bool) == Some(false);
    if error.is_some() || explicit_failure {
        Some(PiUiEvent::Error {
            message: error.unwrap_or_else(|| "Pi RPC response error".to_string()),
            raw_json: raw_json.to_string(),
        })
    } else {
        None
    }
}

fn push_stderr_line(buf: &Mutex<VecDeque<String>>, line: String) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let clipped = if trimmed.len() > MAX_STDERR_LINE_BYTES {
        let mut end = MAX_STDERR_LINE_BYTES;
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &trimmed[..end])
    } else {
        trimmed.to_string()
    };
    let mut guard = buf.lock().expect("stderr_buf lock poisoned");
    if guard.len() >= MAX_STDERR_LINES {
        guard.pop_front();
    }
    guard.push_back(clipped);
}

fn format_stderr_tail(buf: &Mutex<VecDeque<String>>) -> Option<String> {
    let guard = buf.lock().expect("stderr_buf lock poisoned");
    if guard.is_empty() {
        None
    } else {
        let lines: Vec<&str> = guard.iter().map(String::as_str).collect();
        Some(lines.join("\n"))
    }
}

/// Event callback sink so `main.rs` can wire `app.emit("pi:event", event)` without
/// coupling `pi_session` to Tauri runtime types.
pub type EventSink = Arc<dyn Fn(PiUiEvent) + Send + Sync>;

/// Callback invoked when a provider reload executes (either immediately while idle
/// or deferred upon `agent_settled` / `abort`).
pub type ReloadHook = Arc<dyn Fn() + Send + Sync>;

type PendingMap = HashMap<RequestId, mpsc::Sender<Result<PiRpcResponse, PiSessionError>>>;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn apply_no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        let pgid_arg = format!("-{}", pid);
        let pid_str = pid.to_string();
        let _ = Command::new("kill").args(["-9", "--", &pgid_arg]).status();
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

/// Long-lived supervisor managing a `pi --mode rpc --no-session` child process.
pub struct PiSessionSupervisor {
    config: PiSessionConfig,
    child: Arc<Mutex<Option<Child>>>,
    stdin: Mutex<Option<ChildStdin>>,
    reader_handle: Mutex<Option<JoinHandle<()>>>,
    stderr_handle: Mutex<Option<JoinHandle<()>>>,
    stderr_buf: Arc<Mutex<VecDeque<String>>>,
    activity: Arc<Mutex<SessionActivity>>,
    pending: Arc<Mutex<PendingMap>>,
    extension_ui: Arc<Mutex<ExtensionUiTracker>>,
    event_sink: EventSink,
    reload_hook: ReloadHook,
    next_id: AtomicU64,
    shutdown_flag: Arc<AtomicBool>,
}

impl PiSessionSupervisor {
    /// Spawns the configured Pi JSONL RPC child process and starts its background stdout and stderr readers.
    pub fn spawn(
        config: PiSessionConfig,
        event_sink: EventSink,
        reload_hook: ReloadHook,
    ) -> Result<Self, PiSessionError> {
        let resolved_program = resolve_pi_program(&config.program);
        let mut cmd = Command::new(&resolved_program);
        cmd.args(&config.args);
        if config.no_session && !config.args.iter().any(|a| a == "--no-session") {
            cmd.arg("--no-session");
        }
        cmd.current_dir(&config.cwd)
            .envs(config.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        #[cfg(windows)]
        apply_no_window(&mut cmd);
        let mut child = cmd
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
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| PiSessionError::Io("missing child stderr pipe".to_string()))?;

        let child_arc = Arc::new(Mutex::new(Some(child)));
        let stderr_buf = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_STDERR_LINES)));
        let activity = Arc::new(Mutex::new(SessionActivity::Idle));
        let pending: Arc<Mutex<PendingMap>> = Arc::new(Mutex::new(HashMap::new()));
        let extension_ui = Arc::new(Mutex::new(ExtensionUiTracker::new()));
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        let s_buf = Arc::clone(&stderr_buf);
        let stderr_handle = std::thread::Builder::new()
            .name("pi-rpc-stderr".to_string())
            .spawn(move || {
                let reader = BufReader::new(stderr);
                for line_res in reader.lines() {
                    let line = match line_res {
                        Ok(l) => l,
                        Err(_) => break,
                    };
                    push_stderr_line(&s_buf, line);
                }
            })
            .map_err(|e| {
                if let Some(mut c) = child_arc.lock().expect("child lock poisoned").take() {
                    kill_tree(c.id());
                    let _ = c.kill();
                    let _ = c.wait();
                }
                PiSessionError::SpawnFailed(e.to_string())
            })?;

        let r_child = Arc::clone(&child_arc);
        let r_stderr = Arc::clone(&stderr_buf);
        let r_activity = Arc::clone(&activity);
        let r_pending = Arc::clone(&pending);
        let r_ext_ui = Arc::clone(&extension_ui);
        let r_reload = Arc::clone(&reload_hook);
        let r_sink = Arc::clone(&event_sink);
        let r_shutdown = Arc::clone(&shutdown_flag);

        let reader_handle = match std::thread::Builder::new()
            .name("pi-rpc-stdout".to_string())
            .spawn(move || {
                let mut reader = BufReader::with_capacity(4096, stdout);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let trimmed = line.trim().trim_start_matches('\u{feff}');
                    if trimmed.is_empty() {
                        continue;
                    }
                    let Some(val) = parse_jsonl_line(trimmed) else {
                        continue;
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

                    if let Some(err_event) = parse_idless_response_error(&val, trimmed) {
                        r_sink(err_event);
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

                let was_shutdown = r_shutdown.load(Ordering::Acquire);
                let tail = format_stderr_tail(&r_stderr);

                if !was_shutdown {
                    {
                        let mut act = r_activity.lock().expect("activity lock poisoned");
                        if let Ok(tr) = transition_session(*act, SessionCommand::ProcessExited) {
                            *act = tr.activity;
                        }
                    }
                    {
                        let mut tracker = r_ext_ui.lock().expect("extension_ui lock poisoned");
                        *tracker = ExtensionUiTracker::new();
                    }
                }

                // Fail any still-waiting callers so nobody hangs when the child exits.
                {
                    let mut map = r_pending.lock().expect("pending lock poisoned");
                    for (_, tx) in map.drain() {
                        let _ = tx.send(Err(PiSessionError::ProcessExited {
                            stderr_tail: tail.clone(),
                        }));
                    }
                }

                if !was_shutdown {
                    let exit_code = r_child.try_lock().ok().and_then(|mut guard| {
                        guard
                            .as_mut()
                            .and_then(|c| c.try_wait().ok().flatten())
                            .and_then(|status| status.code())
                    });
                    r_sink(PiUiEvent::ProcessExited {
                        exit_code,
                        stderr_tail: tail,
                    });
                }
            }) {
            Ok(h) => h,
            Err(e) => {
                if let Some(mut c) = child_arc.lock().expect("child lock poisoned").take() {
                    kill_tree(c.id());
                    let _ = c.kill();
                    let _ = c.wait();
                }
                let _ = stderr_handle.join();
                return Err(PiSessionError::SpawnFailed(e.to_string()));
            }
        };

        Ok(Self {
            config,
            child: child_arc,
            stdin: Mutex::new(Some(stdin)),
            reader_handle: Mutex::new(Some(reader_handle)),
            stderr_handle: Mutex::new(Some(stderr_handle)),
            stderr_buf,
            activity,
            pending,
            extension_ui,
            event_sink,
            reload_hook,
            next_id: AtomicU64::new(1),
            shutdown_flag,
        })
    }

    /// Returns the current `SessionActivity` snapshot.
    pub fn activity(&self) -> SessionActivity {
        *self.activity.lock().expect("activity lock poisoned")
    }

    /// Returns true if the supervisor has not been shut down and the child process is still running.
    pub fn is_alive(&self) -> bool {
        if self.shutdown_flag.load(Ordering::Acquire) {
            return false;
        }
        let mut child_guard = self.child.lock().expect("child lock poisoned");
        match child_guard.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Returns the recent captured stderr lines from the Pi child process, if any.
    pub fn stderr_tail(&self) -> Option<String> {
        format_stderr_tail(&self.stderr_buf)
    }

    /// Returns the number of currently open Extension UI dialogs.
    pub fn open_dialog_count(&self) -> usize {
        self.extension_ui
            .lock()
            .expect("extension_ui lock poisoned")
            .open_count()
    }

    /// Requests a provider reload: runs immediately if `Idle`, defers until `agent_settled`/`abort`
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

    /// Rolls back a pre-send state transition on error without clobbering `Idle` or `reload_deferred`.
    fn rollback_pre_transition(&self, cmd: &PiRpcCommand, previous: SessionActivity) {
        let mut trigger_reload = false;
        {
            let mut act = self.activity.lock().expect("activity lock poisoned");
            if act.is_idle() {
                return;
            }
            match cmd {
                PiRpcCommand::Prompt { .. } => {
                    if let SessionActivity::Streaming {
                        reload_deferred, ..
                    } = *act
                    {
                        *act = SessionActivity::Idle;
                        trigger_reload = reload_deferred;
                    }
                }
                PiRpcCommand::Steer { .. } | PiRpcCommand::FollowUp { .. } => {
                    if let SessionActivity::Streaming {
                        pending_queue,
                        reload_deferred,
                    } = *act
                    {
                        *act = SessionActivity::Streaming {
                            pending_queue: pending_queue.saturating_sub(1),
                            reload_deferred,
                        };
                    }
                }
                PiRpcCommand::ClearQueue => {
                    if let (
                        SessionActivity::Streaming {
                            reload_deferred, ..
                        },
                        SessionActivity::Streaming {
                            pending_queue: prev_queue,
                            reload_deferred: prev_reload,
                        },
                    ) = (*act, previous)
                    {
                        *act = SessionActivity::Streaming {
                            pending_queue: prev_queue,
                            reload_deferred: reload_deferred || prev_reload,
                        };
                    }
                }
                PiRpcCommand::Compact { .. } => {
                    if let Ok(tr) = transition_session(*act, SessionCommand::FinishCompacting) {
                        *act = tr.activity;
                    }
                }
                PiRpcCommand::SwitchSession { .. }
                | PiRpcCommand::NewSession { .. }
                | PiRpcCommand::Fork { .. }
                | PiRpcCommand::Clone { .. } => {
                    if let Ok(tr) = transition_session(*act, SessionCommand::FinishSwitch) {
                        *act = tr.activity;
                    }
                }
                _ => {}
            }
        }
        if trigger_reload {
            (self.reload_hook)();
        }
    }

    /// Forces recovery from a wedged or dead Pi process during `Abort`:
    /// kills the process tree, transitions activity to `Idle`, clears open dialogs,
    /// emits synthetic `AgentSettled`, and runs any deferred provider reload.
    fn force_abort_recovery(&self) {
        self.shutdown();
        let trigger_reload = {
            let mut act = self.activity.lock().expect("activity lock poisoned");
            if let Ok(tr) = transition_session(*act, SessionCommand::Abort) {
                *act = tr.activity;
                tr.run_provider_reload
            } else {
                *act = SessionActivity::Idle;
                false
            }
        };
        {
            let mut tracker = self
                .extension_ui
                .lock()
                .expect("extension_ui lock poisoned");
            *tracker = ExtensionUiTracker::new();
        }
        (self.event_sink)(PiUiEvent::AgentSettled {
            summary: Some("aborted".to_string()),
            raw_json: r#"{"type":"agent_settled","summary":"aborted"}"#.to_string(),
        });
        if trigger_reload {
            (self.reload_hook)();
        }
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
            PiRpcCommand::NewSession { .. } => Some(SessionCommand::NewSession),
            PiRpcCommand::Fork { .. } => Some(SessionCommand::Fork),
            PiRpcCommand::Clone { .. } => Some(SessionCommand::CloneSession),
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
            if matches!(cmd, PiRpcCommand::Abort) {
                self.force_abort_recovery();
                return Ok(Some(PiRpcResponse {
                    id: request_id,
                    command: Some("abort".to_string()),
                    success: true,
                    data_json: None,
                    error: None,
                }));
            }
            if let Some(prev) = previous_activity {
                self.rollback_pre_transition(&cmd, prev);
            }
            return Err(err);
        }

        let timeout = match &cmd {
            PiRpcCommand::Abort | PiRpcCommand::AbortRetry | PiRpcCommand::AbortBash => {
                self.config.command_timeout.min(ABORT_COMMAND_TIMEOUT)
            }
            _ => self.config.command_timeout,
        };
        let wait_result = rx.recv_timeout(timeout);

        // Complete synchronous exclusive transitions (Compacting / SwitchingSession) regardless of outcome.
        match &cmd {
            PiRpcCommand::Compact { .. } => {
                let mut act = self.activity.lock().expect("activity lock poisoned");
                if let Ok(tr) = transition_session(*act, SessionCommand::FinishCompacting) {
                    *act = tr.activity;
                }
            }
            PiRpcCommand::SwitchSession { .. }
            | PiRpcCommand::NewSession { .. }
            | PiRpcCommand::Fork { .. }
            | PiRpcCommand::Clone { .. } => {
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
                    if let Some(prev) = previous_activity {
                        self.rollback_pre_transition(&cmd, prev);
                    }
                    return Err(PiSessionError::RpcError {
                        command: cmd.command_type().to_string(),
                        message: resp
                            .error
                            .unwrap_or_else(|| "unknown pi rpc error".to_string()),
                    });
                }
                if matches!(cmd, PiRpcCommand::Abort) {
                    let trigger_reload = {
                        let mut act = self.activity.lock().expect("activity lock poisoned");
                        if let Ok(tr) = transition_session(*act, SessionCommand::Abort) {
                            *act = tr.activity;
                            tr.run_provider_reload
                        } else {
                            false
                        }
                    };
                    if trigger_reload {
                        (self.reload_hook)();
                    }
                }
                Ok(Some(resp))
            }
            Ok(Err(e)) => {
                if matches!(cmd, PiRpcCommand::Abort) {
                    self.force_abort_recovery();
                    return Ok(Some(PiRpcResponse {
                        id: request_id,
                        command: Some("abort".to_string()),
                        success: true,
                        data_json: None,
                        error: None,
                    }));
                }
                if let Some(prev) = previous_activity {
                    self.rollback_pre_transition(&cmd, prev);
                }
                Err(e)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending
                    .lock()
                    .expect("pending lock poisoned")
                    .remove(&request_id);
                if matches!(cmd, PiRpcCommand::Abort) {
                    self.force_abort_recovery();
                    return Ok(Some(PiRpcResponse {
                        id: request_id,
                        command: Some("abort".to_string()),
                        success: true,
                        data_json: None,
                        error: None,
                    }));
                }
                if let Some(prev) = previous_activity {
                    self.rollback_pre_transition(&cmd, prev);
                }
                Err(PiSessionError::Timeout {
                    request_id,
                    command: cmd.command_type().to_string(),
                    stderr_tail: self.stderr_tail(),
                })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if matches!(cmd, PiRpcCommand::Abort) {
                    self.force_abort_recovery();
                    return Ok(Some(PiRpcResponse {
                        id: request_id,
                        command: Some("abort".to_string()),
                        success: true,
                        data_json: None,
                        error: None,
                    }));
                }
                if let Some(prev) = previous_activity {
                    self.rollback_pre_transition(&cmd, prev);
                }
                Err(PiSessionError::ProcessExited {
                    stderr_tail: self.stderr_tail(),
                })
            }
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

    pub fn bash(&self, command: impl Into<String>) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::Bash {
            command: command.into(),
        })
    }

    pub fn abort_bash(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::AbortBash)
    }

    pub fn set_auto_compaction(&self, enabled: bool) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::SetAutoCompaction { enabled })
    }

    pub fn set_auto_retry(&self, enabled: bool) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::SetAutoRetry { enabled })
    }

    pub fn set_steering_mode(
        &self,
        mode: impl Into<String>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::SetSteeringMode { mode: mode.into() })
    }

    pub fn set_follow_up_mode(
        &self,
        mode: impl Into<String>,
    ) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::SetFollowUpMode { mode: mode.into() })
    }

    pub fn cycle_model(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::CycleModel)
    }

    pub fn cycle_thinking_level(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::CycleThinkingLevel)
    }

    pub fn get_last_assistant_text(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetLastAssistantText)
    }

    pub fn get_entries(&self) -> Result<PiRpcResponse, PiSessionError> {
        self.send_expect_response(PiRpcCommand::GetEntries)
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
    /// and joins the background stdout and stderr reader threads.
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

        // Join stderr reader thread.
        {
            let mut handle_guard = self
                .stderr_handle
                .lock()
                .expect("stderr_handle lock poisoned");
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
            program: "sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            command_timeout: Duration::from_secs(5),
            no_session: false,
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
                    model_id: "claude-sonnet-5-5".to_string(),
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
            r#"{"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":500,"errorMessage":"rate limited"}"#,
            r#"{"type":"auto_retry_end","attempt":1,"success":false,"finalError":"quota exceeded"}"#,
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
    fn parses_error_events_and_rejects_missing_success_in_rpc_response() {
        let err_raw = r#"{"type":"error","error":"provider auth failed"}"#;
        let err_ev = parse_pi_ui_event(&parse_json(err_raw).unwrap(), err_raw).unwrap();
        assert_eq!(
            err_ev,
            PiUiEvent::Error {
                message: "provider auth failed".to_string(),
                raw_json: err_raw.to_string(),
            }
        );

        let ext_err_raw = r#"{"type":"extension_error","message":"extension crashed"}"#;
        let ext_err_ev = parse_pi_ui_event(&parse_json(ext_err_raw).unwrap(), ext_err_raw).unwrap();
        assert_eq!(
            ext_err_ev,
            PiUiEvent::Error {
                message: "extension crashed".to_string(),
                raw_json: ext_err_raw.to_string(),
            }
        );

        // Missing `success` must NOT default to true.
        let missing_success =
            parse_json(r#"{"type":"response","id":"req-1","error":"bad params"}"#).unwrap();
        let resp = parse_rpc_response(&missing_success).unwrap();
        assert!(!resp.success);
        assert_eq!(resp.error.as_deref(), Some("bad params"));

        let empty_resp = parse_json(r#"{"type":"response","id":"req-2"}"#).unwrap();
        assert!(!parse_rpc_response(&empty_resp).unwrap().success);

        // Id-less parse-error response routes to PiUiEvent::Error.
        let idless_raw = r#"{"type":"response","success":false,"error":"invalid json line"}"#;
        let idless_val = parse_json(idless_raw).unwrap();
        assert!(parse_rpc_response(&idless_val).is_none());
        assert_eq!(
            parse_idless_response_error(&idless_val, idless_raw),
            Some(PiUiEvent::Error {
                message: "invalid json line".to_string(),
                raw_json: idless_raw.to_string(),
            })
        );
    }

    #[test]
    fn mock_sh_prompt_streaming_steer_followup_deferred_reload_and_clean_shutdown() {
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

    #[test]
    fn unexpected_child_exit_captures_stderr_resets_activity_and_emits_process_exited() {
        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  if [ "$typ" = "prompt" ]; then
    printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
    printf 'fatal provider crash\n' >&2
    exit 42
  fi
done
"#;

        let (exit_tx, exit_rx) = mpsc::channel();
        let sink: EventSink = Arc::new(move |ev| {
            if let PiUiEvent::ProcessExited {
                exit_code,
                stderr_tail,
            } = ev
            {
                let _ = exit_tx.send((exit_code, stderr_tail));
            }
        });
        let sup = PiSessionSupervisor::spawn(mock_sh_config(script), sink, Arc::new(|| {}))
            .expect("spawn mock pi");

        sup.prompt("boom", vec![]).expect("prompt");
        let (_exit_code, stderr_tail) = exit_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("must emit ProcessExited on unexpected EOF");
        assert_eq!(sup.activity(), SessionActivity::Idle);
        assert!(
            stderr_tail
                .as_deref()
                .is_some_and(|s| s.contains("fatal provider crash"))
                || sup
                    .stderr_tail()
                    .as_deref()
                    .is_some_and(|s| s.contains("fatal provider crash"))
        );
        sup.shutdown();
    }

    #[test]
    fn abort_timeout_kills_wedged_process_resets_idle_and_emits_agent_settled() {
        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  if [ "$typ" = "prompt" ]; then
    printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
  elif [ "$typ" = "abort" ]; then
    sleep 30
  fi
done
"#;
        let mut cfg = mock_sh_config(script);
        cfg.command_timeout = Duration::from_secs(2);

        let (settled_tx, settled_rx) = mpsc::channel();
        let sink: EventSink = Arc::new(move |ev| {
            if matches!(ev, PiUiEvent::AgentSettled { .. }) {
                let _ = settled_tx.send(());
            }
        });
        let sup = PiSessionSupervisor::spawn(cfg, sink, Arc::new(|| {})).expect("spawn mock pi");
        sup.prompt("hang", vec![]).expect("prompt");
        assert!(sup.activity().is_streaming());

        sup.abort().expect("wedged abort must recover cleanly");
        assert_eq!(sup.activity(), SessionActivity::Idle);
        assert!(!sup.is_alive());
        settled_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("synthetic AgentSettled must be emitted on abort timeout");
    }

    #[test]
    fn parses_nested_pi_rpc_streaming_events_and_agent_end() {
        let start_raw =
            r#"{"type":"message_start","message":{"id":"msg-42","role":"assistant","content":[]}}"#;
        let start_ev = parse_pi_ui_event(&parse_json(start_raw).unwrap(), start_raw).unwrap();
        assert_eq!(
            start_ev,
            PiUiEvent::MessageStart {
                message_id: Some("msg-42".to_string()),
                role: Some("assistant".to_string()),
                raw_json: start_raw.to_string(),
            }
        );

        let think_delta_raw = r#"{"type":"message_update","message":{"id":"msg-42","role":"assistant"},"assistantMessageEvent":{"type":"thinking_delta","delta":"Checking imports"}}"#;
        let think_ev =
            parse_pi_ui_event(&parse_json(think_delta_raw).unwrap(), think_delta_raw).unwrap();
        assert_eq!(
            think_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("msg-42".to_string()),
                delta: None,
                thinking_delta: Some("Checking imports".to_string()),
                thinking_phase: None,
                thinking_content: None,
                raw_json: think_delta_raw.to_string(),
            }
        );

        let reasoning_delta_raw = r#"{"type":"message_update","message":{"id":"msg-42"},"assistantMessageEvent":{"type":"reasoning_delta","delta":"Step 2"}}"#;
        let reasoning_ev = parse_pi_ui_event(
            &parse_json(reasoning_delta_raw).unwrap(),
            reasoning_delta_raw,
        )
        .unwrap();
        assert_eq!(
            reasoning_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("msg-42".to_string()),
                delta: None,
                thinking_delta: Some("Step 2".to_string()),
                thinking_phase: None,
                thinking_content: None,
                raw_json: reasoning_delta_raw.to_string(),
            }
        );

        let text_delta_raw = r#"{"type":"message_update","message":{"id":"msg-42","role":"assistant"},"assistantMessageEvent":{"type":"text_delta","delta":"Hello!"}}"#;
        let text_ev =
            parse_pi_ui_event(&parse_json(text_delta_raw).unwrap(), text_delta_raw).unwrap();
        assert_eq!(
            text_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("msg-42".to_string()),
                delta: Some("Hello!".to_string()),
                thinking_delta: None,
                thinking_phase: None,
                thinking_content: None,
                raw_json: text_delta_raw.to_string(),
            }
        );

        // Lifecycle sub-events (thinking_start / text_end) must not leak full content as deltas.
        let text_end_raw = r#"{"type":"message_update","message":{"id":"msg-42"},"assistantMessageEvent":{"type":"text_end","content":"Hello!"}}"#;
        let text_end_ev =
            parse_pi_ui_event(&parse_json(text_end_raw).unwrap(), text_end_raw).unwrap();
        assert_eq!(
            text_end_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("msg-42".to_string()),
                delta: None,
                thinking_delta: None,
                thinking_phase: None,
                thinking_content: None,
                raw_json: text_end_raw.to_string(),
            }
        );

        // Flat thinking_delta / thinkingDelta backwards compatibility.
        let flat_think_raw =
            r#"{"type":"message_update","messageId":"m-flat","thinkingDelta":"flat thought"}"#;
        let flat_think_ev =
            parse_pi_ui_event(&parse_json(flat_think_raw).unwrap(), flat_think_raw).unwrap();
        assert_eq!(
            flat_think_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("m-flat".to_string()),
                delta: None,
                thinking_delta: Some("flat thought".to_string()),
                thinking_phase: None,
                thinking_content: None,
                raw_json: flat_think_raw.to_string(),
            }
        );

        // Nested message_end with thinking + text + toolCall content blocks + errorMessage.
        let end_raw = r#"{"type":"message_end","message":{"id":"msg-42","role":"assistant","stopReason":"stop","errorMessage":"partial warning","content":[{"type":"thinking","thinking":"First thought"},{"type":"thinking","thinking":"Second thought"},{"type":"toolCall","id":"tc-1","name":"read"},{"type":"text","text":"Final answer"}]}}"#;
        let end_ev = parse_pi_ui_event(&parse_json(end_raw).unwrap(), end_raw).unwrap();
        assert_eq!(
            end_ev,
            PiUiEvent::MessageEnd {
                message_id: Some("msg-42".to_string()),
                role: Some("assistant".to_string()),
                stop_reason: Some("stop".to_string()),
                error_message: Some("partial warning".to_string()),
                text: Some("Final answer".to_string()),
                thinking: Some("First thought\nSecond thought".to_string()),
                raw_json: end_raw.to_string(),
            }
        );

        // agent_end maps to AgentSettled.
        let agent_end_raw = r#"{"type":"agent_end","messages":[]}"#;
        let agent_end_ev =
            parse_pi_ui_event(&parse_json(agent_end_raw).unwrap(), agent_end_raw).unwrap();
        assert_eq!(
            agent_end_ev,
            PiUiEvent::AgentSettled {
                summary: None,
                raw_json: agent_end_raw.to_string(),
            }
        );
    }

    #[test]
    fn mock_sh_agent_end_settles_streaming_session_and_emits_nested_events() {
        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    prompt)
      printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
      printf '{"type":"message_start","message":{"id":"m99","role":"assistant","content":[]}}\n'
      printf '{"type":"message_update","message":{"id":"m99"},"assistantMessageEvent":{"type":"thinking_delta","delta":"hmm"}}\n'
      printf '{"type":"message_update","message":{"id":"m99"},"assistantMessageEvent":{"type":"text_delta","delta":"done"}}\n'
      printf '{"type":"message_end","message":{"id":"m99","role":"assistant","stopReason":"stop","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"done"}]}}\n'
      printf '{"type":"agent_end","messages":[]}\n'
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
        let reload_hook: ReloadHook = Arc::new(|| {});
        let sup = PiSessionSupervisor::spawn(mock_sh_config(script), sink, reload_hook)
            .expect("spawn mock pi");

        sup.prompt("test nested", vec![]).expect("prompt");
        settled_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("agent_end must settle session");
        assert_eq!(sup.activity(), SessionActivity::Idle);

        let recorded = events.lock().unwrap();
        assert!(recorded.iter().any(|e| matches!(
            e,
            PiUiEvent::MessageStart {
                message_id: Some(id),
                role: Some(role),
                ..
            } if id == "m99" && role == "assistant"
        )));
        assert!(recorded.iter().any(|e| matches!(
            e,
            PiUiEvent::MessageUpdate {
                thinking_delta: Some(t),
                delta: None,
                ..
            } if t == "hmm"
        )));
        assert!(recorded.iter().any(|e| matches!(
            e,
            PiUiEvent::MessageUpdate {
                delta: Some(d),
                thinking_delta: None,
                ..
            } if d == "done"
        )));
        assert!(recorded.iter().any(|e| matches!(
            e,
            PiUiEvent::MessageEnd {
                role: Some(role),
                stop_reason: Some(sr),
                text: Some(txt),
                thinking: Some(th),
                ..
            } if role == "assistant" && sr == "stop" && txt == "done" && th == "hmm"
        )));
        drop(recorded);
        sup.shutdown();
    }

    #[test]
    fn default_pi_with_extension_appends_extension_cli_flag() {
        let cfg = PiSessionConfig::default_pi(PathBuf::from("/tmp/repo")).with_extension(
            Path::new("/Users/alice/.verity/extensions/verity-providers.js"),
        );
        assert!(!cfg.no_session);
        assert_eq!(
            cfg.args,
            vec![
                "--mode",
                "rpc",
                "--extension",
                "/Users/alice/.verity/extensions/verity-providers.js",
            ]
        );
        let ephemeral = cfg.with_no_session(true);
        assert!(ephemeral.no_session);
    }

    #[test]
    fn serializes_all_10_new_rpc_commands_with_exact_wire_types() {
        let req_id = RequestId::parse("req-99").unwrap();
        let new_cmds: Vec<(PiRpcCommand, &str)> = vec![
            (
                PiRpcCommand::Bash {
                    command: "ls -la".to_string(),
                },
                "bash",
            ),
            (PiRpcCommand::AbortBash, "abort_bash"),
            (
                PiRpcCommand::SetAutoCompaction { enabled: true },
                "set_auto_compaction",
            ),
            (
                PiRpcCommand::SetAutoRetry { enabled: false },
                "set_auto_retry",
            ),
            (
                PiRpcCommand::SetSteeringMode {
                    mode: "one-at-a-time".to_string(),
                },
                "set_steering_mode",
            ),
            (
                PiRpcCommand::SetFollowUpMode {
                    mode: "all".to_string(),
                },
                "set_follow_up_mode",
            ),
            (PiRpcCommand::CycleModel, "cycle_model"),
            (PiRpcCommand::CycleThinkingLevel, "cycle_thinking_level"),
            (
                PiRpcCommand::GetLastAssistantText,
                "get_last_assistant_text",
            ),
            (PiRpcCommand::GetEntries, "get_entries"),
        ];

        assert_eq!(new_cmds.len(), 10);
        for (cmd, expected_type) in new_cmds {
            assert_eq!(cmd.command_type(), expected_type);
            let parsed = parse_json(&cmd.to_jsonl(&req_id)).expect("valid jsonl");
            assert_eq!(
                parsed.get("type").and_then(JsonValue::as_str),
                Some(expected_type)
            );
            assert_eq!(parsed.get("id").and_then(JsonValue::as_str), Some("req-99"));
        }

        let bash_json = parse_json(
            &PiRpcCommand::Bash {
                command: "echo hi".to_string(),
            }
            .to_jsonl(&req_id),
        )
        .unwrap();
        assert_eq!(
            bash_json.get("command").and_then(JsonValue::as_str),
            Some("echo hi")
        );

        let compact_json =
            parse_json(&PiRpcCommand::SetAutoCompaction { enabled: true }.to_jsonl(&req_id))
                .unwrap();
        assert_eq!(
            compact_json.get("enabled").and_then(JsonValue::as_bool),
            Some(true)
        );

        let steer_mode_json = parse_json(
            &PiRpcCommand::SetSteeringMode {
                mode: "one-at-a-time".to_string(),
            }
            .to_jsonl(&req_id),
        )
        .unwrap();
        assert_eq!(
            steer_mode_json.get("mode").and_then(JsonValue::as_str),
            Some("one-at-a-time")
        );
    }

    #[test]
    fn parses_all_newly_forwarded_rpc_events_and_extension_ui_effects() {
        let cases = [
            (
                r#"{"type":"tool_execution_update","toolCallId":"tc1","toolName":"bash","partialResult":"line 1"}"#,
                "tool_execution_update",
            ),
            (
                r#"{"type":"compaction_start","reason":"threshold"}"#,
                "compaction_start",
            ),
            (
                r#"{"type":"compaction_end","result":"summarized","aborted":false,"willRetry":false}"#,
                "compaction_end",
            ),
            (
                r#"{"type":"bash_execution_update","command":"cargo test","output":"ok","isError":false}"#,
                "bash_execution_update",
            ),
            (
                r#"{"type":"session_info_changed","name":"Sprint 4","sessionId":"s-42"}"#,
                "session_info_changed",
            ),
            (
                r#"{"type":"thinking_level_changed","level":"high"}"#,
                "thinking_level_changed",
            ),
            (r#"{"type":"turn_start"}"#, "turn_start"),
            (r#"{"type":"turn_end"}"#, "turn_end"),
            (r#"{"type":"agent_start"}"#, "agent_start"),
            (
                r#"{"type":"extension_ui_request","id":"fx-1","method":"notify","message":"Done","notifyType":"info"}"#,
                "extension_ui_effect",
            ),
            (
                r#"{"type":"extension_ui_request","id":"fx-2","method":"setStatus","statusKey":"branch","statusText":"main"}"#,
                "extension_ui_effect",
            ),
            (
                r#"{"type":"extension_ui_request","id":"fx-3","method":"setWidget","widgetKey":"ci","widgetLines":["green"]}"#,
                "extension_ui_effect",
            ),
            (
                r#"{"type":"extension_ui_request","id":"fx-4","method":"setTitle","title":"Verity"}"#,
                "extension_ui_effect",
            ),
            (
                r#"{"type":"extension_ui_request","id":"fx-5","method":"set_editor_text","text":"/help"}"#,
                "extension_ui_effect",
            ),
        ];

        for (raw, expected_tag) in cases {
            let val = parse_json(raw).unwrap();
            let ev = parse_pi_ui_event(&val, raw).expect("must parse forwarded event");
            let serialized = serde_json::to_value(&ev).expect("must serialize PiUiEvent");
            assert_eq!(
                serialized.get("type").and_then(|v| v.as_str()),
                Some(expected_tag),
                "unexpected serialized tag for {}",
                raw
            );
        }
    }

    #[test]
    fn parses_snake_case_content_block_delta_whitespace_and_serde_fallback() {
        // 1. Snake-case assistant_message_event with whitespace-only deltas (" " and "\n")
        let space_raw = r#"{"type":"message_update","message":{"id":"m1"},"assistant_message_event":{"type":"text_delta","delta":" "}}"#;
        let space_ev = parse_pi_ui_event(&parse_jsonl_line(space_raw).unwrap(), space_raw).unwrap();
        assert_eq!(
            space_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("m1".to_string()),
                delta: Some(" ".to_string()),
                thinking_delta: None,
                thinking_phase: None,
                thinking_content: None,
                raw_json: space_raw.to_string(),
            }
        );

        let nl_raw = r#"{"type":"message_update","message":{"id":"m1"},"assistantMessageEvent":{"type":"text_delta","delta":"\n"}}"#;
        let nl_ev = parse_pi_ui_event(&parse_jsonl_line(nl_raw).unwrap(), nl_raw).unwrap();
        assert_eq!(
            nl_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("m1".to_string()),
                delta: Some("\n".to_string()),
                thinking_delta: None,
                thinking_phase: None,
                thinking_content: None,
                raw_json: nl_raw.to_string(),
            }
        );

        // 2. Anthropic-style content_block_delta (both top-level and inside assistantMessageEvent)
        let anthropic_text_raw = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"chunk-a"}}"#;
        let anthropic_text_ev = parse_pi_ui_event(
            &parse_jsonl_line(anthropic_text_raw).unwrap(),
            anthropic_text_raw,
        )
        .unwrap();
        assert_eq!(
            anthropic_text_ev,
            PiUiEvent::MessageUpdate {
                message_id: None,
                delta: Some("chunk-a".to_string()),
                thinking_delta: None,
                thinking_phase: None,
                thinking_content: None,
                raw_json: anthropic_text_raw.to_string(),
            }
        );

        let anthropic_think_raw = r#"{"type":"message_update","message":{"id":"m2"},"assistantMessageEvent":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"deep thought"}}}"#;
        let anthropic_think_ev = parse_pi_ui_event(
            &parse_jsonl_line(anthropic_think_raw).unwrap(),
            anthropic_think_raw,
        )
        .unwrap();
        assert_eq!(
            anthropic_think_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("m2".to_string()),
                delta: None,
                thinking_delta: Some("deep thought".to_string()),
                thinking_phase: None,
                thinking_content: None,
                raw_json: anthropic_think_raw.to_string(),
            }
        );

        // 3. Nested incremental delta inside message when assistantMessageEvent is absent
        let msg_delta_raw = r#"{"type":"message_update","message":{"id":"m3","delta":"inc"}}"#;
        let msg_delta_ev =
            parse_pi_ui_event(&parse_jsonl_line(msg_delta_raw).unwrap(), msg_delta_raw).unwrap();
        assert_eq!(
            msg_delta_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("m3".to_string()),
                delta: Some("inc".to_string()),
                thinking_delta: None,
                thinking_phase: None,
                thinking_content: None,
                raw_json: msg_delta_raw.to_string(),
            }
        );

        // 4. BOM-prefixed line and serde_json conversion fallback
        let bom_line = "\u{feff}{\"type\":\"message_update\",\"delta\":\"ok\"}";
        let bom_val = parse_jsonl_line(bom_line).expect("must strip BOM and parse");
        assert_eq!(bom_val.get("delta").and_then(JsonValue::as_str), Some("ok"));

        let converted = serde_value_to_json_value(serde_json::json!({
            "type": "message_update",
            "delta": "fallback",
            "count": 42,
            "ok": true,
            "tags": ["a", null]
        }));
        assert_eq!(
            converted.get("delta").and_then(JsonValue::as_str),
            Some("fallback")
        );
    }

    #[test]
    fn streams_message_update_chunks_incrementally_before_next_chunk_is_written() {
        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    prompt)
      printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
      printf '{"type":"message_start","message":{"id":"ms-1","role":"assistant","content":[]}}\n'
      printf '{"type":"message_update","message":{"id":"ms-1"},"assistantMessageEvent":{"type":"text_delta","delta":"first "}}\n'
      sleep 0.25
      printf '{"type":"message_update","message":{"id":"ms-1"},"assistantMessageEvent":{"type":"text_delta","delta":"second"}}\n'
      printf '{"type":"message_end","message":{"id":"ms-1","role":"assistant","stopReason":"stop","content":[{"type":"text","text":"first second"}]}}\n'
      printf '{"type":"agent_end","messages":[]}\n'
      ;;
    *)
      printf '{"type":"response","id":"%s","command":"%s","success":true}\n' "$id" "$typ"
      ;;
  esac
done
"#;

        let (chunk_tx, chunk_rx) = mpsc::channel::<(String, Instant)>();
        let (settled_tx, settled_rx) = mpsc::channel::<()>();
        let sink: EventSink = Arc::new(move |ev| match ev {
            PiUiEvent::MessageUpdate { delta: Some(d), .. } => {
                let _ = chunk_tx.send((d, Instant::now()));
            }
            PiUiEvent::AgentSettled { .. } => {
                let _ = settled_tx.send(());
            }
            _ => {}
        });

        let sup = PiSessionSupervisor::spawn(mock_sh_config(script), sink, Arc::new(|| {}))
            .expect("spawn streaming mock");
        sup.prompt("stream test", vec![]).expect("prompt");

        let (chunk1, t1) = chunk_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("must receive chunk 1");
        let (chunk2, t2) = chunk_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("must receive chunk 2");
        settled_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("must receive agent_settled");

        assert_eq!(chunk1, "first ");
        assert_eq!(chunk2, "second");
        // Chunk 1 must arrive significantly before chunk 2 (proving line-by-line streaming with zero EOF buffering).
        let gap = t2.saturating_duration_since(t1);
        assert!(
            gap >= Duration::from_millis(120),
            "expected incremental arrival gap >= 120ms between chunk 1 and chunk 2, got {:?}",
            gap
        );

        sup.shutdown();
    }

    #[test]
    fn suppresses_non_prose_toolcall_and_lifecycle_message_updates() {
        for raw in [
            r#"{"type":"message_update","message":{"id":"m1"},"assistantMessageEvent":{"type":"toolcall_start","toolCall":{"id":"tc1","name":"bash"}}}"#,
            r#"{"type":"message_update","message":{"id":"m1"},"assistantMessageEvent":{"type":"toolcall_delta","delta":"{\"cmd\":\"ls\"}"}}"#,
            r#"{"type":"message_update","message":{"id":"m1"},"assistantMessageEvent":{"type":"toolcall_end","toolCall":{"id":"tc1"}}}"#,
            r#"{"type":"message_update","message":{"id":"m1"},"assistantMessageEvent":{"type":"text_start"}}"#,
            r#"{"type":"message_update","message":{"id":"m1"},"assistantMessageEvent":{"type":"start"}}"#,
            r#"{"type":"message_update","message":{"id":"m1"},"assistantMessageEvent":{"type":"done"}}"#,
        ] {
            let val = parse_json(raw).unwrap();
            assert!(
                parse_pi_ui_event(&val, raw).is_none(),
                "expected None for non-prose assistantMessageEvent: {}",
                raw
            );
        }

        // Unknown nested content_block_delta type (e.g. input_json_delta) must NOT leak partial JSON as prose.
        let input_json_raw = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a.rs\"}","delta":"{\"path\":\"a.rs\"}"}}"#;
        let input_json_ev =
            parse_pi_ui_event(&parse_json(input_json_raw).unwrap(), input_json_raw).unwrap();
        assert_eq!(
            input_json_ev,
            PiUiEvent::MessageUpdate {
                message_id: None,
                delta: None,
                thinking_delta: None,
                thinking_phase: None,
                thinking_content: None,
                raw_json: input_json_raw.to_string(),
            }
        );
    }

    #[test]
    fn unwraps_tool_execution_update_and_end_json_wrappers() {
        let upd_raw = r#"{"type":"tool_execution_update","toolCallId":"tc-1","toolName":"bash","partialResult":{"content":[{"type":"text","text":"line 1"},{"text":"line 2"}]}}"#;
        let upd_ev = parse_pi_ui_event(&parse_json(upd_raw).unwrap(), upd_raw).unwrap();
        assert_eq!(
            upd_ev,
            PiUiEvent::ToolExecutionUpdate {
                tool_call_id: Some("tc-1".to_string()),
                tool_name: Some("bash".to_string()),
                partial_result: Some("line 1\nline 2".to_string()),
                raw_json: upd_raw.to_string(),
            }
        );

        let end_raw = r#"{"type":"tool_execution_end","toolCallId":"tc-1","toolName":"read","isError":false,"result":{"content":[{"type":"text","text":"file body"}]}}"#;
        let end_ev = parse_pi_ui_event(&parse_json(end_raw).unwrap(), end_raw).unwrap();
        assert_eq!(
            end_ev,
            PiUiEvent::ToolExecutionEnd {
                tool_call_id: Some("tc-1".to_string()),
                tool_name: Some("read".to_string()),
                is_error: false,
                result: Some("file body".to_string()),
                raw_json: end_raw.to_string(),
            }
        );

        assert_eq!(
            extract_tool_output_text(&parse_json(r#"{"output":"clean output"}"#).unwrap()),
            Some("clean output".to_string())
        );
        assert_eq!(
            extract_tool_output_text(&parse_json(r#"{"text":"clean text"}"#).unwrap()),
            Some("clean text".to_string())
        );
    }

    #[test]
    fn parses_thinking_start_end_lifecycle_and_reasoning_keys() {
        let start_raw = r#"{"type":"message_update","message":{"id":"m-th"},"assistantMessageEvent":{"type":"thinking_start"}}"#;
        let start_ev = parse_pi_ui_event(&parse_json(start_raw).unwrap(), start_raw).unwrap();
        assert_eq!(
            start_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("m-th".to_string()),
                delta: None,
                thinking_delta: None,
                thinking_phase: Some("start".to_string()),
                thinking_content: None,
                raw_json: start_raw.to_string(),
            }
        );

        let end_raw = r#"{"type":"message_update","message":{"id":"m-th"},"assistantMessageEvent":{"type":"thinking_end","content":"Full reasoning trace"}}"#;
        let end_ev = parse_pi_ui_event(&parse_json(end_raw).unwrap(), end_raw).unwrap();
        assert_eq!(
            end_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("m-th".to_string()),
                delta: None,
                thinking_delta: None,
                thinking_phase: Some("end".to_string()),
                thinking_content: Some("Full reasoning trace".to_string()),
                raw_json: end_raw.to_string(),
            }
        );

        let reasoning_content_raw =
            r#"{"type":"message_update","messageId":"m-rc","reasoning_content":"step by step"}"#;
        let rc_ev = parse_pi_ui_event(
            &parse_json(reasoning_content_raw).unwrap(),
            reasoning_content_raw,
        )
        .unwrap();
        assert_eq!(
            rc_ev,
            PiUiEvent::MessageUpdate {
                message_id: Some("m-rc".to_string()),
                delta: None,
                thinking_delta: Some("step by step".to_string()),
                thinking_phase: None,
                thinking_content: None,
                raw_json: reasoning_content_raw.to_string(),
            }
        );
    }
}
