//! Kani symbolic verification harnesses for the Verity chat-turn and
//! Pi RPC to UI streaming contract (`verity_core::pi_wire`).

use verity_core::pi_wire::WireStopReason;
#[cfg(kani)]
use verity_core::pi_wire::{step_ui_turn, UiTurnEvent, UiTurnPhase, UiTurnState, WireRole};

/// Delta payload kind carried by an incremental `message_update` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDeltaKind {
    /// Incremental thinking/reasoning delta.
    Thinking,
    /// Non-whitespace incremental assistant text delta.
    Text,
    /// Non-empty whitespace incremental assistant text delta (e.g. `" "`, `"\n"`).
    WhitespaceText,
}

/// Wire events for the incremental streaming buffer delivery model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingWireEvent {
    MessageStart,
    MessageUpdate {
        kind: StreamDeltaKind,
        delta_len: u8,
    },
    MessageEnd {
        final_text_len: u8,
        final_thinking_len: u8,
        stop_reason: WireStopReason,
    },
}

/// Allocation-free streaming buffer state tracking streamed vs. visible lengths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBufferState {
    pub is_streaming: bool,
    pub message_started: bool,
    pub message_ended: bool,
    pub streamed_text_len: u16,
    pub streamed_thinking_len: u16,
    pub visible_text_len: u16,
    pub visible_thinking_len: u16,
}

impl StreamBufferState {
    pub const fn idle() -> Self {
        Self {
            is_streaming: false,
            message_started: false,
            message_ended: false,
            streamed_text_len: 0,
            streamed_thinking_len: 0,
            visible_text_len: 0,
            visible_thinking_len: 0,
        }
    }
}

/// Returns `true` for terminal `stop_reason` values that must settle streaming.
pub const fn is_terminal_stop_reason(reason: WireStopReason) -> bool {
    matches!(
        reason,
        WireStopReason::Stop
            | WireStopReason::Length
            | WireStopReason::Aborted
            | WireStopReason::Error
    )
}

/// Pure transition function for incremental streaming delta delivery.
///
/// Guarantees:
/// - Every non-empty `Text` or `WhitespaceText` delta immediately increases
///   `visible_text_len` BEFORE `message_end` arrives (never buffered until end).
/// - `message_end` NEVER clobbers already-streamed text or thinking; it only
///   hydrates `visible_*_len` from `final_*_len` when `streamed_*_len == 0`.
/// - Terminal `stop_reason` always settles `is_streaming == false`.
pub const fn step_stream_buffer(
    state: StreamBufferState,
    event: StreamingWireEvent,
) -> StreamBufferState {
    match event {
        StreamingWireEvent::MessageStart => StreamBufferState {
            is_streaming: true,
            message_started: true,
            message_ended: false,
            streamed_text_len: 0,
            streamed_thinking_len: 0,
            visible_text_len: 0,
            visible_thinking_len: 0,
        },
        StreamingWireEvent::MessageUpdate { kind, delta_len } => {
            if !state.is_streaming || !state.message_started || state.message_ended {
                return state;
            }
            let d = delta_len as u16;
            match kind {
                StreamDeltaKind::Thinking => StreamBufferState {
                    streamed_thinking_len: state.streamed_thinking_len.saturating_add(d),
                    visible_thinking_len: state.visible_thinking_len.saturating_add(d),
                    ..state
                },
                StreamDeltaKind::Text | StreamDeltaKind::WhitespaceText => StreamBufferState {
                    streamed_text_len: state.streamed_text_len.saturating_add(d),
                    visible_text_len: state.visible_text_len.saturating_add(d),
                    ..state
                },
            }
        }
        StreamingWireEvent::MessageEnd {
            final_text_len,
            final_thinking_len,
            stop_reason,
        } => {
            if !state.message_started {
                return state;
            }
            let visible_text_len = if state.streamed_text_len == 0 {
                final_text_len as u16
            } else {
                state.visible_text_len
            };
            let visible_thinking_len = if state.streamed_thinking_len == 0 {
                final_thinking_len as u16
            } else {
                state.visible_thinking_len
            };
            let is_streaming = if is_terminal_stop_reason(stop_reason) {
                false
            } else {
                state.is_streaming
            };
            StreamBufferState {
                is_streaming,
                message_ended: true,
                visible_text_len,
                visible_thinking_len,
                ..state
            }
        }
    }
}

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
        has_visible_error: kani::any(),
        assistant_message_count: kani::any(),
        running_tools: kani::any(),
        committed_tools: kani::any(),
    }
}

#[cfg(kani)]
fn any_ui_turn_event() -> UiTurnEvent {
    let tag: u8 = kani::any();
    kani::assume(tag <= 13);
    match tag {
        0 => UiTurnEvent::PromptSubmitted,
        1 => UiTurnEvent::MessageStart {
            role: any_role(),
            event_epoch: kani::any(),
        },
        2 => UiTurnEvent::ThinkingDelta {
            event_epoch: kani::any(),
        },
        3 => UiTurnEvent::TextDelta {
            event_epoch: kani::any(),
        },
        4 => UiTurnEvent::ToolStart {
            event_epoch: kani::any(),
        },
        5 => UiTurnEvent::ToolEnd {
            event_epoch: kani::any(),
        },
        6 => UiTurnEvent::MessageEnd {
            role: any_role(),
            stop_reason: any_stop_reason(),
            has_error_message: kani::any(),
            event_epoch: kani::any(),
        },
        7 => UiTurnEvent::AutoRetryStart {
            event_epoch: kani::any(),
        },
        8 => UiTurnEvent::AutoRetryEnd {
            success: kani::any(),
            event_epoch: kani::any(),
        },
        9 => UiTurnEvent::ErrorEvent {
            event_epoch: kani::any(),
        },
        10 => UiTurnEvent::ProcessExited,
        11 => UiTurnEvent::AgentSettled {
            event_epoch: kani::any(),
        },
        12 => UiTurnEvent::UserAbort,
        _ => UiTurnEvent::SessionSwitched,
    }
}

#[cfg(kani)]
fn any_stream_delta_kind() -> StreamDeltaKind {
    let tag: u8 = kani::any();
    kani::assume(tag <= 2);
    match tag {
        0 => StreamDeltaKind::Thinking,
        1 => StreamDeltaKind::Text,
        _ => StreamDeltaKind::WhitespaceText,
    }
}

#[cfg(kani)]
fn any_streaming_wire_event() -> StreamingWireEvent {
    let tag: u8 = kani::any();
    kani::assume(tag <= 2);
    match tag {
        0 => StreamingWireEvent::MessageStart,
        1 => {
            let delta_len: u8 = kani::any();
            kani::assume(delta_len <= 4);
            StreamingWireEvent::MessageUpdate {
                kind: any_stream_delta_kind(),
                delta_len,
            }
        }
        _ => {
            let final_text_len: u8 = kani::any();
            let final_thinking_len: u8 = kani::any();
            kani::assume(final_text_len <= 8);
            kani::assume(final_thinking_len <= 8);
            StreamingWireEvent::MessageEnd {
                final_text_len,
                final_thinking_len,
                stop_reason: any_stop_reason(),
            }
        }
    }
}

/// Law 1: Single-step universal recovery, thinking-to-text handoff, and user-echo isolation.
///
/// From ANY symbolic `UiTurnState`:
/// - `UserAbort`, `ProcessExited`, `AgentSettled` (matching epoch), `ErrorEvent`
///   (matching epoch), and `MessageEnd(Assistant, Error)` (matching epoch) always
///   yield `thinking_indicator_active() == false`, `is_streaming() == false`, and
///   `running_tools == 0`, with idempotent recovery.
/// - `TextDelta` (matching epoch) always turns off `thinking_indicator_active()`.
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
    assert_eq!(step_ui_turn(after_abort, abort_ev), after_abort);

    let exit_ev = UiTurnEvent::ProcessExited;
    let after_exit = step_ui_turn(state, exit_ev);
    assert!(!after_exit.thinking_indicator_active());
    assert!(!after_exit.is_streaming());
    assert_eq!(after_exit.running_tools, 0);
    assert!(after_exit.has_visible_error);
    assert_eq!(step_ui_turn(after_exit, exit_ev), after_exit);

    let settled_ev = UiTurnEvent::AgentSettled { event_epoch: epoch };
    let after_settled = step_ui_turn(state, settled_ev);
    assert!(!after_settled.thinking_indicator_active());
    assert!(!after_settled.is_streaming());
    assert_eq!(after_settled.running_tools, 0);
    assert_eq!(step_ui_turn(after_settled, settled_ev), after_settled);

    let error_ev = UiTurnEvent::ErrorEvent { event_epoch: epoch };
    let after_error = step_ui_turn(state, error_ev);
    assert!(!after_error.thinking_indicator_active());
    assert!(!after_error.is_streaming());
    assert_eq!(after_error.running_tools, 0);
    assert!(after_error.has_visible_error);
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

    // Part B: Thinking-to-text handoff and user-echo isolation.
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
    }

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
/// - Reachability (`kani::cover!`) for normal OpenRouter completion
///   (thinking -> text -> settled), error turn preservation, user abort
///   recovery, process-exit recovery, and stale-epoch rejection.
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
}

/// Law 3: Incremental Streaming Delta Delivery Law.
///
/// Proves over all bounded 5-step sequences of `message_start`, incremental
/// `message_update` (thinking deltas, text deltas, and non-empty whitespace
/// deltas), and `message_end`:
/// 1. Every non-empty `text_delta` (including non-empty whitespace deltas)
///    strictly increases `visible_text_len` BEFORE `message_end` arrives.
/// 2. `message_end` NEVER clobbers already-streamed text or thinking, only
///    hydrating when the corresponding streamed length was 0.
/// 3. Terminal `stop_reason` (`Stop`, `Length`, `Aborted`, `Error`) always
///    settles `is_streaming == false`.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
pub fn prove_incremental_streaming_delta_delivery_laws() {
    let mut state = StreamBufferState::idle();

    let mut saw_incremental_text = false;
    let mut saw_incremental_whitespace = false;
    let mut saw_incremental_thinking = false;
    let mut saw_end_preserves_streamed_text = false;
    let mut saw_end_hydrates_unstreamed_text = false;
    let mut saw_terminal_stop_settles = false;

    for _ in 0..5 {
        let event = any_streaming_wire_event();
        let prev = state;
        let next = step_stream_buffer(prev, event);

        match event {
            StreamingWireEvent::MessageStart => {
                assert!(next.is_streaming);
                assert!(next.message_started);
                assert!(!next.message_ended);
                assert_eq!(next.streamed_text_len, 0);
                assert_eq!(next.visible_text_len, 0);
                assert_eq!(next.streamed_thinking_len, 0);
                assert_eq!(next.visible_thinking_len, 0);
            }
            StreamingWireEvent::MessageUpdate { kind, delta_len } => {
                if prev.is_streaming && prev.message_started && !prev.message_ended {
                    assert!(!next.message_ended);
                    match kind {
                        StreamDeltaKind::Text | StreamDeltaKind::WhitespaceText => {
                            if delta_len > 0 {
                                assert!(
                                    next.visible_text_len > prev.visible_text_len,
                                    "non-empty text/whitespace delta must strictly increase visible text before message_end"
                                );
                                assert_eq!(
                                    next.visible_text_len,
                                    prev.visible_text_len + delta_len as u16
                                );
                                if kind == StreamDeltaKind::Text {
                                    saw_incremental_text = true;
                                } else {
                                    saw_incremental_whitespace = true;
                                }
                            } else {
                                assert_eq!(next.visible_text_len, prev.visible_text_len);
                            }
                        }
                        StreamDeltaKind::Thinking => {
                            if delta_len > 0 {
                                assert!(
                                    next.visible_thinking_len > prev.visible_thinking_len,
                                    "non-empty thinking delta must strictly increase visible thinking before message_end"
                                );
                                saw_incremental_thinking = true;
                            }
                            assert_eq!(next.visible_text_len, prev.visible_text_len);
                        }
                    }
                } else {
                    assert_eq!(next, prev);
                }
            }
            StreamingWireEvent::MessageEnd {
                final_text_len,
                final_thinking_len,
                stop_reason,
            } => {
                if prev.message_started {
                    assert!(next.message_ended);
                    if prev.streamed_text_len > 0 {
                        assert_eq!(
                            next.visible_text_len, prev.visible_text_len,
                            "message_end must never clobber already-streamed text"
                        );
                        if final_text_len as u16 != prev.visible_text_len {
                            saw_end_preserves_streamed_text = true;
                        }
                    } else {
                        assert_eq!(
                            next.visible_text_len, final_text_len as u16,
                            "message_end must hydrate text when streamed_text_len == 0"
                        );
                        if final_text_len > 0 {
                            saw_end_hydrates_unstreamed_text = true;
                        }
                    }

                    if prev.streamed_thinking_len > 0 {
                        assert_eq!(
                            next.visible_thinking_len, prev.visible_thinking_len,
                            "message_end must never clobber already-streamed thinking"
                        );
                    } else {
                        assert_eq!(
                            next.visible_thinking_len, final_thinking_len as u16,
                            "message_end must hydrate thinking when streamed_thinking_len == 0"
                        );
                    }

                    if is_terminal_stop_reason(stop_reason) {
                        assert!(
                            !next.is_streaming,
                            "terminal stop_reason must always settle is_streaming == false"
                        );
                        if prev.is_streaming {
                            saw_terminal_stop_settles = true;
                        }
                    }
                } else {
                    assert_eq!(next, prev);
                }
            }
        }

        state = next;
    }

    kani::cover!(
        saw_incremental_text,
        "incremental_text_delta_before_message_end"
    );
    kani::cover!(
        saw_incremental_whitespace,
        "incremental_whitespace_delta_before_message_end"
    );
    kani::cover!(
        saw_incremental_thinking,
        "incremental_thinking_delta_before_message_end"
    );
    kani::cover!(
        saw_end_preserves_streamed_text,
        "message_end_preserves_streamed_text_without_clobber"
    );
    kani::cover!(
        saw_end_hydrates_unstreamed_text,
        "message_end_hydrates_unstreamed_text"
    );
    kani::cover!(
        saw_terminal_stop_settles,
        "terminal_stop_reason_settles_streaming"
    );
}
