use crate::attachments::PendingAttachment;
use crate::extension_ui::{DialogId, ExtensionUiAnswer, ExtensionUiError};
use crate::pi_session::{
    EventSink, PiRpcResponse, PiSessionConfig, PiSessionError, PiSessionSupervisor, PiUiEvent,
    ReloadDisposition, ReloadHook,
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
    capture_snapshot, find_checkpoint_with_lineage, load_checkpoint_store,
    normalize_prompt_preview, record_turn_checkpoint, redo_last_revert,
    redo_targeted_revert_checked, revert_to_snapshot, save_last_redo,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
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
pub fn resolve_verity_mcp_bin() -> Option<PathBuf> {
    if let Ok(explicit) = env::var("VERITY_MCP_BIN") {
        let path = PathBuf::from(explicit.trim());
        if path.is_absolute() && path.is_file() {
            return Some(path);
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

    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths).filter(|dir| dir.is_absolute()) {
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
    let bin = verity_mcp_bin
        .map(|path| path.to_string_lossy())
        .unwrap_or_default();
    let args = if bin.is_empty() { "" } else { "mcp" };
    env_pairs.push(("VERITY_MCP_ARGS".to_string(), args.to_string()));
    env_pairs.push(("VERITY_MCP_BIN".to_string(), bin.into_owned()));
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
    let parsed = tauri::Url::parse(trimmed).map_err(|e| format!("invalid external URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("external URL must use http:// or https://".to_string());
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err("external URL must include a non-empty host".to_string());
    }
    Ok(parsed.to_string())
}

#[cfg(windows)]
mod windows_browser {
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            window: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }

    pub(super) fn open(url: &str) -> Result<(), String> {
        open_with(url, |operation, file| unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                operation,
                file,
                ptr::null(),
                ptr::null(),
                1,
            )
        })
    }

    fn open_with(
        url: &str,
        execute: impl FnOnce(*const u16, *const u16) -> isize,
    ) -> Result<(), String> {
        let operation: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        let file: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let result = execute(operation.as_ptr(), file.as_ptr());
        if result <= 32 {
            return Err(format!(
                "failed to launch browser: ShellExecuteW returned {result}"
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn native_open_receives_entire_url_as_one_utf16_file_argument() {
            let url = "https://example.test/auth?a=1&echo(VERITY_SHELL_MARKER&b=%22ok%22#雪";
            open_with(url, |operation, file| {
                let expected_operation: Vec<u16> = "open\0".encode_utf16().collect();
                let expected_file: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
                unsafe {
                    assert_eq!(
                        std::slice::from_raw_parts(operation, expected_operation.len()),
                        expected_operation
                    );
                    assert_eq!(
                        std::slice::from_raw_parts(file, expected_file.len()),
                        expected_file
                    );
                }
                33
            })
            .unwrap();
        }

        #[test]
        fn native_open_reports_os_errors() {
            for code in [0, 2, 5, 31, 32] {
                let error = open_with("https://example.test/", |_, _| code).unwrap_err();
                assert!(error.contains(&code.to_string()));
            }
        }
    }
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
        windows_browser::open(url)
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

fn flatten_history_tree_nodes(
    items: &[JsonValue],
    default_depth: usize,
    out: &mut Vec<HistoryTreeNode>,
) {
    for item in items {
        let node_obj = item
            .get("entry")
            .filter(|v| v.as_object().is_some())
            .unwrap_or(item);
        let msg_obj = node_obj.get("message").filter(|v| v.as_object().is_some());

        let id = opt_str(node_obj, &["id", "entryId", "entry_id"])
            .unwrap_or_else(|| format!("node-{}", out.len() + 1));
        let parent_id = opt_str(node_obj, &["parentId", "parent_id"]);
        let role = normalize_role(
            opt_str(node_obj, &["role"]).or_else(|| msg_obj.and_then(|m| opt_str(m, &["role"]))),
        );
        let preview = opt_str(node_obj, &["preview", "text", "content"])
            .or_else(|| msg_obj.map(extract_message_content))
            .unwrap_or_default();
        let timestamp = opt_timestamp(node_obj, &["timestamp", "createdAt", "created_at"])
            .or_else(|| {
                msg_obj.and_then(|m| opt_timestamp(m, &["timestamp", "createdAt", "created_at"]))
            })
            .unwrap_or_else(|| "now".to_string());
        let depth = opt_usize(item, &["depth"])
            .or_else(|| opt_usize(node_obj, &["depth"]))
            .unwrap_or(default_depth);
        let is_active_branch = opt_bool(item, &["isActiveBranch", "is_active_branch"])
            .or_else(|| opt_bool(node_obj, &["isActiveBranch", "is_active_branch"]))
            .unwrap_or(true);

        out.push(HistoryTreeNode {
            id,
            parent_id,
            role,
            preview,
            timestamp,
            depth,
            is_active_branch,
        });

        if let Some(children) = item.get("children").and_then(JsonValue::as_array) {
            flatten_history_tree_nodes(children, depth + 1, out);
        }
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

    let mut out = Vec::new();
    flatten_history_tree_nodes(items, 0, &mut out);
    out
}

fn fork_candidate_items(val: &JsonValue) -> Option<&[JsonValue]> {
    match val {
        JsonValue::Array(items) => Some(items),
        JsonValue::Object(_) => val
            .get("messages")
            .or_else(|| val.get("candidates"))
            .or_else(|| val.get("entries"))
            .and_then(JsonValue::as_array),
        _ => None,
    }
}

pub fn parse_fork_candidates(val: &JsonValue) -> Vec<ForkCandidate> {
    fork_candidate_items(val)
        .unwrap_or_default()
        .iter()
        .filter_map(|item| {
            let entry_id = opt_str(item, &["entryId", "entry_id", "id"])?;
            if entry_id.trim().is_empty() {
                return None;
            }
            let role = match opt_str(item, &["role"]).as_deref() {
                Some("assistant") => "assistant".to_string(),
                _ => "user".to_string(),
            };
            Some(ForkCandidate {
                entry_id,
                role,
                preview: opt_str(item, &["preview", "text", "content"]).unwrap_or_default(),
                timestamp: opt_timestamp(item, &["timestamp", "createdAt", "created_at"])
                    .unwrap_or_else(|| "now".to_string()),
            })
        })
        .collect()
}

fn checkpoint_candidates(sup: &PiSessionSupervisor) -> Option<Vec<ForkCandidate>> {
    let response = sup.get_fork_messages().ok()?;
    let data = parse_data_json(&response)?;
    let count = fork_candidate_items(&data)?.len();
    let candidates = parse_fork_candidates(&data);
    (candidates.len() == count).then_some(candidates)
}

fn submitted_turn(
    sup: &PiSessionSupervisor,
    before: &[ForkCandidate],
    prompt: &str,
) -> Option<ForkCandidate> {
    // Pi can accept a prompt before its user entry appears in history.
    for attempt in 0..6 {
        if attempt != 0 {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let mut after = checkpoint_candidates(sup)?;
        if after.len() < before.len()
            || !after
                .iter()
                .zip(before)
                .all(|(a, b)| a.entry_id == b.entry_id)
        {
            return None;
        }
        if after.len() == before.len() {
            continue;
        }
        if after.len() != before.len() + 1 {
            return None;
        }
        let turn = after.pop()?;
        return (turn.role == "user"
            && !is_synthetic_local_entry_id(&turn.entry_id)
            && !before.iter().any(|old| old.entry_id == turn.entry_id)
            && !turn.preview.trim().is_empty()
            && turn
                .preview
                .split_whitespace()
                .eq(prompt.split_whitespace()))
        .then_some(turn);
    }
    None
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
    workspace_transition: Mutex<()>,
    pending_supervisor: Mutex<Option<Arc<PiSessionSupervisor>>>,
    provider_store_path: PathBuf,
    base_config: Option<PiSessionConfig>,
    selected_model: Mutex<Option<ModelOption>>,
    thinking_level: Mutex<ThinkingLevel>,
    model_pending_sync: AtomicBool,
    thinking_level_pending_sync: AtomicBool,
    provider_reload_pending: Arc<AtomicBool>,
    active_session_id: Mutex<Option<String>>,
    session_parents: Mutex<HashMap<String, String>>,
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
                workspace_transition: Mutex::new(()),
                pending_supervisor: Mutex::new(None),
                provider_store_path: default_store_path(),
                base_config: None,
                selected_model: Mutex::new(None),
                thinking_level: Mutex::new(ThinkingLevel::Medium),
                model_pending_sync: AtomicBool::new(false),
                thinking_level_pending_sync: AtomicBool::new(false),
                provider_reload_pending: Arc::new(AtomicBool::new(false)),
                active_session_id: Mutex::new(None),
                session_parents: Mutex::new(HashMap::new()),
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
                workspace_transition: Mutex::new(()),
                pending_supervisor: Mutex::new(None),
                provider_store_path,
                base_config: Some(base_config),
                selected_model: Mutex::new(None),
                thinking_level: Mutex::new(ThinkingLevel::Medium),
                model_pending_sync: AtomicBool::new(false),
                thinking_level_pending_sync: AtomicBool::new(false),
                provider_reload_pending: Arc::new(AtomicBool::new(false)),
                active_session_id: Mutex::new(None),
                session_parents: Mutex::new(HashMap::new()),
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
        let _workspace = self.lock_workspace();
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
        let registry = self.inner.registry.lock().expect("registry lock poisoned");
        let mut supervisor = self
            .inner
            .supervisor
            .lock()
            .expect("supervisor lock poisoned");
        if let Some(existing) = supervisor.as_ref() {
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
        }

        let project_path = registry
            .active_project()
            .map(|project| PathBuf::from(&project.path))
            .unwrap_or_else(|| self.inner.default_cwd.clone());
        let arc = self.spawn_supervisor(project_path, event_sink)?;
        let old = supervisor.replace(Arc::clone(&arc));
        drop(supervisor);
        drop(registry);
        if let Some(old) = old {
            old.shutdown();
        }

        self.replay_pending_preferences(&arc);
        Ok(arc)
    }

    fn spawn_supervisor(
        &self,
        project_path: PathBuf,
        event_sink: EventSink,
    ) -> Result<Arc<PiSessionSupervisor>, String> {
        let store = ProviderStore::load(&self.inner.provider_store_path).unwrap_or_default();
        let verity_bin = resolve_verity_mcp_bin();
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
        Ok(Arc::new(spawned))
    }

    fn lock_workspace(&self) -> std::sync::MutexGuard<'_, ()> {
        self.inner
            .workspace_transition
            .lock()
            .expect("workspace lock poisoned")
    }

    /// Replays any model or thinking-level selections made while Pi was unavailable
    /// so a newly started supervisor converges idempotently to `PiAppState`.
    fn require_idle_supervisor(&self) -> Result<(), String> {
        let supervisor = self
            .inner
            .supervisor
            .lock()
            .expect("supervisor lock poisoned");
        if supervisor
            .as_ref()
            .is_some_and(|sup| sup.is_alive() && !sup.activity().is_idle())
        {
            return Err("Wait for Pi to finish before switching projects.".to_string());
        }
        Ok(())
    }

    fn record_session_parent(&self, child_id: &str, parent_id: &str) {
        let child = child_id.trim();
        let parent = parent_id.trim();
        if child.is_empty()
            || parent.is_empty()
            || child == "default-session"
            || parent == "default-session"
            || child == parent
        {
            return;
        }
        self.inner
            .session_parents
            .lock()
            .expect("session_parents lock poisoned")
            .insert(child.to_string(), parent.to_string());
    }

    fn session_lineage(&self, project_dir: &Path, active_session_id: Option<&str>) -> Vec<String> {
        let Some(start) = active_session_id
            .map(str::trim)
            .filter(|sid| !sid.is_empty() && *sid != "default-session")
        else {
            return Vec::new();
        };
        let archived = load_archived_session_ids_from(&self.inner.session_meta_path);
        let project_id = project_from_path(project_dir).id;
        let sessions = scan_project_sessions_with_archived(&project_id, project_dir, &archived);
        let parent_map = self
            .inner
            .session_parents
            .lock()
            .expect("session_parents lock poisoned")
            .clone();

        let mut lineage = Vec::new();
        let mut seen = HashSet::new();
        let mut current = start.to_string();
        while seen.insert(current.clone()) {
            lineage.push(current.clone());
            let disk_parent = sessions
                .iter()
                .find(|s| s.id == current)
                .and_then(|s| s.parent_session_id.as_deref())
                .map(str::trim)
                .filter(|p| !p.is_empty() && *p != "default-session")
                .map(|parent_ref| {
                    sessions
                        .iter()
                        .find(|s| s.id == parent_ref || s.path == parent_ref)
                        .map(|s| s.id.clone())
                        .unwrap_or_else(|| parent_ref.to_string())
                });
            let next = disk_parent.or_else(|| parent_map.get(&current).cloned());
            match next {
                Some(parent_id) if !parent_id.is_empty() && parent_id != "default-session" => {
                    current = parent_id;
                }
                _ => break,
            }
        }
        lineage
    }

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
        let _workspace = self.lock_workspace();
        self.require_idle_supervisor()?;
        let trimmed = require_non_empty("path", &path)?;
        {
            let mut reg = self.inner.registry.lock().expect("registry lock poisoned");
            let mut next = reg.clone();
            next.add_project(Path::new(&trimmed))?;
            next.save_atomic(&self.inner.projects_store_path)?;
            *reg = next;
        }
        self.shutdown();
        Ok(self.bootstrap_with_sink(event_sink))
    }

    pub fn remove_project_with_sink(
        &self,
        event_sink: EventSink,
        path: String,
    ) -> Result<PiBootstrapData, String> {
        let _workspace = self.lock_workspace();
        self.require_idle_supervisor()?;
        let trimmed = require_non_empty("path", &path)?;
        let active_changed = {
            let mut reg = self.inner.registry.lock().expect("registry lock poisoned");
            let mut next = reg.clone();
            let (_removed, active_was_removed, _next) = next.remove_project(Path::new(&trimmed))?;
            next.save_atomic(&self.inner.projects_store_path)?;
            *reg = next;
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
        let _workspace = self.lock_workspace();
        self.require_idle_supervisor()?;
        let trimmed = require_non_empty("path", &path)?;
        {
            let mut reg = self.inner.registry.lock().expect("registry lock poisoned");
            let mut next = reg.clone();
            next.set_active_project(Path::new(&trimmed))?;
            next.save_atomic(&self.inner.projects_store_path)?;
            *reg = next;
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
        let _workspace = self.lock_workspace();
        let session_id = require_non_empty("sessionId", &session_id)?;
        set_session_archived_at(&self.inner.session_meta_path, &session_id, archived)?;
        Ok(self.bootstrap_with_sink(event_sink))
    }

    pub fn delete_session_with_sink(
        &self,
        event_sink: EventSink,
        session_id: String,
    ) -> Result<PiBootstrapData, String> {
        let _workspace = self.lock_workspace();
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

        // If deleting the active session, switch to the next non-archived session in the same
        // project or start a fresh session before deleting the file on disk.
        if is_active {
            let sup = live_sup
                .as_ref()
                .ok_or("Cannot leave the active chat while Pi is unavailable.")?;
            let replacement = disk_sessions.iter().find(|session| {
                session.project_id == active_project.id
                    && session.id != session_id
                    && !session.archived
                    && !session.path.trim().is_empty()
            });
            let response = if let Some(next) = replacement {
                sup.switch_session(&next.path)
            } else {
                sup.new_session(None)
            }
            .map_err(|e| e.to_string())?;
            if activation_cancelled(&response) {
                return Err("Pi cancelled leaving the active chat.".to_string());
            }
            let confirmed = self.active_session_summary(sup, &active_project.id)?;
            if confirmed.id == session_id || replacement.is_some_and(|next| next.id != confirmed.id)
            {
                return Err("Pi did not leave the active chat.".to_string());
            }
            *self
                .inner
                .active_session_id
                .lock()
                .expect("active_session_id lock poisoned") = Some(confirmed.id);
        }

        if !target_path.trim().is_empty() {
            delete_session_file(&target_path)?;
        }
        set_session_archived_at(&self.inner.session_meta_path, &session_id, false)?;

        Ok(self.bootstrap_with_sink(event_sink))
    }

    fn active_session_summary(
        &self,
        sup: &PiSessionSupervisor,
        project_id: &str,
    ) -> Result<PiSessionSummary, String> {
        let state = sup.get_state().map_err(|e| e.to_string())?;
        let data = parse_data_json(&state).ok_or("Pi did not return its active session.")?;
        let mut session = parse_session_summary(&data, project_id, "", "Current session");
        if session.id.is_empty() || session.project_id != project_id {
            return Err(
                "Pi did not confirm the active session for the selected project.".to_string(),
            );
        }
        session.archived =
            load_archived_session_ids_from(&self.inner.session_meta_path).contains(&session.id);
        Ok(session)
    }

    fn confirmed_session(
        &self,
        sup: &PiSessionSupervisor,
        project_id: &str,
    ) -> Result<SessionWithMessages, String> {
        let mut session = self.active_session_summary(sup, project_id)?;
        let response = sup.get_messages().map_err(|e| e.to_string())?;
        let messages = parse_data_json(&response)
            .map(|data| parse_chat_messages(&data))
            .ok_or("Pi did not return its active transcript.")?;
        session.message_count = messages.len();
        Ok(SessionWithMessages { session, messages })
    }

    fn staged_supervisor(
        &self,
        project: &PiProject,
        event_sink: EventSink,
    ) -> Result<(Arc<PiSessionSupervisor>, Arc<AtomicBool>), String> {
        let committed = Arc::new(AtomicBool::new(false));
        let route_events = Arc::clone(&committed);
        let sink: EventSink = Arc::new(move |event| {
            if route_events.load(Ordering::Acquire)
                || matches!(event, PiUiEvent::ExtensionUiRequest { .. })
            {
                event_sink(event);
            }
        });
        let sup = self.spawn_supervisor(PathBuf::from(&project.path), sink)?;
        Ok((sup, committed))
    }

    fn run_staged<R>(
        &self,
        candidate: &Arc<PiSessionSupervisor>,
        operation: impl FnOnce(&PiSessionSupervisor) -> R,
    ) -> R {
        *self
            .inner
            .pending_supervisor
            .lock()
            .expect("pending supervisor lock poisoned") = Some(Arc::clone(candidate));
        let result = operation(candidate);
        self.inner
            .pending_supervisor
            .lock()
            .expect("pending supervisor lock poisoned")
            .take();
        result
    }

    fn select_workspace_session(
        &self,
        event_sink: EventSink,
        project_id: Option<&str>,
        expected_session_id: Option<&str>,
        operation: impl FnOnce(&PiSessionSupervisor, &PiProject) -> Result<PiRpcResponse, String>,
    ) -> Result<Option<SessionWithMessages>, String> {
        let _workspace = self.lock_workspace();
        let (projects, current_project) = self.snapshot_projects();
        let target = match project_id {
            Some(id) => projects
                .iter()
                .find(|project| project.id == id)
                .ok_or("Unknown project.")?,
            None => &current_project,
        };
        let cross_project = current_project.id != target.id;
        self.require_idle_supervisor()?;
        let (candidate, staged) = if cross_project {
            let (candidate, committed) = self.staged_supervisor(target, event_sink)?;
            (candidate, Some(committed))
        } else {
            (self.ensure_supervisor_with_sink(event_sink)?, None)
        };
        let response = if staged.is_some() {
            self.run_staged(&candidate, |sup| operation(sup, target))
        } else {
            operation(&candidate, target)
        }?;
        if activation_cancelled(&response) {
            return Ok(None);
        }
        let confirmed = self.confirmed_session(&candidate, &target.id)?;
        if expected_session_id.is_some_and(|expected| expected != confirmed.session.id) {
            return Err("Pi did not open the selected chat.".to_string());
        }
        if let Some(committed) = staged {
            let mut registry = self.inner.registry.lock().expect("registry lock poisoned");
            let mut updated = registry.clone();
            updated.set_active_project(Path::new(&target.path))?;
            updated.save_atomic(&self.inner.projects_store_path)?;
            let mut supervisor = self
                .inner
                .supervisor
                .lock()
                .expect("supervisor lock poisoned");
            *registry = updated;
            let old = supervisor.replace(Arc::clone(&candidate));
            committed.store(true, Ordering::Release);
            drop(supervisor);
            drop(registry);
            if let Some(old) = old {
                old.shutdown();
            }
            self.replay_pending_preferences(&candidate);
        }
        *self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned") = Some(confirmed.session.id.clone());
        Ok(Some(confirmed))
    }

    fn saved_session(
        &self,
        project: &PiProject,
        session_id: &str,
    ) -> Result<PiSessionSummary, String> {
        let archived = load_archived_session_ids_from(&self.inner.session_meta_path);
        scan_project_sessions_with_archived(&project.id, Path::new(&project.path), &archived)
            .into_iter()
            .find(|session| session.id == session_id)
            .ok_or_else(|| "The selected chat is unavailable in this project.".to_string())
    }

    pub fn switch_session_with_sink(
        &self,
        event_sink: EventSink,
        project_id: String,
        session_id: String,
    ) -> Result<Option<SessionWithMessages>, String> {
        let session_id = require_non_empty("sessionId", &session_id)?;
        self.select_workspace_session(
            event_sink,
            Some(&project_id),
            Some(&session_id),
            |sup, project| {
                let session = self.saved_session(project, &session_id)?;
                sup.switch_session(&session.path).map_err(|e| e.to_string())
            },
        )
    }

    pub fn new_session_with_sink(
        &self,
        event_sink: EventSink,
        project_id: Option<String>,
        parent_session: Option<String>,
    ) -> Result<Option<SessionWithMessages>, String> {
        let mut result =
            self.select_workspace_session(event_sink, project_id.as_deref(), None, |sup, _| {
                sup.new_session(parent_session.clone())
                    .map_err(|e| e.to_string())
            })?;
        if let Some(confirmed) = result.as_mut() {
            if let Some(ref parent_id) = parent_session {
                self.record_session_parent(&confirmed.session.id, parent_id);
            }
            confirmed.session.parent_session_id = parent_session;
        }
        Ok(result)
    }

    pub fn set_session_name_with_sink(
        &self,
        event_sink: EventSink,
        project_id: String,
        session_id: String,
        name: String,
    ) -> Result<PiSessionSummary, String> {
        let _workspace = self.lock_workspace();
        let session_id = require_non_empty("sessionId", &session_id)?;
        let name = require_non_empty("name", &name)?;
        let (projects, active_project) = self.snapshot_projects();
        let project = projects
            .iter()
            .find(|project| project.id == project_id)
            .ok_or("Unknown project.")?;

        if active_project.id == project_id {
            let sup = self.ensure_supervisor_with_sink(Arc::clone(&event_sink))?;
            if self.active_session_summary(&sup, &project_id)?.id == session_id {
                sup.set_session_name(&name).map_err(|e| e.to_string())?;
                return self.active_session_summary(&sup, &project_id);
            }
        }

        let mut saved = self.saved_session(project, &session_id)?;
        let (candidate, _) = self.staged_supervisor(project, event_sink)?;
        self.run_staged(&candidate, |sup| {
            let response = sup.switch_session(&saved.path).map_err(|e| e.to_string())?;
            if activation_cancelled(&response) {
                return Err("Pi cancelled opening the selected chat.".to_string());
            }
            if self.active_session_summary(sup, &project_id)?.id != session_id {
                return Err("Pi did not open the selected chat.".to_string());
            }
            sup.set_session_name(&name).map_err(|e| e.to_string())?;
            saved.name = self.active_session_summary(sup, &project_id)?.name;
            Ok(saved)
        })
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
        let _workspace = self.lock_workspace();
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.fork(&entry_id).map_err(|e| e.to_string())?;
        if activation_cancelled(&resp) {
            return Err("Pi cancelled the chat operation.".to_string());
        }
        let result =
            self.parse_session_with_messages(&sup, &resp, "session-fork", "Forked session")?;
        let mut active_guard = self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned");
        let prev_session_id = active_guard.replace(result.session.id.clone());
        drop(active_guard);
        if let Some(parent_id) = result
            .session
            .parent_session_id
            .as_deref()
            .or(prev_session_id.as_deref())
        {
            self.record_session_parent(&result.session.id, parent_id);
        }
        Ok(result)
    }

    pub fn clone_with_sink(
        &self,
        event_sink: EventSink,
        entry_id: Option<String>,
    ) -> Result<SessionWithMessages, String> {
        let _workspace = self.lock_workspace();
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let resp = sup.clone_session(entry_id).map_err(|e| e.to_string())?;
        if activation_cancelled(&resp) {
            return Err("Pi cancelled the chat operation.".to_string());
        }
        let result =
            self.parse_session_with_messages(&sup, &resp, "session-clone", "Cloned session")?;
        let mut active_guard = self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned");
        let prev_session_id = active_guard.replace(result.session.id.clone());
        drop(active_guard);
        if let Some(parent_id) = result
            .session
            .parent_session_id
            .as_deref()
            .or(prev_session_id.as_deref())
        {
            self.record_session_parent(&result.session.id, parent_id);
        }
        Ok(result)
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
        project_id: String,
        session_id: String,
    ) -> Result<(), String> {
        let _workspace = self.lock_workspace();
        let (_, current_project) = self.snapshot_projects();
        let sup = self.ensure_supervisor_with_sink(event_sink)?;
        let response = sup.get_state().map_err(|e| e.to_string())?;
        let current_session = parse_data_json(&response)
            .and_then(|data| opt_str(&data, &["sessionId", "id", "session_id"]));
        if current_project.id != project_id || current_session.as_deref() != Some(&session_id) {
            return Err("The active chat changed. Reopen the chat before sending.".to_string());
        }
        let project_dir = PathBuf::from(current_project.path);
        let snapshot_and_history = checkpoint_candidates(&sup).and_then(|turns| {
            capture_snapshot(&project_dir)
                .ok()
                .map(|tree_oid| (tree_oid, turns))
        });
        let response = sup
            .prompt(message.clone(), images)
            .map_err(|e| e.to_string())?;
        let disposition = parse_data_json(&response).and_then(|v| opt_str(&v, &["disposition"]));
        if let Some((tree_oid, before)) = snapshot_and_history {
            if !matches!(disposition.as_deref(), Some("queued" | "handled")) {
                if let Some(turn) = submitted_turn(&sup, &before, &message) {
                    let _ = record_turn_checkpoint(
                        &project_dir,
                        session_id,
                        turn.entry_id,
                        None,
                        message,
                        tree_oid,
                    );
                }
            }
        }
        Ok(())
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
        let lineage = self.session_lineage(&project_dir, active_session.as_deref());
        if lineage.is_empty() {
            return Ok(Vec::new());
        }
        let lineage_set: HashSet<&str> = lineage.iter().map(String::as_str).collect();

        let filtered: Vec<TurnCheckpointView> = store
            .checkpoints
            .iter()
            .filter(|cp| lineage_set.contains(cp.session_id.trim()))
            .map(|cp| TurnCheckpointView {
                entry_id: cp.entry_id.clone(),
                prompt_preview: cp.prompt_preview.clone(),
                has_code_snapshot: cp.tree_oid.len() == 40,
                created_at: cp.created_at.clone(),
            })
            .collect();

        Ok(filtered)
    }

    pub fn revert_turn_with_sink(
        &self,
        event_sink: EventSink,
        entry_id: String,
        revert_code: bool,
        user_turn_index: Option<usize>,
        prompt_text: Option<String>,
    ) -> Result<RevertTurnResult, String> {
        let entry_id = require_non_empty("entryId", &entry_id)?;
        let _workspace = self.lock_workspace();
        self.require_idle_supervisor()?;
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

        let fork_candidates = checkpoint_candidates(&sup).unwrap_or_default();

        // Step 2: Resolve target user turn in priority order:
        // (1) exact entry_id match, (2) user_turn_index, (3) normalized prompt_text match.
        let resolved: Option<(usize, &ForkCandidate)> = fork_candidates
            .iter()
            .enumerate()
            .find(|(_, c)| c.entry_id == entry_id)
            .or_else(|| user_turn_index.and_then(|idx| fork_candidates.get(idx).map(|c| (idx, c))))
            .or_else(|| {
                prompt_text
                    .as_deref()
                    .map(normalize_prompt_preview)
                    .filter(|norm| !norm.is_empty())
                    .and_then(|norm| {
                        let mut matches = fork_candidates
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| normalize_prompt_preview(&c.preview) == norm);
                        let first = matches.next()?;
                        matches.next().is_none().then_some(first)
                    })
            });

        enum RevertTargetPlan {
            Fork {
                target_idx: usize,
                target_entry_id: String,
                target_preview: Option<String>,
            },
            FallbackNewSession,
        }

        let plan = if let Some((target_idx, target)) = resolved {
            if is_synthetic_local_entry_id(&target.entry_id) {
                return Err(format!(
                    "Refusing to fork synthetic entry ID '{}'",
                    target.entry_id
                ));
            }
            RevertTargetPlan::Fork {
                target_idx,
                target_entry_id: target.entry_id.clone(),
                target_preview: (!target.preview.trim().is_empty()).then(|| target.preview.clone()),
            }
        } else if fork_candidates.is_empty() && user_turn_index.unwrap_or(0) == 0 {
            RevertTargetPlan::FallbackNewSession
        } else {
            return Err(format!(
                "Unable to resolve entry ID '{}' to a valid user turn for forking.",
                entry_id
            ));
        };

        let project_dir = self.project_path();
        let project_id = project_from_path(&project_dir).id;

        let pending_code_outcome = if let (
            true,
            RevertTargetPlan::Fork {
                target_entry_id,
                target_preview,
                ..
            },
        ) = (revert_code, &plan)
        {
            let store = load_checkpoint_store(&project_dir);
            let lineage = self.session_lineage(&project_dir, session_id.as_deref());
            let preview = target_preview
                .as_deref()
                .or(prompt_text.as_deref())
                .map(normalize_prompt_preview);
            find_checkpoint_with_lineage(&store, &lineage, target_entry_id, preview.as_deref())?
                .map(|checkpoint| revert_to_snapshot(&project_dir, &checkpoint.tree_oid))
                .transpose()?
        } else {
            None
        };

        let fallback_new_session =
            |sup: &PiSessionSupervisor| -> Result<SessionWithMessages, String> {
                let resp = sup.new_session(None).map_err(|e| e.to_string())?;
                if activation_cancelled(&resp) {
                    return Err("Revert was cancelled.".to_string());
                }
                let data = parse_data_json(&resp);
                let has_session_id = data.as_ref().is_some_and(|v| {
                    let obj = v.get("session").unwrap_or(v);
                    opt_str(obj, &["id", "sessionId", "session_id"]).is_some()
                });
                let summary = if has_session_id {
                    parse_session_summary(
                        data.as_ref().unwrap(),
                        &project_id,
                        "session-new",
                        "New session",
                    )
                } else if let Ok(state_resp) = sup.get_state() {
                    parse_data_json(&state_resp)
                        .map(|v| {
                            parse_session_summary(&v, &project_id, "session-new", "New session")
                        })
                        .unwrap_or_else(|| PiSessionSummary {
                            id: "session-new".to_string(),
                            project_id: project_id.clone(),
                            name: "New session".to_string(),
                            path: String::new(),
                            updated_at: "Just now".to_string(),
                            message_count: 0,
                            parent_session_id: None,
                            archived: false,
                        })
                } else {
                    PiSessionSummary {
                        id: "session-new".to_string(),
                        project_id: project_id.clone(),
                        name: "New session".to_string(),
                        path: String::new(),
                        updated_at: "Just now".to_string(),
                        message_count: 0,
                        parent_session_id: None,
                        archived: false,
                    }
                };
                Ok(SessionWithMessages {
                    session: summary,
                    messages: Vec::new(),
                })
            };

        let chat_result: Result<(SessionWithMessages, Option<String>, bool), String> = match plan {
            RevertTargetPlan::Fork {
                target_idx,
                target_entry_id,
                target_preview,
            } => match sup.fork(&target_entry_id) {
                Ok(resp) => {
                    let fork_data = parse_data_json(&resp);
                    if fork_data.as_ref().and_then(|v| opt_bool(v, &["cancelled"])) == Some(true) {
                        Err("Revert was cancelled.".to_string())
                    } else {
                        let fork_text = fork_data
                            .as_ref()
                            .and_then(|v| opt_str(v, &["text"]))
                            .filter(|s| !s.trim().is_empty());
                        let prompt = fork_text.or(target_preview).or_else(|| {
                            prompt_text
                                .as_deref()
                                .filter(|s| !s.trim().is_empty())
                                .map(ToString::to_string)
                        });
                        self.parse_session_with_messages(
                            &sup,
                            &resp,
                            "session-revert",
                            "Reverted session",
                        )
                        .map(|swm| (swm, prompt, true))
                    }
                }
                Err(_) if target_idx == 0 => {
                    let prompt = target_preview.or_else(|| {
                        prompt_text
                            .as_deref()
                            .filter(|s| !s.trim().is_empty())
                            .map(ToString::to_string)
                    });
                    fallback_new_session(&sup).map(|swm| (swm, prompt, false))
                }
                Err(err) => Err(err.to_string()),
            },
            RevertTargetPlan::FallbackNewSession => {
                let prompt = prompt_text
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string);
                fallback_new_session(&sup).map(|swm| (swm, prompt, false))
            }
        };

        let (SessionWithMessages { session, messages }, restored_prompt, forked_from_parent) =
            match chat_result {
                Ok(ok) => ok,
                Err(err) => {
                    if let Some(outcome) = pending_code_outcome {
                        if let Err(rollback) = redo_targeted_revert_checked(
                            &project_dir,
                            &outcome.redo_tree_oid,
                            &outcome.reverted_files,
                            Some(&outcome.target_tree_oid),
                        ) {
                            return Err(format!(
                                "{err}. Could not undo the code restore after Pi failed: {rollback}"
                            ));
                        }
                    }
                    return Err(err);
                }
            };

        if forked_from_parent {
            if let Some(parent_id) = session
                .parent_session_id
                .as_deref()
                .or(session_id.as_deref())
            {
                self.record_session_parent(&session.id, parent_id);
            }
        }

        *self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned") = Some(session.id.clone());

        let (code_reverted, reverted_files, can_redo, code_revert_note) = if revert_code {
            if let Some(outcome) = pending_code_outcome {
                let save_result = save_last_redo(
                    &project_dir,
                    outcome.redo_tree_oid,
                    outcome.reverted_files.clone(),
                    outcome.target_tree_oid,
                    &session.id,
                );
                let note = save_result.err().map(|err| {
                    format!("Reverted chat and code. The redo checkpoint could not be saved. {err}")
                });
                (true, outcome.reverted_files, note.is_none(), note)
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
        let _workspace = self.lock_workspace();
        let project_dir = self.project_path();
        let session_id = self
            .inner
            .active_session_id
            .lock()
            .expect("active_session_id lock poisoned")
            .clone();
        let restored_files = redo_last_revert(&project_dir, session_id.as_deref())?;
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
        let _workspace = self.lock_workspace();
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
        let _workspace = self.lock_workspace();
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
        let pending = self
            .inner
            .pending_supervisor
            .lock()
            .expect("pending supervisor lock poisoned")
            .clone();
        if let Some(pending) = pending {
            match pending.extension_ui_response(&parsed_id, answer.clone()) {
                Ok(_) => return Ok(()),
                Err(PiSessionError::ExtensionUi(ExtensionUiError::UnknownDialog(_))) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
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
        let has_session_id = data.as_ref().is_some_and(|v| {
            let obj = v.get("session").unwrap_or(v);
            opt_str(obj, &["id", "sessionId", "session_id"]).is_some()
        });
        let mut session = if has_session_id {
            parse_session_summary(
                data.as_ref().unwrap(),
                &project_id,
                fallback_id,
                fallback_name,
            )
        } else if let Ok(state_resp) = sup.get_state() {
            parse_data_json(&state_resp)
                .map(|v| parse_session_summary(&v, &project_id, fallback_id, fallback_name))
                .unwrap_or_else(|| PiSessionSummary {
                    id: fallback_id.to_string(),
                    project_id: project_id.clone(),
                    name: fallback_name.to_string(),
                    path: String::new(),
                    updated_at: "Just now".to_string(),
                    message_count: 0,
                    parent_session_id: None,
                    archived: false,
                })
        } else {
            PiSessionSummary {
                id: fallback_id.to_string(),
                project_id: project_id.clone(),
                name: fallback_name.to_string(),
                path: String::new(),
                updated_at: "Just now".to_string(),
                message_count: 0,
                parent_session_id: None,
                archived: false,
            }
        };

        let has_messages = data
            .as_ref()
            .is_some_and(|v| v.get("messages").and_then(JsonValue::as_array).is_some());
        let mut messages = if has_messages {
            data.as_ref().map(parse_chat_messages).unwrap_or_default()
        } else if let Ok(msgs_resp) = sup.get_messages() {
            parse_data_json(&msgs_resp)
                .map(|v| parse_chat_messages(&v))
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        if messages.is_empty() && !has_messages {
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

fn activation_cancelled(response: &PiRpcResponse) -> bool {
    parse_data_json(response)
        .as_ref()
        .and_then(|data| opt_bool(data, &["cancelled"]))
        .unwrap_or(false)
}

fn is_synthetic_local_entry_id(id: &str) -> bool {
    let trimmed = id.trim();
    trimmed.starts_with("local-") || trimmed.starts_with("stream-") || trimmed.starts_with("msg-")
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
    tauri::async_runtime::spawn_blocking(move || {
        let _workspace = app_state.lock_workspace();
        Ok(app_state.bootstrap_with_sink(sink))
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn pi_switch_session(
    app: AppHandle,
    state: State<'_, PiAppState>,
    project_id: String,
    session_id: String,
) -> Result<Option<SessionWithMessages>, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.switch_session_with_sink(sink, project_id, session_id)
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
) -> Result<Option<SessionWithMessages>, String> {
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
    project_id: String,
    session_id: String,
    name: String,
) -> Result<PiSessionSummary, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.set_session_name_with_sink(sink, project_id, session_id, name)
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
    project_id: String,
    session_id: String,
) -> Result<(), String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.prompt_with_sink(sink, message, images, project_id, session_id)
    })
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
    user_turn_index: Option<usize>,
    prompt_text: Option<String>,
) -> Result<RevertTurnResult, String> {
    let app_state = state.inner().clone();
    let sink = tauri_event_sink(&app);
    tauri::async_runtime::spawn_blocking(move || {
        app_state.revert_turn_with_sink(sink, entry_id, revert_code, user_turn_index, prompt_text)
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
            program: "sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            cwd,
            env: Vec::new(),
            command_timeout: Duration::from_secs(5),
            no_session: false,
        }
    }

    #[test]
    fn restore_model_pi_boundaries() {
        use crate::snapshots::checkpoints_path_for_project;
        use crate::snapshots::tests::{
            observe_restore, print_restore_trace, with_checkpoint_write_blocked,
            RestoreObservation, TestEnv,
        };

        let script = r#"
const fs = require("node:fs");
const lines = require("node:readline").createInterface({ input: process.stdin });
const [scenario, checkpoints] = process.argv.slice(1);
let session = "Parent";
function observe(event) {
  const read = (name) => fs.readFileSync(name, "utf8").trim();
  const redo = JSON.parse(read(checkpoints)).lastRedo !== null;
  const state = [event, [read("a.txt"), read("z.txt")], read("sibling.txt"), session, redo];
  fs.appendFileSync("../pi-restore-trace.jsonl", JSON.stringify(state) + "\n");
}
lines.on("line", (line) => {
  const input = JSON.parse(line);
  const reply = { type: "response", id: input.id, command: input.type, success: true };
  switch (input.type) {
    case "get_state":
      reply.data = { id: session, name: session };
      break;
    case "get_messages":
      reply.data = { messages: [] };
      break;
    case "get_fork_messages":
      reply.data = { messages: [
        { entryId: "first", role: "user", preview: "First" },
        { entryId: "second", role: "user", preview: "Second" },
      ] };
      break;
    case "fork":
      observe("PiFork");
      if (scenario === "compensation_stale") {
        fs.writeFileSync("a.txt", "Edited\n");
        observe("EditedPath");
      }
      if (scenario === "fallback" || scenario.startsWith("compensation")) {
        reply.success = false;
        reply.error = "Test Pi rejected fork";
        if (scenario !== "fallback") observe("PiRejected");
      } else {
        session = "Child";
        observe("PiAccepted");
        reply.data = { id: session, name: session, messages: [] };
      }
      break;
    case "new_session":
      observe("PiNewSession");
      session = "Child";
      observe("PiAccepted");
      reply.data = { id: session, name: session };
      break;
  }
  console.log(JSON.stringify(reply));
});
"#;

        for scenario in [
            "fallback",
            "compensation",
            "compensation_stale",
            "save_failure",
            "clear_failure",
        ] {
            let env = TestEnv::new(scenario, true);
            env.write("a.txt", "Checkpoint\n");
            env.write("z.txt", "Checkpoint\n");
            env.write("sibling.txt", "Before\n");
            let target = env.capture();
            let entry = if scenario == "fallback" {
                "first"
            } else {
                "second"
            };
            record_turn_checkpoint(
                &env.project,
                "Parent".into(),
                entry.into(),
                None,
                String::new(),
                target.clone(),
            )
            .unwrap();
            env.write("a.txt", "Before\n");
            env.write("z.txt", "Before\n");
            let before = env.capture();

            let mut config = PiSessionConfig::default_pi(env.project.clone());
            config.program = "node".into();
            config.args = vec![
                "-e".into(),
                script.into(),
                scenario.into(),
                checkpoints_path_for_project(&env.project)
                    .to_string_lossy()
                    .into_owned(),
            ];
            let state = PiAppState::with_config(
                env.project.clone(),
                env.root.join("providers.json"),
                config,
            );
            let sink: EventSink = Arc::new(|_| {});
            let boot = state.bootstrap_with_sink(Arc::clone(&sink));
            assert_eq!(boot.active_session_id, "Parent");
            let revert =
                || state.revert_turn_with_sink(Arc::clone(&sink), entry.into(), true, None, None);
            let result = if scenario == "save_failure" {
                with_checkpoint_write_blocked(&env.project, revert)
            } else {
                revert()
            };
            let mut trace: Vec<RestoreObservation> =
                fs::read_to_string(env.root.join("pi-restore-trace.jsonl"))
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
            let active_session = || {
                state
                    .inner
                    .active_session_id
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap()
            };
            let mut observe =
                |event| trace.push(observe_restore(&env.project, event, &active_session()));

            if scenario.starts_with("compensation") {
                let err = result.unwrap_err();
                assert!(err.contains("Test Pi rejected fork"), "{err}");
                let stale = scenario == "compensation_stale";
                assert_eq!(err.contains("stale redo"), stale, "{err}");
                let a_expected = if stale { "Edited\n" } else { "Before\n" };
                assert_eq!(
                    fs::read_to_string(env.project.join("a.txt")).unwrap(),
                    a_expected
                );
                assert_eq!(
                    fs::read_to_string(env.project.join("z.txt")).unwrap(),
                    if stale { "Checkpoint\n" } else { "Before\n" }
                );
                assert!(load_checkpoint_store(&env.project).last_redo.is_none());
                observe(if stale {
                    "CompensationStale"
                } else {
                    "Compensated"
                });
            } else {
                let reverted = result.unwrap();
                assert_eq!(reverted.session.id, "Child");
                assert!(reverted.code_reverted);
                assert_eq!(reverted.reverted_files, ["a.txt", "z.txt"]);
                assert_eq!(env.capture(), target);
                if scenario == "save_failure" {
                    assert!(!reverted.can_redo);
                    assert!(reverted
                        .code_revert_note
                        .unwrap()
                        .contains("failed to write temp checkpoints file"));
                    assert!(load_checkpoint_store(&env.project).last_redo.is_none());
                    observe("RedoSaveFailed");
                } else {
                    assert!(reverted.can_redo);
                    let persisted = load_checkpoint_store(&env.project).last_redo.unwrap();
                    assert_eq!(persisted.session_id.as_deref(), Some("Child"));
                    assert_eq!(persisted.target_tree_oid.as_deref(), Some(target.as_str()));
                    assert_eq!(persisted.redo_tree_oid, before);
                    assert_eq!(persisted.files, ["a.txt", "z.txt"]);
                    observe("RedoSaved");
                    if scenario == "clear_failure" {
                        let err = redo_last_revert(&env.project, Some("Parent")).unwrap_err();
                        assert!(
                            err.contains("active session does not match revert owner"),
                            "{err}"
                        );
                        observe("RedoWrongOwner");
                        env.write("sibling.txt", "Edited\n");
                        observe("EditedSibling");
                        let err = with_checkpoint_write_blocked(&env.project, || {
                            state.redo_revert_with_sink()
                        })
                        .unwrap_err();
                        assert!(
                            err.contains("failed to write temp checkpoints file"),
                            "{err}"
                        );
                        assert_eq!(
                            load_checkpoint_store(&env.project).last_redo.as_ref(),
                            Some(&persisted)
                        );
                        observe("RedoClearFailed");
                        assert!(state
                            .redo_revert_with_sink()
                            .unwrap_err()
                            .contains("stale redo"));
                        observe("RedoStale");
                    } else {
                        assert_eq!(
                            state.redo_revert_with_sink().unwrap().restored_files,
                            ["a.txt", "z.txt"]
                        );
                        assert_eq!(env.capture(), before);
                        assert!(load_checkpoint_store(&env.project).last_redo.is_none());
                        observe("RedoCleared");
                        assert!(state
                            .redo_revert_with_sink()
                            .unwrap_err()
                            .contains("No revert operation"));
                        observe("RedoUnavailable");
                    }
                }
            }
            state.shutdown();
            print_restore_trace(scenario, &trace);
        }
    }

    #[test]
    fn session_wire_replies_commit_only_confirmed_workspace_and_prompt_target() {
        let root = unique_temp_dir("session-wire");
        let alpha = root.join("alpha");
        let beta = root.join("beta");
        fs::create_dir(&alpha).unwrap();
        fs::create_dir(&beta).unwrap();
        let histories = beta.join(".pi").join("sessions");
        fs::create_dir_all(&histories).unwrap();
        for id in ["saved", "cancel"] {
            fs::write(
                histories.join(format!("{id}.jsonl")),
                serde_json::json!({
                    "type": "session", "id": id, "cwd": &beta
                })
                .to_string(),
            )
            .unwrap();
        }
        let script = r#"
const { basename } = require("node:path");
const fs = require("node:fs");
const lines = require("node:readline").createInterface({ input: process.stdin });
const project = basename(process.cwd());
let session = "initial-" + project;
let serial = 0;
let name = session;
let file;
lines.on("line", (line) => {
  const input = JSON.parse(line);
  let data;
  switch (input.type) {
    case "get_state":
      data = { sessionId: session, sessionName: name, sessionFile: file ?? project + "/" + session + ".jsonl" };
      break;
    case "get_messages":
      data = { messages: [{ role: "user", content: project + ":" + session }] };
      break;
    case "switch_session":
      const selected = basename(input.sessionPath, ".jsonl");
      data = { cancelled: selected === "cancel" };
      if (!data.cancelled) {
        session = selected;
        file = input.sessionPath;
        name = fs.readFileSync(file, "utf8").trim().split("\n")
          .map((line) => JSON.parse(line)).at(-1).name ?? session;
      }
      break;
    case "new_session":
      data = { cancelled: input.parentSession === "cancel" };
      if (!data.cancelled) {
        session = "new-" + project + "-" + (++serial);
        name = session;
        file = undefined;
      }
      break;
    case "set_session_name":
      name = input.name;
      if (file) fs.appendFileSync(file, "\n" + JSON.stringify({ type: "session_info", name }) + "\n");
      break;
    case "prompt":
      if (input.message !== project + ":" + session) {
        console.log(JSON.stringify({ type: "response", id: input.id, command: input.type,
          success: false, error: "misrouted prompt" }));
        return;
      }
  }
  console.log(JSON.stringify({ type: "response", id: input.id, command: input.type,
    success: true, data }));
});
"#;
        let mut config = PiSessionConfig::default_pi(alpha.clone());
        config.program = "node".to_string();
        config.args = vec!["-e".to_string(), script.to_string()];
        let state = PiAppState::with_config(alpha.clone(), root.join("providers.json"), config);
        let sink: EventSink = Arc::new(|_| {});
        let added = state
            .add_project_with_sink(Arc::clone(&sink), beta.to_string_lossy().into_owned())
            .unwrap();
        let beta_id = added.active_project_id;
        let initial = state
            .switch_project_with_sink(Arc::clone(&sink), alpha.to_string_lossy().into_owned())
            .unwrap();
        let alpha_id = initial.active_project_id;
        let initial_id = initial.active_session_id;

        let cancelled = state
            .switch_session_with_sink(Arc::clone(&sink), beta_id.clone(), "cancel".to_string())
            .unwrap();
        assert!(cancelled.is_none());
        assert_eq!(
            state.project_path(),
            crate::projects::normalize_project_path(&alpha)
        );
        assert_eq!(
            state
                .bootstrap_with_sink(Arc::clone(&sink))
                .active_session_id,
            initial_id
        );

        let renamed = state
            .set_session_name_with_sink(
                Arc::clone(&sink),
                beta_id.clone(),
                "saved".to_string(),
                "Saved title".to_string(),
            )
            .unwrap();
        assert_eq!(renamed.name, "Saved title");
        let still_active = state.bootstrap_with_sink(Arc::clone(&sink));
        assert_eq!(still_active.active_project_id, alpha_id);
        assert_eq!(still_active.active_session_id, initial_id);
        assert!(state
            .set_session_name_with_sink(
                Arc::clone(&sink),
                beta_id.clone(),
                "cancel".to_string(),
                "Wrong title".to_string(),
            )
            .is_err());

        let selected = state
            .switch_session_with_sink(Arc::clone(&sink), beta_id.clone(), "saved".to_string())
            .unwrap()
            .unwrap();
        assert_eq!(selected.session.id, "saved");
        assert_eq!(selected.session.name, "Saved title");
        assert_eq!(selected.session.project_id, beta_id);
        assert_eq!(selected.messages[0].content, "beta:saved");
        assert_eq!(
            state.project_path(),
            crate::projects::normalize_project_path(&beta)
        );
        assert_eq!(
            ProjectRegistry::load(&root.join("projects.json"))
                .unwrap()
                .active_project()
                .unwrap()
                .id,
            beta_id
        );
        assert!(state
            .prompt_with_sink(
                Arc::clone(&sink),
                "beta:saved".to_string(),
                vec![],
                alpha_id,
                initial_id,
            )
            .is_err());

        let deletion =
            state.delete_session_with_sink(Arc::clone(&sink), selected.session.id.clone());
        assert_eq!(
            deletion.unwrap_err(),
            "Pi cancelled leaving the active chat."
        );
        assert!(histories.join("saved.jsonl").exists());

        let cancelled = state
            .new_session_with_sink(
                Arc::clone(&sink),
                Some(beta_id.clone()),
                Some("cancel".to_string()),
            )
            .unwrap();
        assert!(cancelled.is_none());
        assert_eq!(
            state
                .bootstrap_with_sink(Arc::clone(&sink))
                .active_session_id,
            selected.session.id
        );
        fs::remove_file(histories.join("cancel.jsonl")).unwrap();
        let replacement = state
            .delete_session_with_sink(Arc::clone(&sink), selected.session.id)
            .unwrap();
        assert_eq!(replacement.active_session_id, "new-beta-1");
        assert!(!histories.join("saved.jsonl").exists());

        let mut last = replacement.sessions[0].clone();
        for expected in ["new-beta-2", "new-beta-3"] {
            let result = state
                .new_session_with_sink(Arc::clone(&sink), Some(beta_id.clone()), None)
                .unwrap()
                .unwrap();
            assert_eq!(result.session.id, expected);
            assert_ne!(result.session.path, last.path);
            last = result.session;
        }
        state
            .prompt_with_sink(
                Arc::clone(&sink),
                "beta:new-beta-3".to_string(),
                vec![],
                beta_id,
                last.id,
            )
            .unwrap();
        state.shutdown();
        let _ = fs::remove_dir_all(root);
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
        for malformed in [
            "https://:443/path",
            "https://[broken]/",
            "https://example.com:bad/",
            "https://a%00b/",
        ] {
            assert!(
                validate_external_open_url(malformed).is_err(),
                "{malformed}"
            );
        }
        let query = "https://example.test/auth?a=1&echo(VERITY_SHELL_MARKER&b=%22ok%22#done";
        assert_eq!(validate_external_open_url(query).unwrap(), query);
        assert_eq!(
            validate_external_open_url("https://[::1]:3000/path?x=1&y=2").unwrap(),
            "https://[::1]:3000/path?x=1&y=2"
        );
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
        let histories = dir.join(".pi").join("sessions");
        fs::create_dir_all(&histories).unwrap();
        fs::write(
            histories.join("sess-2.jsonl"),
            r#"{"type":"session","id":"sess-2"}"#,
        )
        .unwrap();
        let fake_verity_bin = dir.join("target").join("debug").join(if cfg!(windows) {
            "verity.exe"
        } else {
            "verity"
        });
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
ACTIVE=sess-live
TITLE="Live Session"
CONTENT=hi
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_state)
      printf '{{"type":"response","id":"%s","command":"get_state","success":true,"data":{{"sessionId":"%s","sessionName":"%s","sessionFile":"/tmp/%s.jsonl","thinkingLevel":"high","model":{{"provider":"openai-dev","modelId":"gpt-6.1-sol","label":"GPT-6.1 Sol"}}}}}}\n' "$id" "$ACTIVE" "$TITLE" "$ACTIVE"
      ;;
    get_messages)
      printf '{{"type":"response","id":"%s","command":"get_messages","success":true,"data":{{"messages":[{{"id":"m1","role":"user","content":"%s","timestamp":"10:00"}}]}}}}\n' "$id" "$CONTENT"
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
      ACTIVE=sess-2
      CONTENT=switched
      printf '{{"type":"response","id":"%s","command":"switch_session","success":true,"data":{{"cancelled":false}}}}\n' "$id"
      ;;
    new_session)
      ACTIVE=sess-new
      CONTENT=fresh
      printf '{{"type":"response","id":"%s","command":"new_session","success":true,"data":{{"cancelled":false}}}}\n' "$id"
      ;;
    set_session_name)
      TITLE="Renamed Session"
      printf '{{"type":"response","id":"%s","command":"set_session_name","success":true}}\n' "$id"
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

        let dumped_env = fs::read_to_string(&env_dump_path).expect("env dump written");
        assert!(dumped_env.contains("KEY=sk-test-env-injection-42"));
        let launched_mcp = dumped_env
            .lines()
            .find_map(|line| line.strip_prefix("MCP="));
        assert_ne!(
            launched_mcp.and_then(|path| fs::canonicalize(path).ok()),
            Some(fs::canonicalize(fake_verity_bin).unwrap()),
        );

        // 2. pi_switch_session
        let switched = state
            .switch_session_with_sink(
                Arc::clone(&sink),
                boot.active_project_id.clone(),
                "sess-2".to_string(),
            )
            .unwrap();
        let switched = switched.unwrap();
        assert_eq!(switched.session.id, "sess-2");
        assert_eq!(switched.messages[0].content, "switched");

        // 3. pi_new_session
        let created = state
            .new_session_with_sink(Arc::clone(&sink), None, Some("sess-live".to_string()))
            .unwrap();
        let created = created.unwrap();
        assert_eq!(created.session.id, "sess-new");
        assert_eq!(
            created.session.parent_session_id.as_deref(),
            Some("sess-live")
        );

        // 4. pi_set_session_name
        let renamed = state
            .set_session_name_with_sink(
                Arc::clone(&sink),
                boot.active_project_id.clone(),
                created.session.id.clone(),
                "Renamed Session".to_string(),
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
            .prompt_with_sink(
                Arc::clone(&sink),
                "run check".to_string(),
                vec![img],
                boot.active_project_id,
                created.session.id,
            )
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
            .switch_session_with_sink(Arc::clone(&sink), "p1".to_string(), "   ".to_string())
            .is_err());
        assert!(state
            .set_session_name_with_sink(
                Arc::clone(&sink),
                "".to_string(),
                "".to_string(),
                "ok".to_string()
            )
            .is_err());
        assert!(state
            .set_session_name_with_sink(
                Arc::clone(&sink),
                "p1".to_string(),
                "s1".to_string(),
                "  ".to_string()
            )
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
(pwd -W 2>/dev/null || pwd) >> "{cwd_log}"
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
            .map(|path| fs::canonicalize(path).unwrap())
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
    fn additional_rpc_commands_use_the_live_supervisor() {
        let dir = unique_temp_dir("new-rpcs");
        let store_path = dir.join("providers.json");

        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_state)
      printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"id":"s-cur","path":"/tmp/current.jsonl"}}\n' "$id"
      ;;
    bash)
      printf '{"type":"response","id":"%s","command":"bash","success":true,"data":{"output":"hello","exitCode":0}}\n' "$id"
      ;;
    get_session_stats)
      printf '{"type":"response","id":"%s","command":"get_session_stats","success":true,"data":{"totalTokens":1234,"cost":0.02}}\n' "$id"
      ;;
    get_last_assistant_text)
      printf '{"type":"response","id":"%s","command":"get_last_assistant_text","success":true,"data":{"text":"Last reply"}}\n' "$id"
      ;;
    cycle_model)
      printf '{"type":"response","id":"%s","command":"cycle_model","success":true,"data":{"model":{"provider":"anthropic","modelId":"claude-opus-5","label":"Claude Opus 5"}}}\n' "$id"
      ;;
    cycle_thinking_level)
      printf '{"type":"response","id":"%s","command":"cycle_thinking_level","success":true,"data":{"level":"high"}}\n' "$id"
      ;;
    get_commands)
      printf '{"type":"response","id":"%s","command":"get_commands","success":true,"data":{"commands":[{"name":"review","description":"Review code","source":"skill","location":"/skills/review.md"}]}}\n' "$id"
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
PROMPTED=0
FRESH=0
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
      turns=''
      if [ "$PROMPTED" = 1 ]; then
        turns='{"entryId":"u-1","role":"user","text":"Refactor main"}'
      fi
      if [ "$FRESH" = 1 ]; then
        turns='{"entryId":"u-2","role":"user","text":"Unrecorded turn"}'
      fi
      printf '{"type":"response","id":"%s","command":"get_fork_messages","success":true,"data":{"messages":[%s]}}\n' "$id" "$turns"
      ;;
    get_messages)
      printf '{"type":"response","id":"%s","command":"get_messages","success":true,"data":{"messages":[{"id":"u-1","role":"user","content":"Refactor main","timestamp":"t1"},{"id":"u-2","role":"user","content":"Unrecorded turn","timestamp":"t3"}]}}\n' "$id"
      ;;
    fork)
      eid=$(printf '%s\n' "$line" | sed -n 's/.*"entryId":"\([^"]*\)".*/\1/p')
      if [ "$eid" = "u-1" ] && [ "$FRESH" = 0 ]; then
        printf '{"type":"response","id":"%s","command":"fork","success":false,"error":"cannot fork first turn"}\n' "$id"
      else
        printf '{"type":"response","id":"%s","command":"fork","success":true,"data":{"id":"sess-forked","name":"Forked","messages":[{"id":"u-1","role":"user","content":"Refactor main","timestamp":"t1"}]}}\n' "$id"
      fi
      ;;
    prompt)
      PROMPTED=1
      printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
      printf '{"type":"agent_end"}\n'
      ;;
    new_session)
      FRESH=1
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

        let initial = state.bootstrap_with_sink(Arc::clone(&sink));
        state
            .prompt_with_sink(
                Arc::clone(&sink),
                "Refactor main".to_string(),
                vec![],
                initial.active_project_id,
                initial.active_session_id,
            )
            .unwrap();
        settled_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("agent should settle after prompt");
        let cps = state.list_checkpoints().unwrap();
        assert_eq!(cps.len(), 1);
        assert!(cps[0].has_code_snapshot);
        assert_eq!(cps[0].entry_id, "u-1");
        assert_eq!(cps[0].prompt_preview, "Refactor main");

        fs::write(&code_file, "fn main() { /* v2 */ }\n").unwrap();

        let reverted = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-1".to_string(), true, None, None)
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

        let missing = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-2".to_string(), true, None, None)
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
                r#"{{"type":"session","id":"sess-a1","name":"Alpha One","timestamp":"2026-11-22T10:00:00Z","cwd":{}}}"#,
                verity_core::json::escape_json_string(&canon_a.to_string_lossy())
            ),
        )
        .unwrap();
        fs::write(
            &a2_path,
            format!(
                r#"{{"type":"session","id":"sess-a2","name":"Alpha Two","timestamp":"2026-11-21T10:00:00Z","cwd":{}}}"#,
                verity_core::json::escape_json_string(&canon_a.to_string_lossy())
            ),
        )
        .unwrap();
        fs::write(
            &b1_path,
            format!(
                r#"{{"type":"session","id":"sess-b1","name":"Beta One","timestamp":"2026-11-23T10:00:00Z","cwd":{}}}"#,
                verity_core::json::escape_json_string(&canon_b.to_string_lossy())
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
            a1 = a1_path.to_string_lossy().replace('\\', "/")
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

    #[test]
    fn revert_turn_resolves_local_ids_via_fork_messages_and_never_sends_local_id_to_fork() {
        let dir = unique_temp_dir("revert-local-ids");
        let project_dir = dir.join("repo");
        fs::create_dir_all(&project_dir).unwrap();
        let store_path = dir.join("providers.json");

        // Fake Pi that strictly rejects any synthetic local-*/stream-*/msg-* ID on `fork`,
        // and returns real Pi-shaped `{ text, cancelled }` without `{ session, messages }`.
        let script = r#"
FORK_COUNT=0
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_state)
      if [ "$FORK_COUNT" -eq 0 ]; then
        printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"id":"sess-live-1","name":"Live Session"}}\n' "$id"
      else
        printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"id":"sess-forked-%s","name":"Forked Real Session"}}\n' "$id" "$FORK_COUNT"
      fi
      ;;
    get_fork_messages)
      printf '{"type":"response","id":"%s","command":"get_fork_messages","success":true,"data":{"messages":[{"entryId":"srv-u1","role":"user","preview":"First real prompt","timestamp":"t1"},{"entryId":"srv-u2","role":"user","preview":"Second real prompt","timestamp":"t2"}]}}\n' "$id"
      ;;
    get_messages)
      if [ "$FORK_COUNT" -eq 1 ]; then
        printf '{"type":"response","id":"%s","command":"get_messages","success":true,"data":{"messages":[{"id":"srv-u1","role":"user","content":"First real prompt","timestamp":"t1"}]}}\n' "$id"
      else
        printf '{"type":"response","id":"%s","command":"get_messages","success":true,"data":{"messages":[]}}\n' "$id"
      fi
      ;;
    fork)
      eid=$(printf '%s\n' "$line" | sed -n 's/.*"entryId":"\([^"]*\)".*/\1/p')
      case "$eid" in
        local-*|stream-*|msg-*)
          printf '{"type":"response","id":"%s","command":"fork","success":false,"error":"Invalid entry ID for forking"}\n' "$id"
          ;;
        srv-u2)
          FORK_COUNT=1
          printf '{"type":"response","id":"%s","command":"fork","success":true,"data":{"text":"Second real prompt","cancelled":false}}\n' "$id"
          ;;
        srv-u1)
          FORK_COUNT=2
          printf '{"type":"response","id":"%s","command":"fork","success":true,"data":{"text":"First real prompt","cancelled":false}}\n' "$id"
          ;;
        *)
          printf '{"type":"response","id":"%s","command":"fork","success":false,"error":"Unknown entry ID"}\n' "$id"
          ;;
      esac
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
        let sink: EventSink = Arc::new(|_| {});

        // 1. Revert "local-2" resolved via user_turn_index = Some(1) -> forks "srv-u2" directly,
        // refreshes real session metadata via get_state() + get_messages().
        let rev2 = state
            .revert_turn_with_sink(
                Arc::clone(&sink),
                "local-2".to_string(),
                false,
                Some(1),
                Some("Second real prompt".to_string()),
            )
            .expect("local-2 must resolve to srv-u2 and succeed");
        assert_eq!(rev2.session.id, "sess-forked-1");
        assert_eq!(rev2.session.name, "Forked Real Session");
        assert_eq!(rev2.messages.len(), 1);
        assert_eq!(rev2.messages[0].id, "srv-u1");
        assert_eq!(rev2.restored_prompt.as_deref(), Some("Second real prompt"));

        // 2. Revert "local-1" resolved via normalized prompt_text (even without user_turn_index).
        let rev1 = state
            .revert_turn_with_sink(
                Arc::clone(&sink),
                "local-1".to_string(),
                false,
                None,
                Some("  First   real prompt  ".to_string()),
            )
            .expect("local-1 must resolve via prompt_text to srv-u1 and succeed");
        assert_eq!(rev1.session.id, "sess-forked-2");
        assert!(rev1.messages.is_empty());
        assert_eq!(rev1.restored_prompt.as_deref(), Some("First real prompt"));

        // 3. Unresolved local ID with no matching index/prompt must fail cleanly without calling fork.
        let err = state
            .revert_turn_with_sink(
                Arc::clone(&sink),
                "local-999".to_string(),
                false,
                None,
                None,
            )
            .unwrap_err();
        assert!(
            err.contains("Unable to resolve entry ID"),
            "unexpected error: {}",
            err
        );

        state.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn chained_revert_resolves_entry_identity_and_compensates_when_pi_fails() {
        let _env_lock = crate::snapshots::tests::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = unique_temp_dir("chained-revert-lineage");
        let home = dir.join("verity-home");
        let project_dir = dir.join("repo");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&project_dir).unwrap();
        let prev_vh = env::var("VERITY_HOME").ok();
        env::set_var("VERITY_HOME", &home);

        // Initialize git so .gitignore rules apply cleanly.
        assert!(Command::new("git")
            .args(["init", project_dir.to_str().unwrap()])
            .status()
            .unwrap()
            .success());

        let code_file = project_dir.join("app.rs");
        fs::write(&code_file, "v0\n").unwrap();
        let secret_file = project_dir.join("secret.env");
        fs::write(&secret_file, "initial-secret\n").unwrap();

        let store_path = dir.join("providers.json");
        let script = r#"
CUR_ID="sess-root"
FORK_SEQ=0
ROOT_TURNS=0
CHILD_TURNS=0
CHILD_VISIBLE=0
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s\n' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$typ" in
    get_state)
      printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"id":"%s","name":"Live"}}\n' "$id" "$CUR_ID"
      ;;
    get_messages)
      printf '{"type":"response","id":"%s","command":"get_messages","success":true,"data":{"messages":[]}}\n' "$id"
      ;;
    get_available_models)
      printf '{"type":"response","id":"%s","command":"get_available_models","success":true,"data":{"models":[]}}\n' "$id"
      ;;
    get_available_thinking_levels)
      printf '{"type":"response","id":"%s","command":"get_available_thinking_levels","success":true,"data":{"levels":["medium"]}}\n' "$id"
      ;;
    get_commands)
      printf '{"type":"response","id":"%s","command":"get_commands","success":true,"data":{"commands":[]}}\n' "$id"
      ;;
    prompt)
      if printf '%s' "$line" | grep -q 'FAIL_THIS_PROMPT'; then
        printf '{"type":"response","id":"%s","command":"prompt","success":false,"error":"prompt rejected"}\n' "$id"
      else
        if [ "$CUR_ID" = "sess-root" ]; then
          ROOT_TURNS=$((ROOT_TURNS + 1))
        else
          CHILD_TURNS=$((CHILD_TURNS + 1))
        fi
        printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
        printf '{"type":"agent_end","summary":"done"}\n'
      fi
      ;;
    get_fork_messages)
      turns=''
      if [ "$ROOT_TURNS" -ge 1 ]; then
        turns='{"entryId":"u-1","role":"user","preview":"Turn one"}'
      fi
      if [ "$CUR_ID" = "sess-root" ] && [ "$ROOT_TURNS" -ge 2 ]; then
        turns="$turns,{\"entryId\":\"u-2\",\"role\":\"user\",\"preview\":\"Turn two\"}"
      fi
      if [ "$CUR_ID" = "sess-fork-1" ] && [ "$CHILD_VISIBLE" = 1 ]; then
        turns="$turns,{\"entryId\":\"u-child\",\"role\":\"user\",\"preview\":\"Turn one\"}"
      fi
      printf '{"type":"response","id":"%s","command":"get_fork_messages","success":true,"data":{"messages":[%s]}}\n' "$id" "$turns"
      if [ "$CHILD_TURNS" -ge 1 ]; then CHILD_VISIBLE=1; fi
      ;;
    fork)
      eid=$(printf '%s\n' "$line" | sed -n 's/.*"entryId":"\([^"]*\)".*/\1/p')
      if [ -f ../reject-fork ]; then
        cat app.rs > ../observed-during-fork
        if [ -f ../conflict-during-fork ]; then printf 'changed during fork\n' > app.rs; fi
        printf '{"type":"response","id":"%s","command":"fork","success":false,"error":"Pi rejected fork"}\n' "$id"
        continue
      fi
      FORK_SEQ=$((FORK_SEQ + 1))
      CUR_ID="sess-fork-$FORK_SEQ"
      if [ "$eid" = "u-2" ]; then
        printf '{"type":"response","id":"%s","command":"fork","success":true,"data":{"text":"Turn two","cancelled":false}}\n' "$id"
      else
        printf '{"type":"response","id":"%s","command":"fork","success":true,"data":{"text":"Turn one","cancelled":false}}\n' "$id"
      fi
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

        let boot = state.bootstrap_with_sink(Arc::clone(&sink));
        assert_eq!(boot.active_session_id, "sess-root");

        // Record foreign + default-session checkpoints; list_checkpoints must never leak them.
        crate::snapshots::record_turn_checkpoint(
            &project_dir,
            "sess-foreign".into(),
            "u-1".into(),
            Some(0),
            "Turn one".into(),
            "1111111111111111111111111111111111111111".into(),
        )
        .unwrap();
        crate::snapshots::record_turn_checkpoint(
            &project_dir,
            "default-session".into(),
            "u-1".into(),
            Some(0),
            "Turn one".into(),
            "2222222222222222222222222222222222222222".into(),
        )
        .unwrap();
        assert!(state.list_checkpoints().unwrap().is_empty());

        // Failed prompt must NOT persist an orphan checkpoint.
        assert!(state
            .prompt_with_sink(
                Arc::clone(&sink),
                "FAIL_THIS_PROMPT".to_string(),
                vec![],
                boot.active_project_id.clone(),
                "sess-root".to_string(),
            )
            .is_err());
        assert!(state.list_checkpoints().unwrap().is_empty());

        // Turn 1 (captures v0 synchronously before prompt returns).
        state
            .prompt_with_sink(
                Arc::clone(&sink),
                "Turn one".to_string(),
                vec![],
                boot.active_project_id.clone(),
                "sess-root".to_string(),
            )
            .unwrap();
        settled_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        // Immediately visible without any polling sleep!
        assert_eq!(state.list_checkpoints().unwrap().len(), 1);

        // Turn 2 (captures v1 synchronously).
        fs::write(&code_file, "v1\n").unwrap();
        state
            .prompt_with_sink(
                Arc::clone(&sink),
                "Turn two".to_string(),
                vec![],
                boot.active_project_id.clone(),
                "sess-root".to_string(),
            )
            .unwrap();
        settled_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(state.list_checkpoints().unwrap().len(), 2);

        // Edit to v2, then revert to Turn two -> forks into sess-fork-1 and restores v1.
        fs::write(&code_file, "v2\n").unwrap();
        fs::write(dir.join("reject-fork"), "").unwrap();
        let err = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-2".to_string(), true, None, None)
            .unwrap_err();
        assert!(err.contains("Pi rejected fork"), "{err}");
        assert_eq!(
            fs::read_to_string(dir.join("observed-during-fork")).unwrap(),
            "v1\n"
        );
        assert_eq!(fs::read_to_string(&code_file).unwrap(), "v2\n");
        assert!(state.redo_revert_with_sink().is_err());

        fs::write(dir.join("conflict-during-fork"), "").unwrap();
        let err = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-2".to_string(), true, None, None)
            .unwrap_err();
        assert!(
            err.contains("Pi rejected fork") && err.contains("stale redo"),
            "{err}"
        );
        assert_eq!(
            fs::read_to_string(&code_file).unwrap(),
            "changed during fork\n"
        );
        assert!(state.redo_revert_with_sink().is_err());
        fs::remove_file(dir.join("reject-fork")).unwrap();
        fs::remove_file(dir.join("conflict-during-fork")).unwrap();
        fs::write(&code_file, "v2\n").unwrap();
        let rev_t2 = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-2".to_string(), true, None, None)
            .unwrap();
        assert!(rev_t2.code_reverted);
        assert_eq!(rev_t2.session.id, "sess-fork-1");
        assert_eq!(fs::read_to_string(&code_file).unwrap(), "v1\n");

        // list_checkpoints on sess-fork-1 sees parent lineage checkpoints (2) and still excludes foreign/default.
        assert_eq!(state.list_checkpoints().unwrap().len(), 2);

        // Add secret.env to .gitignore and edit it -> preflight collision must fail BEFORE forking Pi!
        fs::write(project_dir.join(".gitignore"), "secret.env\n").unwrap();
        fs::write(&secret_file, "edited-secret\n").unwrap();
        let err = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-1".to_string(), true, None, None)
            .unwrap_err();
        assert!(err.contains("preflight collision"), "unexpected err: {err}");
        // Chat session MUST NOT have forked to sess-fork-2, and secret.env must be untouched.
        assert_eq!(
            state.inner.active_session_id.lock().unwrap().as_deref(),
            Some("sess-fork-1")
        );
        assert_eq!(fs::read_to_string(&secret_file).unwrap(), "edited-secret\n");
        assert_eq!(fs::read_to_string(&code_file).unwrap(), "v1\n");

        fs::remove_file(project_dir.join(".gitignore")).unwrap();
        fs::write(&code_file, "child-before\n").unwrap();
        state
            .prompt_with_sink(
                Arc::clone(&sink),
                "Turn one".to_string(),
                vec![],
                boot.active_project_id.clone(),
                "sess-fork-1".to_string(),
            )
            .unwrap();
        settled_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        fs::write(&code_file, "child-after\n").unwrap();

        let ambiguous = state
            .revert_turn_with_sink(
                Arc::clone(&sink),
                "local-1".to_string(),
                true,
                None,
                Some("Turn one".to_string()),
            )
            .unwrap_err();
        assert!(ambiguous.contains("Unable to resolve entry ID"));
        assert_eq!(fs::read_to_string(&code_file).unwrap(), "child-after\n");

        let project_store = crate::snapshots::load_checkpoint_store(&project_dir);
        let mut legacy = project_store.clone();
        for (index, checkpoint) in legacy.checkpoints.iter_mut().enumerate() {
            if matches!(checkpoint.entry_id.as_str(), "u-1" | "u-child") {
                checkpoint.entry_id = format!("turn-{}", index + 1);
                checkpoint.turn_index = Some(0);
            }
        }
        crate::snapshots::save_checkpoint_store(&project_dir, &legacy).unwrap();
        let err = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-1".to_string(), true, None, None)
            .unwrap_err();
        assert!(err.contains("older code snapshot"));
        assert_eq!(fs::read_to_string(&code_file).unwrap(), "child-after\n");

        crate::snapshots::save_checkpoint_store(&project_dir, &project_store).unwrap();
        let rev_t1 = state
            .revert_turn_with_sink(Arc::clone(&sink), "u-1".to_string(), true, None, None)
            .unwrap();
        assert!(rev_t1.code_reverted);
        assert_eq!(rev_t1.session.id, "sess-fork-2");
        assert_eq!(fs::read_to_string(&code_file).unwrap(), "v0\n");

        state.shutdown();
        match prev_vh {
            Some(v) => env::set_var("VERITY_HOME", v),
            None => env::remove_var("VERITY_HOME"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_history_tree_flattens_nested_pi_tree_entries() {
        let nested = parse_json(
            r#"{
                "tree": [
                    {
                        "entry": {
                            "id": "e1",
                            "parentId": null,
                            "type": "message",
                            "timestamp": "2026-11-24T10:00:00Z",
                            "message": { "role": "user", "content": "Hello" }
                        },
                        "children": [
                            {
                                "entry": {
                                    "id": "e2",
                                    "parentId": "e1",
                                    "type": "message",
                                    "timestamp": "2026-11-24T10:00:05Z",
                                    "message": { "role": "assistant", "content": [{ "type": "text", "text": "Hi there" }] }
                                },
                                "children": []
                            }
                        ]
                    }
                ]
            }"#,
        )
        .unwrap();

        let nodes = parse_history_tree(&nested);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].id, "e1");
        assert_eq!(nodes[0].parent_id, None);
        assert_eq!(nodes[0].role, "user");
        assert_eq!(nodes[0].preview, "Hello");
        assert_eq!(nodes[0].depth, 0);

        assert_eq!(nodes[1].id, "e2");
        assert_eq!(nodes[1].parent_id.as_deref(), Some("e1"));
        assert_eq!(nodes[1].role, "assistant");
        assert_eq!(nodes[1].preview, "Hi there");
        assert_eq!(nodes[1].depth, 1);
    }
}
