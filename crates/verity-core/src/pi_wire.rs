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

/// Assistant message update event kind parsed at the Pi RPC wire boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantEventKind {
    TextDelta,
    TextEnd,
    ThinkingStart,
    ThinkingDelta,
    ThinkingEnd,
    ToolCall,
    Ignored,
}

impl AssistantEventKind {
    /// Parses a wire `assistantMessageEvent.type` string using an explicit allowlist.
    ///
    /// Tool-call and unknown strings NEVER fall through to `TextDelta` or `ThinkingDelta`.
    pub fn parse_str(raw: &str) -> Self {
        match raw {
            "text_delta" => Self::TextDelta,
            "text_end" => Self::TextEnd,
            "thinking_start" => Self::ThinkingStart,
            "thinking_delta" | "reasoning_delta" => Self::ThinkingDelta,
            "thinking_end" => Self::ThinkingEnd,
            "toolcall_start" | "toolcall_delta" | "toolcall_end" | "input_json_delta" => {
                Self::ToolCall
            }
            _ => Self::Ignored,
        }
    }
}

/// Distinguishes server-issued Pi session entry IDs from client-local UI message IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryIdKind {
    Server(u8),
    ClientLocal(u8),
}

/// Validated rewind target returned by `resolve_rewind_target`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindTarget {
    ForkToServer(u8),
    NewSession,
}

/// Error returned when a requested rewind entry ID cannot be safely resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertResolutionError {
    UnresolvableEntryId,
}

/// Resolves a requested revert entry ID against the authoritative list of server entry IDs.
///
/// Guarantees:
/// - If `requested == EntryIdKind::Server(id)` and `server_entries.contains(&id)`,
///   returns `Ok(RewindTarget::ForkToServer(id))`.
/// - Else if `user_turn_idx == Some(idx)` and `idx < server_entries.len()`,
///   returns `Ok(RewindTarget::ForkToServer(server_entries[idx]))`.
/// - Else if `user_turn_idx == Some(0)` and `server_entries.is_empty()`,
///   returns `Ok(RewindTarget::NewSession)`.
/// - Else returns `Err(RevertResolutionError::UnresolvableEntryId)`, NEVER returning
///   a `ClientLocal` ID.
pub fn resolve_rewind_target(
    requested: EntryIdKind,
    user_turn_idx: Option<usize>,
    server_entries: &[u8],
) -> Result<RewindTarget, RevertResolutionError> {
    if let EntryIdKind::Server(id) = requested {
        if server_entries.contains(&id) {
            return Ok(RewindTarget::ForkToServer(id));
        }
    }
    if let Some(idx) = user_turn_idx {
        if idx < server_entries.len() {
            return Ok(RewindTarget::ForkToServer(server_entries[idx]));
        }
        if idx == 0 && server_entries.is_empty() {
            return Ok(RewindTarget::NewSession);
        }
    }
    Err(RevertResolutionError::UnresolvableEntryId)
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
    pub thinking_duration_sealed: bool,
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
            thinking_duration_sealed: false,
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
    ThinkingEnd {
        event_epoch: u8,
    },
    TextDelta {
        event_epoch: u8,
    },
    ToolCallDelta {
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
            | Self::ThinkingEnd { event_epoch }
            | Self::TextDelta { event_epoch }
            | Self::ToolCallDelta { event_epoch }
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
/// - Thinking duration sealing: `ThinkingEnd`, first `TextDelta`, or terminal
///   settlement while `has_assistant_thinking` sets `thinking_duration_sealed = true`.
/// - Tool-call delta isolation: `ToolCallDelta` leaves `has_assistant_text`,
///   `has_assistant_thinking`, and `thinking_duration_sealed` strictly unchanged.
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
                    thinking_duration_sealed: false,
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
                    thinking_duration_sealed: state.thinking_duration_sealed
                        || state.has_assistant_text,
                    ..state
                }
            }
        }

        UiTurnEvent::ThinkingEnd { .. } => {
            if !state.is_streaming() {
                state
            } else {
                UiTurnState {
                    thinking_duration_sealed: state.thinking_duration_sealed
                        || state.has_assistant_thinking,
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
                    thinking_duration_sealed: state.thinking_duration_sealed
                        || state.has_assistant_thinking,
                    ..state
                }
            }
        }

        UiTurnEvent::ToolCallDelta { .. } => state,

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
                        thinking_duration_sealed: state.thinking_duration_sealed
                            || state.has_assistant_thinking,
                        has_visible_error: true,
                        assistant_message_count: state.assistant_message_count.saturating_add(1),
                        running_tools: 0,
                        ..state
                    }
                } else if stop_reason == WireStopReason::Aborted {
                    UiTurnState {
                        phase: settled_non_streaming_phase(state),
                        thinking_duration_sealed: state.thinking_duration_sealed
                            || state.has_assistant_thinking,
                        assistant_message_count: state.assistant_message_count.saturating_add(1),
                        running_tools: 0,
                        ..state
                    }
                } else if !state.is_streaming() {
                    state
                } else {
                    UiTurnState {
                        phase: resume_streaming_phase(state),
                        thinking_duration_sealed: state.thinking_duration_sealed
                            || state.has_assistant_thinking,
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
                    thinking_duration_sealed: state.thinking_duration_sealed
                        || state.has_assistant_thinking,
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
                thinking_duration_sealed: state.thinking_duration_sealed
                    || state.has_assistant_thinking,
                has_visible_error: true,
                running_tools: 0,
                ..state
            }
        }

        UiTurnEvent::ProcessExited => UiTurnState {
            phase: UiTurnPhase::Disconnected,
            thinking_duration_sealed: state.thinking_duration_sealed
                || state.has_assistant_thinking,
            has_visible_error: true,
            running_tools: 0,
            ..state
        },

        UiTurnEvent::AgentSettled { .. } => UiTurnState {
            phase: settled_non_streaming_phase(state),
            thinking_duration_sealed: state.thinking_duration_sealed
                || state.has_assistant_thinking,
            running_tools: 0,
            ..state
        },

        UiTurnEvent::UserAbort => UiTurnState {
            phase: settled_non_streaming_phase(state),
            thinking_duration_sealed: state.thinking_duration_sealed
                || state.has_assistant_thinking,
            running_tools: 0,
            ..state
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_parsers_handle_known_and_unknown_wire_strings() {
        assert_eq!(WireRole::parse_str("user"), WireRole::User);
        assert_eq!(WireRole::parse_str("assistant"), WireRole::Assistant);
        assert_eq!(WireRole::parse_str("toolResult"), WireRole::ToolResult);
        assert_eq!(WireRole::parse_str("system"), WireRole::Other);

        assert_eq!(WireStopReason::parse_str("stop"), WireStopReason::Stop);
        assert_eq!(
            WireStopReason::parse_str("tool_use"),
            WireStopReason::ToolUse
        );
        assert_eq!(WireStopReason::parse_str("error"), WireStopReason::Error);
        assert_eq!(
            WireStopReason::parse_str("aborted"),
            WireStopReason::Aborted
        );
        assert_eq!(WireStopReason::parse_str("other"), WireStopReason::Unknown);
    }

    #[test]
    fn rewind_target_resolution_never_leaks_client_local_ids() {
        assert_eq!(
            resolve_rewind_target(EntryIdKind::Server(10), Some(0), &[10, 20]),
            Ok(RewindTarget::ForkToServer(10))
        );
        assert_eq!(
            resolve_rewind_target(EntryIdKind::ClientLocal(99), Some(1), &[10, 20]),
            Ok(RewindTarget::ForkToServer(20))
        );
        assert_eq!(
            resolve_rewind_target(EntryIdKind::ClientLocal(99), Some(0), &[]),
            Ok(RewindTarget::NewSession)
        );
        assert_eq!(
            resolve_rewind_target(EntryIdKind::ClientLocal(99), Some(2), &[10, 20]),
            Err(RevertResolutionError::UnresolvableEntryId)
        );
        assert_eq!(
            resolve_rewind_target(EntryIdKind::Server(77), None, &[10, 20]),
            Err(RevertResolutionError::UnresolvableEntryId)
        );
    }
}
