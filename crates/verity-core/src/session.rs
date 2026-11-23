use std::fmt;

/// Maximum number of queued steer/follow-up turns while streaming.
pub const MAX_PENDING_QUEUE: u16 = u16::MAX;

/// Pure Pi desktop session activity state machine.
/// Illegal combinations (such as a non-zero pending queue or deferred reload
/// while idle, compacting, or switching sessions) are unrepresentable by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionActivity {
    Idle,
    Streaming {
        pending_queue: u16,
        reload_deferred: bool,
    },
    Compacting,
    SwitchingSession,
}

impl SessionActivity {
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn is_streaming(&self) -> bool {
        matches!(self, Self::Streaming { .. })
    }

    pub fn pending_queue(&self) -> u16 {
        match *self {
            Self::Streaming { pending_queue, .. } => pending_queue,
            Self::Idle | Self::Compacting | Self::SwitchingSession => 0,
        }
    }

    pub fn reload_deferred(&self) -> bool {
        match *self {
            Self::Streaming {
                reload_deferred, ..
            } => reload_deferred,
            Self::Idle | Self::Compacting | Self::SwitchingSession => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCommand {
    SendPrompt,
    Steer,
    FollowUp,
    ClearQueue,
    Compact,
    SwitchSession,
    RequestProviderReload,
    AgentSettled,
    FinishCompacting,
    FinishSwitch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    NotIdle,
    NotStreaming,
    NotCompacting,
    NotSwitching,
    QueueFull,
    Busy,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::NotIdle => write!(f, "command requires an idle session"),
            SessionError::NotStreaming => write!(f, "command requires an active streaming turn"),
            SessionError::NotCompacting => write!(f, "session is not currently compacting"),
            SessionError::NotSwitching => write!(f, "session is not currently switching"),
            SessionError::QueueFull => write!(f, "pending steer/follow-up queue is full"),
            SessionError::Busy => write!(f, "session is busy and cannot reload providers now"),
        }
    }
}

impl std::error::Error for SessionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionTransition {
    pub activity: SessionActivity,
    pub run_provider_reload: bool,
}

/// Applies a single `SessionCommand` to `activity`, enforcing session exclusivity,
/// bounded queue increments, and deferred provider reloads during streaming.
pub fn transition_session(
    activity: SessionActivity,
    command: SessionCommand,
) -> Result<SessionTransition, SessionError> {
    match (activity, command) {
        (SessionActivity::Idle, SessionCommand::SendPrompt) => Ok(SessionTransition {
            activity: SessionActivity::Streaming {
                pending_queue: 0,
                reload_deferred: false,
            },
            run_provider_reload: false,
        }),
        (_, SessionCommand::SendPrompt) => Err(SessionError::NotIdle),

        (SessionActivity::Idle, SessionCommand::Compact) => Ok(SessionTransition {
            activity: SessionActivity::Compacting,
            run_provider_reload: false,
        }),
        (_, SessionCommand::Compact) => Err(SessionError::NotIdle),

        (SessionActivity::Idle, SessionCommand::SwitchSession) => Ok(SessionTransition {
            activity: SessionActivity::SwitchingSession,
            run_provider_reload: false,
        }),
        (_, SessionCommand::SwitchSession) => Err(SessionError::NotIdle),

        (
            SessionActivity::Streaming {
                pending_queue,
                reload_deferred,
            },
            SessionCommand::Steer | SessionCommand::FollowUp,
        ) => {
            let Some(next_queue) = pending_queue.checked_add(1) else {
                return Err(SessionError::QueueFull);
            };
            Ok(SessionTransition {
                activity: SessionActivity::Streaming {
                    pending_queue: next_queue,
                    reload_deferred,
                },
                run_provider_reload: false,
            })
        }
        (_, SessionCommand::Steer | SessionCommand::FollowUp) => Err(SessionError::NotStreaming),

        (
            SessionActivity::Streaming {
                reload_deferred, ..
            },
            SessionCommand::ClearQueue,
        ) => Ok(SessionTransition {
            activity: SessionActivity::Streaming {
                pending_queue: 0,
                reload_deferred,
            },
            run_provider_reload: false,
        }),
        (_, SessionCommand::ClearQueue) => Err(SessionError::NotStreaming),

        (
            SessionActivity::Streaming { pending_queue, .. },
            SessionCommand::RequestProviderReload,
        ) => Ok(SessionTransition {
            activity: SessionActivity::Streaming {
                pending_queue,
                reload_deferred: true,
            },
            run_provider_reload: false,
        }),
        (SessionActivity::Idle, SessionCommand::RequestProviderReload) => Ok(SessionTransition {
            activity: SessionActivity::Idle,
            run_provider_reload: true,
        }),
        (
            SessionActivity::Compacting | SessionActivity::SwitchingSession,
            SessionCommand::RequestProviderReload,
        ) => Err(SessionError::Busy),

        (
            SessionActivity::Streaming {
                reload_deferred, ..
            },
            SessionCommand::AgentSettled,
        ) => Ok(SessionTransition {
            activity: SessionActivity::Idle,
            run_provider_reload: reload_deferred,
        }),
        (_, SessionCommand::AgentSettled) => Err(SessionError::NotStreaming),

        (SessionActivity::Compacting, SessionCommand::FinishCompacting) => Ok(SessionTransition {
            activity: SessionActivity::Idle,
            run_provider_reload: false,
        }),
        (_, SessionCommand::FinishCompacting) => Err(SessionError::NotCompacting),

        (SessionActivity::SwitchingSession, SessionCommand::FinishSwitch) => {
            Ok(SessionTransition {
                activity: SessionActivity::Idle,
                run_provider_reload: false,
            })
        }
        (_, SessionCommand::FinishSwitch) => Err(SessionError::NotSwitching),
    }
}
