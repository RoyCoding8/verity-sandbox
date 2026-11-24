//! Pure, allocation-free formal specification of the Verity chat-turn and
//! Pi RPC to UI streaming state machine.

/// Message role parsed at the Pi RPC wire boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireRole {
    User,
    Assistant,
    ToolResult,
    Other,
}

impl WireRole {
    /// Parses a wire role string into a typed `WireRole` at the RPC boundary.
    pub fn parse_str(raw: &str) -> Self {
        match raw {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "toolResult" | "tool_result" | "tool" => Self::ToolResult,
            _ => Self::Other,
        }
    }
}

/// Assistant message stop reason parsed at the Pi RPC wire boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireStopReason {
    Stop,
    Length,
    ToolUse,
    Aborted,
    Error,
    Unknown,
}

impl WireStopReason {
    /// Parses a wire stop-reason string into a typed `WireStopReason` at the RPC boundary.
    pub fn parse_str(raw: &str) -> Self {
        match raw {
            "stop" | "end_turn" => Self::Stop,
            "length" | "max_tokens" => Self::Length,
            "toolUse" | "tool_use" | "tool_calls" => Self::ToolUse,
            "aborted" | "cancelled" | "canceled" => Self::Aborted,
            "error" => Self::Error,
            _ => Self::Unknown,
        }
    }
}

/// Lifecycle phase of the active chat turn in the desktop UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiTurnPhase {
    Idle,
    AwaitingFirstEvent,
    StreamingThinking,
    StreamingText,
    Retrying,
    Failed,
    Disconnected,
}

/// Complete, allocation-free UI state for a single session's chat turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiTurnState {
    pub phase: UiTurnPhase,
    pub session_epoch: u8,
    pub has_assistant_text: bool,
    pub has_assistant_thinking: bool,
    pub has_visible_error: bool,
    pub assistant_message_count: u8,
    pub running_tools: u8,
    pub committed_tools: u8,
}

impl UiTurnState {
    /// Constructs a clean `Idle` state at `session_epoch`.
    pub const fn idle(session_epoch: u8) -> Self {
        Self {
            phase: UiTurnPhase::Idle,
            session_epoch,
            has_assistant_text: false,
            has_assistant_thinking: false,
            has_visible_error: false,
            assistant_message_count: 0,
            running_tools: 0,
            committed_tools: 0,
        }
    }

    /// Single source of truth for the UI thinking indicator.
    ///
    /// Active only while `phase` is `AwaitingFirstEvent` or `StreamingThinking`,
    /// and never active once assistant text has arrived or when `Idle`, `Failed`,
    /// or `Disconnected`.
    pub const fn thinking_indicator_active(&self) -> bool {
        !self.has_assistant_text
            && matches!(
                self.phase,
                UiTurnPhase::AwaitingFirstEvent | UiTurnPhase::StreamingThinking
            )
    }

    /// Single source of truth for whether the UI is actively streaming a turn.
    pub const fn is_streaming(&self) -> bool {
        matches!(
            self.phase,
            UiTurnPhase::AwaitingFirstEvent
                | UiTurnPhase::StreamingThinking
                | UiTurnPhase::StreamingText
                | UiTurnPhase::Retrying
        )
    }
}

impl Default for UiTurnState {
    fn default() -> Self {
        Self::idle(0)
    }
}

/// Events driving the chat-turn UI state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiTurnEvent {
    PromptSubmitted,
    MessageStart {
        role: WireRole,
        event_epoch: u8,
    },
    ThinkingDelta {
        event_epoch: u8,
    },
    TextDelta {
        event_epoch: u8,
    },
    ToolStart {
        event_epoch: u8,
    },
    ToolEnd {
        event_epoch: u8,
    },
    MessageEnd {
        role: WireRole,
        stop_reason: WireStopReason,
        has_error_message: bool,
        event_epoch: u8,
    },
    AutoRetryStart {
        event_epoch: u8,
    },
    AutoRetryEnd {
        success: bool,
        event_epoch: u8,
    },
    ErrorEvent {
        event_epoch: u8,
    },
    ProcessExited,
    AgentSettled {
        event_epoch: u8,
    },
    UserAbort,
    SessionSwitched,
}

impl UiTurnEvent {
    /// Returns the session epoch carried by wire events, or `None` for local UI
    /// and process-lifecycle events.
    pub const fn event_epoch(&self) -> Option<u8> {
        match *self {
            Self::MessageStart { event_epoch, .. }
            | Self::ThinkingDelta { event_epoch }
            | Self::TextDelta { event_epoch }
            | Self::ToolStart { event_epoch }
            | Self::ToolEnd { event_epoch }
            | Self::MessageEnd { event_epoch, .. }
            | Self::AutoRetryStart { event_epoch }
            | Self::AutoRetryEnd { event_epoch, .. }
            | Self::ErrorEvent { event_epoch }
            | Self::AgentSettled { event_epoch } => Some(event_epoch),
            Self::PromptSubmitted
            | Self::ProcessExited
            | Self::UserAbort
            | Self::SessionSwitched => None,
        }
    }
}

const fn resume_streaming_phase(state: UiTurnState) -> UiTurnPhase {
    if state.has_assistant_text {
        UiTurnPhase::StreamingText
    } else if state.has_assistant_thinking {
        UiTurnPhase::StreamingThinking
    } else {
        UiTurnPhase::AwaitingFirstEvent
    }
}

const fn settled_non_streaming_phase(state: UiTurnState) -> UiTurnPhase {
    match state.phase {
        UiTurnPhase::Disconnected => UiTurnPhase::Disconnected,
        UiTurnPhase::Failed => UiTurnPhase::Failed,
        _ if state.has_visible_error => UiTurnPhase::Failed,
        _ => UiTurnPhase::Idle,
    }
}

/// Pure, total, allocation-free state transition function for the chat UI turn.
///
/// Enforces:
/// - Session-epoch fencing: any wire event whose `event_epoch` differs from
///   `state.session_epoch` is dropped without mutating `state`.
/// - Universal idempotent recovery on `UserAbort`, `ProcessExited`,
///   `AgentSettled`, `ErrorEvent`, and `MessageEnd(Assistant, Error)`.
/// - Monotonic thinking-to-text handoff: `TextDelta` turns off the thinking
///   indicator and prevents returning to `StreamingThinking` in the same turn.
/// - User-echo isolation: `MessageStart(User)` and `MessageEnd(User, _)` never
///   mutate assistant fields or counts.
/// - Zero silent error erasure: once `has_visible_error` is set during a turn,
///   trailing `AgentSettled` or `UserAbort` events preserve it until a new turn
///   (`PromptSubmitted`) or `SessionSwitched`.
/// - Zero stuck tool spinners: transitioning to any non-streaming phase clears
///   `running_tools` to `0`, and `ToolStart` is ignored when not streaming.
pub fn step_ui_turn(state: UiTurnState, event: UiTurnEvent) -> UiTurnState {
    if let Some(event_epoch) = event.event_epoch() {
        if event_epoch != state.session_epoch {
            return state;
        }
    }

    match event {
        UiTurnEvent::PromptSubmitted => {
            if state.is_streaming() {
                state
            } else {
                UiTurnState {
                    phase: UiTurnPhase::AwaitingFirstEvent,
                    session_epoch: state.session_epoch,
                    has_assistant_text: false,
                    has_assistant_thinking: false,
                    has_visible_error: false,
                    assistant_message_count: 0,
                    running_tools: 0,
                    committed_tools: 0,
                }
            }
        }

        UiTurnEvent::SessionSwitched => UiTurnState::idle(state.session_epoch.wrapping_add(1)),

        UiTurnEvent::MessageStart { role, .. } => match role {
            WireRole::Assistant if state.is_streaming() => UiTurnState {
                phase: resume_streaming_phase(state),
                ..state
            },
            WireRole::Assistant | WireRole::User | WireRole::ToolResult | WireRole::Other => state,
        },

        UiTurnEvent::ThinkingDelta { .. } => {
            if !state.is_streaming() {
                state
            } else {
                let phase = if state.has_assistant_text {
                    UiTurnPhase::StreamingText
                } else {
                    UiTurnPhase::StreamingThinking
                };
                UiTurnState {
                    phase,
                    has_assistant_thinking: true,
                    ..state
                }
            }
        }

        UiTurnEvent::TextDelta { .. } => {
            if !state.is_streaming() {
                state
            } else {
                UiTurnState {
                    phase: UiTurnPhase::StreamingText,
                    has_assistant_text: true,
                    ..state
                }
            }
        }

        UiTurnEvent::ToolStart { .. } => {
            if !state.is_streaming() {
                state
            } else {
                UiTurnState {
                    running_tools: state.running_tools.saturating_add(1),
                    ..state
                }
            }
        }

        UiTurnEvent::ToolEnd { .. } => {
            if !state.is_streaming() || state.running_tools == 0 {
                state
            } else {
                UiTurnState {
                    running_tools: state.running_tools - 1,
                    committed_tools: state.committed_tools.saturating_add(1),
                    ..state
                }
            }
        }

        UiTurnEvent::MessageEnd {
            role,
            stop_reason,
            has_error_message,
            ..
        } => match role {
            WireRole::User | WireRole::ToolResult | WireRole::Other => state,
            WireRole::Assistant => {
                if stop_reason == WireStopReason::Error || has_error_message {
                    let phase = if state.phase == UiTurnPhase::Disconnected {
                        UiTurnPhase::Disconnected
                    } else {
                        UiTurnPhase::Failed
                    };
                    UiTurnState {
                        phase,
                        has_visible_error: true,
                        assistant_message_count: state.assistant_message_count.saturating_add(1),
                        running_tools: 0,
                        ..state
                    }
                } else if stop_reason == WireStopReason::Aborted {
                    UiTurnState {
                        phase: settled_non_streaming_phase(state),
                        assistant_message_count: state.assistant_message_count.saturating_add(1),
                        running_tools: 0,
                        ..state
                    }
                } else if !state.is_streaming() {
                    state
                } else {
                    UiTurnState {
                        phase: resume_streaming_phase(state),
                        assistant_message_count: state.assistant_message_count.saturating_add(1),
                        ..state
                    }
                }
            }
        },

        UiTurnEvent::AutoRetryStart { .. } => {
            if !state.is_streaming() {
                state
            } else {
                UiTurnState {
                    phase: UiTurnPhase::Retrying,
                    running_tools: 0,
                    ..state
                }
            }
        }

        UiTurnEvent::AutoRetryEnd { success, .. } => {
            if !state.is_streaming() {
                state
            } else if success {
                UiTurnState {
                    phase: resume_streaming_phase(state),
                    ..state
                }
            } else {
                UiTurnState {
                    phase: UiTurnPhase::Failed,
                    has_visible_error: true,
                    running_tools: 0,
                    ..state
                }
            }
        }

        UiTurnEvent::ErrorEvent { .. } => {
            let phase = if state.phase == UiTurnPhase::Disconnected {
                UiTurnPhase::Disconnected
            } else {
                UiTurnPhase::Failed
            };
            UiTurnState {
                phase,
                has_visible_error: true,
                running_tools: 0,
                ..state
            }
        }

        UiTurnEvent::ProcessExited => UiTurnState {
            phase: UiTurnPhase::Disconnected,
            has_visible_error: true,
            running_tools: 0,
            ..state
        },

        UiTurnEvent::AgentSettled { .. } => UiTurnState {
            phase: settled_non_streaming_phase(state),
            running_tools: 0,
            ..state
        },

        UiTurnEvent::UserAbort => UiTurnState {
            phase: settled_non_streaming_phase(state),
            running_tools: 0,
            ..state
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_openrouter_thinking_to_text_to_settled_flow() {
        let s0 = UiTurnState::idle(0);
        assert!(!s0.thinking_indicator_active());
        assert!(!s0.is_streaming());

        let s1 = step_ui_turn(s0, UiTurnEvent::PromptSubmitted);
        assert_eq!(s1.phase, UiTurnPhase::AwaitingFirstEvent);
        assert!(s1.thinking_indicator_active());
        assert!(s1.is_streaming());

        // User echo must not touch assistant state.
        let s1_echo = step_ui_turn(
            s1,
            UiTurnEvent::MessageStart {
                role: WireRole::User,
                event_epoch: 0,
            },
        );
        assert_eq!(s1_echo, s1);

        let s2 = step_ui_turn(s1, UiTurnEvent::ThinkingDelta { event_epoch: 0 });
        assert_eq!(s2.phase, UiTurnPhase::StreamingThinking);
        assert!(s2.has_assistant_thinking);
        assert!(s2.thinking_indicator_active());

        let s3 = step_ui_turn(s2, UiTurnEvent::TextDelta { event_epoch: 0 });
        assert_eq!(s3.phase, UiTurnPhase::StreamingText);
        assert!(s3.has_assistant_text);
        assert!(!s3.thinking_indicator_active());
        assert!(s3.is_streaming());

        let s4 = step_ui_turn(
            s3,
            UiTurnEvent::MessageEnd {
                role: WireRole::Assistant,
                stop_reason: WireStopReason::Stop,
                has_error_message: false,
                event_epoch: 0,
            },
        );
        assert_eq!(s4.assistant_message_count, 1);

        let s5 = step_ui_turn(s4, UiTurnEvent::AgentSettled { event_epoch: 0 });
        assert_eq!(s5.phase, UiTurnPhase::Idle);
        assert!(!s5.is_streaming());
        assert!(!s5.thinking_indicator_active());
        assert!(!s5.has_visible_error);
        assert_eq!(s5.running_tools, 0);
    }

    #[test]
    fn error_is_preserved_across_trailing_agent_settled() {
        let s1 = step_ui_turn(UiTurnState::idle(2), UiTurnEvent::PromptSubmitted);
        let s2 = step_ui_turn(s1, UiTurnEvent::ErrorEvent { event_epoch: 2 });
        assert_eq!(s2.phase, UiTurnPhase::Failed);
        assert!(s2.has_visible_error);
        assert!(!s2.is_streaming());

        let s3 = step_ui_turn(s2, UiTurnEvent::AgentSettled { event_epoch: 2 });
        assert_eq!(s3.phase, UiTurnPhase::Failed);
        assert!(s3.has_visible_error);
        assert!(!s3.is_streaming());
    }

    #[test]
    fn session_switch_fences_stale_epoch_events() {
        let s1 = step_ui_turn(UiTurnState::idle(0), UiTurnEvent::PromptSubmitted);
        let s2 = step_ui_turn(s1, UiTurnEvent::SessionSwitched);
        assert_eq!(s2, UiTurnState::idle(1));

        let stale = step_ui_turn(s2, UiTurnEvent::ErrorEvent { event_epoch: 0 });
        assert_eq!(stale, s2);
    }

    #[test]
    fn boundary_parsers_handle_known_and_unknown_wire_strings() {
        assert_eq!(WireRole::parse_str("user"), WireRole::User);
        assert_eq!(WireRole::parse_str("assistant"), WireRole::Assistant);
        assert_eq!(WireRole::parse_str("toolResult"), WireRole::ToolResult);
        assert_eq!(WireRole::parse_str("system"), WireRole::Other);

        assert_eq!(WireStopReason::parse_str("stop"), WireStopReason::Stop);
        assert_eq!(WireStopReason::parse_str("tool_use"), WireStopReason::ToolUse);
        assert_eq!(WireStopReason::parse_str("error"), WireStopReason::Error);
        assert_eq!(WireStopReason::parse_str("aborted"), WireStopReason::Aborted);
        assert_eq!(WireStopReason::parse_str("other"), WireStopReason::Unknown);
    }
}
