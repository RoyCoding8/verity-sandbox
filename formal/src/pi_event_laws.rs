//! Kani symbolic verification harnesses for the Verity chat-turn and
//! Pi RPC to UI streaming contract (`verity_core::pi_wire`).

#[cfg(kani)]
use verity_core::pi_wire::{
    resolve_rewind_target, step_ui_turn, AssistantEventKind, EntryIdKind, RevertResolutionError,
    RewindTarget, UiTurnEvent, UiTurnPhase, UiTurnState, WireRole, WireStopReason,
};

#[cfg(kani)]
fn any_phase() -> UiTurnPhase {
    let tag: u8 = kani::any();
    kani::assume(tag <= 6);
    match tag {
        0 => UiTurnPhase::Idle,
        1 => UiTurnPhase::AwaitingFirstEvent,
        2 => UiTurnPhase::StreamingThinking,
        3 => UiTurnPhase::StreamingText,
        4 => UiTurnPhase::Retrying,
        5 => UiTurnPhase::Failed,
        _ => UiTurnPhase::Disconnected,
    }
}

#[cfg(kani)]
fn any_role() -> WireRole {
    let tag: u8 = kani::any();
    kani::assume(tag <= 3);
    match tag {
        0 => WireRole::User,
        1 => WireRole::Assistant,
        2 => WireRole::ToolResult,
        _ => WireRole::Other,
    }
}

#[cfg(kani)]
fn any_stop_reason() -> WireStopReason {
    let tag: u8 = kani::any();
    kani::assume(tag <= 5);
    match tag {
        0 => WireStopReason::Stop,
        1 => WireStopReason::Length,
        2 => WireStopReason::ToolUse,
        3 => WireStopReason::Aborted,
        4 => WireStopReason::Error,
        _ => WireStopReason::Unknown,
    }
}

#[cfg(kani)]
fn any_ui_turn_state() -> UiTurnState {
    UiTurnState {
        phase: any_phase(),
        session_epoch: kani::any(),
        has_assistant_text: kani::any(),
        has_assistant_thinking: kani::any(),
        thinking_duration_sealed: kani::any(),
        has_visible_error: kani::any(),
        assistant_message_count: kani::any(),
        running_tools: kani::any(),
        committed_tools: kani::any(),
    }
}

#[cfg(kani)]
fn any_ui_turn_event() -> UiTurnEvent {
    let tag: u8 = kani::any();
    kani::assume(tag <= 15);
    match tag {
        0 => UiTurnEvent::PromptSubmitted,
        1 => UiTurnEvent::MessageStart {
            role: any_role(),
            event_epoch: kani::any(),
        },
        2 => UiTurnEvent::ThinkingDelta {
            event_epoch: kani::any(),
        },
        3 => UiTurnEvent::ThinkingEnd {
            event_epoch: kani::any(),
        },
        4 => UiTurnEvent::TextDelta {
            event_epoch: kani::any(),
        },
        5 => UiTurnEvent::ToolCallDelta {
            event_epoch: kani::any(),
        },
        6 => UiTurnEvent::ToolStart {
            event_epoch: kani::any(),
        },
        7 => UiTurnEvent::ToolEnd {
            event_epoch: kani::any(),
        },
        8 => UiTurnEvent::MessageEnd {
            role: any_role(),
            stop_reason: any_stop_reason(),
            has_error_message: kani::any(),
            event_epoch: kani::any(),
        },
        9 => UiTurnEvent::AutoRetryStart {
            event_epoch: kani::any(),
        },
        10 => UiTurnEvent::AutoRetryEnd {
            success: kani::any(),
            event_epoch: kani::any(),
        },
        11 => UiTurnEvent::ErrorEvent {
            event_epoch: kani::any(),
        },
        12 => UiTurnEvent::ProcessExited,
        13 => UiTurnEvent::AgentSettled {
            event_epoch: kani::any(),
        },
        14 => UiTurnEvent::UserAbort,
        _ => UiTurnEvent::SessionSwitched,
    }
}

/// Law 1: Single-step universal recovery, thinking-to-text handoff, thinking duration sealing,
/// tool-call delta isolation, and user-echo isolation.
///
/// From ANY symbolic `UiTurnState`:
/// - `UserAbort`, `ProcessExited`, `AgentSettled` (matching epoch), `ErrorEvent`
///   (matching epoch), and `MessageEnd(Assistant, Error)` (matching epoch) always
///   yield `thinking_indicator_active() == false`, `is_streaming() == false`,
///   `running_tools == 0`, and seal `thinking_duration_sealed` whenever
///   `has_assistant_thinking`, with idempotent recovery.
/// - `TextDelta` (matching epoch) always turns off `thinking_indicator_active()` and
///   seals `thinking_duration_sealed` when streaming with `has_assistant_thinking`.
/// - `ThinkingEnd` (matching epoch) seals `thinking_duration_sealed` when streaming
///   with `has_assistant_thinking`.
/// - `ToolCallDelta` never mutates `has_assistant_text`, `has_assistant_thinking`,
///   or `thinking_duration_sealed`.
/// - `MessageStart(User)` and `MessageEnd(User, _)` never increment
///   `assistant_message_count` or mutate `has_assistant_text` / `has_assistant_thinking`.
#[cfg(kani)]
#[kani::proof]
pub fn prove_ui_turn_single_step_laws() {
    let state = any_ui_turn_state();
    let epoch = state.session_epoch;
    let any_epoch: u8 = kani::any();
    let stop_reason = any_stop_reason();
    let has_error_message: bool = kani::any();

    // Part A: Universal idempotent recovery & no stuck thinking indicator.
    let abort_ev = UiTurnEvent::UserAbort;
    let after_abort = step_ui_turn(state, abort_ev);
    assert!(!after_abort.thinking_indicator_active());
    assert!(!after_abort.is_streaming());
    assert_eq!(after_abort.running_tools, 0);
    if after_abort.has_assistant_thinking {
        assert!(after_abort.thinking_duration_sealed);
    }
    assert_eq!(step_ui_turn(after_abort, abort_ev), after_abort);

    let exit_ev = UiTurnEvent::ProcessExited;
    let after_exit = step_ui_turn(state, exit_ev);
    assert!(!after_exit.thinking_indicator_active());
    assert!(!after_exit.is_streaming());
    assert_eq!(after_exit.running_tools, 0);
    assert!(after_exit.has_visible_error);
    if after_exit.has_assistant_thinking {
        assert!(after_exit.thinking_duration_sealed);
    }
    assert_eq!(step_ui_turn(after_exit, exit_ev), after_exit);

    let settled_ev = UiTurnEvent::AgentSettled { event_epoch: epoch };
    let after_settled = step_ui_turn(state, settled_ev);
    assert!(!after_settled.thinking_indicator_active());
    assert!(!after_settled.is_streaming());
    assert_eq!(after_settled.running_tools, 0);
    if after_settled.has_assistant_thinking {
        assert!(after_settled.thinking_duration_sealed);
    }
    assert_eq!(step_ui_turn(after_settled, settled_ev), after_settled);

    let error_ev = UiTurnEvent::ErrorEvent { event_epoch: epoch };
    let after_error = step_ui_turn(state, error_ev);
    assert!(!after_error.thinking_indicator_active());
    assert!(!after_error.is_streaming());
    assert_eq!(after_error.running_tools, 0);
    assert!(after_error.has_visible_error);
    if after_error.has_assistant_thinking {
        assert!(after_error.thinking_duration_sealed);
    }
    assert_eq!(step_ui_turn(after_error, error_ev), after_error);

    let msg_err_ev = UiTurnEvent::MessageEnd {
        role: WireRole::Assistant,
        stop_reason: WireStopReason::Error,
        has_error_message,
        event_epoch: epoch,
    };
    let after_msg_err = step_ui_turn(state, msg_err_ev);
    assert!(!after_msg_err.thinking_indicator_active());
    assert!(!after_msg_err.is_streaming());
    assert_eq!(after_msg_err.running_tools, 0);
    assert!(after_msg_err.has_visible_error);
    if after_msg_err.has_assistant_thinking {
        assert!(after_msg_err.thinking_duration_sealed);
    }

    // Part B: Thinking-to-text handoff, ThinkingEnd sealing, ToolCallDelta isolation,
    // and user-echo isolation.
    let after_text = step_ui_turn(
        state,
        UiTurnEvent::TextDelta {
            event_epoch: state.session_epoch,
        },
    );
    assert!(!after_text.thinking_indicator_active());
    if state.is_streaming() {
        assert_eq!(after_text.phase, UiTurnPhase::StreamingText);
        assert!(after_text.has_assistant_text);
        if state.has_assistant_thinking {
            assert!(after_text.thinking_duration_sealed);
        }
    }

    let after_thinking_end = step_ui_turn(
        state,
        UiTurnEvent::ThinkingEnd {
            event_epoch: state.session_epoch,
        },
    );
    if state.is_streaming() && state.has_assistant_thinking {
        assert!(after_thinking_end.thinking_duration_sealed);
    }

    let after_toolcall = step_ui_turn(
        state,
        UiTurnEvent::ToolCallDelta {
            event_epoch: any_epoch,
        },
    );
    assert_eq!(after_toolcall.has_assistant_text, state.has_assistant_text);
    assert_eq!(
        after_toolcall.has_assistant_thinking,
        state.has_assistant_thinking
    );
    assert_eq!(
        after_toolcall.thinking_duration_sealed,
        state.thinking_duration_sealed
    );
    assert_eq!(after_toolcall, state);

    let after_user_start = step_ui_turn(
        state,
        UiTurnEvent::MessageStart {
            role: WireRole::User,
            event_epoch: any_epoch,
        },
    );
    assert_eq!(after_user_start, state);

    let after_user_end = step_ui_turn(
        state,
        UiTurnEvent::MessageEnd {
            role: WireRole::User,
            stop_reason,
            has_error_message,
            event_epoch: any_epoch,
        },
    );
    assert_eq!(after_user_end, state);

    kani::cover!(
        state.thinking_indicator_active() && !after_abort.thinking_indicator_active(),
        "abort_clears_active_thinking"
    );
    kani::cover!(
        state.running_tools > 0 && after_exit.phase == UiTurnPhase::Disconnected,
        "process_exit_clears_running_tools"
    );
    kani::cover!(
        state.is_streaming() && after_msg_err.phase == UiTurnPhase::Failed,
        "message_end_error_recovers_streaming_turn"
    );
    kani::cover!(
        state.thinking_indicator_active() && after_text.phase == UiTurnPhase::StreamingText,
        "thinking_hands_off_to_streaming_text"
    );
    kani::cover!(
        state.is_streaming()
            && state.has_assistant_thinking
            && !state.thinking_duration_sealed
            && after_thinking_end.thinking_duration_sealed,
        "thinking_end_seals_active_thinking_duration"
    );
    kani::cover!(
        state.is_streaming() && any_epoch == state.session_epoch && !state.has_assistant_text,
        "toolcall_delta_isolated_in_matching_epoch"
    );
    kani::cover!(
        state.is_streaming() && any_epoch == state.session_epoch,
        "matching_epoch_user_echo_isolated"
    );
}

/// Law 2: Bounded 5-step symbolic trace invariants starting from `Idle`.
///
/// Proves over all 5-step sequences of arbitrary `UiTurnEvent`s:
/// - Cross-epoch events (`event_epoch != state.session_epoch`) never mutate `state`.
/// - Any turn that received an error (`ErrorEvent`, `ProcessExited`, or
///   `MessageEnd(Assistant, Error)`) keeps `has_visible_error == true` even
///   after a trailing `AgentSettled`.
/// - `running_tools == 0` whenever `!is_streaming()`.
/// - `thinking_duration_sealed` holds whenever
///   `has_assistant_thinking && (has_assistant_text || !is_streaming())`.
/// - `ToolCallDelta` never sets `has_assistant_text` or `has_assistant_thinking`.
/// - Reachability (`kani::cover!`) for normal OpenRouter completion
///   (thinking -> text -> settled), error turn preservation, user abort
///   recovery, process-exit recovery, stale-epoch rejection, `ThinkingEnd`
///   sealing, and `ToolCallDelta` isolation.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
pub fn prove_bounded_ui_turn_trace_invariants() {
    let mut state = UiTurnState::idle(0);

    let mut turn_saw_error = false;
    let mut saw_error_then_settled = false;
    let mut saw_thinking_then_text_then_settled = false;
    let mut saw_user_abort_recovery = false;
    let mut saw_process_exit_recovery = false;
    let mut saw_stale_epoch_rejection = false;
    let mut saw_thinking_end_sealed = false;
    let mut saw_toolcall_delta_isolated = false;

    for _ in 0..5 {
        let event = any_ui_turn_event();
        let prev = state;
        let next = step_ui_turn(prev, event);

        // 1. Cross-epoch fencing: zero cross-session leakage.
        if let Some(ev_epoch) = event.event_epoch() {
            if ev_epoch != prev.session_epoch {
                assert_eq!(next, prev, "cross-epoch event must never mutate state");
                saw_stale_epoch_rejection = true;
            }
        }

        // 2. Track turn-scoped error provenance and verify zero silent error erasure.
        match event {
            UiTurnEvent::PromptSubmitted if !prev.is_streaming() => {
                turn_saw_error = false;
            }
            UiTurnEvent::SessionSwitched => {
                turn_saw_error = false;
            }
            UiTurnEvent::ProcessExited => {
                turn_saw_error = true;
            }
            UiTurnEvent::ErrorEvent { event_epoch } if event_epoch == prev.session_epoch => {
                turn_saw_error = true;
            }
            UiTurnEvent::MessageEnd {
                role: WireRole::Assistant,
                stop_reason: WireStopReason::Error,
                event_epoch,
                ..
            } if event_epoch == prev.session_epoch => {
                turn_saw_error = true;
            }
            _ => {}
        }

        if turn_saw_error {
            assert!(
                next.has_visible_error,
                "error in active turn must remain visible"
            );
        }

        // 3. No stuck tool spinners when not streaming.
        if !next.is_streaming() {
            assert_eq!(
                next.running_tools, 0,
                "non-streaming state must never retain running tools"
            );
        }

        // 4. Thinking indicator predicate discipline.
        if next.has_assistant_text || !next.is_streaming() {
            assert!(!next.thinking_indicator_active());
        }

        // 5. Thinking duration sealing discipline.
        if next.has_assistant_thinking && (next.has_assistant_text || !next.is_streaming()) {
            assert!(
                next.thinking_duration_sealed,
                "thinking duration must be sealed once text arrives or turn settles"
            );
        }

        // 6. ToolCallDelta isolation.
        if let UiTurnEvent::ToolCallDelta { event_epoch } = event {
            assert_eq!(next.has_assistant_text, prev.has_assistant_text);
            assert_eq!(next.has_assistant_thinking, prev.has_assistant_thinking);
            assert_eq!(next.thinking_duration_sealed, prev.thinking_duration_sealed);
            if event_epoch == prev.session_epoch && prev.is_streaming() {
                saw_toolcall_delta_isolated = true;
            }
        }

        if let UiTurnEvent::ThinkingEnd { event_epoch } = event {
            if event_epoch == prev.session_epoch
                && prev.is_streaming()
                && prev.has_assistant_thinking
                && !prev.thinking_duration_sealed
                && next.thinking_duration_sealed
            {
                saw_thinking_end_sealed = true;
            }
        }

        // Track reachability milestones for kani::cover! probes.
        if let UiTurnEvent::AgentSettled { event_epoch } = event {
            if event_epoch == prev.session_epoch {
                if turn_saw_error && next.has_visible_error && !next.is_streaming() {
                    saw_error_then_settled = true;
                }
                if prev.phase == UiTurnPhase::StreamingText
                    && prev.has_assistant_thinking
                    && prev.has_assistant_text
                    && !next.has_visible_error
                    && next.phase == UiTurnPhase::Idle
                {
                    saw_thinking_then_text_then_settled = true;
                }
            }
        }
        if matches!(event, UiTurnEvent::UserAbort)
            && prev.is_streaming()
            && !next.is_streaming()
            && !next.thinking_indicator_active()
        {
            saw_user_abort_recovery = true;
        }
        if matches!(event, UiTurnEvent::ProcessExited)
            && prev.is_streaming()
            && next.phase == UiTurnPhase::Disconnected
            && next.has_visible_error
        {
            saw_process_exit_recovery = true;
        }

        state = next;
    }

    kani::cover!(
        saw_thinking_then_text_then_settled,
        "reach_normal_openrouter_completion"
    );
    kani::cover!(saw_error_then_settled, "reach_error_turn_preservation");
    kani::cover!(saw_user_abort_recovery, "reach_user_abort_recovery");
    kani::cover!(saw_process_exit_recovery, "reach_process_exit_recovery");
    kani::cover!(saw_stale_epoch_rejection, "reach_stale_epoch_rejection");
    kani::cover!(saw_thinking_end_sealed, "reach_thinking_end_sealed");
    kani::cover!(saw_toolcall_delta_isolated, "reach_toolcall_delta_isolated");
}

#[cfg(kani)]
fn contains_u8(slice: &[u8], needle: u8) -> bool {
    let mut i = 0;
    while i < slice.len() {
        if slice[i] == needle {
            return true;
        }
        i += 1;
    }
    false
}

/// Law 3: Direct `verity-core` assistant event allowlist classification and
/// rewind target resolution laws.
///
/// Part 1 directly proves `AssistantEventKind::parse_str` across real Pi RPC
/// wire event strings, ensuring non-prose events NEVER classify as `TextDelta`
/// or `ThinkingDelta`.
///
/// Part 2 directly proves `resolve_rewind_target` over symbolic `EntryIdKind`,
/// symbolic `user_turn_idx: Option<usize>`, and symbolic bounded `server_entries`
/// (`len <= 3`), proving:
/// - `Ok(RewindTarget::ForkToServer(id))` ALWAYS satisfies `contains_u8(server_entries, id)`
///   so a `ClientLocal` ID can NEVER leak into `ForkToServer`.
/// - `Ok(RewindTarget::NewSession)` ONLY happens when `server_entries.is_empty()`
///   and `user_turn_idx == Some(0)`.
/// - Out-of-bounds / missing IDs ALWAYS return `Err(RevertResolutionError::UnresolvableEntryId)`.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(18)]
pub fn prove_assistant_event_allowlist_and_rewind_resolution() {
    // Part 1: AssistantEventKind::parse_str allowlist across real wire strings.
    let wire_tag: u8 = kani::any();
    kani::assume(wire_tag <= 12);
    let raw = match wire_tag {
        0 => "text_delta",
        1 => "text_end",
        2 => "thinking_start",
        3 => "thinking_delta",
        4 => "reasoning_delta",
        5 => "thinking_end",
        6 => "toolcall_start",
        7 => "toolcall_delta",
        8 => "toolcall_end",
        9 => "input_json_delta",
        10 => "start",
        11 => "done",
        _ => "unknown",
    };

    let parsed = AssistantEventKind::parse_str(raw);
    match wire_tag {
        0 => assert_eq!(parsed, AssistantEventKind::TextDelta),
        1 => assert_eq!(parsed, AssistantEventKind::TextEnd),
        2 => assert_eq!(parsed, AssistantEventKind::ThinkingStart),
        3 | 4 => assert_eq!(parsed, AssistantEventKind::ThinkingDelta),
        5 => assert_eq!(parsed, AssistantEventKind::ThinkingEnd),
        6..=9 => assert_eq!(parsed, AssistantEventKind::ToolCall),
        _ => assert_eq!(parsed, AssistantEventKind::Ignored),
    }

    if !matches!(wire_tag, 0 | 3 | 4) {
        assert_ne!(
            parsed,
            AssistantEventKind::TextDelta,
            "non-prose wire event must never classify as TextDelta"
        );
        assert_ne!(
            parsed,
            AssistantEventKind::ThinkingDelta,
            "non-prose wire event must never classify as ThinkingDelta"
        );
    }

    // Part 2: resolve_rewind_target over symbolic EntryIdKind, user_turn_idx, and server_entries.
    let requested = if kani::any() {
        EntryIdKind::Server(kani::any())
    } else {
        EntryIdKind::ClientLocal(kani::any())
    };
    let user_turn_idx: Option<usize> = if kani::any() {
        let idx: usize = kani::any();
        kani::assume(idx <= 4);
        Some(idx)
    } else {
        None
    };
    let buf: [u8; 3] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 3);
    let server_entries = &buf[..len];

    let resolved = resolve_rewind_target(requested, user_turn_idx, server_entries);

    match resolved {
        Ok(RewindTarget::ForkToServer(id)) => {
            assert!(
                contains_u8(server_entries, id),
                "ForkToServer must always resolve to an authoritative server entry"
            );
            match requested {
                EntryIdKind::Server(req_id) if contains_u8(server_entries, req_id) => {
                    assert_eq!(id, req_id);
                }
                _ => {
                    let idx = user_turn_idx.expect("fallback ForkToServer requires user_turn_idx");
                    assert!(idx < server_entries.len());
                    assert_eq!(id, server_entries[idx]);
                }
            }
        }
        Ok(RewindTarget::NewSession) => {
            assert!(
                server_entries.is_empty() && user_turn_idx == Some(0),
                "NewSession is only permitted when server_entries is empty and user_turn_idx == Some(0)"
            );
        }
        Err(RevertResolutionError::UnresolvableEntryId) => {
            let direct_server_match = matches!(
                requested,
                EntryIdKind::Server(req_id) if contains_u8(server_entries, req_id)
            );
            assert!(!direct_server_match);
            if let Some(idx) = user_turn_idx {
                assert!(idx >= server_entries.len());
                assert!(!(idx == 0 && server_entries.is_empty()));
            }
        }
    }

    // Explicit out-of-bounds / missing ID invariant:
    let has_direct_server_match = matches!(
        requested,
        EntryIdKind::Server(req_id) if contains_u8(server_entries, req_id)
    );
    let has_valid_turn_idx = matches!(user_turn_idx, Some(idx) if idx < server_entries.len());
    let is_empty_first_turn = server_entries.is_empty() && user_turn_idx == Some(0);
    if !has_direct_server_match && !has_valid_turn_idx && !is_empty_first_turn {
        assert_eq!(resolved, Err(RevertResolutionError::UnresolvableEntryId));
    }

    // Cover probes: every branch must be reachable (SATISFIED).
    kani::cover!(
        matches!(
            requested,
            EntryIdKind::Server(req_id)
                if resolved == Ok(RewindTarget::ForkToServer(req_id))
                    && contains_u8(server_entries, req_id)
        ),
        "server_direct_match"
    );
    kani::cover!(
        matches!(requested, EntryIdKind::ClientLocal(_))
            && matches!(resolved, Ok(RewindTarget::ForkToServer(_))),
        "client_local_resolved_via_user_turn_idx"
    );
    kani::cover!(
        resolved == Ok(RewindTarget::NewSession),
        "new_session_fallback"
    );
    kani::cover!(
        resolved == Err(RevertResolutionError::UnresolvableEntryId),
        "unresolvable_entry_id"
    );
    kani::cover!(
        parsed == AssistantEventKind::ToolCall,
        "toolcall_allowlist_rejected"
    );
    kani::cover!(
        parsed == AssistantEventKind::Ignored,
        "ignored_allowlist_rejected"
    );
}
