//! Kani symbolic verification harnesses and unit tests for:
//! 1. Slash vs. Bash input prefix disjointness and `/` normalization laws.
//! 2. Project registry deduplication and active-fallback invariants.
//! 3. Extended Pi RPC command single-line JSONL framing laws.

/// Four mutually exclusive input classes for non-empty composer inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// Leading `/` — slash command.
    Slash,
    /// Leading `!` not followed by `!` — bash command included in LLM context.
    BashContext,
    /// Leading `!!` — bash command excluded from LLM context.
    BashExcluded,
    /// Any other non-empty input — plain user prompt.
    PlainPrompt,
}

/// Classifies a non-empty input byte slice into one of four mutually exclusive classes.
pub fn classify_input(input: &[u8]) -> Option<InputKind> {
    match input {
        [] => None,
        [b'/', ..] => Some(InputKind::Slash),
        [b'!', b'!', ..] => Some(InputKind::BashExcluded),
        [b'!', ..] => Some(InputKind::BashContext),
        _ => Some(InputKind::PlainPrompt),
    }
}

/// Strips all leading `/` bytes from a slash command name or query.
pub fn normalize_slash_name(name: &[u8]) -> &[u8] {
    let mut start = 0usize;
    while start < name.len() && name[start] == b'/' {
        start += 1;
    }
    &name[start..]
}

/// Prefix-matches a slash command name against a user query after normalizing
/// leading `/` characters on both sides.
pub fn slash_matches(cmd_name: &[u8], query: &[u8]) -> bool {
    let norm_cmd = normalize_slash_name(cmd_name);
    let norm_query = normalize_slash_name(query);
    if norm_query.len() > norm_cmd.len() {
        return false;
    }
    let mut i = 0usize;
    while i < norm_query.len() {
        if norm_cmd[i] != norm_query[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Maximum number of canonical projects tracked in the bounded registry model.
pub const MAX_PROJECTS: usize = 4;

/// Bounded project registry tracking unique canonical project IDs and the active slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectRegistry {
    pub ids: [u8; MAX_PROJECTS],
    pub len: usize,
    pub active_idx: Option<usize>,
}

impl ProjectRegistry {
    pub const fn empty() -> Self {
        Self {
            ids: [0; MAX_PROJECTS],
            len: 0,
            active_idx: None,
        }
    }

    pub fn find(&self, id: u8) -> Option<usize> {
        let mut i = 0usize;
        while i < self.len {
            if self.ids[i] == id {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    pub fn count_id(&self, id: u8) -> usize {
        let mut count = 0usize;
        let mut i = 0usize;
        while i < self.len {
            if self.ids[i] == id {
                count += 1;
            }
            i += 1;
        }
        count
    }

    pub fn has_duplicates(&self) -> bool {
        let mut i = 0usize;
        while i < self.len {
            let mut j = i + 1;
            while j < self.len {
                if self.ids[i] == self.ids[j] {
                    return true;
                }
                j += 1;
            }
            i += 1;
        }
        false
    }

    pub fn active_invariant_holds(&self) -> bool {
        if self.len > MAX_PROJECTS {
            return false;
        }
        match self.active_idx {
            None => self.len == 0,
            Some(idx) => self.len > 0 && idx < self.len,
        }
    }

    /// Adds a canonical project ID. If it already exists, does not insert a
    /// duplicate and selects the unique existing slot as active.
    /// Returns `true` if a new slot was inserted, `false` if duplicate or full.
    pub fn add(&mut self, id: u8) -> bool {
        if let Some(existing_idx) = self.find(id) {
            self.active_idx = Some(existing_idx);
            return false;
        }
        if self.len >= MAX_PROJECTS {
            return false;
        }
        let idx = self.len;
        self.ids[idx] = id;
        self.len += 1;
        self.active_idx = Some(idx);
        true
    }

    /// Removes a canonical project ID if present, shifting remaining entries
    /// left and maintaining a valid `active_idx` fallback.
    pub fn remove(&mut self, id: u8) -> bool {
        let Some(pos) = self.find(id) else {
            return false;
        };
        let mut i = pos;
        while i + 1 < self.len {
            self.ids[i] = self.ids[i + 1];
            i += 1;
        }
        self.len -= 1;
        self.ids[self.len] = 0;

        if self.len == 0 {
            self.active_idx = None;
        } else {
            self.active_idx = match self.active_idx {
                Some(a) if a == pos => {
                    if pos < self.len {
                        Some(pos)
                    } else {
                        Some(self.len - 1)
                    }
                }
                Some(a) if a > pos => Some(a - 1),
                Some(a) => Some(a),
                None => Some(0),
            };
        }
        true
    }
}

/// Queue delivery mode for `set_steering_mode` and `set_follow_up_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueDeliveryMode {
    All,
    OneAtATime,
}

impl QueueDeliveryMode {
    pub const fn as_wire_str(self) -> &'static [u8] {
        match self {
            Self::All => b"all",
            Self::OneAtATime => b"one-at-a-time",
        }
    }
}

/// Maximum raw command byte length modeled for the `bash` RPC command.
pub const MAX_BASH_CMD_LEN: usize = 4;

/// Extended Pi RPC commands modeled for single-line JSONL wire framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedRpcCommand {
    Bash {
        command: [u8; MAX_BASH_CMD_LEN],
        len: usize,
        exclude_from_context: bool,
    },
    AbortBash,
    SetAutoCompaction {
        enabled: bool,
    },
    SetAutoRetry {
        enabled: bool,
    },
    SetSteeringMode {
        mode: QueueDeliveryMode,
    },
    SetFollowUpMode {
        mode: QueueDeliveryMode,
    },
    CycleModel,
    CycleThinkingLevel,
    GetLastAssistantText,
    GetEntries,
}

impl ExtendedRpcCommand {
    pub const fn type_tag(self) -> &'static [u8] {
        match self {
            Self::Bash { .. } => b"bash",
            Self::AbortBash => b"abort_bash",
            Self::SetAutoCompaction { .. } => b"set_auto_compaction",
            Self::SetAutoRetry { .. } => b"set_auto_retry",
            Self::SetSteeringMode { .. } => b"set_steering_mode",
            Self::SetFollowUpMode { .. } => b"set_follow_up_mode",
            Self::CycleModel => b"cycle_model",
            Self::CycleThinkingLevel => b"cycle_thinking_level",
            Self::GetLastAssistantText => b"get_last_assistant_text",
            Self::GetEntries => b"get_entries",
        }
    }
}

/// Fixed-capacity buffer holding a single framed JSONL wire line.
/// Longest modeled frame is 80 bytes (`bash` with 4 control bytes + `false`).
pub const MAX_FRAME_LEN: usize = 84;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonlFrame {
    pub buf: [u8; MAX_FRAME_LEN],
    pub len: usize,
}

impl JsonlFrame {
    const fn new() -> Self {
        Self {
            buf: [0; MAX_FRAME_LEN],
            len: 0,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    fn push_byte(&mut self, b: u8) {
        debug_assert!(self.len < self.buf.len());
        self.buf[self.len] = b;
        self.len += 1;
    }

    fn push_static(&mut self, bytes: &[u8]) {
        debug_assert!(bytes.len() <= 32);
        let mut i = 0usize;
        while i < 32 && i < bytes.len() {
            self.push_byte(bytes[i]);
            i += 1;
        }
    }

    fn push_json_escaped_command(&mut self, command: &[u8; MAX_BASH_CMD_LEN], len: usize) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut i = 0usize;
        while i < MAX_BASH_CMD_LEN {
            if i >= len {
                break;
            }
            let b = command[i];
            match b {
                b'"' => {
                    self.push_byte(b'\\');
                    self.push_byte(b'"');
                }
                b'\\' => {
                    self.push_byte(b'\\');
                    self.push_byte(b'\\');
                }
                b'\n' => {
                    self.push_byte(b'\\');
                    self.push_byte(b'n');
                }
                b'\r' => {
                    self.push_byte(b'\\');
                    self.push_byte(b'r');
                }
                b'\t' => {
                    self.push_byte(b'\\');
                    self.push_byte(b't');
                }
                0x00..=0x1f => {
                    self.push_byte(b'\\');
                    self.push_byte(b'u');
                    self.push_byte(b'0');
                    self.push_byte(b'0');
                    self.push_byte(HEX[(b >> 4) as usize]);
                    self.push_byte(HEX[(b & 0x0f) as usize]);
                }
                _ => self.push_byte(b),
            }
            i += 1;
        }
    }
}

/// Serializes an `ExtendedRpcCommand` into a single-line JSONL wire frame.
pub fn frame_rpc_command(cmd: ExtendedRpcCommand) -> JsonlFrame {
    let mut frame = JsonlFrame::new();
    frame.push_static(b"{\"type\":\"");
    frame.push_static(cmd.type_tag());
    frame.push_byte(b'"');

    match cmd {
        ExtendedRpcCommand::Bash {
            command,
            len,
            exclude_from_context,
        } => {
            frame.push_static(b",\"command\":\"");
            frame.push_json_escaped_command(&command, len);
            frame.push_static(b"\",\"excludeFromContext\":");
            frame.push_static(if exclude_from_context {
                b"true"
            } else {
                b"false"
            });
        }
        ExtendedRpcCommand::SetAutoCompaction { enabled }
        | ExtendedRpcCommand::SetAutoRetry { enabled } => {
            frame.push_static(b",\"enabled\":");
            frame.push_static(if enabled { b"true" } else { b"false" });
        }
        ExtendedRpcCommand::SetSteeringMode { mode }
        | ExtendedRpcCommand::SetFollowUpMode { mode } => {
            frame.push_static(b",\"mode\":\"");
            frame.push_static(mode.as_wire_str());
            frame.push_byte(b'"');
        }
        ExtendedRpcCommand::AbortBash
        | ExtendedRpcCommand::CycleModel
        | ExtendedRpcCommand::CycleThinkingLevel
        | ExtendedRpcCommand::GetLastAssistantText
        | ExtendedRpcCommand::GetEntries => {}
    }

    frame.push_byte(b'}');
    frame.push_byte(b'\n');
    frame
}

#[cfg(kani)]
fn any_queue_delivery_mode() -> QueueDeliveryMode {
    if kani::any() {
        QueueDeliveryMode::All
    } else {
        QueueDeliveryMode::OneAtATime
    }
}

#[cfg(kani)]
fn any_extended_rpc_command() -> ExtendedRpcCommand {
    let tag: u8 = kani::any();
    kani::assume(tag <= 9);
    match tag {
        0 => {
            let command: [u8; MAX_BASH_CMD_LEN] = kani::any();
            let len: usize = kani::any();
            kani::assume(len <= MAX_BASH_CMD_LEN);
            ExtendedRpcCommand::Bash {
                command,
                len,
                exclude_from_context: kani::any(),
            }
        }
        1 => ExtendedRpcCommand::AbortBash,
        2 => ExtendedRpcCommand::SetAutoCompaction {
            enabled: kani::any(),
        },
        3 => ExtendedRpcCommand::SetAutoRetry {
            enabled: kani::any(),
        },
        4 => ExtendedRpcCommand::SetSteeringMode {
            mode: any_queue_delivery_mode(),
        },
        5 => ExtendedRpcCommand::SetFollowUpMode {
            mode: any_queue_delivery_mode(),
        },
        6 => ExtendedRpcCommand::CycleModel,
        7 => ExtendedRpcCommand::CycleThinkingLevel,
        8 => ExtendedRpcCommand::GetLastAssistantText,
        _ => ExtendedRpcCommand::GetEntries,
    }
}

/// Harness 1 — Slash vs. Bash Prefix Disjointness & Normalization Law.
///
/// Proves on bounded byte slices:
/// - `Slash`, `BashContext`, `BashExcluded`, and `PlainPrompt` form a total,
///   mutually exclusive 4-way partition of all non-empty inputs.
/// - `normalize_slash_name` is idempotent and never leaves a leading `/`.
/// - `slash_matches(cmd_name, query)` returns `true` for identical base names
///   and is invariant to whether `cmd_name` has a leading `/`.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
pub fn prove_slash_bash_disjointness_and_normalization_laws() {
    // Part A: 4-way total & disjoint partition of non-empty inputs.
    let input_buf: [u8; 4] = kani::any();
    let input_len: usize = kani::any();
    kani::assume(input_len >= 1 && input_len <= 4);
    let input = &input_buf[..input_len];

    let is_slash = input[0] == b'/';
    let is_bash_excluded = input_len >= 2 && input[0] == b'!' && input[1] == b'!';
    let is_bash_context = input[0] == b'!' && !(input_len >= 2 && input[1] == b'!');
    let is_plain = input[0] != b'/' && input[0] != b'!';

    let active_classes =
        (is_slash as u8) + (is_bash_context as u8) + (is_bash_excluded as u8) + (is_plain as u8);
    assert_eq!(
        active_classes, 1,
        "every non-empty input must belong to exactly one class"
    );

    let kind = classify_input(input).expect("non-empty input must classify");
    match kind {
        InputKind::Slash => {
            assert!(is_slash && !is_bash_context && !is_bash_excluded && !is_plain);
        }
        InputKind::BashContext => {
            assert!(is_bash_context && !is_slash && !is_bash_excluded && !is_plain);
        }
        InputKind::BashExcluded => {
            assert!(is_bash_excluded && !is_slash && !is_bash_context && !is_plain);
        }
        InputKind::PlainPrompt => {
            assert!(is_plain && !is_slash && !is_bash_context && !is_bash_excluded);
        }
    }

    // Part B: normalize_slash_name idempotence & no leading '/'.
    let cmd_buf: [u8; 4] = kani::any();
    let cmd_len: usize = kani::any();
    kani::assume(cmd_len <= 4);
    let cmd = &cmd_buf[..cmd_len];

    let norm_once = normalize_slash_name(cmd);
    let norm_twice = normalize_slash_name(norm_once);
    assert_eq!(
        norm_twice, norm_once,
        "normalize_slash_name must be idempotent"
    );
    if !norm_once.is_empty() {
        assert_ne!(
            norm_once[0], b'/',
            "normalize_slash_name must never leave a leading '/'"
        );
    }

    // Part C: slash_matches leading-slash invariance & identical base-name matching.
    let query_buf: [u8; 4] = kani::any();
    let query_len: usize = kani::any();
    kani::assume(query_len <= 4);
    let query = &query_buf[..query_len];

    let norm_query = normalize_slash_name(query);
    if norm_once == norm_query {
        assert!(
            slash_matches(cmd, query),
            "identical base names must always match regardless of leading '/'"
        );
    }

    // Explicitly construct a prefixed version of `cmd` (when len <= 3) and prove
    // `slash_matches` yields the exact same boolean with or without a leading '/'.
    if cmd_len <= 3 {
        let mut prefixed = [0u8; 4];
        prefixed[0] = b'/';
        let mut i = 0usize;
        while i < cmd_len {
            prefixed[i + 1] = cmd[i];
            i += 1;
        }
        let prefixed_cmd = &prefixed[..cmd_len + 1];
        assert_eq!(
            slash_matches(prefixed_cmd, query),
            slash_matches(cmd, query),
            "slash_matches must be invariant to leading '/' on cmd_name"
        );
    }

    // Reachability probes for all 4 classes and leading-slash vs no-leading-slash matches.
    kani::cover!(kind == InputKind::Slash, "classify_slash");
    kani::cover!(kind == InputKind::BashContext, "classify_bash_context");
    kani::cover!(kind == InputKind::BashExcluded, "classify_bash_excluded");
    kani::cover!(kind == InputKind::PlainPrompt, "classify_plain_prompt");

    let cmd_has_leading_slash = !cmd.is_empty() && cmd[0] == b'/';
    let query_has_leading_slash = !query.is_empty() && query[0] == b'/';
    kani::cover!(
        cmd_has_leading_slash && !norm_once.is_empty() && slash_matches(cmd, query),
        "match_with_leading_slash"
    );
    kani::cover!(
        !cmd_has_leading_slash
            && query_has_leading_slash
            && !norm_once.is_empty()
            && !norm_query.is_empty()
            && slash_matches(cmd, query),
        "match_without_leading_slash_against_slash_query"
    );
}

/// Harness 2 — Project Registry Deduplication & Active-Fallback Invariant.
///
/// Proves over all bounded 5-step sequences of `add` and `remove` operations on
/// a registry of capacity `MAX_PROJECTS == 4`:
/// - Adding an existing project ID never creates duplicates and always sets
///   `active_idx` to the unique matching slot.
/// - Removing a project ID leaves no duplicates and maintains the active-index
///   invariant: non-empty => `Some(i)` with `i < len`; empty => `None`.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(7)]
pub fn prove_project_registry_dedup_and_active_fallback() {
    let mut reg = ProjectRegistry::empty();
    assert!(!reg.has_duplicates());
    assert!(reg.active_invariant_holds());

    let mut saw_add_new = false;
    let mut saw_add_duplicate = false;
    let mut saw_remove_active = false;
    let mut saw_remove_last = false;

    let steps: u8 = kani::any();
    kani::assume(steps <= 5);

    for _ in 0..steps {
        let is_add: bool = kani::any();
        let id: u8 = kani::any();

        let prev_len = reg.len;
        let prev_pos = reg.find(id);
        let prev_active = reg.active_idx;

        if is_add {
            let inserted = reg.add(id);
            match prev_pos {
                Some(existing_slot) => {
                    assert!(!inserted, "adding existing project ID must not insert");
                    assert_eq!(reg.len, prev_len, "duplicate add must not grow registry");
                    assert_eq!(
                        reg.active_idx,
                        Some(existing_slot),
                        "duplicate add must activate the unique matching slot"
                    );
                    assert_eq!(reg.count_id(id), 1);
                    saw_add_duplicate = true;
                }
                None => {
                    if prev_len < MAX_PROJECTS {
                        assert!(inserted, "adding new project ID below capacity must insert");
                        assert_eq!(reg.len, prev_len + 1);
                        assert_eq!(reg.active_idx, Some(prev_len));
                        assert_eq!(reg.ids[prev_len], id);
                        assert_eq!(reg.count_id(id), 1);
                        saw_add_new = true;
                    } else {
                        assert!(!inserted);
                        assert_eq!(reg.len, prev_len);
                    }
                }
            }
        } else {
            let removed = reg.remove(id);
            match prev_pos {
                Some(removed_slot) => {
                    assert!(removed, "removing present project ID must succeed");
                    assert_eq!(reg.len, prev_len - 1);
                    assert_eq!(reg.count_id(id), 0);
                    if prev_active == Some(removed_slot) && reg.len > 0 {
                        saw_remove_active = true;
                    }
                    if prev_len == 1 && reg.len == 0 {
                        assert_eq!(reg.active_idx, None);
                        saw_remove_last = true;
                    }
                }
                None => {
                    assert!(!removed);
                    assert_eq!(reg.len, prev_len);
                    assert_eq!(reg.active_idx, prev_active);
                }
            }
        }

        assert!(
            !reg.has_duplicates(),
            "registry must never contain duplicate project IDs"
        );
        assert!(
            reg.active_invariant_holds(),
            "active_idx must be Some(i < len) when non-empty and None when empty"
        );
    }

    kani::cover!(saw_add_new, "registry_add_new");
    kani::cover!(saw_add_duplicate, "registry_add_duplicate");
    kani::cover!(saw_remove_active, "registry_remove_active");
    kani::cover!(saw_remove_last, "registry_remove_last");
}

/// Harness 3 — Extended Pi RPC Command Framing Law.
///
/// Proves that all 10 extended Pi RPC command types serialize to valid
/// single-line JSONL frames starting with `{"type":"`, ending with `}\n`,
/// carrying the exact variant type tag, and containing zero interior `\n` or `\r`.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(86)]
pub fn prove_extended_pi_rpc_command_framing_laws() {
    let cmd = any_extended_rpc_command();
    let frame = frame_rpc_command(cmd);
    let bytes = frame.as_bytes();

    // Must start with `{"type":"` and carry the exact command type tag followed by `"`.
    let prefix = b"{\"type\":\"";
    let tag = cmd.type_tag();
    assert!(bytes.len() >= prefix.len() + tag.len() + 3);
    assert!(bytes.len() <= MAX_FRAME_LEN);

    let mut p = 0usize;
    while p < 9 {
        assert_eq!(bytes[p], prefix[p]);
        p += 1;
    }

    let mut t = 0usize;
    while t < 24 && t < tag.len() {
        assert_eq!(bytes[9 + t], tag[t]);
        t += 1;
    }
    assert_eq!(bytes[9 + tag.len()], b'"');

    // Must end with `}\n`.
    assert_eq!(bytes[bytes.len() - 1], b'\n');
    assert_eq!(bytes[bytes.len() - 2], b'}');

    // Must contain zero interior `\n` or `\r` bytes (strict single-line JSONL).
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        assert_ne!(bytes[i], b'\n', "JSONL frame must not contain interior LF");
        assert_ne!(bytes[i], b'\r', "JSONL frame must not contain interior CR");
        i += 1;
    }

    // Reachability probes across all 10 extended command variants plus newline escaping.
    let bash_with_raw_newline = match cmd {
        ExtendedRpcCommand::Bash { command, len, .. } => {
            let mut has_nl = false;
            let mut k = 0usize;
            while k < MAX_BASH_CMD_LEN {
                if k >= len {
                    break;
                }
                if command[k] == b'\n' || command[k] == b'\r' {
                    has_nl = true;
                }
                k += 1;
            }
            has_nl
        }
        _ => false,
    };

    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::Bash { .. }),
        "frame_cmd_bash"
    );
    kani::cover!(bash_with_raw_newline, "frame_cmd_bash_escapes_newline");
    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::AbortBash),
        "frame_cmd_abort_bash"
    );
    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::SetAutoCompaction { .. }),
        "frame_cmd_set_auto_compaction"
    );
    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::SetAutoRetry { .. }),
        "frame_cmd_set_auto_retry"
    );
    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::SetSteeringMode { .. }),
        "frame_cmd_set_steering_mode"
    );
    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::SetFollowUpMode { .. }),
        "frame_cmd_set_follow_up_mode"
    );
    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::CycleModel),
        "frame_cmd_cycle_model"
    );
    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::CycleThinkingLevel),
        "frame_cmd_cycle_thinking_level"
    );
    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::GetLastAssistantText),
        "frame_cmd_get_last_assistant_text"
    );
    kani::cover!(
        matches!(cmd, ExtendedRpcCommand::GetEntries),
        "frame_cmd_get_entries"
    );
}

// ============================================================================
// Proof B — Session Archive, Delete & Project-Grouping Partition Laws
// ============================================================================

/// Bounded number of projects in the session-grouping workspace model (`P <= 2`).
pub const MAX_WORKSPACE_PROJECTS: usize = 2;

/// Bounded number of sessions in the session-grouping workspace model (`S <= 4`).
pub const MAX_WORKSPACE_SESSIONS: usize = 4;

/// Single session tagged with its owning `project_id` and `archived` state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceSession {
    pub project_id: u8,
    pub archived: bool,
}

/// Bucket of session indices belonging to a single project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectBucket {
    pub session_indices: [usize; MAX_WORKSPACE_SESSIONS],
    pub len: usize,
}

impl ProjectBucket {
    pub const fn empty() -> Self {
        Self {
            session_indices: [0; MAX_WORKSPACE_SESSIONS],
            len: 0,
        }
    }
}

/// Bounded workspace of `P <= 2` projects and `S <= 4` sessions with active selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionWorkspace {
    pub project_count: usize,
    pub sessions: [WorkspaceSession; MAX_WORKSPACE_SESSIONS],
    pub len: usize,
    pub active_session: Option<usize>,
}

impl SessionWorkspace {
    /// Returns whether session `idx` is visible under the `show_archived` filter.
    pub fn is_visible(&self, idx: usize, show_archived: bool) -> bool {
        idx < self.len && (show_archived || !self.sessions[idx].archived)
    }

    /// Archives session `idx` without deleting it.
    pub fn archive(&mut self, idx: usize) -> bool {
        if idx >= self.len {
            return false;
        }
        self.sessions[idx].archived = true;
        true
    }

    /// Unarchives session `idx`.
    pub fn unarchive(&mut self, idx: usize) -> bool {
        if idx >= self.len {
            return false;
        }
        self.sessions[idx].archived = false;
        true
    }

    /// Deletes session `idx` and maintains a safe non-dangling `active_session`.
    ///
    /// If `idx` was the active session, falls back to the first remaining
    /// non-archived session in the same project if one exists, or `None` if none remain.
    pub fn delete(&mut self, idx: usize) -> bool {
        if idx >= self.len {
            return false;
        }
        let deleted_project = self.sessions[idx].project_id;
        let was_active = self.active_session == Some(idx);

        let mut i = idx;
        while i + 1 < self.len {
            self.sessions[i] = self.sessions[i + 1];
            i += 1;
        }
        self.len -= 1;
        self.sessions[self.len] = WorkspaceSession {
            project_id: 0,
            archived: false,
        };

        if was_active {
            let mut fallback = None;
            let mut k = 0usize;
            while k < self.len {
                if self.sessions[k].project_id == deleted_project && !self.sessions[k].archived {
                    fallback = Some(k);
                    break;
                }
                k += 1;
            }
            self.active_session = fallback;
        } else if let Some(a) = self.active_session {
            if a > idx {
                self.active_session = Some(a - 1);
            }
        }
        true
    }

    /// Partitions all sessions into per-project buckets.
    pub fn group_by_project(&self) -> [ProjectBucket; MAX_WORKSPACE_PROJECTS] {
        let mut buckets = [ProjectBucket::empty(); MAX_WORKSPACE_PROJECTS];
        let mut i = 0usize;
        while i < self.len {
            let p = self.sessions[i].project_id as usize;
            if p < MAX_WORKSPACE_PROJECTS {
                let slot = buckets[p].len;
                buckets[p].session_indices[slot] = i;
                buckets[p].len += 1;
            }
            i += 1;
        }
        buckets
    }
}

/// Harness 4 — Session Archive, Delete & Project-Grouping Partition Laws.
///
/// Proves on a bounded workspace (`P <= 2` projects, `S <= 4` sessions) over
/// symbolic `archive`, `unarchive`, `delete`, and `group_by_project` operations:
/// 1. Disjoint Project Partition: sum of bucket counts equals total sessions,
///    and every session in bucket `p` has `session.project_id == p`.
/// 2. Archive Reversibility: `archive(id)` never deletes the session, excludes
///    it from default active view (`show_archived == false`), and
///    `unarchive(archive(id))` restores exact original state.
/// 3. Safe Active-Session Fallback on Delete: deleting the active session always
///    falls back to a valid non-archived session in the same project if one
///    exists, or `None` if none remain — never leaving a dangling `active_session`.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
pub fn prove_session_archive_delete_and_project_grouping_laws() {
    let project_count: usize = kani::any();
    kani::assume(project_count >= 1 && project_count <= MAX_WORKSPACE_PROJECTS);

    let init_len: usize = kani::any();
    kani::assume(init_len <= MAX_WORKSPACE_SESSIONS);

    let mut sessions = [WorkspaceSession {
        project_id: 0,
        archived: false,
    }; MAX_WORKSPACE_SESSIONS];
    let mut i = 0usize;
    while i < init_len {
        let project_id: u8 = kani::any();
        kani::assume((project_id as usize) < project_count);
        sessions[i] = WorkspaceSession {
            project_id,
            archived: kani::any(),
        };
        i += 1;
    }

    let has_active: bool = kani::any();
    let active_session = if has_active && init_len > 0 {
        let a: usize = kani::any();
        kani::assume(a < init_len);
        Some(a)
    } else {
        None
    };

    let mut ws = SessionWorkspace {
        project_count,
        sessions,
        len: init_len,
        active_session,
    };

    let mut saw_two_project_partition = false;
    let mut saw_archive_round_trip = false;
    let mut saw_delete_active_same_project_fallback = false;
    let mut saw_delete_active_none_fallback = false;

    for _ in 0..3 {
        // Law 1: Disjoint Project Partition holds before every operation.
        let buckets = ws.group_by_project();
        let mut total_bucketed = 0usize;
        let mut p = 0usize;
        while p < MAX_WORKSPACE_PROJECTS {
            total_bucketed += buckets[p].len;
            let mut k = 0usize;
            while k < buckets[p].len {
                let s_idx = buckets[p].session_indices[k];
                assert!(s_idx < ws.len, "bucketed session index must be in bounds");
                assert_eq!(
                    ws.sessions[s_idx].project_id as usize, p,
                    "every session in bucket p must have project_id == p"
                );
                k += 1;
            }
            p += 1;
        }
        assert_eq!(
            total_bucketed, ws.len,
            "sum of session counts across project buckets must equal total sessions"
        );
        if buckets[0].len > 0 && buckets[1].len > 0 {
            saw_two_project_partition = true;
        }

        let op: u8 = kani::any();
        kani::assume(op <= 2);
        let target: usize = kani::any();
        kani::assume(target <= MAX_WORKSPACE_SESSIONS);

        let before = ws;
        match op {
            0 => {
                // archive + Law 2: Archive Reversibility.
                let ok = ws.archive(target);
                if target < before.len {
                    assert!(ok);
                    assert_eq!(ws.len, before.len, "archive must never delete a session");
                    assert!(
                        !ws.is_visible(target, false),
                        "archived session must be excluded from default active view"
                    );
                    assert!(
                        ws.is_visible(target, true),
                        "archived session must remain visible when show_archived == true"
                    );
                    if !before.sessions[target].archived {
                        let mut restored = ws;
                        assert!(restored.unarchive(target));
                        assert_eq!(
                            restored, before,
                            "unarchive(archive(id)) must restore exact original state"
                        );
                        saw_archive_round_trip = true;
                    }
                } else {
                    assert!(!ok);
                    assert_eq!(ws, before);
                }
            }
            1 => {
                // unarchive
                let ok = ws.unarchive(target);
                if target < before.len {
                    assert!(ok);
                    assert_eq!(ws.len, before.len);
                    assert!(ws.is_visible(target, false));
                } else {
                    assert!(!ok);
                    assert_eq!(ws, before);
                }
            }
            _ => {
                // delete + Law 3: Safe Active-Session Fallback on Delete.
                let ok = ws.delete(target);
                if target < before.len {
                    assert!(ok);
                    assert_eq!(ws.len, before.len - 1);

                    // Active session index must NEVER dangle.
                    if let Some(a) = ws.active_session {
                        assert!(a < ws.len, "active_session must never dangle after delete");
                    }

                    if before.active_session == Some(target) {
                        let deleted_project = before.sessions[target].project_id;
                        let mut has_remaining_active_in_project = false;
                        let mut r = 0usize;
                        while r < ws.len {
                            if ws.sessions[r].project_id == deleted_project
                                && !ws.sessions[r].archived
                            {
                                has_remaining_active_in_project = true;
                            }
                            r += 1;
                        }

                        if has_remaining_active_in_project {
                            let fallback = ws.active_session.expect(
                                "deleting active session must fall back when non-archived peer exists",
                            );
                            assert!(fallback < ws.len);
                            assert_eq!(ws.sessions[fallback].project_id, deleted_project);
                            assert!(!ws.sessions[fallback].archived);
                            saw_delete_active_same_project_fallback = true;
                        } else {
                            assert_eq!(
                                ws.active_session, None,
                                "deleting active session must yield None when no non-archived peer remains"
                            );
                            saw_delete_active_none_fallback = true;
                        }
                    }
                } else {
                    assert!(!ok);
                    assert_eq!(ws, before);
                }
            }
        }
    }

    kani::cover!(saw_two_project_partition, "partition_across_two_projects");
    kani::cover!(saw_archive_round_trip, "archive_unarchive_round_trip");
    kani::cover!(
        saw_delete_active_same_project_fallback,
        "delete_active_falls_back_same_project"
    );
    kani::cover!(
        saw_delete_active_none_fallback,
        "delete_active_falls_back_to_none"
    );
}
