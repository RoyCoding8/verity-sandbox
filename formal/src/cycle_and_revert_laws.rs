//! Kani symbolic verification harnesses for:
//! 1. Cycle Model resolution across wire shapes (`NestedModel`, `FlatModel`,
//!    `NullOrEmpty`, `SameAsCurrent`) and Thinking Level cycling (`off..=xhigh`).
//! 2. Targeted Revert & Lossless Redo laws (chat-only isolation, targeted
//!    `mtime` preservation, exact restoration & deletion, lossless redo
//!    round-trip, and missing/failed snapshot rejection).

/// Maximum number of configured models in the bounded Cycle Model contract.
pub const MAX_MODELS: usize = 4;

/// Canonical `(provider, id)` identifier for a configured model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelKey {
    pub provider: u8,
    pub id: u8,
}

/// Wire response shapes returned by Pi RPC `cycle_model`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleModelWireResponse {
    /// Nested JSON shape: `{ "model": { "provider": ..., "id": ... } }`.
    NestedModel { provider: u8, id: u8 },
    /// Flat JSON shape: `{ "provider": ..., "id": ... }`.
    FlatModel { provider: u8, id: u8 },
    /// Empty, missing, or `null` wire payload.
    NullOrEmpty,
    /// Wire response echoed the already-active model without advancing.
    SameAsCurrent,
}

/// Resolved outcome of a `cycle_model` action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleModelResolution {
    /// Successfully cycled to a distinct, in-bounds model index.
    Cycled(usize),
    /// Exactly one model is configured; cycling is a safe no-op.
    SingleModelNoOp,
    /// Zero models are configured; cycling is a safe no-op.
    NoModelsAvailable,
}

/// Finds the first index matching `key` in `models`.
pub fn find_model_index(models: &[ModelKey], key: ModelKey) -> Option<usize> {
    let mut i = 0usize;
    while i < models.len() {
        if models[i] == key {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Resolves the next model index for `cycle_model` across all wire shapes.
///
/// Guarantees:
/// - `N == 0` => `NoModelsAvailable`
/// - `N == 1` => `SingleModelNoOp`
/// - `N >= 2` => `Cycled(next_idx)` with `next_idx < N` and `next_idx != current`
///   (wrapping `N - 1 -> 0` on fallback when Pi returns `null`, `SameAsCurrent`,
///   an unknown model, or the current model).
/// - `NestedModel { provider, id }` and `FlatModel { provider, id }` resolve
///   identically for all inputs.
pub fn resolve_cycle_model(
    models: &[ModelKey],
    current_idx: usize,
    wire_response: CycleModelWireResponse,
) -> CycleModelResolution {
    let n = models.len();
    if n == 0 {
        return CycleModelResolution::NoModelsAvailable;
    }
    if n == 1 {
        return CycleModelResolution::SingleModelNoOp;
    }

    let current = if current_idx < n { current_idx } else { 0 };
    let fallback_idx = (current + 1) % n;

    match wire_response {
        CycleModelWireResponse::NestedModel { provider, id }
        | CycleModelWireResponse::FlatModel { provider, id } => {
            let key = ModelKey { provider, id };
            match find_model_index(models, key) {
                Some(idx) if idx != current => CycleModelResolution::Cycled(idx),
                _ => CycleModelResolution::Cycled(fallback_idx),
            }
        }
        CycleModelWireResponse::NullOrEmpty | CycleModelWireResponse::SameAsCurrent => {
            CycleModelResolution::Cycled(fallback_idx)
        }
    }
}

/// Number of ordered thinking levels (`Off..=XHigh`).
pub const THINKING_LEVEL_COUNT: u8 = 6;

/// Ordered thinking levels supported by Verity and Pi RPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ThinkingLevel {
    Off = 0,
    Minimal = 1,
    Low = 2,
    Medium = 3,
    High = 4,
    XHigh = 5,
}

impl ThinkingLevel {
    pub const fn as_index(self) -> u8 {
        self as u8
    }

    pub const fn from_index(idx: u8) -> Option<Self> {
        match idx {
            0 => Some(Self::Off),
            1 => Some(Self::Minimal),
            2 => Some(Self::Low),
            3 => Some(Self::Medium),
            4 => Some(Self::High),
            5 => Some(Self::XHigh),
            _ => None,
        }
    }

    /// Advances to the next thinking level modulo 6 (`XHigh -> Off`).
    pub const fn next(self) -> Self {
        match (self as u8 + 1) % THINKING_LEVEL_COUNT {
            0 => Self::Off,
            1 => Self::Minimal,
            2 => Self::Low,
            3 => Self::Medium,
            4 => Self::High,
            _ => Self::XHigh,
        }
    }
}

/// Wire response shapes returned by Pi RPC `cycle_thinking_level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleThinkingWireResponse {
    /// Wire returned a valid thinking level.
    Level(ThinkingLevel),
    /// Wire returned `null`, empty, or omitted the level.
    NullOrUnchanged,
}

/// Resolves the next thinking level across the 6 ordered levels.
///
/// Accepts any valid distinct wire level; when Pi returns `NullOrUnchanged` or
/// echoes the current level unchanged, advances modulo 6 (`XHigh -> Off`).
pub const fn resolve_cycle_thinking_level(
    current: ThinkingLevel,
    wire: CycleThinkingWireResponse,
) -> ThinkingLevel {
    match wire {
        CycleThinkingWireResponse::Level(lvl) if lvl as u8 != current as u8 => lvl,
        CycleThinkingWireResponse::Level(_) | CycleThinkingWireResponse::NullOrUnchanged => {
            current.next()
        }
    }
}

// ============================================================================
// Targeted Revert & Lossless Redo Model
// ============================================================================

/// Bounded number of files in the workspace model.
pub const NUM_FILES: usize = 3;

/// Maximum content value (`0 = absent/deleted`, `1..=3 = distinct file contents`).
pub const MAX_CONTENT: u8 = 3;

/// Maximum number of snapshots stored in the snapshot table.
pub const MAX_SNAPSHOTS: usize = 3;

/// Single file entry in the bounded workspace model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileEntry {
    /// `0` represents an absent/deleted file; `1..=3` represent distinct contents.
    pub content: u8,
    /// Monotonic-modulo-256 modification timestamp; incremented only on actual disk writes.
    pub mtime: u8,
}

/// Redo state captured BEFORE mutating disk during `revert_chat_and_code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedoRecord {
    pub tree: [u8; NUM_FILES],
    pub affected_mask: u8,
}

/// Error returned when a revert or redo operation cannot safely proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertError {
    MissingSnapshot,
    SnapshotCaptureFailed,
    NoRedoAvailable,
}

/// Bounded workspace + snapshot table + redo record for targeted revert/redo verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceModel {
    pub files: [FileEntry; NUM_FILES],
    pub snapshots: [[u8; NUM_FILES]; MAX_SNAPSHOTS],
    pub snap_count: usize,
    pub chat_turn: u8,
    pub redo: Option<RedoRecord>,
}

impl WorkspaceModel {
    pub const fn new(files: [FileEntry; NUM_FILES], chat_turn: u8) -> Self {
        Self {
            files,
            snapshots: [[0; NUM_FILES]; MAX_SNAPSHOTS],
            snap_count: 0,
            chat_turn,
            redo: None,
        }
    }

    pub const fn current_tree(&self) -> [u8; NUM_FILES] {
        [
            self.files[0].content,
            self.files[1].content,
            self.files[2].content,
        ]
    }

    /// Edits a single file; only bumps `mtime` if `content` actually changes.
    pub fn edit_file(&mut self, file_idx: usize, new_content: u8) {
        if file_idx < NUM_FILES
            && new_content <= MAX_CONTENT
            && self.files[file_idx].content != new_content
        {
            self.files[file_idx].content = new_content;
            self.files[file_idx].mtime = self.files[file_idx].mtime.wrapping_add(1);
        }
    }

    /// Captures the current workspace tree into the snapshot table.
    /// Returns `Err(SnapshotCaptureFailed)` if capture fails or table is full.
    pub fn capture_snapshot(&mut self, capture_ok: bool) -> Result<usize, RevertError> {
        if !capture_ok || self.snap_count >= MAX_SNAPSHOTS {
            return Err(RevertError::SnapshotCaptureFailed);
        }
        let id = self.snap_count;
        self.snapshots[id] = self.current_tree();
        self.snap_count += 1;
        Ok(id)
    }

    /// Reverts only the chat cursor, leaving every file's `content` and `mtime` untouched.
    pub fn revert_chat_only(&mut self, target_turn: u8) {
        self.chat_turn = target_turn;
    }

    /// Targeted chat + code revert to `snap_id`.
    ///
    /// - Rejects missing `snap_id` or failed pre-revert redo snapshot capture
    ///   without mutating any file or `mtime`.
    /// - Captures `RedoRecord { tree, affected_mask }` BEFORE mutating disk.
    /// - Only writes/deletes files whose `content` differs from `snapshots[snap_id]`,
    ///   preserving `mtime` of every already-matching file.
    pub fn revert_chat_and_code(
        &mut self,
        snap_id: usize,
        target_turn: u8,
        capture_redo_ok: bool,
    ) -> Result<(), RevertError> {
        if snap_id >= self.snap_count {
            return Err(RevertError::MissingSnapshot);
        }
        if !capture_redo_ok {
            return Err(RevertError::SnapshotCaptureFailed);
        }

        let pre_revert_tree = self.current_tree();
        let target_tree = self.snapshots[snap_id];

        let mut affected_mask = 0u8;
        let mut f = 0usize;
        while f < NUM_FILES {
            if pre_revert_tree[f] != target_tree[f] {
                affected_mask |= 1u8 << f;
            }
            f += 1;
        }

        self.redo = Some(RedoRecord {
            tree: pre_revert_tree,
            affected_mask,
        });

        let mut i = 0usize;
        while i < NUM_FILES {
            if self.files[i].content != target_tree[i] {
                self.files[i].content = target_tree[i];
                self.files[i].mtime = self.files[i].mtime.wrapping_add(1);
            }
            i += 1;
        }

        self.chat_turn = target_turn;
        Ok(())
    }

    /// Targeted redo of the most recent `revert_chat_and_code`.
    ///
    /// Restores every affected file to its exact pre-revert `content` while
    /// never touching `mtime` of unaffected files.
    pub fn redo_revert(&mut self) -> Result<(), RevertError> {
        let Some(record) = self.redo.take() else {
            return Err(RevertError::NoRedoAvailable);
        };

        let mut i = 0usize;
        while i < NUM_FILES {
            let bit = 1u8 << i;
            if (record.affected_mask & bit) != 0 && self.files[i].content != record.tree[i] {
                self.files[i].content = record.tree[i];
                self.files[i].mtime = self.files[i].mtime.wrapping_add(1);
            }
            i += 1;
        }

        Ok(())
    }
}

#[cfg(kani)]
fn any_thinking_level() -> ThinkingLevel {
    let idx: u8 = kani::any();
    kani::assume(idx < THINKING_LEVEL_COUNT);
    ThinkingLevel::from_index(idx).unwrap()
}

#[cfg(kani)]
fn any_wire_model_response() -> CycleModelWireResponse {
    let tag: u8 = kani::any();
    kani::assume(tag <= 3);
    match tag {
        0 => CycleModelWireResponse::NestedModel {
            provider: kani::any(),
            id: kani::any(),
        },
        1 => CycleModelWireResponse::FlatModel {
            provider: kani::any(),
            id: kani::any(),
        },
        2 => CycleModelWireResponse::NullOrEmpty,
        _ => CycleModelWireResponse::SameAsCurrent,
    }
}

/// Harness 1 — Cycle Model & Thinking Level Cycling Laws.
///
/// Proves across all configured model lists of size `N in 0..=4` and all wire
/// response shapes (`NestedModel`, `FlatModel`, `NullOrEmpty`, `SameAsCurrent`):
/// 1. For `N >= 2`, `resolve_cycle_model` ALWAYS returns `Cycled(next_idx)` with
///    `next_idx < N` and `next_idx != current_idx` (wrapping `N - 1 -> 0` on fallback).
/// 2. `NestedModel { provider, id }` and `FlatModel { provider, id }` resolve to
///    the exact same result and `next_idx`.
/// 3. For `N == 1`, returns `SingleModelNoOp` without panicking.
/// 4. For `N == 0`, returns `NoModelsAvailable` without panicking.
/// 5. Thinking level cycling across `Off..=XHigh` always advances modulo 6 on
///    `NullOrUnchanged`/same-level and accepts any valid distinct wire level.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
pub fn prove_cycle_model_and_thinking_laws() {
    let n: usize = kani::any();
    kani::assume(n <= MAX_MODELS);

    let mut models_buf = [ModelKey { provider: 0, id: 0 }; MAX_MODELS];
    let mut i = 0usize;
    while i < n {
        models_buf[i] = ModelKey {
            provider: kani::any(),
            id: kani::any(),
        };
        i += 1;
    }
    let models = &models_buf[..n];

    let current_idx: usize = kani::any();
    if n > 0 {
        kani::assume(current_idx < n);
    }

    let wire = any_wire_model_response();
    let res = resolve_cycle_model(models, current_idx, wire);

    // Law 1, 3, 4: Exact resolution contract by N.
    match n {
        0 => assert_eq!(res, CycleModelResolution::NoModelsAvailable),
        1 => assert_eq!(res, CycleModelResolution::SingleModelNoOp),
        _ => match res {
            CycleModelResolution::Cycled(next_idx) => {
                assert!(next_idx < n, "next_idx must be in bounds");
                assert_ne!(
                    next_idx, current_idx,
                    "cycle_model with N >= 2 must never silently no-op"
                );
                if matches!(
                    wire,
                    CycleModelWireResponse::NullOrEmpty | CycleModelWireResponse::SameAsCurrent
                ) {
                    assert_eq!(
                        next_idx,
                        (current_idx + 1) % n,
                        "fallback must advance modulo N (wrapping N - 1 -> 0)"
                    );
                }
            }
            _ => panic!("N >= 2 must always return Cycled(next_idx)"),
        },
    }

    // Law 2: NestedModel and FlatModel with identical (provider, id) always match.
    let p: u8 = kani::any();
    let id: u8 = kani::any();
    let nested_res = resolve_cycle_model(
        models,
        current_idx,
        CycleModelWireResponse::NestedModel { provider: p, id },
    );
    let flat_res = resolve_cycle_model(
        models,
        current_idx,
        CycleModelWireResponse::FlatModel { provider: p, id },
    );
    assert_eq!(
        nested_res, flat_res,
        "NestedModel and FlatModel with identical (provider, id) must resolve identically"
    );

    // Law 5: Thinking level cycling across Off..=XHigh (modulo 6).
    let current_thinking = any_thinking_level();
    let thinking_wire = if kani::any() {
        CycleThinkingWireResponse::Level(any_thinking_level())
    } else {
        CycleThinkingWireResponse::NullOrUnchanged
    };
    let next_thinking = resolve_cycle_thinking_level(current_thinking, thinking_wire);

    assert!(next_thinking.as_index() < THINKING_LEVEL_COUNT);
    assert_ne!(
        next_thinking, current_thinking,
        "thinking level cycling must always advance"
    );
    match thinking_wire {
        CycleThinkingWireResponse::Level(lvl) if lvl != current_thinking => {
            assert_eq!(
                next_thinking, lvl,
                "valid distinct wire thinking level must be accepted"
            );
        }
        CycleThinkingWireResponse::Level(_) | CycleThinkingWireResponse::NullOrUnchanged => {
            assert_eq!(
                next_thinking.as_index(),
                (current_thinking.as_index() + 1) % THINKING_LEVEL_COUNT,
                "null/unchanged thinking response must advance modulo 6"
            );
        }
    }

    // Reachability probes (all must be SATISFIED).
    let matched_nested = matches!(
        wire,
        CycleModelWireResponse::NestedModel { provider, id }
            if n >= 2
                && matches!(
                    find_model_index(models, ModelKey { provider, id }),
                    Some(idx) if idx != current_idx && res == CycleModelResolution::Cycled(idx)
                )
    );
    let matched_flat = matches!(
        wire,
        CycleModelWireResponse::FlatModel { provider, id }
            if n >= 2
                && matches!(
                    find_model_index(models, ModelKey { provider, id }),
                    Some(idx) if idx != current_idx && res == CycleModelResolution::Cycled(idx)
                )
    );
    let fallback_wrap_around = n >= 2
        && current_idx == n - 1
        && matches!(
            wire,
            CycleModelWireResponse::NullOrEmpty | CycleModelWireResponse::SameAsCurrent
        )
        && res == CycleModelResolution::Cycled(0);

    kani::cover!(matched_nested, "cycle_model_nested_wire_match");
    kani::cover!(matched_flat, "cycle_model_flat_wire_match");
    kani::cover!(fallback_wrap_around, "cycle_model_fallback_wrap_around");
    kani::cover!(
        n == 1 && res == CycleModelResolution::SingleModelNoOp,
        "cycle_model_single_model_noop"
    );
    kani::cover!(
        n == 0 && res == CycleModelResolution::NoModelsAvailable,
        "cycle_model_empty_no_models"
    );
    kani::cover!(
        current_thinking == ThinkingLevel::XHigh
            && thinking_wire == CycleThinkingWireResponse::NullOrUnchanged
            && next_thinking == ThinkingLevel::Off,
        "thinking_level_wrap_xhigh_to_off"
    );
}

/// Harness 2 — Targeted Revert & Lossless Redo Safety Laws.
///
/// Proves all 5 safety invariants across bounded symbolic traces of
/// `edit_file`, `capture_snapshot`, `revert_chat_only`,
/// `revert_chat_and_code`, and `redo_revert` on a 3-file workspace:
/// 1. Chat-only isolation (`revert_chat_only` preserves every `content` and `mtime`).
/// 2. Targeted `mtime` preservation (`revert_chat_and_code` never mutates
///    `content` or `mtime` of any already-matching file).
/// 3. Exact restoration & deletion (every file matches `target_snapshot[f]`,
///    including `0 = deleted` for newly created files).
/// 4. Lossless targeted Redo round-trip (`redo_revert` immediately after
///    `revert_chat_and_code` restores exact pre-revert `content` while never
///    touching `mtime` of unaffected files).
/// 5. No silent stale/missing revert (missing snapshot ID or failed snapshot
///    capture returns `Err` and mutates zero files and zero `mtime`s).
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
pub fn prove_targeted_revert_and_redo_laws() {
    let mut init_files = [FileEntry {
        content: 0,
        mtime: 0,
    }; NUM_FILES];
    let mut f = 0usize;
    while f < NUM_FILES {
        let content: u8 = kani::any();
        kani::assume(content <= MAX_CONTENT);
        let mtime: u8 = kani::any();
        init_files[f] = FileEntry { content, mtime };
        f += 1;
    }

    let mut ws = WorkspaceModel::new(init_files, kani::any());

    // Tracks the exact pre-revert files when the immediately preceding step
    // was a successful `revert_chat_and_code`, enabling direct round-trip proof.
    let mut immediate_pre_revert: Option<[FileEntry; NUM_FILES]> = None;

    let mut saw_chat_only = false;
    let mut saw_targeted_revert_untouched_preserved = false;
    let mut saw_new_file_deleted = false;
    let mut saw_lossless_redo_round_trip = false;
    let mut saw_missing_snapshot_rejected = false;

    let steps: u8 = kani::any();
    kani::assume(steps <= 4);

    for _ in 0..steps {
        let op: u8 = kani::any();
        kani::assume(op <= 4);

        let before_files = ws.files;

        match op {
            0 => {
                // edit_file
                let file_idx: usize = kani::any();
                kani::assume(file_idx < NUM_FILES);
                let new_content: u8 = kani::any();
                kani::assume(new_content <= MAX_CONTENT);
                ws.edit_file(file_idx, new_content);
                immediate_pre_revert = None;
            }
            1 => {
                // capture_snapshot
                let capture_ok: bool = kani::any();
                let before_count = ws.snap_count;
                let res = ws.capture_snapshot(capture_ok);
                if !capture_ok || before_count >= MAX_SNAPSHOTS {
                    assert_eq!(res, Err(RevertError::SnapshotCaptureFailed));
                    assert_eq!(ws.snap_count, before_count);
                } else {
                    assert_eq!(res, Ok(before_count));
                    assert_eq!(ws.snap_count, before_count + 1);
                }
                // Capturing a snapshot never mutates workspace files.
                assert_eq!(ws.files, before_files);
                immediate_pre_revert = None;
            }
            2 => {
                // revert_chat_only — Law 1: Chat-only isolation.
                let target_turn: u8 = kani::any();
                ws.revert_chat_only(target_turn);
                assert_eq!(
                    ws.files, before_files,
                    "revert_chat_only must leave every file content and mtime byte-for-byte identical"
                );
                saw_chat_only = true;
                immediate_pre_revert = None;
            }
            3 => {
                // revert_chat_and_code — Laws 2, 3, 5.
                let snap_id: usize = kani::any();
                kani::assume(snap_id <= MAX_SNAPSHOTS);
                let target_turn: u8 = kani::any();
                let capture_redo_ok: bool = kani::any();

                let res = ws.revert_chat_and_code(snap_id, target_turn, capture_redo_ok);
                if snap_id >= ws.snap_count {
                    // Law 5a: Missing snapshot ID rejected with zero mutations.
                    assert_eq!(res, Err(RevertError::MissingSnapshot));
                    assert_eq!(
                        ws.files, before_files,
                        "missing snapshot must mutate zero files and zero mtimes"
                    );
                    saw_missing_snapshot_rejected = true;
                    immediate_pre_revert = None;
                } else if !capture_redo_ok {
                    // Law 5b: Failed redo snapshot capture rejected with zero mutations.
                    assert_eq!(res, Err(RevertError::SnapshotCaptureFailed));
                    assert_eq!(
                        ws.files, before_files,
                        "failed snapshot capture must mutate zero files and zero mtimes"
                    );
                    immediate_pre_revert = None;
                } else {
                    assert_eq!(res, Ok(()));
                    let target_tree = ws.snapshots[snap_id];

                    let mut any_reverted = false;
                    let mut any_untouched = false;
                    let mut any_deleted = false;

                    let mut i = 0usize;
                    while i < NUM_FILES {
                        // Law 3: Exact restoration & deletion.
                        assert_eq!(
                            ws.files[i].content, target_tree[i],
                            "reverted file content must exactly match target snapshot"
                        );
                        if before_files[i].content == target_tree[i] {
                            // Law 2: Targeted mtime preservation.
                            assert_eq!(
                                ws.files[i].mtime, before_files[i].mtime,
                                "unchanged file must keep its exact original mtime"
                            );
                            any_untouched = true;
                        } else {
                            any_reverted = true;
                            if before_files[i].content != 0 && target_tree[i] == 0 {
                                any_deleted = true;
                            }
                        }
                        i += 1;
                    }

                    if any_reverted && any_untouched {
                        saw_targeted_revert_untouched_preserved = true;
                    }
                    if any_deleted {
                        saw_new_file_deleted = true;
                    }
                    immediate_pre_revert = Some(before_files);
                }
            }
            _ => {
                // redo_revert — Law 4: Lossless targeted Redo round-trip.
                let had_redo = ws.redo;
                let res = ws.redo_revert();
                match had_redo {
                    None => {
                        assert_eq!(res, Err(RevertError::NoRedoAvailable));
                        assert_eq!(ws.files, before_files);
                    }
                    Some(record) => {
                        assert_eq!(res, Ok(()));
                        let mut i = 0usize;
                        while i < NUM_FILES {
                            let bit = 1u8 << i;
                            if (record.affected_mask & bit) == 0 {
                                // Unaffected files must never have content or mtime touched.
                                assert_eq!(ws.files[i], before_files[i]);
                            }
                            i += 1;
                        }
                        if let Some(pre_revert) = immediate_pre_revert {
                            let mut any_restored = false;
                            let mut any_unaffected = false;
                            let mut j = 0usize;
                            while j < NUM_FILES {
                                // Exact pre-revert content restored for EVERY file.
                                assert_eq!(
                                    ws.files[j].content, pre_revert[j].content,
                                    "immediate redo must restore exact pre-revert content"
                                );
                                let bit = 1u8 << j;
                                if (record.affected_mask & bit) == 0 {
                                    assert_eq!(
                                        ws.files[j].mtime, pre_revert[j].mtime,
                                        "immediate redo must never touch mtime of unaffected file"
                                    );
                                    any_unaffected = true;
                                } else {
                                    any_restored = true;
                                }
                                j += 1;
                            }
                            if any_restored && any_unaffected {
                                saw_lossless_redo_round_trip = true;
                            }
                        }
                    }
                }
                immediate_pre_revert = None;
            }
        }
    }

    // Reachability probes (all must be SATISFIED).
    kani::cover!(saw_chat_only, "revert_chat_only_reachable");
    kani::cover!(
        saw_targeted_revert_untouched_preserved,
        "targeted_code_revert_preserves_untouched_mtime"
    );
    kani::cover!(
        saw_new_file_deleted,
        "targeted_code_revert_deletes_newly_created_file"
    );
    kani::cover!(
        saw_lossless_redo_round_trip,
        "lossless_targeted_redo_round_trip"
    );
    kani::cover!(
        saw_missing_snapshot_rejected,
        "missing_snapshot_rejected_without_mutation"
    );
}
