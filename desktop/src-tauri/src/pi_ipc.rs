use crate::attachments::PendingAttachment;
use crate::extension_ui::{DialogId, ExtensionUiAnswer};
use crate::pi_session::{
    EventSink, PiRpcResponse, PiSessionConfig, PiSessionSupervisor, PiUiEvent, ReloadDisposition,
    ReloadHook,
};
use crate::projects::{
    default_projects_store_path, default_session_meta_path, delete_session_file,
    load_archived_session_ids_from, load_or_init_projects, merge_live_and_disk_sessions,
    project_from_path, scan_project_sessions_with_archived, set_session_archived_at,
    ProjectRegistry,
};
pub use crate::projects::{PiProject, PiSessionSummary};
use crate::providers::{
    build_pi_provider_env, default_store_path, ensure_verity_pi_extension_path, ProviderStore,
    PublicProviderProfile,
};
use crate::snapshots::{
    capture_snapshot, clear_last_redo, find_checkpoint, load_checkpoint_store, load_last_redo,
    record_turn_checkpoint, redo_targeted_revert, revert_to_snapshot, save_last_redo,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Runtime, State};
use verity_core::json::{parse_json, JsonValue};

/// Tauri event name emitted for every Pi stdout event.
pub const PI_EVENT_NAME: &str = "pi:event";

// ============================================================
// IPC Domain Types (matching desktop/ui/src/api/tauri.ts)
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

impl ThinkingLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::Xhigh),
            _ => None,
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Self::Off,
            Self::Minimal,
            Self::Low,
            Self::Medium,
            Self::High,
            Self::Xhigh,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub provider: String,
    pub model_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashOrSkillCommand {
    pub trigger: String,
    pub name: String,
    pub description: String,
    pub insert_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiCommandInfo {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiToolCallState {
    pub id: String,
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<PendingAttachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<PiToolCallState>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTreeNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub preview: String,
    pub timestamp: String,
    pub depth: usize,
    pub is_active_branch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkCandidate {
    pub entry_id: String,
    pub role: String,
    pub preview: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWithMessages {
    pub session: PiSessionSummary,
    pub messages: Vec<PiChatMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportHtmlResult {
    pub path: String,
    pub html: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCheckpointView {
    pub entry_id: String,
    pub prompt_preview: String,
    pub has_code_snapshot: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertTurnResult {
    pub session: PiSessionSummary,
    pub messages: Vec<PiChatMessage>,
    pub restored_prompt: Option<String>,
    pub code_reverted: bool,
    pub reverted_files: Vec<String>,
    pub can_redo: bool,
    pub code_revert_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedoRevertResult {
    pub restored_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiBootstrapData {
    pub projects: Vec<PiProject>,
    pub sessions: Vec<PiSessionSummary>,
    pub active_project_id: String,
    pub active_project_path: String,
    pub active_session_id: String,
    pub messages: Vec<PiChatMessage>,
    pub models: Vec<ModelOption>,
    pub selected_model: ModelOption,
    pub thinking_levels: Vec<ThinkingLevel>,
    pub thinking_level: ThinkingLevel,
    pub commands: Vec<SlashOrSkillCommand>,
    pub providers: Vec<PublicProviderProfile>,
}

// ============================================================
// Launch Env & MCP Binary Resolution (Pure Boundary Helpers)
// ============================================================

/// Resolves the local `verity` CLI binary for `verity mcp` launch wiring.
pub fn resolve_verity_mcp_bin(project_path: &Path) -> Option<PathBuf> {
    if let Ok(explicit) = env::var("VERITY_MCP_BIN") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    let bin_name = if cfg!(windows) {
        "verity.exe"
    } else {
        "verity"
    };

    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(bin_name);
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }

    for rel in [
        Path::new("target").join("release").join(bin_name),
        Path::new("target").join("debug").join(bin_name),
    ] {
        let candidate = project_path.join(rel);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            let candidate = dir.join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Builds the full child-process environment for Pi:
/// - `VERITY_DESKTOP_PROVIDERS` + `VERITY_DESKTOP_KEY_<ID>` from the `0o600` provider store
/// - `VERITY_MCP_BIN` + `VERITY_MCP_ARGS=mcp` when a local `verity` binary is available
pub fn build_pi_launch_env(
    store: &ProviderStore,
    verity_mcp_bin: Option<&Path>,
) -> Vec<(String, String)> {
    let mut env_pairs = build_pi_provider_env(store);
    if let Some(bin) = verity_mcp_bin {
        env_pairs.push((
            "VERITY_MCP_BIN".to_string(),
            bin.to_string_lossy().to_string(),
        ));
        env_pairs.push(("VERITY_MCP_ARGS".to_string(), "mcp".to_string()));
    }
    env_pairs
}

/// Builds the default Pi RPC session configuration, attaching `--extension <path>`
/// when the Verity provider registration extension is available.
pub fn build_default_pi_config(
    project_path: PathBuf,
    extension_path: Option<&Path>,
) -> PiSessionConfig {
    let cfg = PiSessionConfig::default_pi(project_path);
    match extension_path {
        Some(path) => cfg.with_extension(path),
        None => cfg,
    }
}

/// Validates an external URL at the boundary, allowing only `http://` or `https://`
/// with a non-empty host and no whitespace or control characters.
pub fn validate_external_open_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("external URL must not be empty".to_string());
    }
    if trimmed
        .bytes()
        .any(|b| b < 0x20 || b == 0x7f || b.is_ascii_whitespace())
    {
        return Err("external URL must not contain whitespace or control characters".to_string());
    }
    let rest = if let Some(r) = trimmed.strip_prefix("https://") {
        r
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        r
    } else {
        return Err("external URL must use http:// or https://".to_string());
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Err("external URL must include a non-empty host".to_string());
    }
    Ok(trimmed.to_string())
}

fn launch_os_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("failed to launch browser: {}", e))?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("failed to launch browser: {}", e))?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("failed to launch browser: {}", e))?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = url;
        Err("opening external URLs is not supported on this platform".to_string())
    }
}

// ============================================================
// Pure RPC Response Boundary Parsers
// ============================================================

fn parse_data_json(resp: &PiRpcResponse) -> Option<JsonValue> {
    resp.data_json
        .as_deref()
        .and_then(|raw| parse_json(raw).ok())
}

fn opt_str(val: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| val.get(k).and_then(JsonValue::as_str))
        .map(ToString::to_string)
}

fn opt_timestamp(val: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match val.get(k) {
        Some(JsonValue::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(JsonValue::Number(n)) if n.is_finite() && *n >= 0.0 => Some((*n as u64).to_string()),
        _ => None,
    })
}

fn opt_usize(val: &JsonValue, keys: &[&str]) -> Option<usize> {
    keys.iter().find_map(|k| match val.get(k) {
        Some(JsonValue::Number(n)) if *n >= 0.0 && n.is_finite() => Some(*n as usize),
        _ => None,
    })
}

fn opt_bool(val: &JsonValue, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|k| val.get(k).and_then(JsonValue::as_bool))
}

fn normalize_role(raw: Option<String>) -> String {
    match raw.as_deref() {
        Some("user") => "user".to_string(),
        Some("system") => "system".to_string(),
        _ => "assistant".to_string(),
    }
}

fn extract_message_content(val: &JsonValue) -> String {
    match val.get("content") {
        Some(JsonValue::String(s)) => s.clone(),
        Some(JsonValue::Array(blocks)) => {
            let mut parts = Vec::new();
            for block in blocks {
                if let Some(text) = block.as_str() {
                    parts.push(text.to_string());
                } else if let Some(text) = opt_str(block, &["text", "content"]) {
                    parts.push(text);
                }
            }
            parts.join("\n")
        }
        _ => opt_str(val, &["text", "message"]).unwrap_or_default(),
    }
}

fn parse_tool_calls(val: &JsonValue) -> Option<Vec<PiToolCallState>> {
    let arr = val
        .get("toolCalls")
        .or_else(|| val.get("tool_calls"))
        .and_then(JsonValue::as_array)?;
    let parsed: Vec<PiToolCallState> = arr
        .iter()
        .enumerate()
        .map(|(idx, item)| PiToolCallState {
            id: opt_str(item, &["id", "toolCallId", "tool_call_id"])
                .unwrap_or_else(|| format!("tc-{}", idx + 1)),
            name: opt_str(item, &["name", "toolName", "tool_name"])
                .unwrap_or_else(|| "tool".to_string()),
            status: opt_str(item, &["status"]).unwrap_or_else(|| "done".to_string()),
            detail: opt_str(item, &["detail", "output", "error"]),
        })
        .collect();
    if parsed.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

pub fn parse_chat_messages(val: &JsonValue) -> Vec<PiChatMessage> {
    let items = match val {
        JsonValue::Array(arr) => arr,
        JsonValue::Object(_) => match val.get("messages").and_then(JsonValue::as_array) {
            Some(arr) => arr,
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };

    items
        .iter()
        .enumerate()
        .map(|(idx, item)| PiChatMessage {
            id: opt_str(item, &["id", "messageId", "message_id"])
                .unwrap_or_else(|| format!("msg-{}", idx + 1)),
            role: normalize_role(opt_str(item, &["role"])),
            content: extract_message_content(item),
            timestamp: opt_timestamp(item, &["timestamp", "createdAt", "created_at"])
                .unwrap_or_else(|| "now".to_string()),
            streaming: opt_bool(item, &["streaming"]),
            attachments: None,
            tool_calls: parse_tool_calls(item),
        })
        .collect()
}

pub fn parse_session_summary(
    val: &JsonValue,
    fallback_project_id: &str,
    fallback_id: &str,
    fallback_name: &str,
) -> PiSessionSummary {
    let obj = val.get("session").unwrap_or(val);
    let id =
        opt_str(obj, &["id", "sessionId", "session_id"]).unwrap_or_else(|| fallback_id.to_string());
    let project_id = opt_str(obj, &["projectId", "project_id"])
        .unwrap_or_else(|| fallback_project_id.to_string());
    let name = opt_str(obj, &["name", "sessionName", "session_name", "title"])
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| fallback_name.to_string());
    let path = opt_str(
        obj,
        &[
            "path",
            "sessionPath",
            "session_path",
            "sessionFile",
            "session_file",
        ],
    )
    .unwrap_or_default();
    let updated_at = opt_timestamp(obj, &["updatedAt", "updated_at", "timestamp"])
        .unwrap_or_else(|| "Just now".to_string());
    let message_count = opt_usize(obj, &["messageCount", "message_count"]).unwrap_or(0);
    let parent_session_id = opt_str(obj, &["parentSessionId", "parent_session_id"]);

    let archived = opt_bool(obj, &["archived"]).unwrap_or(false);

    PiSessionSummary {
        id,
        project_id,
        name,
        path,
        updated_at,
        message_count,
        parent_session_id,
        archived,
    }
}

pub fn parse_history_tree(val: &JsonValue) -> Vec<HistoryTreeNode> {
    let items = match val {
        JsonValue::Array(arr) => arr,
        JsonValue::Object(_) => match val
            .get("nodes")
            .or_else(|| val.get("tree"))
            .or_else(|| val.get("entries"))
            .and_then(JsonValue::as_array)
        {
            Some(arr) => arr,
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };

    items
        .iter()
        .enumerate()
        .map(|(idx, item)| HistoryTreeNode {
            id: opt_str(item, &["id", "entryId", "entry_id"])
                .unwrap_or_else(|| format!("node-{}", idx + 1)),
            parent_id: opt_str(item, &["parentId", "parent_id"]),
            role: normalize_role(opt_str(item, &["role"])),
            preview: opt_str(item, &["preview", "text", "content"]).unwrap_or_default(),
            timestamp: opt_timestamp(item, &["timestamp", "createdAt", "created_at"])
                .unwrap_or_else(|| "now".to_string()),
            depth: opt_usize(item, &["depth"]).unwrap_or(0),
            is_active_branch: opt_bool(item, &["isActiveBranch", "is_active_branch"])
                .unwrap_or(true),
        })
        .collect()
}

pub fn parse_fork_candidates(val: &JsonValue) -> Vec<ForkCandidate> {
    let items = match val {
        JsonValue::Array(arr) => arr,
        JsonValue::Object(_) => match val
            .get("messages")
            .or_else(|| val.get("candidates"))
            .or_else(|| val.get("entries"))
            .and_then(JsonValue::as_array)
        {
            Some(arr) => arr,
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };

    items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let role = match opt_str(item, &["role"]).as_deref() {
                Some("assistant") => "assistant".to_string(),
                _ => "user".to_string(),
            };
            ForkCandidate {
                entry_id: opt_str(item, &["entryId", "entry_id", "id"])
                    .unwrap_or_else(|| format!("entry-{}", idx + 1)),
                role,
                preview: opt_str(item, &["preview", "text", "content"]).unwrap_or_default(),
                timestamp: opt_timestamp(item, &["timestamp", "createdAt", "created_at"])
                    .unwrap_or_else(|| "now".to_string()),
            }
        })
        .collect()
}

pub fn parse_model_option(
    val: &JsonValue,
    fallback_provider: &str,
    fallback_model_id: &str,
) -> ModelOption {
    let obj = val.get("model").unwrap_or(val);
    let provider = opt_str(obj, &["provider"]).unwrap_or_else(|| fallback_provider.to_string());
    let model_id = opt_str(obj, &["modelId", "model_id", "id"])
        .unwrap_or_else(|| fallback_model_id.to_string());
    let label = opt_str(obj, &["label", "name"]).unwrap_or_else(|| model_id.clone());
    ModelOption {
        provider,
        model_id,
        label,
    }
}

pub fn try_parse_model_option(val: &JsonValue) -> Option<ModelOption> {
    let obj = val
        .get("model")
        .filter(|v| matches!(v, JsonValue::Object(_)))
        .unwrap_or(val);
    if !matches!(obj, JsonValue::Object(_)) {
        return None;
    }
    let model_id = opt_str(obj, &["modelId", "model_id", "id"])?;
    let provider = opt_str(obj, &["provider"]).unwrap_or_else(|| "default".to_string());
    let label = opt_str(obj, &["label", "name"]).unwrap_or_else(|| model_id.clone());
    Some(ModelOption {
        provider,
        model_id,
        label,
    })
}

pub fn parse_models_list(val: &JsonValue) -> Vec<ModelOption> {
    let items = match val {
        JsonValue::Array(arr) => arr,
        JsonValue::Object(_) => match val.get("models").and_then(JsonValue::as_array) {
            Some(arr) => arr,
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };

    items
        .iter()
        .filter_map(|item| {
            let model_id = opt_str(item, &["modelId", "model_id", "id"])?;
            let provider = opt_str(item, &["provider"]).unwrap_or_else(|| "default".to_string());
            let label = opt_str(item, &["label", "name"]).unwrap_or_else(|| model_id.clone());
            Some(ModelOption {
                provider,
                model_id,
                label,
            })
        })
        .collect()
}

pub fn parse_thinking_levels(val: &JsonValue) -> Vec<ThinkingLevel> {
    let items = match val {
        JsonValue::Array(arr) => arr,
        JsonValue::Object(_) => match val
            .get("levels")
            .or_else(|| val.get("thinkingLevels"))
            .or_else(|| val.get("thinking_levels"))
            .and_then(JsonValue::as_array)
        {
            Some(arr) => arr,
            None => return ThinkingLevel::all(),
        },
        _ => return ThinkingLevel::all(),
    };

    let parsed: Vec<ThinkingLevel> = items
        .iter()
        .filter_map(|v| v.as_str().and_then(ThinkingLevel::parse))
        .collect();
    if parsed.is_empty() {
        ThinkingLevel::all()
    } else {
        parsed
    }
}

pub fn parse_commands_list(val: &JsonValue) -> Vec<SlashOrSkillCommand> {
    let items = match val {
        JsonValue::Array(arr) => arr,
        JsonValue::Object(_) => match val.get("commands").and_then(JsonValue::as_array) {
            Some(arr) => arr,
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };

    items
        .iter()
        .filter_map(|item| {
            let name = opt_str(item, &["name"])?;
            let trigger = match opt_str(item, &["trigger"]).as_deref() {
                Some("$") => "$".to_string(),
                _ => "/".to_string(),
            };
            let description = opt_str(item, &["description"]).unwrap_or_default();
            let insert_text = opt_str(item, &["insertText", "insert_text"])
                .unwrap_or_else(|| format!("{}{} ", trigger, name));
            let source = opt_str(item, &["source"]);
            let location = opt_str(item, &["location", "path"]);
            Some(SlashOrSkillCommand {
                trigger,
                name,
                description,
                insert_text,
                source,
                location,
            })
        })
        .collect()
}

pub fn parse_command_info_list(val: &JsonValue) -> Vec<PiCommandInfo> {
    parse_commands_list(val)
        .into_iter()
        .map(|cmd| PiCommandInfo {
            name: cmd.name,
            description: cmd.description,
            source: cmd.source,
            location: cmd.location,
        })
        .collect()
}

fn rpc_response_to_serde_value(resp: &PiRpcResponse) -> serde_json::Value {
    resp.data_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn validate_queue_mode(field: &str, mode: &str) -> Result<String, String> {
    let trimmed = mode.trim();
    match trimmed {
        "all" | "one-at-a-time" => Ok(trimmed.to_string()),
        other => Err(format!(
            "{} must be 'all' or 'one-at-a-time' (got '{}')",
            field, other
        )),
    }
}

pub fn parse_export_html(val: &JsonValue, fallback_path: Option<&str>) -> ExportHtmlResult {
    let path = opt_str(val, &["path", "outputPath", "output_path"])
        .or_else(|| fallback_path.map(ToString::to_string))
        .unwrap_or_default();
    let html = opt_str(val, &["html", "content"]).unwrap_or_else(|| {
        if !path.is_empty() {
            fs::read_to_string(&path).unwrap_or_default()
        } else {
            String::new()
        }
    });
    ExportHtmlResult { path, html }
}

fn models_from_providers(providers: &[PublicProviderProfile]) -> Vec<ModelOption> {
    let mut out = Vec::new();
    for p in providers {
        for m in &p.models {
            out.push(ModelOption {
                provider: p.id.clone(),
                model_id: m.clone(),
                label: m.clone(),
            });
        }
    }
    out
}

/// Built-in fallback catalog used whenever live Pi RPC, custom `ProviderStore`,
/// and `~/.pi/agent/models.json` yield no models so the picker is never empty or `"default"`.
pub fn default_catalog_models() -> Vec<ModelOption> {
    vec![
        ModelOption {
            provider: "anthropic".to_string(),
            model_id: "claude-sonnet-5-5".to_string(),
            label: "Claude Sonnet 5.5".to_string(),
        },
        ModelOption {
            provider: "anthropic".to_string(),
            model_id: "claude-opus-5".to_string(),
            label: "Claude Opus 5".to_string(),
        },
        ModelOption {
            provider: "openai".to_string(),
            model_id: "gpt-6.1-sol".to_string(),
            label: "GPT-6.1 Sol".to_string(),
        },
        ModelOption {
            provider: "google".to_string(),
            model_id: "gemini-2.5-pro".to_string(),
            label: "Gemini 2.5 Pro".to_string(),
        },
    ]
}

/// Pure boundary parser for `~/.pi/agent/models.json` supporting both Pi's
/// `{"providers": {"<provider>": {"models": [...]}}}` map and top-level `models` arrays.
pub fn parse_pi_agent_models(val: &JsonValue) -> Vec<ModelOption> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    let mut push_unique = |provider: String, model_id: String, label: String| {
        let p = provider.trim();
        let m = model_id.trim();
        if p.is_empty() || m.is_empty() {
            return;
        }
        let key = (p.to_string(), m.to_string());
        if seen.insert(key) {
            let lbl = if label.trim().is_empty() {
                m.to_string()
            } else {
                label.trim().to_string()
            };
            out.push(ModelOption {
                provider: p.to_string(),
                model_id: m.to_string(),
                label: lbl,
            });
        }
    };

    if let Some(JsonValue::Object(providers)) = val.get("providers") {
        for (provider_id, provider_cfg) in providers {
            if let Some(models) = provider_cfg.get("models").and_then(JsonValue::as_array) {
                for item in models {
                    match item {
                        JsonValue::String(id) => {
                            push_unique(provider_id.clone(), id.clone(), id.clone());
                        }
                        JsonValue::Object(_) => {
                            if let Some(model_id) = opt_str(item, &["id", "modelId", "model_id"]) {
                                let provider = opt_str(item, &["provider"])
                                    .unwrap_or_else(|| provider_id.clone());
                                let label = opt_str(item, &["label", "name"])
                                    .unwrap_or_else(|| model_id.clone());
                                push_unique(provider, model_id, label);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    for model in parse_models_list(val) {
        push_unique(model.provider, model.model_id, model.label);
    }

    out
}

/// Reads and parses a Pi agent `models.json` file from an explicit path.
pub fn load_pi_agent_models_from_path(path: &Path) -> Vec<ModelOption> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(val) = parse_json(&raw) else {
        return Vec::new();
    };
    parse_pi_agent_models(&val)
}

/// Optionally reads `$PI_CODING_AGENT_DIR/models.json` (or `~/.pi/agent/models.json`)
/// and extracts configured models cleanly.
pub fn load_pi_agent_models() -> Vec<ModelOption> {
    let models_path = env::var("PI_CODING_AGENT_DIR")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|dir| PathBuf::from(dir).join("models.json"))
        .or_else(|| {
            env::var("HOME")
                .ok()
                .or_else(|| env::var("USERPROFILE").ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|home| {
                    PathBuf::from(home)
                        .join(".pi")
                        .join("agent")
                        .join("models.json")
                })
        });

    match models_path {
        Some(path) => load_pi_agent_models_from_path(&path),
        None => Vec::new(),
    }
}

/// Merges RPC, custom provider store, and Pi agent models deduplicated by `(provider, model_id)`.
/// Falls back to `default_catalog_models()` if all sources yield no models.
pub fn merge_models_with_fallback(
    rpc_models: Vec<ModelOption>,
    provider_models: Vec<ModelOption>,
    agent_models: Vec<ModelOption>,
) -> Vec<ModelOption> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut merged = Vec::new();
    for model in rpc_models
        .into_iter()
        .chain(provider_models)
        .chain(agent_models)
    {
        let p = model.provider.trim();
        let m = model.model_id.trim();
        if p.is_empty() || m.is_empty() {
            continue;
        }
        let key = (p.to_string(), m.to_string());
        if seen.insert(key) {
            merged.push(model);
        }
    }
    if merged.is_empty() {
        default_catalog_models()
    } else {
        merged
    }
}

fn resolve_model_label(provider: &str, model_id: &str) -> String {
    default_catalog_models()
        .into_iter()
        .find(|m| m.provider == provider && m.model_id == model_id)
        .map(|m| m.label)
        .unwrap_or_else(|| model_id.to_string())
}

fn fallback_model(models: &[ModelOption]) -> ModelOption {
    models.first().cloned().unwrap_or_else(|| {
        default_catalog_models()
            .into_iter()
            .next()
            .expect("default catalog must not be empty")
    })
}

fn require_non_empty(field: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{} must not be empty", field));
    }
    Ok(trimmed.to_string())
}

// ============================================================
// Managed Tauri Application State
// ============================================================

struct PiAppStateInner {
    supervisor: Mutex<Option<Arc<PiSessionSupervisor>>>,
    default_cwd: PathBuf,
    projects_store_path: PathBuf,
    session_meta_path: PathBuf,
    registry: Mutex<ProjectRegistry>,
    provider_store_path: PathBuf,
    base_config: Option<PiSessionConfig>,
    selected_model: Mutex<Option<ModelOption>>,
    thinking_level: Mutex<ThinkingLevel>,
    model_pending_sync: AtomicBool,
    thinking_level_pending_sync: AtomicBool,
    provider_reload_pending: Arc<AtomicBool>,
    active_session_id: Mutex<Option<String>>,
}

impl PiAppStateInner {
    fn shutdown(&self) {
        let taken = {
            let mut guard = self.supervisor.lock().expect("supervisor lock poisoned");
            guard.take()
        };
        if let Some(sup) = taken {
            sup.shutdown();
        }
    }
}

impl Drop for PiAppStateInner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Managed Tauri v2 state owning the long-lived `PiSessionSupervisor` lifecycle.
/// Cheaply cloneable (`Arc`) so async Tauri commands can offload blocking IPC
/// work onto `tauri::async_runtime::spawn_blocking`.
#[derive(Clone)]
pub struct PiAppState {
    inner: Arc<PiAppStateInner>,
}

impl PiAppState {
    /// Creates default desktop state using the persisted project registry and standard provider store.
    pub fn new_default() -> Self {
        let default_cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let projects_store_path = default_projects_store_path();
        let session_meta_path = default_session_meta_path();
        let registry = load_or_init_projects(&projects_store_path, &default_cwd);
        Self {
            inner: Arc::new(PiAppStateInner {
                supervisor: Mutex::new(None),
                default_cwd,
                projects_store_path,
                session_meta_path,
                registry: Mutex::new(registry),
                provider_store_path: default_store_path(),
                base_config: None,
                selected_model: Mutex::new(None),
                thinking_level: Mutex::new(ThinkingLevel::Medium),
                model_pending_sync: AtomicBool::new(false),
                thinking_level_pending_sync: AtomicBool::new(false),
                provider_reload_pending: Arc::new(AtomicBool::new(false)),
                active_session_id: Mutex::new(None),
            }),
        }
    }

    /// Creates state with explicit paths and base `PiSessionConfig` (used for deterministic tests).
    pub fn with_config(
        project_path: PathBuf,
        provider_store_path: PathBuf,
        base_config: PiSessionConfig,
    ) -> Self {
        let projects_store_path = provider_store_path.with_file_name("projects.json");
        let session_meta_path = provider_store_path.with_file_name("session-meta.json");
        let registry = load_or_init_projects(&projects_store_path, &project_path);
        Self {
            inner: Arc::new(PiAppStateInner {
                supervisor: Mutex::new(None),
                default_cwd: project_path,
                projects_store_path,
                session_meta_path,
                registry: Mutex::new(registry),
                provider_store_path,
                base_config: Some(base_config),
                selected_model: Mutex::new(None),
                thinking_level: Mutex::new(ThinkingLevel::Medium),
                model_pending_sync: AtomicBool::new(false),
                thinking_level_pending_sync: AtomicBool::new(false),
                provider_reload_pending: Arc::new(AtomicBool::new(false)),
                active_session_id: Mutex::new(None),
            }),
        }
    }

    /// Returns the active project path.
    pub fn project_path(&self) -> PathBuf {
        let guard = self.inner.registry.lock().expect("registry lock poisoned");
        guard
            .active_project()
            .map(|p| PathBuf::from(&p.path))
            .unwrap_or_else(|| self.inner.default_cwd.clone())
    }

    fn snapshot_projects(&self) -> (Vec<PiProject>, PiProject) {
        let guard = self.inner.registry.lock().expect("registry lock poisoned");
        if guard.projects.is_empty() {
            let fallback = project_from_path(&self.inner.default_cwd);
            return (vec![fallback.clone()], fallback);
        }
        let active = guard
            .active_project()
            .cloned()
            .unwrap_or_else(|| guard.projects[0].clone());
        (guard.projects.clone(), active)
    }

    /// Lazily spawns (or reuses an existing live) `PiSessionSupervisor` wired to `app.emit("pi:event", ...)`.
    pub fn ensure_supervisor<R: Runtime>(
        &self,
        app: &AppHandle<R>,
    ) -> Result<Arc<PiSessionSupervisor>, String> {
        let app_handle = app.clone();
        let sink: EventSink = Arc::new(move |event| {
            let _ = app_handle.emit(PI_EVENT_NAME, event);
        });
        self.ensure_supervisor_with_sink(sink)
    }

    /// Notifies the running Pi supervisor that `providers.json` changed so Pi
    /// restarts with updated `VERITY_DESKTOP_PROVIDERS` and `VERITY_DESKTOP_KEY_<ID>`
    /// environment variables once idle (or immediately if already idle).
    pub fn notify_providers_changed(&self) {
        let live = {
            let guard = self
                .inner
                .supervisor
                .lock()
                .expect("supervisor lock poisoned");
            guard.as_ref().filter(|s| s.is_alive()).cloned()
        };
        let Some(sup) = live else {
            return;
        };
        match sup.request_provider_reload() {
            Ok(ReloadDisposition::ExecutedImmediately) => {
                self.inner
                    .provider_reload_pending
                    .store(false, Ordering::Release);
                self.shutdown();
            }
            Ok(ReloadDisposition::DeferredUntilSettled) => {
                // `reload_hook` will set `provider_reload_pending` once `agent_settled` or `abort` arrives.
            }
            Err(_) => {
                self.inner
                    .provider_reload_pending
                    .store(true, Ordering::Release);
            }
        }
    }

    /// Idempotently returns a live `PiSessionSupervisor`, reconciling any exited/stale instance
    /// and injecting `0o600` provider env vars + `VERITY_MCP_BIN` into the child process.
    /// Always swaps out and shuts down any replaced supervisor outside the mutex.
    pub fn ensure_supervisor_with_sink(
        &self,
        event_sink: EventSink,
    ) -> Result<Arc<PiSessionSupervisor>, String> {
        let to_shutdown = {
            let mut guard = self
                .inner
                .supervisor
                .lock()
                .expect("supervisor lock poisoned");
            if let Some(existing) = guard.as_ref() {
                let should_recycle = self.inner.provider_reload_pending.load(Ordering::Acquire)
                    && existing.activity().is_idle();
                if existing.is_alive() && !should_recycle {
                    return Ok(Arc::clone(existing));
                }
                if should_recycle {
                    self.inner
                        .provider_reload_pending
                        .store(false, Ordering::Release);
                }
                guard.take()
            } else {
                None
            }
        };

        if let Some(old) = to_shutdown {
            old.shutdown();
        }

        let project_path = self.project_path();
        let store = ProviderStore::load(&self.inner.provider_store_path).unwrap_or_default();
        let verity_bin = resolve_verity_mcp_bin(&project_path);
        let launch_env = build_pi_launch_env(&store, verity_bin.as_deref());

        let mut config = match &self.inner.base_config {
            Some(cfg) => {
                let mut cloned = cfg.clone();
                cloned.cwd = project_path;
                cloned
            }
            None => {
                let ext_path = ensure_verity_pi_extension_path().ok();
                build_default_pi_config(project_path, ext_path.as_deref())
            }
        };
        config.env.extend(launch_env);

        let pending_flag = Arc::clone(&self.inner.provider_reload_pending);
        let reload_hook: ReloadHook = Arc::new(move || {
            pending_flag.store(true, Ordering::Release);
        });
        let spawned = PiSessionSupervisor::spawn(config, event_sink, reload_hook)
            .map_err(|e| e.to_string())?;
        let arc = Arc::new(spawned);

        let loser = {
            let mut guard = self
                .inner
                .supervisor
                .lock()
                .expect("supervisor lock poisoned");
            if let Some(existing) = guard.as_ref() {
                if existing.is_alive() {
                    let winner = Arc::clone(existing);
                    drop(guard);
                    arc.shutdown();
                    return Ok(winner);
                }
            }
            guard.replace(Arc::clone(&arc))
        };
        if let Some(old) = loser {
            old.shutdown();
        }

        self.replay_pending_preferences(&arc);
        Ok(arc)
    }

    /// Replays any model or thinking-level selections made while Pi was unavailable
    /// so a newly started supervisor converges idempotently to `PiAppState`.
    fn replay_pending_preferences(&self, sup: &PiSessionSupervisor) {
        if self.inner.model_pending_sync.load(Ordering::Acquire) {
            let pending_model = self
                .inner
                .selected_model
                .lock()
                .expect("selected_model lock poisoned")
                .clone();
            if let Some(model) = pending_model {
                if let Ok(resp) = sup.set_model(&model.provider, &model.model_id) {
                    let confirmed = parse_data_json(&resp)
                        .map(|v| parse_model_option(&v, &model.provider, &model.model_id))
                        .unwrap_or(model);
                    *self
                        .inner
                        .selected_model
                        .lock()
                        .expect("selected_model lock poisoned") = Some(confirmed);
                    self.inner
                        .model_pending_sync
                        .store(false, Ordering::Release);
                }
            } else {
                self.inner
                    .model_pending_sync
                    .store(false, Ordering::Release);
            }
        }

        if self
            .inner
            .thinking_level_pending_sync
            .load(Ordering::Acquire)
        {
            let pending_level = *self
                .inner
                .thinking_level
                .lock()
                .expect("thinking_level lock poisoned");
            if let Ok(resp) = sup.set_thinking_level(pending_level.as_str()) {
                let confirmed = parse_data_json(&resp)
                    .and_then(|v| opt_str(&v, &["level", "thinkingLevel", "thinking_level"]))
                    .and_then(|raw| ThinkingLevel::parse(&raw))
                    .unwrap_or(pending_level);
                *self
                    .inner
                    .thinking_level
                    .lock()
                    .expect("thinking_level lock poisoned") = confirmed;
                self.inner
                    .thinking_level_pending_sync
                    .store(false, Ordering::Release);
            }
        }
    }

    /// Idempotently shuts down any running `PiSessionSupervisor` (safe to call on `RunEvent::Exit` and `Drop`).
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }

    /// Returns whether a live supervisor is currently active.
    pub fn has_live_supervisor(&self) -> bool {
        let guard = self
            .inner
            .supervisor
            .lock()
            .expect("supervisor lock poisoned");
        guard.as_ref().is_some_and(|s| s.is_alive())
    }

    // --- Pure/Testable IPC Operation Implementations ---

    fn scan_all_project_sessions(
        &self,
        projects: &[PiProject],
        archived_ids: &HashSet<String>,
    ) -> Vec<PiSessionSummary> {
        let mut all = Vec::new();
        for proj in projects {
            let mut proj_sessions =
                scan_project_sessions_with_archived(&proj.id, Path::new(&proj.path), archived_ids);
            all.append(&mut proj_sessions);
        }
        all.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.path.cmp(&a.path))
        });
        all
    }

    pub fn bootstrap_with_sink(&self, event_sink: EventSink) -> PiBootstrapData {
        let (projects, project) = self.snapshot_projects();
        let archived_ids = load_archived_session_ids_from(&self.inner.session_meta_path);
        let disk_sessions = self.scan_all_project_sessions(&projects, &archived_ids);
        let store = ProviderStore::load(&self.inner.provider_store_path).unwrap_or_default();
        let providers = store.public_profiles();
        let provider_models = models_from_providers(&providers);
        let agent_models = load_pi_agent_models();

        let Ok(sup) = self.ensure_supervisor_with_sink(event_sink) else {
            let models = merge_models_with_fallback(Vec::new(), provider_models, agent_models);
            let stored_model = self
                .inner
                .selected_model
                .lock()
                .expect("selected_model lock poisoned")
                .clone();
            let selected_model = stored_model.unwrap_or_else(|| fallback_model(&models));
            let thinking_level = *self
                .inner
                .thinking_level
                .lock()
                .expect("thinking_level lock poisoned");
            let active_session_id = disk_sessions
                .iter()
                .find(|s| s.project_id == project.id && !s.archived)
                .or_else(|| disk_sessions.iter().find(|s| s.project_id == project.id))
                .or_else(|| disk_sessions.first())
                .map(|s| s.id.clone())
                .unwrap_or_default();
            return PiBootstrapData {
                active_project_id: project.id.clone(),
                active_project_path: project.path.clone(),
                projects,
                sessions: disk_sessions,
                active_session_id,
                messages: Vec::new(),
                models,
                selected_model,
                thinking_levels: ThinkingLevel::all(),
                thinking_level,
                commands: Vec::new(),
                providers,
            };
        };

        self.replay_pending_preferences(&sup);

        let state_val = sup.get_state().ok().and_then(|r| parse_data_json(&r));
        let messages = sup
            .get_messages()
            .ok()
            .and_then(|r| parse_data_json(&r))
            .map(|v| parse_chat_messages(&v))
            .unwrap_or_default();

        let rpc_models = sup
            .get_available_models()
            .ok()
            .and_then(|r| parse_data_json(&r))
            .map(|v| parse_models_list(&v))
            .unwrap_or_default();
        let models = merge_models_with_fallback(rpc_models, provider_models, agent_models);

        let thinking_levels = sup
            .get_available_thinking_levels()
            .ok()
            .and_then(|r| parse_data_json(&r))
            .map(|v| parse_thinking_levels(&v))
            .unwrap_or_else(ThinkingLevel::all);

        let commands = sup
            .get_commands()
            .ok()
            .and_then(|r| parse_data_json(&r))
            .map(|v| parse_commands_list(&v))
            .unwrap_or_default();

        let stored_model = self
            .inner
            .selected_model
            .lock()
            .expect("selected_model lock poisoned")
            .clone();
        let default_model = stored_model.unwrap_or_else(|| fallback_model(&models));
        let selected_model = state_val
            .as_ref()
            .and_then(|v| v.get("model"))
            .map(|m| parse_model_option(m, &default_model.provider, &default_model.model_id))
            .unwrap_or(default_model);
        *self
            .inner
            .selected_model
            .lock()
            .expect("selected_model lock poisoned") = Some(selected_model.clone());

        let stored_thinking = *self
            .inner
            .thinking_level
            .lock()
            .expect("thinking_level lock poisoned");
        let thinking_level = state_val
            .as_ref()
            .and_then(|v| opt_str(v, &["thinkingLevel", "thinking_level"]))
            .and_then(|raw| ThinkingLevel::parse(&raw))
            .unwrap_or(stored_thinking);
        *self
            .inner
            .thinking_level
            .lock()
            .expect("thinking_level lock poisoned") = thinking_level;

        let mut session = state_val
            .as_ref()
            .map(|v| parse_session_summary(v, &project.id, "session-1", "Current session"))
            .unwrap_or_else(|| PiSessionSummary {
                id: "session-1".to_string(),
                project_id: project.id.clone(),
                name: "Current session".to_string(),
                path: String::new(),
                updated_at: "Just now".to_string(),
                message_count: messages.len(),
                parent_session_id: None,
                archived: false,
            });
        session.archived = archived_ids.contains(&session.id);

        let active_session_id = session.id.clone();
        *self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned") = Some(active_session_id.clone());
        let sessions = merge_live_and_disk_sessions(disk_sessions, Some(session));
        PiBootstrapData {
            active_project_id: project.id.clone(),
            active_project_path: project.path.clone(),
            projects,
            sessions,
            active_session_id,
            messages,
            models,
            selected_model,
            thinking_levels,
            thinking_level,
            commands,
            providers,
        }
    }

    pub fn add_project_with_sink(
        &self,
        event_sink: EventSink,
        path: String,
    ) -> Result<PiBootstrapData, String> {
        let trimmed = require_non_empty("path", &path)?;
        {
            let mut reg = self.inner.registry.lock().expect("registry lock poisoned");
            reg.add_project(Path::new(&trimmed))?;
            reg.save_atomic(&self.inner.projects_store_path)?;
        }
        self.shutdown();
        Ok(self.bootstrap_with_sink(event_sink))
    }

    pub fn remove_project_with_sink(
        &self,
        event_sink: EventSink,
        path: String,
    ) -> Result<PiBootstrapData, String> {
        let trimmed = require_non_empty("path", &path)?;
        let active_changed = {
            let mut reg = self.inner.registry.lock().expect("registry lock poisoned");
            let (_removed, active_was_removed, _next) = reg.remove_project(Path::new(&trimmed))?;
            reg.save_atomic(&self.inner.projects_store_path)?;
            active_was_removed
        };
        if active_changed {
            self.shutdown();
        }
        Ok(self.bootstrap_with_sink(event_sink))
    }

    pub fn switch_project_with_sink(
        &self,
        event_sink: EventSink,
        path: String,
    ) -> Result<PiBootstrapData, String> {
        let trimmed = require_non_empty("path", &path)?;
        {
            let mut reg = self.inner.registry.lock().expect("registry lock poisoned");
            reg.set_active_project(Path::new(&trimmed))?;
            reg.save_atomic(&self.inner.projects_store_path)?;
        }
        self.shutdown();
        Ok(self.bootstrap_with_sink(event_sink))
    }

    pub fn archive_session_with_sink(
        &self,
        event_sink: EventSink,
        session_id: String,
        archived: bool,
    ) -> Result<PiBootstrapData, String> {
        let session_id = require_non_empty("sessionId", &session_id)?;
        set_session_archived_at(&self.inner.session_meta_path, &session_id, archived)?;
        Ok(self.bootstrap_with_sink(event_sink))
    }

    pub fn delete_session_with_sink(
        &self,
        event_sink: EventSink,
        session_id: String,
    ) -> Result<PiBootstrapData, String> {
        let session_id = require_non_empty("sessionId", &session_id)?;
        let (projects, active_project) = self.snapshot_projects();
        let archived_ids = load_archived_session_ids_from(&self.inner.session_meta_path);
        let disk_sessions = self.scan_all_project_sessions(&projects, &archived_ids);

        let target_disk = disk_sessions.iter().find(|s| s.id == session_id);
        let mut target_path = target_disk.map(|s| s.path.clone()).unwrap_or_default();

        let tracked_active_id = self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned")
            .clone();

        let live_sup = self
            .ensure_supervisor_with_sink(Arc::clone(&event_sink))
            .ok();
        let live_session = live_sup.as_ref().and_then(|sup| {
            sup.get_state()
                .ok()
                .and_then(|r| parse_data_json(&r))
                .map(|v| {
                    parse_session_summary(&v, &active_project.id, "session-1", "Current session")
                })
        });

        if target_path.trim().is_empty() {
            if let Some(ref live) = live_session {
                if live.id == session_id && !live.path.trim().is_empty() {
                    target_path = live.path.clone();
                }
            }
        }

        let is_active = tracked_active_id.as_deref() == Some(session_id.as_str())
            || live_session
                .as_ref()
                .is_some_and(|live| live.id == session_id);

        // Remove from archived metadata store first.
        let _ = set_session_archived_at(&self.inner.session_meta_path, &session_id, false);

        // If deleting the active session, switch to the next non-archived session in the same
        // project or start a fresh session before deleting the file on disk.
        if is_active {
            if let Some(ref sup) = live_sup {
                let next_in_project = disk_sessions.iter().find(|s| {
                    s.project_id == active_project.id
                        && s.id != session_id
                        && !s.archived
                        && !s.path.trim().is_empty()
                });
                if let Some(next_sess) = next_in_project {
                    let _ = sup.switch_session(&next_sess.path);
                    *self
                        .inner
                        .active_session_id
                        .lock()
                        .expect("active_session_id lock poisoned") = Some(next_sess.id.clone());
                } else {
                    let _ = sup.new_session(None);
                    *self
                        .inner
                        .active_session_id
                        .lock()
                        .expect("active_session_id lock poisoned") = None;
                }
            }
        }

        if !target_path.trim().is_empty() {
            delete_session_file(&target_path)?;
        }

        Ok(self.bootstrap_with_sink(event_sink))
    }

    pub fn switch_session_with_sink(
        &self,
        event_sink: EventSink,
        session_path: String,
    ) -> Result<Vec<PiChatMessage>, String> {
        let session_path = require_non_empty("sessionPath", &session_path)?;
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup
            .switch_session(session_path)
            .map_err(|e| e.to_string())?;
        if let Some(val) = parse_data_json(&resp) {
            let msgs = parse_chat_messages(&val);
            if !msgs.is_empty() {
                return Ok(msgs);
            }
        }
        let msgs_resp = sup.get_messages().map_err(|e| e.to_string())?;
        Ok(parse_data_json(&msgs_resp)
            .map(|v| parse_chat_messages(&v))
            .unwrap_or_default())
    }

    pub fn new_session_with_sink(
        &self,
        event_sink: EventSink,
        project_id: Option<String>,
        parent_session: Option<String>,
    ) -> Result<PiSessionSummary, String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup
            .new_session(parent_session.clone())
            .map_err(|e| e.to_string())?;
        let fallback_project_id =
            project_id.unwrap_or_else(|| project_from_path(&self.project_path()).id);
        let mut summary = parse_data_json(&resp)
            .map(|v| parse_session_summary(&v, &fallback_project_id, "session-new", "New session"))
            .unwrap_or_else(|| PiSessionSummary {
                id: "session-new".to_string(),
                project_id: fallback_project_id,
                name: "New session".to_string(),
                path: String::new(),
                updated_at: "Just now".to_string(),
                message_count: 0,
                parent_session_id: parent_session.clone(),
                archived: false,
            });
        if summary.parent_session_id.is_none() {
            summary.parent_session_id = parent_session;
        }
        Ok(summary)
    }

    pub fn set_session_name_with_sink(
        &self,
        event_sink: EventSink,
        session_id: String,
        name: String,
        session_path: Option<String>,
    ) -> Result<PiSessionSummary, String> {
        let session_id = require_non_empty("sessionId", &session_id)?;
        let name = require_non_empty("name", &name)?;
        let sup = self.ensure_supervisor_with_sink(event_sink)?;

        if let Some(target_path) = session_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let current_path = sup
                .get_state()
                .ok()
                .and_then(|r| parse_data_json(&r))
                .and_then(|v| {
                    opt_str(
                        &v,
                        &[
                            "path",
                            "sessionPath",
                            "session_path",
                            "sessionFile",
                            "session_file",
                        ],
                    )
                });
            if current_path.as_deref() != Some(target_path) {
                sup.switch_session(target_path).map_err(|e| e.to_string())?;
            }
        }

        let resp = sup.set_session_name(&name).map_err(|e| e.to_string())?;
        let project_id = project_from_path(&self.project_path()).id;
        Ok(parse_data_json(&resp)
            .map(|v| parse_session_summary(&v, &project_id, &session_id, &name))
            .unwrap_or_else(|| PiSessionSummary {
                id: session_id,
                project_id,
                name,
                path: session_path.unwrap_or_default(),
                updated_at: "Just now".to_string(),
                message_count: 0,
                parent_session_id: None,
                archived: false,
            }))
    }

    pub fn get_tree_with_sink(
        &self,
        event_sink: EventSink,
    ) -> Result<Vec<HistoryTreeNode>, String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.get_tree().map_err(|e| e.to_string())?;
        Ok(parse_data_json(&resp)
            .map(|v| parse_history_tree(&v))
            .unwrap_or_default())
    }

    pub fn get_fork_messages_with_sink(
        &self,
        event_sink: EventSink,
    ) -> Result<Vec<ForkCandidate>, String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.get_fork_messages().map_err(|e| e.to_string())?;
        Ok(parse_data_json(&resp)
            .map(|v| parse_fork_candidates(&v))
            .unwrap_or_default())
    }

    pub fn fork_with_sink(
        &self,
        event_sink: EventSink,
        entry_id: String,
    ) -> Result<SessionWithMessages, String> {
        let entry_id = require_non_empty("entryId", &entry_id)?;
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.fork(&entry_id).map_err(|e| e.to_string())?;
        self.parse_session_with_messages(&sup, &resp, "session-fork", "Forked session")
    }

    pub fn clone_with_sink(
        &self,
        event_sink: EventSink,
        entry_id: Option<String>,
    ) -> Result<SessionWithMessages, String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.clone_session(entry_id).map_err(|e| e.to_string())?;
        self.parse_session_with_messages(&sup, &resp, "session-clone", "Cloned session")
    }

    pub fn export_html_with_sink(
        &self,
        event_sink: EventSink,
        output_path: Option<String>,
    ) -> Result<ExportHtmlResult, String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup
            .export_html(output_path.clone())
            .map_err(|e| e.to_string())?;
        Ok(parse_data_json(&resp)
            .map(|v| parse_export_html(&v, output_path.as_deref()))
            .unwrap_or_else(|| ExportHtmlResult {
                path: output_path.unwrap_or_default(),
                html: String::new(),
            }))
    }

    pub fn set_model_with_sink(
        &self,
        event_sink: EventSink,
        provider: String,
        model_id: String,
    ) -> Result<ModelOption, String> {
        let provider = require_non_empty("provider", &provider)?;
        let model_id = require_non_empty("modelId", &model_id)?;
        let updated_model = ModelOption {
            label: resolve_model_label(&provider, &model_id),
            provider: provider.clone(),
            model_id: model_id.clone(),
        };

        let Ok(sup) = self.ensure_supervisor_with_sink(event_sink) else {
            *self
                .inner
                .selected_model
                .lock()
                .expect("selected_model lock poisoned") = Some(updated_model.clone());
            self.inner.model_pending_sync.store(true, Ordering::Release);
            return Ok(updated_model);
        };

        let resp = sup
            .set_model(&provider, &model_id)
            .map_err(|e| e.to_string())?;
        let confirmed = parse_data_json(&resp)
            .map(|v| parse_model_option(&v, &provider, &model_id))
            .unwrap_or(updated_model);
        *self
            .inner
            .selected_model
            .lock()
            .expect("selected_model lock poisoned") = Some(confirmed.clone());
        self.inner
            .model_pending_sync
            .store(false, Ordering::Release);
        Ok(confirmed)
    }

    pub fn set_thinking_level_with_sink(
        &self,
        event_sink: EventSink,
        level: ThinkingLevel,
    ) -> Result<ThinkingLevel, String> {
        let Ok(sup) = self.ensure_supervisor_with_sink(event_sink) else {
            *self
                .inner
                .thinking_level
                .lock()
                .expect("thinking_level lock poisoned") = level;
            self.inner
                .thinking_level_pending_sync
                .store(true, Ordering::Release);
            return Ok(level);
        };

        let resp = sup
            .set_thinking_level(level.as_str())
            .map_err(|e| e.to_string())?;
        let confirmed = parse_data_json(&resp)
            .and_then(|v| opt_str(&v, &["level", "thinkingLevel", "thinking_level"]))
            .and_then(|raw| ThinkingLevel::parse(&raw))
            .unwrap_or(level);
        *self
            .inner
            .thinking_level
            .lock()
            .expect("thinking_level lock poisoned") = confirmed;
        self.inner
            .thinking_level_pending_sync
            .store(false, Ordering::Release);
        Ok(confirmed)
    }

    pub fn prompt_with_sink(
        &self,
        event_sink: EventSink,
        message: String,
        images: Vec<PendingAttachment>,
    ) -> Result<(), String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        self.record_pre_prompt_checkpoint(&message);
        sup.prompt(message, images).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn record_pre_prompt_checkpoint(&self, message: &str) {
        let project_dir = self.project_path();
        let Ok(tree_oid) = capture_snapshot(&project_dir) else {
            return;
        };
        let session_id = self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned")
            .clone()
            .unwrap_or_else(|| "default-session".to_string());
        let existing_count = load_checkpoint_store(&project_dir).checkpoints.len();
        let entry_id = format!("turn-{}", existing_count + 1);
        let _ = record_turn_checkpoint(
            &project_dir,
            session_id,
            entry_id,
            message.to_string(),
            tree_oid,
        );
    }

    pub fn list_checkpoints(&self) -> Result<Vec<TurnCheckpointView>, String> {
        let project_dir = self.project_path();
        let store = load_checkpoint_store(&project_dir);
        let active_session = self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned")
            .clone();

        let filtered: Vec<TurnCheckpointView> = store
            .checkpoints
            .iter()
            .filter(|cp| match active_session.as_deref() {
                Some(sid) if !sid.is_empty() => {
                    cp.session_id == sid || cp.session_id == "default-session"
                }
                _ => true,
            })
            .map(|cp| TurnCheckpointView {
                entry_id: cp.entry_id.clone(),
                prompt_preview: cp.prompt_preview.clone(),
                has_code_snapshot: cp.tree_oid.len() == 40,
                created_at: cp.created_at.clone(),
            })
            .collect();

        if filtered.is_empty() && !store.checkpoints.is_empty() {
            return Ok(store
                .checkpoints
                .into_iter()
                .map(|cp| TurnCheckpointView {
                    entry_id: cp.entry_id,
                    prompt_preview: cp.prompt_preview,
                    has_code_snapshot: cp.tree_oid.len() == 40,
                    created_at: cp.created_at,
                })
                .collect());
        }

        Ok(filtered)
    }

    pub fn revert_turn_with_sink(
        &self,
        event_sink: EventSink,
        entry_id: String,
        revert_code: bool,
    ) -> Result<RevertTurnResult, String> {
        let entry_id = require_non_empty("entryId", &entry_id)?;
        let sup = self.ensure_supervisor_with_sink(Arc::clone(&event_sink))?;

        let session_id = self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned")
            .clone()
            .or_else(|| {
                sup.get_state()
                    .ok()
                    .and_then(|r| parse_data_json(&r))
                    .and_then(|v| opt_str(&v, &["id", "sessionId", "session_id"]))
            });

        let tree = sup
            .get_tree()
            .ok()
            .and_then(|r| parse_data_json(&r))
            .map(|v| parse_history_tree(&v))
            .unwrap_or_default();
        let fork_candidates = sup
            .get_fork_messages()
            .ok()
            .and_then(|r| parse_data_json(&r))
            .map(|v| parse_fork_candidates(&v))
            .unwrap_or_default();
        let current_messages = sup
            .get_messages()
            .ok()
            .and_then(|r| parse_data_json(&r))
            .map(|v| parse_chat_messages(&v))
            .unwrap_or_default();

        let project_dir = self.project_path();
        let store = load_checkpoint_store(&project_dir);

        let restored_prompt = current_messages
            .iter()
            .find(|m| m.id == entry_id)
            .map(|m| m.content.clone())
            .or_else(|| {
                fork_candidates
                    .iter()
                    .find(|c| c.entry_id == entry_id)
                    .map(|c| c.preview.clone())
            })
            .or_else(|| {
                tree.iter()
                    .find(|n| n.id == entry_id)
                    .map(|n| n.preview.clone())
            })
            .or_else(|| {
                store
                    .checkpoints
                    .iter()
                    .rev()
                    .find(|c| c.entry_id == entry_id)
                    .map(|c| c.prompt_preview.clone())
            });

        // Determine rewind target: fork to parent entry, or start a new session if reverting the first user turn.
        enum RewindAction {
            ForkTo(String),
            NewSession,
        }

        let rewind_action = if let Some(node) = tree.iter().find(|n| n.id == entry_id) {
            match node
                .parent_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(parent) => RewindAction::ForkTo(parent.to_string()),
                None => RewindAction::NewSession,
            }
        } else if let Some(idx) = current_messages.iter().position(|m| m.id == entry_id) {
            if idx == 0 {
                RewindAction::NewSession
            } else {
                RewindAction::ForkTo(current_messages[idx - 1].id.clone())
            }
        } else if let Some(idx) = fork_candidates.iter().position(|c| c.entry_id == entry_id) {
            if idx == 0 {
                RewindAction::NewSession
            } else {
                RewindAction::ForkTo(fork_candidates[idx - 1].entry_id.clone())
            }
        } else {
            RewindAction::ForkTo(entry_id.clone())
        };

        let SessionWithMessages { session, messages } = match rewind_action {
            RewindAction::ForkTo(parent_id) => {
                let resp = sup.fork(&parent_id).map_err(|e| e.to_string())?;
                self.parse_session_with_messages(&sup, &resp, "session-revert", "Reverted session")?
            }
            RewindAction::NewSession => {
                let resp = sup.new_session(None).map_err(|e| e.to_string())?;
                let project_id = project_from_path(&project_dir).id;
                let summary = parse_data_json(&resp)
                    .map(|v| parse_session_summary(&v, &project_id, "session-new", "New session"))
                    .unwrap_or_else(|| PiSessionSummary {
                        id: "session-new".to_string(),
                        project_id,
                        name: "New session".to_string(),
                        path: String::new(),
                        updated_at: "Just now".to_string(),
                        message_count: 0,
                        parent_session_id: None,
                        archived: false,
                    });
                SessionWithMessages {
                    session: summary,
                    messages: Vec::new(),
                }
            }
        };

        *self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned") = Some(session.id.clone());

        let (code_reverted, reverted_files, can_redo, code_revert_note) = if revert_code {
            if let Some(cp) = find_checkpoint(
                &store,
                session_id.as_deref(),
                &entry_id,
                restored_prompt.as_deref(),
            ) {
                let outcome = revert_to_snapshot(&project_dir, &cp.tree_oid)?;
                save_last_redo(
                    &project_dir,
                    outcome.redo_tree_oid.clone(),
                    outcome.reverted_files.clone(),
                )?;
                (true, outcome.reverted_files, true, None)
            } else {
                (
                    false,
                    Vec::new(),
                    false,
                    Some("No code snapshot was recorded for this turn.".to_string()),
                )
            }
        } else {
            (false, Vec::new(), false, None)
        };

        Ok(RevertTurnResult {
            session,
            messages,
            restored_prompt,
            code_reverted,
            reverted_files,
            can_redo,
            code_revert_note,
        })
    }

    pub fn redo_revert_with_sink(&self) -> Result<RedoRevertResult, String> {
        let project_dir = self.project_path();
        let redo = load_last_redo(&project_dir)
            .ok_or_else(|| "No revert operation is available to redo.".to_string())?;
        let restored_files = redo_targeted_revert(&project_dir, &redo.redo_tree_oid, &redo.files)?;
        clear_last_redo(&project_dir)?;
        Ok(RedoRevertResult { restored_files })
    }

    pub fn compact_with_sink(
        &self,
        event_sink: EventSink,
        custom_instructions: Option<String>,
    ) -> Result<(), String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.compact(custom_instructions)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn steer_with_sink(
        &self,
        event_sink: EventSink,
        message: String,
        images: Vec<PendingAttachment>,
    ) -> Result<(), String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.steer(message, images).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn follow_up_with_sink(
        &self,
        event_sink: EventSink,
        message: String,
        images: Vec<PendingAttachment>,
    ) -> Result<(), String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.follow_up(message, images).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn clear_queue_with_sink(&self, event_sink: EventSink) -> Result<(), String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.clear_queue().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn abort_with_sink(&self, event_sink: EventSink) -> Result<(), String> {
        let live = {
            let guard = self
                .inner
                .supervisor
                .lock()
                .expect("supervisor lock poisoned");
            guard.as_ref().filter(|s| s.is_alive()).cloned()
        };
        let Some(sup) = live else {
            event_sink(PiUiEvent::AgentSettled {
                summary: Some("aborted".to_string()),
                raw_json: r#"{"type":"agent_settled","summary":"aborted"}"#.to_string(),
            });
            return Ok(());
        };
        sup.abort().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn abort_retry_with_sink(&self, event_sink: EventSink) -> Result<(), String> {
        let live = {
            let guard = self
                .inner
                .supervisor
                .lock()
                .expect("supervisor lock poisoned");
            guard.as_ref().filter(|s| s.is_alive()).cloned()
        };
        let Some(sup) = live else {
            event_sink(PiUiEvent::AgentSettled {
                summary: Some("aborted".to_string()),
                raw_json: r#"{"type":"agent_settled","summary":"aborted"}"#.to_string(),
            });
            return Ok(());
        };
        sup.abort_retry().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn extension_ui_response_with_sink(
        &self,
        event_sink: EventSink,
        dialog_id: String,
        answer: ExtensionUiAnswer,
    ) -> Result<(), String> {
        let parsed_id = DialogId::parse(dialog_id).map_err(|e| e.to_string())?;
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.extension_ui_response(&parsed_id, answer)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn bash_with_sink(
        &self,
        event_sink: EventSink,
        command: String,
    ) -> Result<serde_json::Value, String> {
        let command = require_non_empty("command", &command)?;
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.bash(command).map_err(|e| e.to_string())?;
        Ok(rpc_response_to_serde_value(&resp))
    }

    pub fn abort_bash_with_sink(&self, event_sink: EventSink) -> Result<(), String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.abort_bash().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_session_stats_with_sink(
        &self,
        event_sink: EventSink,
    ) -> Result<serde_json::Value, String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.get_session_stats().map_err(|e| e.to_string())?;
        Ok(rpc_response_to_serde_value(&resp))
    }

    pub fn get_last_assistant_text_with_sink(
        &self,
        event_sink: EventSink,
    ) -> Result<Option<String>, String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.get_last_assistant_text().map_err(|e| e.to_string())?;
        Ok(parse_data_json(&resp).and_then(|v| match v {
            JsonValue::String(s) => Some(s),
            JsonValue::Object(_) => opt_str(&v, &["text", "content", "message"]),
            _ => None,
        }))
    }

    pub fn set_auto_compaction_with_sink(
        &self,
        event_sink: EventSink,
        enabled: bool,
    ) -> Result<(), String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.set_auto_compaction(enabled)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_auto_retry_with_sink(
        &self,
        event_sink: EventSink,
        enabled: bool,
    ) -> Result<(), String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.set_auto_retry(enabled).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_steering_mode_with_sink(
        &self,
        event_sink: EventSink,
        mode: String,
    ) -> Result<(), String> {
        let mode = validate_queue_mode("steeringMode", &mode)?;
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.set_steering_mode(mode).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_follow_up_mode_with_sink(
        &self,
        event_sink: EventSink,
        mode: String,
    ) -> Result<(), String> {
        let mode = validate_queue_mode("followUpMode", &mode)?;
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        sup.set_follow_up_mode(mode).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn available_merged_models(&self, sup: Option<&PiSessionSupervisor>) -> Vec<ModelOption> {
        let store = ProviderStore::load(&self.inner.provider_store_path).unwrap_or_default();
        let providers = store.public_profiles();
        let provider_models = models_from_providers(&providers);
        let agent_models = load_pi_agent_models();
        let rpc_models = sup
            .and_then(|s| s.get_available_models().ok())
            .and_then(|r| parse_data_json(&r))
            .map(|v| parse_models_list(&v))
            .unwrap_or_default();
        merge_models_with_fallback(rpc_models, provider_models, agent_models)
    }

    pub fn cycle_model_with_sink(
        &self,
        event_sink: EventSink,
    ) -> Result<serde_json::Value, String> {
        let sup = self.ensure_supervisor_with_sink(Arc::clone(&event_sink))?;
        let current_model = self
            .inner
            .selected_model
            .lock()
            .expect("selected_model lock poisoned")
            .clone();

        let rpc_result = sup.cycle_model();
        let mut parsed_from_rpc: Option<ModelOption> = None;
        let mut parsed_thinking: Option<ThinkingLevel> = None;

        if let Ok(resp) = rpc_result {
            if let Some(val) = parse_data_json(&resp) {
                parsed_thinking = opt_str(&val, &["thinkingLevel", "thinking_level", "level"])
                    .and_then(|raw| ThinkingLevel::parse(&raw));
                parsed_from_rpc = try_parse_model_option(&val);
            }
        }

        let same_as_current = match (&parsed_from_rpc, &current_model) {
            (Some(rpc_m), Some(cur)) => {
                rpc_m.provider == cur.provider && rpc_m.model_id == cur.model_id
            }
            _ => false,
        };

        let (final_model, cycled, reason): (ModelOption, bool, Option<String>) =
            if let Some(rpc_m) = parsed_from_rpc.filter(|_| !same_as_current) {
                *self
                    .inner
                    .selected_model
                    .lock()
                    .expect("selected_model lock poisoned") = Some(rpc_m.clone());
                self.inner
                    .model_pending_sync
                    .store(false, Ordering::Release);
                if let Some(lvl) = parsed_thinking {
                    *self
                        .inner
                        .thinking_level
                        .lock()
                        .expect("thinking_level lock poisoned") = lvl;
                    self.inner
                        .thinking_level_pending_sync
                        .store(false, Ordering::Release);
                }
                (rpc_m, true, None)
            } else {
                let models = self.available_merged_models(Some(&sup));
                if models.len() >= 2 {
                    let next_idx = if let Some(ref cur) = current_model {
                        models
                            .iter()
                            .position(|m| m.provider == cur.provider && m.model_id == cur.model_id)
                            .map(|idx| (idx + 1) % models.len())
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    let next = &models[next_idx];
                    let mut switched = self.set_model_with_sink(
                        event_sink,
                        next.provider.clone(),
                        next.model_id.clone(),
                    )?;
                    if switched.label == switched.model_id && next.label != next.model_id {
                        switched.label = next.label.clone();
                        *self
                            .inner
                            .selected_model
                            .lock()
                            .expect("selected_model lock poisoned") = Some(switched.clone());
                    }
                    (switched, true, None)
                } else {
                    let single = current_model
                        .or_else(|| models.into_iter().next())
                        .unwrap_or_else(|| fallback_model(&[]));
                    *self
                        .inner
                        .selected_model
                        .lock()
                        .expect("selected_model lock poisoned") = Some(single.clone());
                    (single, false, Some("single_model".to_string()))
                }
            };

        let thinking_level = *self
            .inner
            .thinking_level
            .lock()
            .expect("thinking_level lock poisoned");

        Ok(serde_json::json!({
            "model": {
                "provider": final_model.provider,
                "modelId": final_model.model_id,
                "label": final_model.label,
            },
            "provider": final_model.provider,
            "id": final_model.model_id,
            "modelId": final_model.model_id,
            "label": final_model.label,
            "thinkingLevel": thinking_level.as_str(),
            "cycled": cycled,
            "reason": reason,
        }))
    }

    pub fn cycle_thinking_level_with_sink(
        &self,
        event_sink: EventSink,
    ) -> Result<serde_json::Value, String> {
        let sup = self.ensure_supervisor_with_sink(Arc::clone(&event_sink))?;
        let current_level = *self
            .inner
            .thinking_level
            .lock()
            .expect("thinking_level lock poisoned");

        let rpc_level = sup
            .cycle_thinking_level()
            .ok()
            .and_then(|resp| parse_data_json(&resp))
            .and_then(|v| opt_str(&v, &["level", "thinkingLevel", "thinking_level"]))
            .and_then(|raw| ThinkingLevel::parse(&raw));

        let final_level = match rpc_level {
            Some(level) if level != current_level => {
                *self
                    .inner
                    .thinking_level
                    .lock()
                    .expect("thinking_level lock poisoned") = level;
                self.inner
                    .thinking_level_pending_sync
                    .store(false, Ordering::Release);
                level
            }
            _ => {
                let levels = ThinkingLevel::all();
                let cur_idx = levels.iter().position(|&l| l == current_level).unwrap_or(0);
                let next = levels[(cur_idx + 1) % levels.len()];
                self.set_thinking_level_with_sink(event_sink, next)?
            }
        };

        Ok(serde_json::json!({
            "level": final_level.as_str(),
            "thinkingLevel": final_level.as_str(),
        }))
    }

    pub fn get_commands_with_sink(
        &self,
        event_sink: EventSink,
    ) -> Result<Vec<PiCommandInfo>, String> {
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.get_commands().map_err(|e| e.to_string())?;
        Ok(parse_data_json(&resp)
            .map(|v| parse_command_info_list(&v))
            .unwrap_or_default())
    }

    fn parse_session_with_messages(
        &self,
        sup: &PiSessionSupervisor,
        resp: &PiRpcResponse,
        fallback_id: &str,
        fallback_name: &str,
    ) -> Result<SessionWithMessages, String> {
        let project_id = project_from_path(&self.project_path()).id;
        let data = parse_data_json(resp);
        let mut session = data
            .as_ref()
            .map(|v| parse_session_summary(v, &project_id, fallback_id, fallback_name))
            .unwrap_or_else(|| PiSessionSummary {
                id: fallback_id.to_string(),
                project_id,
                name: fallback_name.to_string(),
                path: String::new(),
                updated_at: "Just now".to_string(),
                message_count: 0,
                parent_session_id: None,
                archived: false,
            });

        let mut messages = data.as_ref().map(parse_chat_messages).unwrap_or_default();

        if messages.is_empty() {
            if let Ok(msgs_resp) = sup.get_messages() {
                if let Some(val) = parse_data_json(&msgs_resp) {
                    messages = parse_chat_messages(&val);
                }
            }
        }
        if session.message_count == 0 {
            session.message_count = messages.len();
        }
        Ok(SessionWithMessages { session, messages })
    }
}

fn tauri_event_sink<R: Runtime>(app: &AppHandle<R>) -> EventSink {
    let app_handle = app.clone();
    Arc::new(move |event| {
        let _ = app_handle.emit(PI_EVENT_NAME, event);
    })
}

// ============================================================
// 20 Thin Async Tauri v2 IPC Commands (offloaded via spawn_blocking)
// ============================================================

#[tauri::command]
pub async fn pi_bootstrap(
    app: AppHandle,
    state: State<'_, PiAppState>,
) -> Result<PiBootstrapData, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || Ok(app_state.bootstrap_with_sink(sink)))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_switch_session(
    app: AppHandle,
    state: State<'_, PiAppState>,
    session_path: String,
) -> Result<Vec<PiChatMessage>, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.switch_session_with_sink(sink, session_path)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_new_session(
    app: AppHandle,
    state: State<'_, PiAppState>,
    project_id: Option<String>,
    parent_session: Option<String>,
) -> Result<PiSessionSummary, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.new_session_with_sink(sink, project_id, parent_session)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_set_session_name(
    app: AppHandle,
    state: State<'_, PiAppState>,
    session_id: String,
    name: String,
    session_path: Option<String>,
) -> Result<PiSessionSummary, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.set_session_name_with_sink(sink, session_id, name, session_path)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_get_tree(
    app: AppHandle,
    state: State<'_, PiAppState>,
) -> Result<Vec<HistoryTreeNode>, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.get_tree_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_get_fork_messages(
    app: AppHandle,
    state: State<'_, PiAppState>,
) -> Result<Vec<ForkCandidate>, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.get_fork_messages_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_fork(
    app: AppHandle,
    state: State<'_, PiAppState>,
    entry_id: String,
) -> Result<SessionWithMessages, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.fork_with_sink(sink, entry_id))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_clone(
    app: AppHandle,
    state: State<'_, PiAppState>,
    entry_id: Option<String>,
) -> Result<SessionWithMessages, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.clone_with_sink(sink, entry_id))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_export_html(
    app: AppHandle,
    state: State<'_, PiAppState>,
    output_path: Option<String>,
) -> Result<ExportHtmlResult, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.export_html_with_sink(sink, output_path))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_set_model(
    app: AppHandle,
    state: State<'_, PiAppState>,
    provider: String,
    model_id: String,
) -> Result<ModelOption, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.set_model_with_sink(sink, provider, model_id)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_set_thinking_level(
    app: AppHandle,
    state: State<'_, PiAppState>,
    level: ThinkingLevel,
) -> Result<ThinkingLevel, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.set_thinking_level_with_sink(sink, level)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_prompt(
    app: AppHandle,
    state: State<'_, PiAppState>,
    message: String,
    images: Vec<PendingAttachment>,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.prompt_with_sink(sink, message, images))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_compact(
    app: AppHandle,
    state: State<'_, PiAppState>,
    custom_instructions: Option<String>,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.compact_with_sink(sink, custom_instructions)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_steer(
    app: AppHandle,
    state: State<'_, PiAppState>,
    message: String,
    images: Vec<PendingAttachment>,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.steer_with_sink(sink, message, images))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_follow_up(
    app: AppHandle,
    state: State<'_, PiAppState>,
    message: String,
    images: Vec<PendingAttachment>,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.follow_up_with_sink(sink, message, images)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_clear_queue(app: AppHandle, state: State<'_, PiAppState>) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.clear_queue_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_abort(app: AppHandle, state: State<'_, PiAppState>) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.abort_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_abort_retry(app: AppHandle, state: State<'_, PiAppState>) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.abort_retry_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_extension_ui_response(
    app: AppHandle,
    state: State<'_, PiAppState>,
    dialog_id: String,
    answer: ExtensionUiAnswer,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.extension_ui_response_with_sink(sink, dialog_id, answer)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_open_external_url(url: String) -> Result<(), String> {
    let validated = validate_external_open_url(&url)?;
    tauri::async_runtime::spawn_blocking(move || launch_os_browser(&validated))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_pick_project_folder(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        let picked = app.dialog().file().blocking_pick_folder();
        Ok(picked
            .and_then(|fp| fp.into_path().ok())
            .map(|p| p.to_string_lossy().to_string()))
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_add_project(
    app: AppHandle,
    state: State<'_, PiAppState>,
    path: String,
) -> Result<PiBootstrapData, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.add_project_with_sink(sink, path))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_remove_project(
    app: AppHandle,
    state: State<'_, PiAppState>,
    path: String,
) -> Result<PiBootstrapData, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.remove_project_with_sink(sink, path))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_switch_project(
    app: AppHandle,
    state: State<'_, PiAppState>,
    path: String,
) -> Result<PiBootstrapData, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.switch_project_with_sink(sink, path))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_bash(
    app: AppHandle,
    state: State<'_, PiAppState>,
    command: String,
) -> Result<serde_json::Value, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.bash_with_sink(sink, command))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_abort_bash(app: AppHandle, state: State<'_, PiAppState>) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.abort_bash_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_get_session_stats(
    app: AppHandle,
    state: State<'_, PiAppState>,
) -> Result<serde_json::Value, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.get_session_stats_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_get_last_assistant_text(
    app: AppHandle,
    state: State<'_, PiAppState>,
) -> Result<Option<String>, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.get_last_assistant_text_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_set_auto_compaction(
    app: AppHandle,
    state: State<'_, PiAppState>,
    enabled: bool,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.set_auto_compaction_with_sink(sink, enabled)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_set_auto_retry(
    app: AppHandle,
    state: State<'_, PiAppState>,
    enabled: bool,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.set_auto_retry_with_sink(sink, enabled))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_set_steering_mode(
    app: AppHandle,
    state: State<'_, PiAppState>,
    mode: String,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.set_steering_mode_with_sink(sink, mode))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_set_follow_up_mode(
    app: AppHandle,
    state: State<'_, PiAppState>,
    mode: String,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.set_follow_up_mode_with_sink(sink, mode))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_cycle_model(
    app: AppHandle,
    state: State<'_, PiAppState>,
) -> Result<serde_json::Value, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.cycle_model_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_cycle_thinking_level(
    app: AppHandle,
    state: State<'_, PiAppState>,
) -> Result<serde_json::Value, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.cycle_thinking_level_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_get_commands(
    app: AppHandle,
    state: State<'_, PiAppState>,
) -> Result<Vec<PiCommandInfo>, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || app_state.get_commands_with_sink(sink))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_list_checkpoints(
    state: State<'_, PiAppState>,
) -> Result<Vec<TurnCheckpointView>, String> {
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || app_state.list_checkpoints())
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_revert_turn(
    app: AppHandle,
    state: State<'_, PiAppState>,
    entry_id: String,
    revert_code: bool,
) -> Result<RevertTurnResult, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.revert_turn_with_sink(sink, entry_id, revert_code)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_redo_revert(
    _app: AppHandle,
    state: State<'_, PiAppState>,
) -> Result<RedoRevertResult, String> {
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || app_state.redo_revert_with_sink())
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_archive_session(
    app: AppHandle,
    state: State<'_, PiAppState>,
    session_id: String,
    archived: bool,
) -> Result<PiBootstrapData, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.archive_session_with_sink(sink, session_id, archived)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_delete_session(
    app: AppHandle,
    state: State<'_, PiAppState>,
    session_id: String,
) -> Result<PiBootstrapData, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.delete_session_with_sink(sink, session_id)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{validate_image_bytes, ImageMimeType};
    use crate::providers::ProviderProfileInput;
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "verity-pi-ipc-{}-{}-{}",
            label,
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mock_sh_config(cwd: PathBuf, script: &str) -> PiSessionConfig {
        PiSessionConfig {
            program: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            cwd,
            env: Vec::new(),
            command_timeout: Duration::from_secs(5),
            no_session: false,
        }
    }

    #[test]
    fn builds_launch_env_with_provider_secrets_and_verity_mcp_bin() {
        let mut store = ProviderStore::default();
        store
            .upsert(ProviderProfileInput {
                id: "anthropic-main".to_string(),
                name: "Anthropic".to_string(),
                api: "anthropic-messages".to_string(),
                base_url: "https://api.anthropic.com/v1".to_string(),
                sign_in_url: None,
                models: vec!["claude-sonnet-5-5".to_string()],
                api_key: Some("sk-secret-token".to_string()),
            })
            .unwrap();

        let env_pairs = build_pi_launch_env(&store, Some(Path::new("/usr/local/bin/verity")));
        assert!(env_pairs
            .iter()
            .any(|(k, v)| k == "VERITY_DESKTOP_PROVIDERS" && !v.contains("sk-secret-token")));
        assert!(env_pairs.contains(&(
            "VERITY_DESKTOP_KEY_ANTHROPIC_MAIN".to_string(),
            "sk-secret-token".to_string()
        )));
        assert!(env_pairs.contains(&(
            "VERITY_MCP_BIN".to_string(),
            "/usr/local/bin/verity".to_string()
        )));
        assert!(env_pairs.contains(&("VERITY_MCP_ARGS".to_string(), "mcp".to_string())));
    }

    #[test]
    fn parses_all_ipc_response_payloads_to_frontend_shapes() {
        let msgs_val = parse_json(
            r#"{"messages":[{"id":"m1","role":"user","content":"hello","timestamp":"10:00"},{"id":"m2","role":"assistant","content":[{"text":"world"}],"timestamp":1700000000000,"toolCalls":[{"id":"tc1","name":"read","status":"done"}]}]}"#,
        )
        .unwrap();
        let msgs = parse_chat_messages(&msgs_val);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].content, "world");
        assert_eq!(msgs[1].timestamp, "1700000000000");
        assert_eq!(msgs[1].tool_calls.as_ref().unwrap()[0].name, "read");

        let sess_val =
            parse_json(r#"{"id":"s1","sessionFile":"/tmp/s1.jsonl","updatedAt":1700000000}"#)
                .unwrap();
        let sess = parse_session_summary(&sess_val, "p1", "fb", "Fallback");
        assert_eq!(sess.path, "/tmp/s1.jsonl");
        assert_eq!(sess.updated_at, "1700000000");

        let tree_val = parse_json(
            r#"{"nodes":[{"id":"n1","parentId":null,"role":"user","preview":"start","timestamp":1700000001,"depth":0,"isActiveBranch":true}]}"#,
        )
        .unwrap();
        let tree = parse_history_tree(&tree_val);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].parent_id, None);
        assert_eq!(tree[0].timestamp, "1700000001");
        assert!(tree[0].is_active_branch);

        let forks_val = parse_json(
            r#"{"messages":[{"entryId":"e1","role":"user","preview":"fork point","timestamp":1700000002}]}"#,
        )
        .unwrap();
        let forks = parse_fork_candidates(&forks_val);
        assert_eq!(forks.len(), 1);
        assert_eq!(forks[0].entry_id, "e1");
        assert_eq!(forks[0].timestamp, "1700000002");

        let models_val = parse_json(
            r#"{"models":[{"provider":"anthropic","modelId":"claude-sonnet-5-5","label":"Claude Sonnet 5.5"}]}"#,
        )
        .unwrap();
        let models = parse_models_list(&models_val);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "claude-sonnet-5-5");

        let levels_val = parse_json(r#"{"levels":["off","medium","high","xhigh"]}"#).unwrap();
        assert_eq!(
            parse_thinking_levels(&levels_val),
            vec![
                ThinkingLevel::Off,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
                ThinkingLevel::Xhigh
            ]
        );

        let cmds_val = parse_json(
            r#"{"commands":[{"trigger":"/","name":"compact","description":"Compact session"}]}"#,
        )
        .unwrap();
        let cmds = parse_commands_list(&cmds_val);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].insert_text, "/compact ");

        let export_val =
            parse_json(r#"{"path":"/tmp/export.html","html":"<html>ok</html>"}"#).unwrap();
        let exported = parse_export_html(&export_val, None);
        assert_eq!(exported.path, "/tmp/export.html");
        assert_eq!(exported.html, "<html>ok</html>");
    }

    #[test]
    fn validates_external_open_url_schemes_strictly() {
        assert_eq!(
            validate_external_open_url("https://example.com/auth?code=1#ok").unwrap(),
            "https://example.com/auth?code=1#ok"
        );
        assert_eq!(
            validate_external_open_url("http://localhost:3000/login").unwrap(),
            "http://localhost:3000/login"
        );
        assert!(validate_external_open_url("file:///etc/passwd").is_err());
        assert!(validate_external_open_url("javascript:alert(1)").is_err());
        assert!(validate_external_open_url("https://").is_err());
        assert!(validate_external_open_url("https://example.com/a b").is_err());
    }

    #[test]
    fn abort_without_live_supervisor_does_not_spawn_and_emits_agent_settled() {
        let dir = unique_temp_dir("abort-no-spawn");
        let store_path = dir.join("providers.json");
        let missing_cfg = PiSessionConfig {
            program: "/nonexistent/verity-missing-pi-binary".to_string(),
            args: vec![],
            cwd: dir.clone(),
            env: Vec::new(),
            command_timeout: Duration::from_secs(1),
            no_session: false,
        };
        let state = PiAppState::with_config(dir.clone(), store_path, missing_cfg);

        let events: Arc<Mutex<Vec<PiUiEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let sink: EventSink = Arc::new(move |ev| {
            events_clone.lock().unwrap().push(ev);
        });

        state.abort_with_sink(Arc::clone(&sink)).unwrap();
        state.abort_retry_with_sink(Arc::clone(&sink)).unwrap();
        assert!(!state.has_live_supervisor());
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| matches!(e, PiUiEvent::AgentSettled { .. }))
                .count(),
            2
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn degraded_bootstrap_succeeds_cleanly_when_pi_cli_is_missing() {
        let dir = unique_temp_dir("degraded");
        let store_path = dir.join("providers.json");
        let mut store = ProviderStore::default();
        store
            .upsert(ProviderProfileInput {
                id: "local-vllm".to_string(),
                name: "Local vLLM".to_string(),
                api: "openai-completions".to_string(),
                base_url: "http://127.0.0.1:8000/v1".to_string(),
                sign_in_url: None,
                models: vec!["qwen-2.5".to_string()],
                api_key: Some("sk-local".to_string()),
            })
            .unwrap();
        store.save_atomic(&store_path).unwrap();

        let missing_cfg = PiSessionConfig {
            program: "/nonexistent/verity-missing-pi-binary".to_string(),
            args: vec![],
            cwd: dir.clone(),
            env: Vec::new(),
            command_timeout: Duration::from_secs(1),
            no_session: false,
        };
        let state = PiAppState::with_config(dir.clone(), store_path, missing_cfg);
        let sink: EventSink = Arc::new(|_| {});

        let data = state.bootstrap_with_sink(sink);
        assert!(!state.has_live_supervisor());
        assert_eq!(data.projects.len(), 1);
        assert_eq!(data.providers.len(), 1);
        assert_eq!(data.providers[0].id, "local-vllm");
        assert!(data.providers[0].has_api_key);
        assert_eq!(data.models.len(), 1);
        assert_eq!(data.selected_model.model_id, "qwen-2.5");
        assert_eq!(data.thinking_level, ThinkingLevel::Medium);
        assert_eq!(data.thinking_levels.len(), 6);
        assert!(data.sessions.is_empty());

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_lifecycle_all_18_operations_env_injection_and_live_events_against_mock_sh() {
        let dir = unique_temp_dir("lifecycle");
        let store_path = dir.join("providers.json");
        let env_dump_path = dir.join("captured_env.txt");
        let fake_verity_bin = dir.join("target").join("debug").join("verity");
        fs::create_dir_all(fake_verity_bin.parent().unwrap()).unwrap();
        fs::write(&fake_verity_bin, "#!/bin/sh\nexit 0\n").unwrap();

        let mut store = ProviderStore::default();
        store
            .upsert(ProviderProfileInput {
                id: "openai-dev".to_string(),
                name: "OpenAI Dev".to_string(),
                api: "openai-responses".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                sign_in_url: None,
                models: vec!["gpt-6.1-sol".to_string()],
                api_key: Some("sk-test-env-injection-42".to_string()),
            })
            .unwrap();
        store.save_atomic(&store_path).unwrap();

        let script = format!(
            r#"
printf 'KEY=%s\nMCP=%s\nARGS=%s\n' "$VERITY_DESKTOP_KEY_OPENAI_DEV" "$VERITY_MCP_BIN" "$VERITY_MCP_ARGS" > "{env_dump}"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_state)
      printf '{{"type":"response","id":"%s","command":"get_state","success":true,"data":{{"id":"sess-live","name":"Live Session","path":"/tmp/sess-live.jsonl","thinkingLevel":"high","model":{{"provider":"openai-dev","modelId":"gpt-6.1-sol","label":"GPT-6.1 Sol"}}}}}}\n' "$id"
      ;;
    get_messages)
      printf '{{"type":"response","id":"%s","command":"get_messages","success":true,"data":{{"messages":[{{"id":"m1","role":"user","content":"hi","timestamp":"10:00"}}]}}}}\n' "$id"
      ;;
    get_available_models)
      printf '{{"type":"response","id":"%s","command":"get_available_models","success":true,"data":{{"models":[{{"provider":"openai-dev","modelId":"gpt-6.1-sol","label":"GPT-6.1 Sol"}}]}}}}\n' "$id"
      ;;
    get_available_thinking_levels)
      printf '{{"type":"response","id":"%s","command":"get_available_thinking_levels","success":true,"data":{{"levels":["off","low","medium","high"]}}}}\n' "$id"
      ;;
    get_commands)
      printf '{{"type":"response","id":"%s","command":"get_commands","success":true,"data":{{"commands":[{{"trigger":"/","name":"compact","description":"Compact context"}}]}}}}\n' "$id"
      ;;
    switch_session)
      printf '{{"type":"response","id":"%s","command":"switch_session","success":true,"data":{{"messages":[{{"id":"sw1","role":"assistant","content":"switched","timestamp":"10:02"}}]}}}}\n' "$id"
      ;;
    new_session)
      printf '{{"type":"response","id":"%s","command":"new_session","success":true,"data":{{"id":"sess-2","name":"Fresh Session","path":"/tmp/sess-2.jsonl"}}}}\n' "$id"
      ;;
    set_session_name)
      printf '{{"type":"response","id":"%s","command":"set_session_name","success":true,"data":{{"id":"sess-2","name":"Renamed Session","path":"/tmp/sess-2.jsonl"}}}}\n' "$id"
      ;;
    get_tree)
      printf '{{"type":"response","id":"%s","command":"get_tree","success":true,"data":{{"nodes":[{{"id":"n1","parentId":null,"role":"user","preview":"root","timestamp":"10:00","depth":0,"isActiveBranch":true}}]}}}}\n' "$id"
      ;;
    get_fork_messages)
      printf '{{"type":"response","id":"%s","command":"get_fork_messages","success":true,"data":{{"messages":[{{"entryId":"e1","role":"user","preview":"branch here","timestamp":"10:00"}}]}}}}\n' "$id"
      ;;
    fork)
      printf '{{"type":"response","id":"%s","command":"fork","success":true,"data":{{"session":{{"id":"sess-fork","name":"Forked","path":"/tmp/fork.jsonl"}},"messages":[{{"id":"fm1","role":"user","content":"forked msg","timestamp":"10:03"}}]}}}}\n' "$id"
      ;;
    clone)
      printf '{{"type":"response","id":"%s","command":"clone","success":true,"data":{{"session":{{"id":"sess-clone","name":"Cloned","path":"/tmp/clone.jsonl"}},"messages":[{{"id":"cm1","role":"user","content":"cloned msg","timestamp":"10:04"}}]}}}}\n' "$id"
      ;;
    export_html)
      printf '{{"type":"response","id":"%s","command":"export_html","success":true,"data":{{"path":"/tmp/out.html","html":"<html>exported</html>"}}}}\n' "$id"
      ;;
    set_model)
      printf '{{"type":"response","id":"%s","command":"set_model","success":true,"data":{{"provider":"openai-dev","modelId":"gpt-6.1-sol","label":"GPT-6.1 Sol"}}}}\n' "$id"
      ;;
    set_thinking_level)
      printf '{{"type":"response","id":"%s","command":"set_thinking_level","success":true,"data":{{"level":"xhigh"}}}}\n' "$id"
      ;;
    prompt)
      printf '{{"type":"response","id":"%s","command":"prompt","success":true}}\n' "$id"
      printf '{{"type":"message_start","messageId":"m2","role":"assistant"}}\n'
      printf '{{"type":"extension_ui_request","id":"dlg-1","method":"confirm","title":"Approve?"}}\n'
      ;;
    steer)
      printf '{{"type":"response","id":"%s","command":"steer","success":true}}\n' "$id"
      ;;
    follow_up)
      printf '{{"type":"response","id":"%s","command":"follow_up","success":true}}\n' "$id"
      printf '{{"type":"agent_settled","summary":"done"}}\n'
      ;;
    clear_queue|abort|abort_retry|compact)
      printf '{{"type":"response","id":"%s","command":"%s","success":true}}\n' "$id" "$typ"
      ;;
    extension_ui_response)
      ;;
    *)
      printf '{{"type":"response","id":"%s","command":"%s","success":true}}\n' "$id" "$typ"
      ;;
  esac
done
"#,
            env_dump = env_dump_path.display()
        );

        let state = PiAppState::with_config(
            dir.clone(),
            store_path,
            mock_sh_config(dir.clone(), &script),
        );

        let events: Arc<Mutex<Vec<PiUiEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let (dlg_tx, dlg_rx) = mpsc::channel();
        let (settled_tx, settled_rx) = mpsc::channel();
        let events_clone = Arc::clone(&events);
        let sink: EventSink = Arc::new(move |ev| {
            let is_dlg = matches!(ev, PiUiEvent::ExtensionUiRequest { .. });
            let is_settled = matches!(ev, PiUiEvent::AgentSettled { .. });
            events_clone.lock().unwrap().push(ev);
            if is_dlg {
                let _ = dlg_tx.send(());
            }
            if is_settled {
                let _ = settled_tx.send(());
            }
        });

        // 1. pi_bootstrap (lazily spawns supervisor and injects env)
        let boot = state.bootstrap_with_sink(Arc::clone(&sink));
        assert!(state.has_live_supervisor());
        assert_eq!(boot.active_session_id, "sess-live");
        assert_eq!(boot.selected_model.model_id, "gpt-6.1-sol");
        assert_eq!(boot.thinking_level, ThinkingLevel::High);
        assert_eq!(boot.messages.len(), 1);
        assert_eq!(boot.commands.len(), 1);

        // Verify 0o600 provider secret + VERITY_MCP_BIN were injected into child process.
        let dumped_env = fs::read_to_string(&env_dump_path).expect("env dump written");
        assert!(dumped_env.contains("KEY=sk-test-env-injection-42"));
        assert!(dumped_env.contains(&format!("MCP={}", fake_verity_bin.display())));
        assert!(dumped_env.contains("ARGS=mcp"));

        // 2. pi_switch_session
        let switched = state
            .switch_session_with_sink(Arc::clone(&sink), "/tmp/sess-2.jsonl".to_string())
            .unwrap();
        assert_eq!(switched[0].content, "switched");

        // 3. pi_new_session
        let created = state
            .new_session_with_sink(Arc::clone(&sink), None, Some("sess-live".to_string()))
            .unwrap();
        assert_eq!(created.id, "sess-2");
        assert_eq!(created.parent_session_id.as_deref(), Some("sess-live"));

        // 4. pi_set_session_name
        let renamed = state
            .set_session_name_with_sink(
                Arc::clone(&sink),
                "sess-2".to_string(),
                "Renamed Session".to_string(),
                None,
            )
            .unwrap();
        assert_eq!(renamed.name, "Renamed Session");

        // 5. pi_get_tree
        let tree = state.get_tree_with_sink(Arc::clone(&sink)).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id, "n1");

        // 6. pi_get_fork_messages
        let forks = state
            .get_fork_messages_with_sink(Arc::clone(&sink))
            .unwrap();
        assert_eq!(forks.len(), 1);
        assert_eq!(forks[0].entry_id, "e1");

        // 7. pi_fork
        let forked = state
            .fork_with_sink(Arc::clone(&sink), "e1".to_string())
            .unwrap();
        assert_eq!(forked.session.id, "sess-fork");
        assert_eq!(forked.messages[0].content, "forked msg");

        // 8. pi_clone
        let cloned = state
            .clone_with_sink(Arc::clone(&sink), Some("e1".to_string()))
            .unwrap();
        assert_eq!(cloned.session.id, "sess-clone");
        assert_eq!(cloned.messages[0].content, "cloned msg");

        // 9. pi_export_html
        let exported = state
            .export_html_with_sink(Arc::clone(&sink), Some("/tmp/out.html".to_string()))
            .unwrap();
        assert_eq!(exported.html, "<html>exported</html>");

        // 10. pi_set_model
        let model = state
            .set_model_with_sink(
                Arc::clone(&sink),
                "openai-dev".to_string(),
                "gpt-6.1-sol".to_string(),
            )
            .unwrap();
        assert_eq!(model.model_id, "gpt-6.1-sol");

        // 11. pi_set_thinking_level
        let level = state
            .set_thinking_level_with_sink(Arc::clone(&sink), ThinkingLevel::Xhigh)
            .unwrap();
        assert_eq!(level, ThinkingLevel::Xhigh);

        // 11b. pi_compact
        state
            .compact_with_sink(Arc::clone(&sink), Some("brief".to_string()))
            .unwrap();

        // 12. pi_prompt (triggers live message_start + extension_ui_request)
        let img = validate_image_bytes("shot.png", ImageMimeType::Png, b"pngdata").unwrap();
        state
            .prompt_with_sink(Arc::clone(&sink), "run check".to_string(), vec![img])
            .unwrap();
        dlg_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("should receive extension_ui_request event");

        // 18. pi_extension_ui_response
        state
            .extension_ui_response_with_sink(
                Arc::clone(&sink),
                "dlg-1".to_string(),
                ExtensionUiAnswer::Confirmed(true),
            )
            .unwrap();

        // 13. pi_steer
        state
            .steer_with_sink(Arc::clone(&sink), "focus".to_string(), vec![])
            .unwrap();

        // 15. pi_clear_queue
        state.clear_queue_with_sink(Arc::clone(&sink)).unwrap();

        // 14. pi_follow_up (triggers agent_settled)
        state
            .follow_up_with_sink(Arc::clone(&sink), "wrap up".to_string(), vec![])
            .unwrap();
        settled_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("should receive agent_settled event");

        // 16. pi_abort & 17. pi_abort_retry
        state.abort_with_sink(Arc::clone(&sink)).unwrap();
        state.abort_retry_with_sink(Arc::clone(&sink)).unwrap();

        // Verify live events reached the EventSink.
        let recorded = events.lock().unwrap();
        assert!(recorded
            .iter()
            .any(|e| matches!(e, PiUiEvent::MessageStart { .. })));
        assert!(recorded
            .iter()
            .any(|e| matches!(e, PiUiEvent::ExtensionUiRequest { id, .. } if id == "dlg-1")));
        assert!(recorded
            .iter()
            .any(|e| matches!(e, PiUiEvent::AgentSettled { .. })));
        drop(recorded);

        // Idempotent shutdown.
        state.shutdown();
        state.shutdown();
        assert!(!state.has_live_supervisor());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validates_empty_boundary_inputs_and_reuses_or_respawns_supervisor_idempotently() {
        let dir = unique_temp_dir("idempotent");
        let store_path = dir.join("providers.json");
        let counter_path = dir.join("spawns.log");

        let script = format!(
            r#"
printf 'spawn\n' >> "{counter}"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  printf '{{"type":"response","id":"%s","command":"%s","success":true}}\n' "$id" "$typ"
done
"#,
            counter = counter_path.display()
        );

        let state = PiAppState::with_config(
            dir.clone(),
            store_path,
            mock_sh_config(dir.clone(), &script),
        );
        let sink: EventSink = Arc::new(|_| {});

        // Boundary validation rejects empty inputs without spawning the child process.
        assert!(state
            .switch_session_with_sink(Arc::clone(&sink), "   ".to_string())
            .is_err());
        assert!(state
            .set_session_name_with_sink(Arc::clone(&sink), "".to_string(), "ok".to_string(), None)
            .is_err());
        assert!(state
            .set_session_name_with_sink(Arc::clone(&sink), "s1".to_string(), "  ".to_string(), None)
            .is_err());
        assert!(state
            .fork_with_sink(Arc::clone(&sink), "".to_string())
            .is_err());
        assert!(state
            .set_model_with_sink(Arc::clone(&sink), "".to_string(), "m".to_string())
            .is_err());
        assert!(state
            .set_model_with_sink(Arc::clone(&sink), "p".to_string(), "".to_string())
            .is_err());
        assert!(state
            .extension_ui_response_with_sink(
                Arc::clone(&sink),
                " ".to_string(),
                ExtensionUiAnswer::Cancelled,
            )
            .is_err());
        assert!(!state.has_live_supervisor());

        // Calling ensure_supervisor_with_sink twice reuses the exact same live supervisor.
        let sup1 = state
            .ensure_supervisor_with_sink(Arc::clone(&sink))
            .unwrap();
        sup1.get_state().unwrap();
        let sup2 = state
            .ensure_supervisor_with_sink(Arc::clone(&sink))
            .unwrap();
        assert!(Arc::ptr_eq(&sup1, &sup2));

        // After shutting down the underlying supervisor, ensure_supervisor_with_sink reconciles and respawns.
        sup1.shutdown();
        let sup3 = state
            .ensure_supervisor_with_sink(Arc::clone(&sink))
            .unwrap();
        sup3.get_state().unwrap();
        assert!(!Arc::ptr_eq(&sup1, &sup3));
        assert!(sup3.is_alive());

        let spawns = fs::read_to_string(&counter_path).unwrap();
        assert_eq!(spawns.lines().count(), 2);

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn degraded_bootstrap_with_empty_provider_store_returns_selectable_catalog_models() {
        let dir = unique_temp_dir("degraded-empty-providers");
        let store_path = dir.join("providers.json");
        let missing_cfg = PiSessionConfig {
            program: "/nonexistent/verity-missing-pi-binary".to_string(),
            args: vec![],
            cwd: dir.clone(),
            env: Vec::new(),
            command_timeout: Duration::from_secs(1),
            no_session: false,
        };
        let state = PiAppState::with_config(dir.clone(), store_path, missing_cfg);
        let sink: EventSink = Arc::new(|_| {});

        let data = state.bootstrap_with_sink(sink);
        assert!(!state.has_live_supervisor());
        assert!(!data.models.is_empty());
        assert!(data.models.len() >= 4);
        assert_ne!(data.selected_model.provider, "default");
        assert_ne!(data.selected_model.model_id, "default");
        assert!(data
            .models
            .iter()
            .any(|m| m.provider == "anthropic" && m.model_id == "claude-sonnet-5-5"));
        assert!(data
            .models
            .iter()
            .any(|m| m.provider == "openai" && m.model_id == "gpt-6.1-sol"));

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn degraded_set_thinking_level_succeeds_and_persists_across_bootstrap() {
        let dir = unique_temp_dir("degraded-thinking");
        let store_path = dir.join("providers.json");
        let missing_cfg = PiSessionConfig {
            program: "/nonexistent/verity-missing-pi-binary".to_string(),
            args: vec![],
            cwd: dir.clone(),
            env: Vec::new(),
            command_timeout: Duration::from_secs(1),
            no_session: false,
        };
        let state = PiAppState::with_config(dir.clone(), store_path, missing_cfg);
        let sink: EventSink = Arc::new(|_| {});

        let updated = state
            .set_thinking_level_with_sink(Arc::clone(&sink), ThinkingLevel::High)
            .expect("degraded set_thinking_level should succeed");
        assert_eq!(updated, ThinkingLevel::High);

        let boot = state.bootstrap_with_sink(sink);
        assert_eq!(boot.thinking_level, ThinkingLevel::High);

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn degraded_set_model_succeeds_and_persists_across_bootstrap() {
        let dir = unique_temp_dir("degraded-model");
        let store_path = dir.join("providers.json");
        let missing_cfg = PiSessionConfig {
            program: "/nonexistent/verity-missing-pi-binary".to_string(),
            args: vec![],
            cwd: dir.clone(),
            env: Vec::new(),
            command_timeout: Duration::from_secs(1),
            no_session: false,
        };
        let state = PiAppState::with_config(dir.clone(), store_path, missing_cfg);
        let sink: EventSink = Arc::new(|_| {});

        let updated = state
            .set_model_with_sink(
                Arc::clone(&sink),
                "openai".to_string(),
                "gpt-6.1-sol".to_string(),
            )
            .expect("degraded set_model should succeed");
        assert_eq!(updated.provider, "openai");
        assert_eq!(updated.model_id, "gpt-6.1-sol");

        let boot = state.bootstrap_with_sink(sink);
        assert_eq!(boot.selected_model.provider, "openai");
        assert_eq!(boot.selected_model.model_id, "gpt-6.1-sol");

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_pi_agent_models_and_merges_deduplicated_sources() {
        let dir = unique_temp_dir("agent-models");
        let models_json = dir.join("models.json");
        fs::write(
            &models_json,
            r#"{"providers":{"anthropic":{"models":[{"id":"claude-sonnet-5-5","name":"Claude Sonnet 5.5"},"claude-opus-5"]},"openai":{"models":[{"id":"gpt-6.1-sol","name":"GPT-6.1 Sol"}]}}}"#,
        )
        .unwrap();

        let agent_models = load_pi_agent_models_from_path(&models_json);
        assert_eq!(agent_models.len(), 3);

        let rpc_models = vec![ModelOption {
            provider: "openai".to_string(),
            model_id: "gpt-6.1-sol".to_string(),
            label: "GPT-6.1 Sol RPC".to_string(),
        }];
        let provider_models = vec![
            ModelOption {
                provider: "openai".to_string(),
                model_id: "gpt-6.1-sol".to_string(),
                label: "gpt-6.1-sol".to_string(),
            },
            ModelOption {
                provider: "local-vllm".to_string(),
                model_id: "qwen-2.5".to_string(),
                label: "qwen-2.5".to_string(),
            },
        ];

        let merged = merge_models_with_fallback(rpc_models, provider_models, agent_models);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].label, "GPT-6.1 Sol RPC");
        assert!(merged
            .iter()
            .any(|m| m.provider == "local-vllm" && m.model_id == "qwen-2.5"));
        assert!(merged
            .iter()
            .any(|m| m.provider == "anthropic" && m.model_id == "claude-opus-5"));

        let fallback = merge_models_with_fallback(Vec::new(), Vec::new(), Vec::new());
        assert_eq!(fallback, default_catalog_models());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_default_pi_config_attaches_extension_flag_when_present() {
        let ext = PathBuf::from("/Users/alice/.verity/extensions/verity-providers.js");
        let with_ext = build_default_pi_config(PathBuf::from("/tmp/repo"), Some(&ext));
        assert!(!with_ext.no_session);
        assert_eq!(
            with_ext.args,
            vec![
                "--mode",
                "rpc",
                "--extension",
                "/Users/alice/.verity/extensions/verity-providers.js",
            ]
        );

        let without_ext = build_default_pi_config(PathBuf::from("/tmp/repo"), None);
        assert!(!without_ext.no_session);
        assert_eq!(without_ext.args, vec!["--mode", "rpc"]);
    }

    #[test]
    fn notify_providers_changed_recycles_idle_supervisor_with_fresh_provider_env() {
        let dir = unique_temp_dir("provider-reload");
        let store_path = dir.join("providers.json");
        let providers_dump = dir.join("providers_env.log");

        let script = format!(
            r#"
printf '%s\n' "$VERITY_DESKTOP_PROVIDERS" >> "{dump}"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  printf '{{"type":"response","id":"%s","command":"%s","success":true}}\n' "$id" "$typ"
done
"#,
            dump = providers_dump.display()
        );

        let state = PiAppState::with_config(
            dir.clone(),
            store_path.clone(),
            mock_sh_config(dir.clone(), &script),
        );
        let sink: EventSink = Arc::new(|_| {});

        // First spawn sees empty provider list.
        let sup1 = state
            .ensure_supervisor_with_sink(Arc::clone(&sink))
            .unwrap();
        sup1.get_state().unwrap();

        // Save a new custom provider and notify PiAppState while Idle.
        let mut store = ProviderStore::default();
        store
            .upsert(ProviderProfileInput {
                id: "openrouter".to_string(),
                name: "OpenRouter".to_string(),
                api: "openai-completions".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                sign_in_url: None,
                models: vec!["qwen/qwen4-max:free".to_string()],
                api_key: Some("sk-or-test-key".to_string()),
            })
            .unwrap();
        store.save_atomic(&store_path).unwrap();
        state.notify_providers_changed();
        assert!(!state.has_live_supervisor());

        // Next call spawns a fresh supervisor with the updated VERITY_DESKTOP_PROVIDERS.
        let sup2 = state
            .ensure_supervisor_with_sink(Arc::clone(&sink))
            .unwrap();
        sup2.get_state().unwrap();
        assert!(!Arc::ptr_eq(&sup1, &sup2));

        let lines: Vec<String> = fs::read_to_string(&providers_dump)
            .unwrap()
            .lines()
            .map(ToString::to_string)
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "[]");
        assert!(lines[1].contains("openrouter"));
        assert!(lines[1].contains("qwen/qwen4-max:free"));
        assert!(!lines[1].contains("sk-or-test-key"));

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tauri_capabilities_grant_event_listen_for_main_window() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let conf_path = manifest_dir.join("tauri.conf.json");
        let conf_raw =
            fs::read_to_string(&conf_path).expect("desktop/src-tauri/tauri.conf.json must exist");
        let conf = parse_json(&conf_raw).expect("tauri.conf.json must be valid JSON");
        let main_label = conf
            .get("app")
            .and_then(|app| app.get("windows"))
            .and_then(JsonValue::as_array)
            .and_then(|windows| windows.first())
            .and_then(|win| win.get("label"))
            .and_then(JsonValue::as_str)
            .expect("tauri.conf.json app.windows[0].label must be set");
        assert_eq!(main_label, "main");

        let cap_path = manifest_dir.join("capabilities").join("default.json");
        let cap_raw = fs::read_to_string(&cap_path)
            .expect("desktop/src-tauri/capabilities/default.json must exist");
        let cap = parse_json(&cap_raw).expect("capabilities/default.json must be valid JSON");

        let windows: Vec<&str> = cap
            .get("windows")
            .and_then(JsonValue::as_array)
            .expect("capabilities/default.json must declare a windows array")
            .iter()
            .filter_map(JsonValue::as_str)
            .collect();
        assert!(
            windows.contains(&main_label),
            "capabilities/default.json must target window label {:?}",
            main_label
        );

        let permissions: Vec<&str> = cap
            .get("permissions")
            .and_then(JsonValue::as_array)
            .expect("capabilities/default.json must declare a permissions array")
            .iter()
            .filter_map(JsonValue::as_str)
            .collect();
        for required in [
            "core:default",
            "core:event:default",
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            "core:event:allow-emit",
            "dialog:allow-open",
        ] {
            assert!(
                permissions.contains(&required),
                "capabilities/default.json missing required permission {required}"
            );
        }
    }

    #[test]
    fn preserves_command_source_and_location_metadata() {
        let cmds_val = parse_json(
            r#"{"commands":[{"trigger":"/","name":"review","description":"Code review","source":"skill","location":"/home/user/.pi/skills/review/SKILL.md"},{"trigger":"/","name":"plan","description":"Plan task","source":"prompt","location":"/repo/.pi/prompts/plan.md"},{"trigger":"/","name":"deploy","description":"Deploy ext","source":"extension"}]}"#,
        )
        .unwrap();

        let cmds = parse_commands_list(&cmds_val);
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0].source.as_deref(), Some("skill"));
        assert_eq!(
            cmds[0].location.as_deref(),
            Some("/home/user/.pi/skills/review/SKILL.md")
        );
        assert_eq!(cmds[1].source.as_deref(), Some("prompt"));
        assert_eq!(
            cmds[1].location.as_deref(),
            Some("/repo/.pi/prompts/plan.md")
        );
        assert_eq!(cmds[2].source.as_deref(), Some("extension"));
        assert_eq!(cmds[2].location, None);

        let infos = parse_command_info_list(&cmds_val);
        assert_eq!(infos.len(), 3);
        assert_eq!(infos[0].name, "review");
        assert_eq!(infos[0].source.as_deref(), Some("skill"));
        assert_eq!(
            infos[0].location.as_deref(),
            Some("/home/user/.pi/skills/review/SKILL.md")
        );
    }

    #[test]
    fn multi_project_add_switch_remove_respawns_pi_in_new_cwd() {
        let root = unique_temp_dir("multi-proj");
        let proj_a = root.join("proj-a");
        let proj_b = root.join("proj-b");
        fs::create_dir_all(&proj_a).unwrap();
        fs::create_dir_all(&proj_b).unwrap();
        let store_path = root.join("providers.json");
        let cwd_log = root.join("cwd.log");

        let script = format!(
            r#"
pwd >> "{cwd_log}"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_state)
      printf '{{"type":"response","id":"%s","command":"get_state","success":true,"data":{{"id":"sess-live","name":"Active","path":"/tmp/live.jsonl"}}}}\n' "$id"
      ;;
    *)
      printf '{{"type":"response","id":"%s","command":"%s","success":true}}\n' "$id" "$typ"
      ;;
  esac
done
"#,
            cwd_log = cwd_log.display()
        );

        let state = PiAppState::with_config(
            proj_a.clone(),
            store_path,
            mock_sh_config(proj_a.clone(), &script),
        );
        let sink: EventSink = Arc::new(|_| {});

        let boot_a = state.bootstrap_with_sink(Arc::clone(&sink));
        assert_eq!(boot_a.projects.len(), 1);
        assert_eq!(
            PathBuf::from(&boot_a.active_project_path),
            fs::canonicalize(&proj_a).unwrap()
        );

        // Add project B -> persists both, switches active, respawns Pi in proj_b.
        let boot_b = state
            .add_project_with_sink(Arc::clone(&sink), proj_b.to_string_lossy().to_string())
            .unwrap();
        assert_eq!(boot_b.projects.len(), 2);
        assert_eq!(
            PathBuf::from(&boot_b.active_project_path),
            fs::canonicalize(&proj_b).unwrap()
        );

        // Switch back to project A -> respawns Pi in proj_a.
        let boot_back = state
            .switch_project_with_sink(Arc::clone(&sink), proj_a.to_string_lossy().to_string())
            .unwrap();
        assert_eq!(
            PathBuf::from(&boot_back.active_project_path),
            fs::canonicalize(&proj_a).unwrap()
        );

        // Remove active project A -> falls back to project B and respawns in proj_b.
        let boot_removed = state
            .remove_project_with_sink(Arc::clone(&sink), proj_a.to_string_lossy().to_string())
            .unwrap();
        assert_eq!(boot_removed.projects.len(), 1);
        assert_eq!(
            PathBuf::from(&boot_removed.active_project_path),
            fs::canonicalize(&proj_b).unwrap()
        );

        let logged_cwds: Vec<PathBuf> = fs::read_to_string(&cwd_log)
            .unwrap()
            .lines()
            .map(PathBuf::from)
            .collect();
        let canon_a = fs::canonicalize(&proj_a).unwrap();
        let canon_b = fs::canonicalize(&proj_b).unwrap();
        assert_eq!(
            logged_cwds,
            vec![canon_a.clone(), canon_b.clone(), canon_a, canon_b]
        );

        state.shutdown();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn set_session_name_switches_session_when_path_differs_and_new_rpc_commands_work() {
        let dir = unique_temp_dir("new-rpcs");
        let store_path = dir.join("providers.json");
        let cmd_log = dir.join("cmds.log");

        let script = format!(
            r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  printf '%s\n' "$typ" >> "{cmd_log}"
  case "$typ" in
    get_state)
      printf '{{"type":"response","id":"%s","command":"get_state","success":true,"data":{{"id":"s-cur","path":"/tmp/current.jsonl"}}}}\n' "$id"
      ;;
    bash)
      printf '{{"type":"response","id":"%s","command":"bash","success":true,"data":{{"output":"hello","exitCode":0}}}}\n' "$id"
      ;;
    get_session_stats)
      printf '{{"type":"response","id":"%s","command":"get_session_stats","success":true,"data":{{"totalTokens":1234,"cost":0.02}}}}\n' "$id"
      ;;
    get_last_assistant_text)
      printf '{{"type":"response","id":"%s","command":"get_last_assistant_text","success":true,"data":{{"text":"Last reply"}}}}\n' "$id"
      ;;
    cycle_model)
      printf '{{"type":"response","id":"%s","command":"cycle_model","success":true,"data":{{"model":{{"provider":"anthropic","modelId":"claude-opus-5","label":"Claude Opus 5"}}}}}}\n' "$id"
      ;;
    cycle_thinking_level)
      printf '{{"type":"response","id":"%s","command":"cycle_thinking_level","success":true,"data":{{"level":"high"}}}}\n' "$id"
      ;;
    get_commands)
      printf '{{"type":"response","id":"%s","command":"get_commands","success":true,"data":{{"commands":[{{"name":"review","description":"Review code","source":"skill","location":"/skills/review.md"}}]}}}}\n' "$id"
      ;;
    *)
      printf '{{"type":"response","id":"%s","command":"%s","success":true}}\n' "$id" "$typ"
      ;;
  esac
done
"#,
            cmd_log = cmd_log.display()
        );

        let state = PiAppState::with_config(
            dir.clone(),
            store_path,
            mock_sh_config(dir.clone(), &script),
        );
        let sink: EventSink = Arc::new(|_| {});

        // Renaming a different session_path must issue switch_session before set_session_name.
        state
            .set_session_name_with_sink(
                Arc::clone(&sink),
                "s-other".to_string(),
                "Renamed Other".to_string(),
                Some("/tmp/other.jsonl".to_string()),
            )
            .unwrap();

        let bash_res = state
            .bash_with_sink(Arc::clone(&sink), "echo hello".to_string())
            .unwrap();
        assert_eq!(bash_res["output"], "hello");
        assert_eq!(bash_res["exitCode"], 0);

        state.abort_bash_with_sink(Arc::clone(&sink)).unwrap();

        let stats = state
            .get_session_stats_with_sink(Arc::clone(&sink))
            .unwrap();
        assert_eq!(stats["totalTokens"], 1234);

        let last = state
            .get_last_assistant_text_with_sink(Arc::clone(&sink))
            .unwrap();
        assert_eq!(last.as_deref(), Some("Last reply"));

        state
            .set_auto_compaction_with_sink(Arc::clone(&sink), true)
            .unwrap();
        state
            .set_auto_retry_with_sink(Arc::clone(&sink), false)
            .unwrap();
        state
            .set_steering_mode_with_sink(Arc::clone(&sink), "one-at-a-time".to_string())
            .unwrap();
        assert!(state
            .set_steering_mode_with_sink(Arc::clone(&sink), "invalid".to_string())
            .is_err());
        state
            .set_follow_up_mode_with_sink(Arc::clone(&sink), "all".to_string())
            .unwrap();

        let cycled_model = state.cycle_model_with_sink(Arc::clone(&sink)).unwrap();
        assert_eq!(cycled_model["model"]["modelId"], "claude-opus-5");

        let cycled_thinking = state
            .cycle_thinking_level_with_sink(Arc::clone(&sink))
            .unwrap();
        assert_eq!(cycled_thinking["level"], "high");

        let cmds = state.get_commands_with_sink(Arc::clone(&sink)).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].source.as_deref(), Some("skill"));
        assert_eq!(cmds[0].location.as_deref(), Some("/skills/review.md"));

        let log_lines: Vec<String> = fs::read_to_string(&cmd_log)
            .unwrap()
            .lines()
            .map(ToString::to_string)
            .collect();
        let sw_idx = log_lines
            .iter()
            .position(|c| c == "switch_session")
            .unwrap();
        let rn_idx = log_lines
            .iter()
            .position(|c| c == "set_session_name")
            .unwrap();
        assert!(
            sw_idx < rn_idx,
            "switch_session must happen before set_session_name"
        );

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cycle_model_falls_back_to_merged_models_when_pi_returns_null_and_normalizes_shape() {
        let dir = unique_temp_dir("cycle-model-fallback");
        let store_path = dir.join("providers.json");
        let mut store = ProviderStore::default();
        store
            .upsert(ProviderProfileInput {
                id: "custom-a".to_string(),
                name: "Custom A".to_string(),
                api: "openai-completions".to_string(),
                base_url: "https://a.example.com/v1".to_string(),
                sign_in_url: None,
                models: vec!["model-alpha".to_string(), "model-beta".to_string()],
                api_key: Some("k-a".to_string()),
            })
            .unwrap();
        store.save_atomic(&store_path).unwrap();

        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_available_models)
      printf '{"type":"response","id":"%s","command":"get_available_models","success":true,"data":{"models":[]}}\n' "$id"
      ;;
    cycle_model)
      printf '{"type":"response","id":"%s","command":"cycle_model","success":true,"data":null}\n' "$id"
      ;;
    set_model)
      model_id=$(printf '%s\n' "$line" | sed -n 's/.*"modelId":"\([^"]*\)".*/\1/p')
      provider=$(printf '%s\n' "$line" | sed -n 's/.*"provider":"\([^"]*\)".*/\1/p')
      printf '{"type":"response","id":"%s","command":"set_model","success":true,"data":{"provider":"%s","modelId":"%s","label":"%s"}}\n' "$id" "$provider" "$model_id" "$model_id"
      ;;
    cycle_thinking_level)
      printf '{"type":"response","id":"%s","command":"cycle_thinking_level","success":true,"data":null}\n' "$id"
      ;;
    set_thinking_level)
      lvl=$(printf '%s\n' "$line" | sed -n 's/.*"level":"\([^"]*\)".*/\1/p')
      printf '{"type":"response","id":"%s","command":"set_thinking_level","success":true,"data":{"level":"%s"}}\n' "$id" "$lvl"
      ;;
    *)
      printf '{"type":"response","id":"%s","command":"%s","success":true}\n' "$id" "$typ"
      ;;
  esac
done
"#;

        let state =
            PiAppState::with_config(dir.clone(), store_path, mock_sh_config(dir.clone(), script));
        let sink: EventSink = Arc::new(|_| {});

        state
            .set_model_with_sink(
                Arc::clone(&sink),
                "custom-a".to_string(),
                "model-alpha".to_string(),
            )
            .unwrap();

        let cycled = state.cycle_model_with_sink(Arc::clone(&sink)).unwrap();
        assert_eq!(cycled["cycled"], true);
        assert_eq!(cycled["provider"], "custom-a");
        assert_eq!(cycled["id"], "model-beta");
        assert_eq!(cycled["modelId"], "model-beta");
        assert_eq!(cycled["model"]["modelId"], "model-beta");
        assert!(cycled["reason"].is_null());

        let t1 = state
            .cycle_thinking_level_with_sink(Arc::clone(&sink))
            .unwrap();
        assert_eq!(t1["level"], "high");
        assert_eq!(t1["thinkingLevel"], "high");
        let t2 = state
            .cycle_thinking_level_with_sink(Arc::clone(&sink))
            .unwrap();
        assert_eq!(t2["level"], "xhigh");
        let t3 = state
            .cycle_thinking_level_with_sink(Arc::clone(&sink))
            .unwrap();
        assert_eq!(t3["level"], "off");

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn revert_turn_and_redo_work_with_snapshot_and_missing_snapshot_note() {
        let _env_lock = crate::snapshots::tests::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = unique_temp_dir("revert-turn-e2e");
        let verity_home = dir.join("verity-home");
        let prev_vh = env::var("VERITY_HOME").ok();
        env::set_var("VERITY_HOME", &verity_home);

        let project_dir = dir.join("proj");
        fs::create_dir_all(&project_dir).unwrap();
        let code_file = project_dir.join("main.rs");
        fs::write(&code_file, "fn main() { /* v1 */ }\n").unwrap();

        let store_path = dir.join("providers.json");
        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_state)
      printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"id":"sess-rev","name":"Rev Session"}}\n' "$id"
      ;;
    get_tree)
      printf '{"type":"response","id":"%s","command":"get_tree","success":true,"data":{"nodes":[{"id":"u-1","parentId":null,"role":"user","preview":"Refactor main","timestamp":"t1","depth":0,"isActiveBranch":true},{"id":"a-1","parentId":"u-1","role":"assistant","preview":"Done","timestamp":"t2","depth":1,"isActiveBranch":true},{"id":"u-2","parentId":"a-1","role":"user","preview":"Unrecorded turn","timestamp":"t3","depth":2,"isActiveBranch":true}]}}\n' "$id"
      ;;
    get_fork_messages)
      printf '{"type":"response","id":"%s","command":"get_fork_messages","success":true,"data":{"messages":[{"entryId":"u-1","role":"user","preview":"Refactor main","timestamp":"t1"},{"entryId":"u-2","role":"user","preview":"Unrecorded turn","timestamp":"t3"}]}}\n' "$id"
      ;;
    get_messages)
      printf '{"type":"response","id":"%s","command":"get_messages","success":true,"data":{"messages":[{"id":"u-1","role":"user","content":"Refactor main","timestamp":"t1"},{"id":"u-2","role":"user","content":"Unrecorded turn","timestamp":"t3"}]}}\n' "$id"
      ;;
    fork)
      printf '{"type":"response","id":"%s","command":"fork","success":true,"data":{"id":"sess-forked","name":"Forked","messages":[{"id":"u-1","role":"user","content":"Refactor main","timestamp":"t1"}]}}\n' "$id"
      ;;
    prompt)
      printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
      printf '{"type":"agent_end"}\n'
      ;;
    new_session)
      printf '{"type":"response","id":"%s","command":"new_session","success":true,"data":{"id":"sess-fresh","name":"New session"}}\n' "$id"
      ;;
    *)
      printf '{"type":"response","id":"%s","command":"%s","success":true}\n' "$id" "$typ"
      ;;
  esac
done
"#;

        let state = PiAppState::with_config(
            project_dir.clone(),
            store_path,
            mock_sh_config(project_dir.clone(), script),
        );
        let (settled_tx, settled_rx) = mpsc::channel();
        let sink: EventSink = Arc::new(move |ev| {
            if matches!(ev, PiUiEvent::AgentSettled { .. }) {
                let _ = settled_tx.send(());
            }
        });

        state
            .prompt_with_sink(Arc::clone(&sink), "Refactor main".to_string(), vec![])
            .unwrap();
        settled_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("agent should settle after prompt");
        let cps = state.list_checkpoints().unwrap();
        assert_eq!(cps.len(), 1);
        assert!(cps[0].has_code_snapshot);
        assert_eq!(cps[0].prompt_preview, "Refactor main");

        fs::write(&code_file, "fn main() { /* v2 */ }\n").unwrap();

        let missing = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-2".to_string(), true)
            .unwrap();
        assert!(!missing.code_reverted);
        assert!(!missing.can_redo);
        assert_eq!(
            missing.code_revert_note.as_deref(),
            Some("No code snapshot was recorded for this turn.")
        );
        assert_eq!(
            fs::read_to_string(&code_file).unwrap(),
            "fn main() { /* v2 */ }\n"
        );

        let reverted = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-1".to_string(), true)
            .unwrap();
        assert!(reverted.code_reverted);
        assert!(reverted.can_redo);
        assert!(reverted.code_revert_note.is_none());
        assert_eq!(reverted.session.id, "sess-fresh");
        assert!(reverted.messages.is_empty());
        assert_eq!(reverted.restored_prompt.as_deref(), Some("Refactor main"));
        assert_eq!(reverted.reverted_files, vec!["main.rs".to_string()]);
        assert_eq!(
            fs::read_to_string(&code_file).unwrap(),
            "fn main() { /* v1 */ }\n"
        );

        let redone = state.redo_revert_with_sink().unwrap();
        assert_eq!(redone.restored_files, vec!["main.rs".to_string()]);
        assert_eq!(
            fs::read_to_string(&code_file).unwrap(),
            "fn main() { /* v2 */ }\n"
        );

        state.shutdown();
        match prev_vh {
            Some(v) => env::set_var("VERITY_HOME", v),
            None => env::remove_var("VERITY_HOME"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bootstrap_groups_sessions_across_projects_and_supports_archive_and_delete() {
        let dir = unique_temp_dir("multi-proj-archive-delete");
        let proj_a = dir.join("proj-alpha");
        let proj_b = dir.join("proj-beta");
        let a_sessions = proj_a.join(".pi").join("sessions");
        let b_sessions = proj_b.join(".pi").join("sessions");
        fs::create_dir_all(&a_sessions).unwrap();
        fs::create_dir_all(&b_sessions).unwrap();

        let canon_a = fs::canonicalize(&proj_a).unwrap();
        let canon_b = fs::canonicalize(&proj_b).unwrap();

        let a1_path = a_sessions.join("a1.jsonl");
        let a2_path = a_sessions.join("a2.jsonl");
        let b1_path = b_sessions.join("b1.jsonl");

        fs::write(
            &a1_path,
            format!(
                r#"{{"type":"session","id":"sess-a1","name":"Alpha One","timestamp":"2026-11-22T10:00:00Z","cwd":"{}"}}"#,
                canon_a.display()
            ),
        )
        .unwrap();
        fs::write(
            &a2_path,
            format!(
                r#"{{"type":"session","id":"sess-a2","name":"Alpha Two","timestamp":"2026-11-21T10:00:00Z","cwd":"{}"}}"#,
                canon_a.display()
            ),
        )
        .unwrap();
        fs::write(
            &b1_path,
            format!(
                r#"{{"type":"session","id":"sess-b1","name":"Beta One","timestamp":"2026-11-23T10:00:00Z","cwd":"{}"}}"#,
                canon_b.display()
            ),
        )
        .unwrap();

        // Mock Pi script tracks current session id/path across switch_session and new_session.
        let script = format!(
            r#"
CUR_ID="sess-a1"
CUR_PATH="{a1}"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_state)
      printf '{{"type":"response","id":"%s","command":"get_state","success":true,"data":{{"id":"%s","name":"Live","path":"%s","model":{{"provider":"anthropic","modelId":"claude-sonnet-5-5","label":"Claude Sonnet 5.5"}},"thinkingLevel":"medium"}}}}\n' "$id" "$CUR_ID" "$CUR_PATH"
      ;;
    get_messages)
      printf '{{"type":"response","id":"%s","command":"get_messages","success":true,"data":{{"messages":[]}}}}\n' "$id"
      ;;
    get_available_models)
      printf '{{"type":"response","id":"%s","command":"get_available_models","success":true,"data":{{"models":[]}}}}\n' "$id"
      ;;
    get_available_thinking_levels)
      printf '{{"type":"response","id":"%s","command":"get_available_thinking_levels","success":true,"data":{{"levels":["medium"]}}}}\n' "$id"
      ;;
    get_commands)
      printf '{{"type":"response","id":"%s","command":"get_commands","success":true,"data":{{"commands":[]}}}}\n' "$id"
      ;;
    switch_session)
      CUR_PATH=$(printf '%s\n' "$line" | sed -n 's/.*"sessionPath":"\([^"]*\)".*/\1/p')
      case "$CUR_PATH" in
        *a2.jsonl) CUR_ID="sess-a2" ;;
        *b1.jsonl) CUR_ID="sess-b1" ;;
        *) CUR_ID="sess-switched" ;;
      esac
      printf '{{"type":"response","id":"%s","command":"switch_session","success":true,"data":{{"messages":[]}}}}\n' "$id"
      ;;
    new_session)
      CUR_ID="sess-fresh"
      CUR_PATH=""
      printf '{{"type":"response","id":"%s","command":"new_session","success":true,"data":{{"id":"sess-fresh","name":"New session"}}}}\n' "$id"
      ;;
    *)
      printf '{{"type":"response","id":"%s","command":"%s","success":true}}\n' "$id" "$typ"
      ;;
  esac
done
"#,
            a1 = a1_path.display()
        );

        let store_path = dir.join("providers.json");
        let state = PiAppState::with_config(
            canon_a.clone(),
            store_path,
            mock_sh_config(canon_a.clone(), &script),
        );
        let sink: EventSink = Arc::new(|_| {});

        // Register project B, then switch back to project A as active.
        let _ = state
            .add_project_with_sink(Arc::clone(&sink), canon_b.to_string_lossy().to_string())
            .unwrap();
        let boot = state
            .switch_project_with_sink(Arc::clone(&sink), canon_a.to_string_lossy().to_string())
            .unwrap();

        let proj_a_id = project_from_path(&canon_a).id;
        let proj_b_id = project_from_path(&canon_b).id;

        // Bootstrap must return sessions for BOTH registered projects, tagged with exact project_id.
        assert_eq!(boot.projects.len(), 2);
        assert_eq!(boot.active_project_id, proj_a_id);
        let s_a1 = boot
            .sessions
            .iter()
            .find(|s| s.id == "sess-a1")
            .expect("sess-a1");
        let s_a2 = boot
            .sessions
            .iter()
            .find(|s| s.id == "sess-a2")
            .expect("sess-a2");
        let s_b1 = boot
            .sessions
            .iter()
            .find(|s| s.id == "sess-b1")
            .expect("sess-b1");
        assert_eq!(s_a1.project_id, proj_a_id);
        assert_eq!(s_a2.project_id, proj_a_id);
        assert_eq!(s_b1.project_id, proj_b_id);
        assert!(!s_a1.archived && !s_a2.archived && !s_b1.archived);

        // Archive sess-b1 and verify persisted archived flag.
        let after_arch = state
            .archive_session_with_sink(Arc::clone(&sink), "sess-b1".to_string(), true)
            .unwrap();
        assert!(
            after_arch
                .sessions
                .iter()
                .find(|s| s.id == "sess-b1")
                .unwrap()
                .archived
        );

        // Unarchive sess-b1.
        let after_unarch = state
            .archive_session_with_sink(Arc::clone(&sink), "sess-b1".to_string(), false)
            .unwrap();
        assert!(
            !after_unarch
                .sessions
                .iter()
                .find(|s| s.id == "sess-b1")
                .unwrap()
                .archived
        );

        // Delete active session sess-a1: must delete a1.jsonl on disk and switch to sess-a2.
        let after_del_a1 = state
            .delete_session_with_sink(Arc::clone(&sink), "sess-a1".to_string())
            .unwrap();
        assert!(!a1_path.exists(), "a1.jsonl must be deleted from disk");
        assert!(after_del_a1.sessions.iter().all(|s| s.id != "sess-a1"));
        assert_eq!(after_del_a1.active_session_id, "sess-a2");

        // Delete remaining active session sess-a2: must delete a2.jsonl and start a fresh session.
        let after_del_a2 = state
            .delete_session_with_sink(Arc::clone(&sink), "sess-a2".to_string())
            .unwrap();
        assert!(!a2_path.exists(), "a2.jsonl must be deleted from disk");
        assert!(after_del_a2.sessions.iter().all(|s| s.id != "sess-a2"));
        assert_eq!(after_del_a2.active_session_id, "sess-fresh");

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }
}
