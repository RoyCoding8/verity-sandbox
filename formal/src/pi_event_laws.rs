//! Kani 0.68.0 symbolic verification harnesses and unit tests for the Verity
//! chat-turn and Pi RPC to UI streaming contract (`verity_core::pi_wire`).

#[cfg(any(kani, test))]
use verity_core::pi_wire::{
    step_ui_turn, UiTurnEvent, UiTurnPhase, UiTurnState, WireRole, WireStopReason,
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

/// Law 1: No stuck thinking indicator and universal recovery.
///
/// From ANY symbolic `UiTurnState`, `UserAbort`, `ProcessExited`,
/// `AgentSettled` (matching epoch), `ErrorEvent` (matching epoch), and
/// `MessageEnd(Assistant, Error)` (matching epoch) always yield
/// `thinking_indicator_active() == false`, `is_streaming() == false`, and
/// `running_tools == 0`, and recovery operations are idempotent.
#[cfg(kani)]
#[kani::proof]
pub fn prove_no_stuck_thinking_and_universal_recovery() {
    let state = any_ui_turn_state();
    let epoch = state.session_epoch;
    let has_error_message: bool = kani::any();

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
}

/// Law 2: Thinking-to-text handoff and user-echo isolation.
///
/// From ANY symbolic `UiTurnState`:
/// - `TextDelta` (matching epoch) always turns off `thinking_indicator_active()`.
/// - `MessageStart(User)` and `MessageEnd(User, _)` never increment
///   `assistant_message_count` or mutate `has_assistant_text` /
///   `has_assistant_thinking`.
#[cfg(kani)]
#[kani::proof]
pub fn prove_thinking_to_text_handoff_and_user_echo_isolation() {
    let state = any_ui_turn_state();
    let any_epoch: u8 = kani::any();
    let stop_reason = any_stop_reason();
    let has_error_message: bool = kani::any();

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
    assert_eq!(
        after_user_start.assistant_message_count,
        state.assistant_message_count
    );
    assert_eq!(
        after_user_start.has_assistant_text,
        state.has_assistant_text
    );
    assert_eq!(
        after_user_start.has_assistant_thinking,
        state.has_assistant_thinking
    );

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
    assert_eq!(
        after_user_end.assistant_message_count,
        state.assistant_message_count
    );
    assert_eq!(after_user_end.has_assistant_text, state.has_assistant_text);
    assert_eq!(
        after_user_end.has_assistant_thinking,
        state.has_assistant_thinking
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

/// Law 3: Bounded 5-step symbolic trace invariants starting from `Idle`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_spinners_clear_on_abort_and_process_exit() {
        let s1 = step_ui_turn(UiTurnState::idle(0), UiTurnEvent::PromptSubmitted);
        let s2 = step_ui_turn(s1, UiTurnEvent::ToolStart { event_epoch: 0 });
        assert_eq!(s2.running_tools, 1);

        let aborted = step_ui_turn(s2, UiTurnEvent::UserAbort);
        assert!(!aborted.is_streaming());
        assert_eq!(aborted.running_tools, 0);

        let exited = step_ui_turn(s2, UiTurnEvent::ProcessExited);
        assert_eq!(exited.phase, UiTurnPhase::Disconnected);
        assert!(exited.has_visible_error);
        assert_eq!(exited.running_tools, 0);
    }

    #[test]
    fn message_end_error_preserves_error_after_agent_settled() {
        let s1 = step_ui_turn(UiTurnState::idle(0), UiTurnEvent::PromptSubmitted);
        let s2 = step_ui_turn(
            s1,
            UiTurnEvent::MessageEnd {
                role: WireRole::Assistant,
                stop_reason: WireStopReason::Error,
                has_error_message: true,
                event_epoch: 0,
            },
        );
        assert_eq!(s2.phase, UiTurnPhase::Failed);
        assert!(s2.has_visible_error);

        let s3 = step_ui_turn(s2, UiTurnEvent::AgentSettled { event_epoch: 0 });
        assert_eq!(s3.phase, UiTurnPhase::Failed);
        assert!(s3.has_visible_error);
        assert!(!s3.is_streaming());
    }

    #[test]
    fn retry_failure_marks_visible_error_and_stops_streaming() {
        let s1 = step_ui_turn(UiTurnState::idle(0), UiTurnEvent::PromptSubmitted);
        let s2 = step_ui_turn(s1, UiTurnEvent::AutoRetryStart { event_epoch: 0 });
        assert_eq!(s2.phase, UiTurnPhase::Retrying);
        assert!(s2.is_streaming());
        assert!(!s2.thinking_indicator_active());

        let s3 = step_ui_turn(
            s2,
            UiTurnEvent::AutoRetryEnd {
                success: false,
                event_epoch: 0,
            },
        );
        assert_eq!(s3.phase, UiTurnPhase::Failed);
        assert!(s3.has_visible_error);
        assert!(!s3.is_streaming());
    }
}
