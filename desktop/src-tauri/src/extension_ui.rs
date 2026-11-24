use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, Instant};
use verity_core::json::{to_json_string, JsonValue};

/// Branded identifier for an Extension UI dialog request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DialogId(String);

impl DialogId {
    pub fn parse(raw: impl Into<String>) -> Result<Self, ExtensionUiError> {
        let s = raw.into();
        if s.trim().is_empty() {
            return Err(ExtensionUiError::InvalidDialogId);
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DialogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Supported Pi Extension UI dialog types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialogKind {
    Select,
    Confirm,
    Input,
    Editor,
}

impl DialogKind {
    pub fn parse(raw: &str) -> Result<Self, ExtensionUiError> {
        match raw {
            "select" => Ok(Self::Select),
            "confirm" => Ok(Self::Confirm),
            "input" => Ok(Self::Input),
            "editor" => Ok(Self::Editor),
            other => Err(ExtensionUiError::UnsupportedKind(other.to_string())),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Confirm => "confirm",
            Self::Input => "input",
            Self::Editor => "editor",
        }
    }
}

/// Open Extension UI dialog tracked by `ExtensionUiTracker`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDialog {
    pub id: DialogId,
    pub kind: DialogKind,
    pub title: Option<String>,
    pub options: Vec<String>,
    pub deadline: Option<Instant>,
}

/// User or timeout answer for an Extension UI dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum ExtensionUiAnswer {
    Cancelled,
    Confirmed(bool),
    Value(String),
}

/// Validated response ready to be serialized as a Pi `extension_ui_response` JSONL line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionUiResponse {
    pub id: DialogId,
    pub answer: ExtensionUiAnswer,
}

impl ExtensionUiResponse {
    /// Converts this validated response into a `JsonValue` object matching Pi RPC schema.
    pub fn to_json_value(&self) -> JsonValue {
        let mut map = BTreeMap::new();
        map.insert(
            "type".to_string(),
            JsonValue::String("extension_ui_response".to_string()),
        );
        map.insert("id".to_string(), JsonValue::String(self.id.0.clone()));
        match &self.answer {
            ExtensionUiAnswer::Cancelled => {
                map.insert("cancelled".to_string(), JsonValue::Bool(true));
            }
            ExtensionUiAnswer::Confirmed(b) => {
                map.insert("confirmed".to_string(), JsonValue::Bool(*b));
            }
            ExtensionUiAnswer::Value(v) => {
                map.insert("value".to_string(), JsonValue::String(v.clone()));
            }
        }
        JsonValue::Object(map)
    }

    /// Serializes this response to a single-line JSON string (without trailing newline).
    pub fn to_jsonl(&self) -> String {
        to_json_string(&self.to_json_value())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionUiError {
    InvalidDialogId,
    UnsupportedKind(String),
    DuplicateDialog(DialogId),
    UnknownDialog(DialogId),
    Expired(DialogId),
    InvalidAnswerForKind { id: DialogId, kind: DialogKind },
    InvalidSelection { id: DialogId, value: String },
}

impl fmt::Display for ExtensionUiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDialogId => write!(f, "extension ui dialog id must not be empty"),
            Self::UnsupportedKind(k) => write!(f, "unsupported extension ui dialog kind '{}'", k),
            Self::DuplicateDialog(id) => write!(f, "dialog '{}' is already open", id),
            Self::UnknownDialog(id) => write!(f, "dialog '{}' is not open", id),
            Self::Expired(id) => write!(f, "dialog '{}' has expired", id),
            Self::InvalidAnswerForKind { id, kind } => write!(
                f,
                "answer payload does not match dialog '{}' kind '{}'",
                id,
                kind.as_str()
            ),
            Self::InvalidSelection { id, value } => write!(
                f,
                "selection '{}' is not one of the allowed options for dialog '{}'",
                value, id
            ),
        }
    }
}

impl std::error::Error for ExtensionUiError {}

/// Parses an incoming Pi `extension_ui_request` JSON object at the RPC boundary.
pub fn parse_extension_ui_request(
    val: &JsonValue,
    now: Instant,
) -> Result<PendingDialog, ExtensionUiError> {
    let raw_id = val
        .get("id")
        .and_then(JsonValue::as_str)
        .ok_or(ExtensionUiError::InvalidDialogId)?;
    let id = DialogId::parse(raw_id)?;

    let raw_kind = val
        .get("method")
        .or_else(|| val.get("kind"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ExtensionUiError::UnsupportedKind("<missing>".to_string()))?;
    let kind = DialogKind::parse(raw_kind)?;

    let title = val
        .get("title")
        .and_then(JsonValue::as_str)
        .map(ToString::to_string);

    let options = val
        .get("options")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .map(ToString::to_string)
                        .or_else(|| {
                            item.get("value")
                                .and_then(JsonValue::as_str)
                                .map(ToString::to_string)
                        })
                        .or_else(|| {
                            item.get("label")
                                .and_then(JsonValue::as_str)
                                .map(ToString::to_string)
                        })
                })
                .collect()
        })
        .unwrap_or_default();

    let timeout_ms = val
        .get("timeout_ms")
        .or_else(|| val.get("timeoutMs"))
        .or_else(|| val.get("timeout"))
        .and_then(|v| match v {
            JsonValue::Number(n) if *n > 0.0 && n.is_finite() => Some(*n as u64),
            _ => None,
        });

    let deadline = timeout_ms.map(|ms| now + Duration::from_millis(ms));

    Ok(PendingDialog {
        id,
        kind,
        title,
        options,
        deadline,
    })
}

/// Fire-and-forget Extension UI effect (`notify`, `setStatus`, `setWidget`, `setTitle`, `set_editor_text`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionUiEffectPayload {
    pub method: String,
    pub message: Option<String>,
    pub notify_type: Option<String>,
    pub status_key: Option<String>,
    pub status_text: Option<String>,
    pub widget_key: Option<String>,
    pub widget_lines: Option<Vec<String>>,
    pub title: Option<String>,
    pub text: Option<String>,
}

fn opt_str_field(val: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| val.get(k).and_then(JsonValue::as_str))
        .map(ToString::to_string)
}

/// Parses a fire-and-forget Pi `extension_ui_request` effect (`notify`, `setStatus`,
/// `setWidget`, `setTitle`, `set_editor_text`) so it can be forwarded to the UI instead of dropped.
pub fn parse_extension_ui_effect(val: &JsonValue) -> Option<ExtensionUiEffectPayload> {
    let raw_method = val
        .get("method")
        .or_else(|| val.get("kind"))
        .and_then(JsonValue::as_str)?;

    match raw_method {
        "notify" => Some(ExtensionUiEffectPayload {
            method: "notify".to_string(),
            message: opt_str_field(val, &["message", "text"]),
            notify_type: opt_str_field(val, &["notifyType", "notify_type", "level"]),
            status_key: None,
            status_text: None,
            widget_key: None,
            widget_lines: None,
            title: opt_str_field(val, &["title"]),
            text: None,
        }),
        "setStatus" | "set_status" => Some(ExtensionUiEffectPayload {
            method: raw_method.to_string(),
            message: None,
            notify_type: None,
            status_key: opt_str_field(val, &["statusKey", "status_key", "key"]),
            status_text: opt_str_field(val, &["statusText", "status_text", "text", "value"]),
            widget_key: None,
            widget_lines: None,
            title: None,
            text: None,
        }),
        "setWidget" | "set_widget" => {
            let widget_lines = val
                .get("widgetLines")
                .or_else(|| val.get("widget_lines"))
                .or_else(|| val.get("lines"))
                .and_then(JsonValue::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                });
            Some(ExtensionUiEffectPayload {
                method: raw_method.to_string(),
                message: None,
                notify_type: None,
                status_key: None,
                status_text: None,
                widget_key: opt_str_field(val, &["widgetKey", "widget_key", "key"]),
                widget_lines,
                title: None,
                text: None,
            })
        }
        "setTitle" | "set_title" => Some(ExtensionUiEffectPayload {
            method: raw_method.to_string(),
            message: None,
            notify_type: None,
            status_key: None,
            status_text: None,
            widget_key: None,
            widget_lines: None,
            title: opt_str_field(val, &["title", "value"]),
            text: None,
        }),
        "set_editor_text" | "setEditorText" => Some(ExtensionUiEffectPayload {
            method: raw_method.to_string(),
            message: None,
            notify_type: None,
            status_key: None,
            status_text: None,
            widget_key: None,
            widget_lines: None,
            title: None,
            text: opt_str_field(val, &["text", "value"]),
        }),
        _ => None,
    }
}

/// Tracks open Pi Extension UI dialogs (`select`/`confirm`/`input`/`editor`) with optional
/// timeout deadlines and produces validated `extension_ui_response` payloads.
#[derive(Debug, Default)]
pub struct ExtensionUiTracker {
    open: BTreeMap<DialogId, PendingDialog>,
}

impl ExtensionUiTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    pub fn get(&self, id: &DialogId) -> Option<&PendingDialog> {
        self.open.get(id)
    }

    /// Registers a new open dialog using `Instant::now()`.
    pub fn register(&mut self, dialog: PendingDialog) -> Result<(), ExtensionUiError> {
        self.register_at(dialog, Instant::now())
    }

    /// Registers a new open dialog at an explicit timestamp (purging stale expired entries first).
    pub fn register_at(
        &mut self,
        dialog: PendingDialog,
        now: Instant,
    ) -> Result<(), ExtensionUiError> {
        let _ = self.purge_expired_at(now);
        if self.open.contains_key(&dialog.id) {
            return Err(ExtensionUiError::DuplicateDialog(dialog.id));
        }
        self.open.insert(dialog.id.clone(), dialog);
        Ok(())
    }

    /// Validates an answer against the tracked dialog, removes it, and returns the response payload.
    pub fn resolve(
        &mut self,
        id: &DialogId,
        answer: ExtensionUiAnswer,
    ) -> Result<ExtensionUiResponse, ExtensionUiError> {
        self.resolve_at(id, answer, Instant::now())
    }

    /// Validates and resolves an open dialog at an explicit timestamp.
    pub fn resolve_at(
        &mut self,
        id: &DialogId,
        answer: ExtensionUiAnswer,
        now: Instant,
    ) -> Result<ExtensionUiResponse, ExtensionUiError> {
        let dialog = self
            .open
            .get(id)
            .ok_or_else(|| ExtensionUiError::UnknownDialog(id.clone()))?;

        if dialog.deadline.is_some_and(|d| now >= d) {
            self.open.remove(id);
            return Err(ExtensionUiError::Expired(id.clone()));
        }

        validate_answer(dialog, &answer)?;
        let removed = self
            .open
            .remove(id)
            .expect("dialog was verified present above");

        Ok(ExtensionUiResponse {
            id: removed.id,
            answer,
        })
    }

    /// Purges all expired dialogs using `Instant::now()`, returning cancellation responses.
    /// Idempotent: calling twice in succession returns an empty list on the second call.
    pub fn purge_expired(&mut self) -> Vec<ExtensionUiResponse> {
        self.purge_expired_at(Instant::now())
    }

    /// Purges all dialogs whose deadline has elapsed at `now`, returning cancellation responses.
    pub fn purge_expired_at(&mut self, now: Instant) -> Vec<ExtensionUiResponse> {
        let expired_ids: Vec<DialogId> = self
            .open
            .iter()
            .filter_map(|(id, dlg)| {
                if dlg.deadline.is_some_and(|d| now >= d) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        expired_ids
            .into_iter()
            .filter_map(|id| {
                self.open.remove(&id).map(|dlg| ExtensionUiResponse {
                    id: dlg.id,
                    answer: ExtensionUiAnswer::Cancelled,
                })
            })
            .collect()
    }

    /// Cancels and removes all currently open dialogs (idempotent).
    pub fn cancel_all(&mut self) -> Vec<ExtensionUiResponse> {
        let dialogs = std::mem::take(&mut self.open);
        dialogs
            .into_keys()
            .map(|id| ExtensionUiResponse {
                id,
                answer: ExtensionUiAnswer::Cancelled,
            })
            .collect()
    }
}

fn validate_answer(
    dialog: &PendingDialog,
    answer: &ExtensionUiAnswer,
) -> Result<(), ExtensionUiError> {
    match (dialog.kind, answer) {
        (_, ExtensionUiAnswer::Cancelled) => Ok(()),
        (DialogKind::Confirm, ExtensionUiAnswer::Confirmed(_)) => Ok(()),
        (DialogKind::Select, ExtensionUiAnswer::Value(val)) => {
            if !dialog.options.is_empty() && !dialog.options.iter().any(|opt| opt == val) {
                return Err(ExtensionUiError::InvalidSelection {
                    id: dialog.id.clone(),
                    value: val.clone(),
                });
            }
            Ok(())
        }
        (DialogKind::Input | DialogKind::Editor, ExtensionUiAnswer::Value(_)) => Ok(()),
        (kind, _) => Err(ExtensionUiError::InvalidAnswerForKind {
            id: dialog.id.clone(),
            kind,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verity_core::json::parse_json;

    #[test]
    fn tracks_and_validates_all_four_dialog_kinds() {
        let now = Instant::now();
        let mut tracker = ExtensionUiTracker::new();

        let confirm_id = DialogId::parse("dlg-confirm").unwrap();
        tracker
            .register_at(
                PendingDialog {
                    id: confirm_id.clone(),
                    kind: DialogKind::Confirm,
                    title: Some("Proceed?".to_string()),
                    options: Vec::new(),
                    deadline: None,
                },
                now,
            )
            .unwrap();

        // Wrong answer kind is rejected and dialog remains open.
        assert_eq!(
            tracker.resolve_at(
                &confirm_id,
                ExtensionUiAnswer::Value("yes".to_string()),
                now
            ),
            Err(ExtensionUiError::InvalidAnswerForKind {
                id: confirm_id.clone(),
                kind: DialogKind::Confirm,
            })
        );
        assert_eq!(tracker.open_count(), 1);

        let resp = tracker
            .resolve_at(&confirm_id, ExtensionUiAnswer::Confirmed(true), now)
            .unwrap();
        let parsed = parse_json(&resp.to_jsonl()).unwrap();
        assert_eq!(
            parsed.get("type").and_then(JsonValue::as_str),
            Some("extension_ui_response")
        );
        assert_eq!(
            parsed.get("id").and_then(JsonValue::as_str),
            Some("dlg-confirm")
        );
        assert_eq!(
            parsed.get("confirmed").and_then(JsonValue::as_bool),
            Some(true)
        );
        assert_eq!(tracker.open_count(), 0);

        // Select dialog validates options.
        let select_id = DialogId::parse("dlg-select").unwrap();
        tracker
            .register_at(
                PendingDialog {
                    id: select_id.clone(),
                    kind: DialogKind::Select,
                    title: Some("Pick one".to_string()),
                    options: vec!["alpha".to_string(), "beta".to_string()],
                    deadline: None,
                },
                now,
            )
            .unwrap();
        assert!(matches!(
            tracker.resolve_at(
                &select_id,
                ExtensionUiAnswer::Value("gamma".to_string()),
                now
            ),
            Err(ExtensionUiError::InvalidSelection { .. })
        ));
        let sel_resp = tracker
            .resolve_at(
                &select_id,
                ExtensionUiAnswer::Value("beta".to_string()),
                now,
            )
            .unwrap();
        let sel_json = parse_json(&sel_resp.to_jsonl()).unwrap();
        assert_eq!(
            sel_json.get("value").and_then(JsonValue::as_str),
            Some("beta")
        );

        // Input and Editor accept values and cancellation.
        let input_id = DialogId::parse("dlg-input").unwrap();
        let editor_id = DialogId::parse("dlg-editor").unwrap();
        tracker
            .register_at(
                PendingDialog {
                    id: input_id.clone(),
                    kind: DialogKind::Input,
                    title: None,
                    options: Vec::new(),
                    deadline: None,
                },
                now,
            )
            .unwrap();
        tracker
            .register_at(
                PendingDialog {
                    id: editor_id.clone(),
                    kind: DialogKind::Editor,
                    title: None,
                    options: Vec::new(),
                    deadline: None,
                },
                now,
            )
            .unwrap();
        let in_resp = tracker
            .resolve_at(
                &input_id,
                ExtensionUiAnswer::Value("hello".to_string()),
                now,
            )
            .unwrap();
        assert_eq!(
            parse_json(&in_resp.to_jsonl())
                .unwrap()
                .get("value")
                .and_then(JsonValue::as_str),
            Some("hello")
        );
        let ed_resp = tracker
            .resolve_at(&editor_id, ExtensionUiAnswer::Cancelled, now)
            .unwrap();
        assert_eq!(
            parse_json(&ed_resp.to_jsonl())
                .unwrap()
                .get("cancelled")
                .and_then(JsonValue::as_bool),
            Some(true)
        );
    }

    #[test]
    fn expires_timed_out_dialogs_idempotently() {
        let t0 = Instant::now();
        let mut tracker = ExtensionUiTracker::new();
        let id = DialogId::parse("dlg-timeout").unwrap();
        tracker
            .register_at(
                PendingDialog {
                    id: id.clone(),
                    kind: DialogKind::Input,
                    title: Some("Timed".to_string()),
                    options: Vec::new(),
                    deadline: Some(t0 + Duration::from_secs(5)),
                },
                t0,
            )
            .unwrap();

        assert!(tracker
            .purge_expired_at(t0 + Duration::from_secs(4))
            .is_empty());
        let expired = tracker.purge_expired_at(t0 + Duration::from_secs(5));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, id);
        assert_eq!(expired[0].answer, ExtensionUiAnswer::Cancelled);
        // Second purge is idempotent.
        assert!(tracker
            .purge_expired_at(t0 + Duration::from_secs(6))
            .is_empty());
        // Resolving an already-purged dialog returns UnknownDialog.
        assert_eq!(
            tracker.resolve_at(
                &id,
                ExtensionUiAnswer::Cancelled,
                t0 + Duration::from_secs(6)
            ),
            Err(ExtensionUiError::UnknownDialog(id))
        );
    }

    #[test]
    fn parses_extension_ui_request_json() {
        let now = Instant::now();
        let raw = parse_json(
            r#"{"type":"extension_ui_request","id":"req-ui-1","method":"select","title":"Choose","options":["a","b"],"timeoutMs":1500}"#,
        )
        .unwrap();
        let dialog = parse_extension_ui_request(&raw, now).unwrap();
        assert_eq!(dialog.id.as_str(), "req-ui-1");
        assert_eq!(dialog.kind, DialogKind::Select);
        assert_eq!(dialog.title.as_deref(), Some("Choose"));
        assert_eq!(dialog.options, vec!["a".to_string(), "b".to_string()]);
        assert!(dialog.deadline.is_some());
    }

    #[test]
    fn parses_all_five_fire_and_forget_extension_ui_effects() {
        let notify = parse_json(
            r#"{"type":"extension_ui_request","id":"e1","method":"notify","message":"Saved!","notifyType":"info"}"#,
        )
        .unwrap();
        let eff = parse_extension_ui_effect(&notify).unwrap();
        assert_eq!(eff.method, "notify");
        assert_eq!(eff.message.as_deref(), Some("Saved!"));
        assert_eq!(eff.notify_type.as_deref(), Some("info"));

        let status = parse_json(
            r#"{"type":"extension_ui_request","id":"e2","method":"setStatus","statusKey":"git","statusText":"main"}"#,
        )
        .unwrap();
        let eff = parse_extension_ui_effect(&status).unwrap();
        assert_eq!(eff.method, "setStatus");
        assert_eq!(eff.status_key.as_deref(), Some("git"));
        assert_eq!(eff.status_text.as_deref(), Some("main"));

        let widget = parse_json(
            r#"{"type":"extension_ui_request","id":"e3","method":"setWidget","widgetKey":"tests","widgetLines":["3 passed","0 failed"]}"#,
        )
        .unwrap();
        let eff = parse_extension_ui_effect(&widget).unwrap();
        assert_eq!(eff.method, "setWidget");
        assert_eq!(eff.widget_key.as_deref(), Some("tests"));
        assert_eq!(
            eff.widget_lines,
            Some(vec!["3 passed".to_string(), "0 failed".to_string()])
        );

        let title = parse_json(
            r#"{"type":"extension_ui_request","id":"e4","method":"setTitle","title":"Verity - Auth"}"#,
        )
        .unwrap();
        let eff = parse_extension_ui_effect(&title).unwrap();
        assert_eq!(eff.method, "setTitle");
        assert_eq!(eff.title.as_deref(), Some("Verity - Auth"));

        let editor_text = parse_json(
            r#"{"type":"extension_ui_request","id":"e5","method":"set_editor_text","text":"/compact focus on auth"}"#,
        )
        .unwrap();
        let eff = parse_extension_ui_effect(&editor_text).unwrap();
        assert_eq!(eff.method, "set_editor_text");
        assert_eq!(eff.text.as_deref(), Some("/compact focus on auth"));

        // Interactive dialog methods are NOT fire-and-forget effects.
        let confirm = parse_json(
            r#"{"type":"extension_ui_request","id":"d1","method":"confirm","title":"Sure?"}"#,
        )
        .unwrap();
        assert!(parse_extension_ui_effect(&confirm).is_none());
    }
}
