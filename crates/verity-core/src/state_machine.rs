use crate::scope::{check_scope, ScopeError};
use crate::types::{CommitOid, RelPath, SpecDigest, TaskMode};
use crate::verdict::{can_approve_gate, CheckerVerdict, GateError, HygieneReport};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecHandoff {
    pub candidate_commit: CommitOid,
    pub changed_paths: Vec<RelPath>,
    pub extracted_spec: String,
    pub back_translation: Option<String>,
    pub hygiene: HygieneReport,
    pub verdict: CheckerVerdict,
    pub spec_digest: SpecDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectionReason {
    ScopeViolation(ScopeError),
    GateFailed(GateError),
    ReviewerRejected(String),
    AttemptBudgetExhausted,
    MergeConflict(Vec<RelPath>),
    DigestMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Running {
        attempt: u8,
    },
    AwaitingSpecReview {
        handoff: SpecHandoff,
    },
    ApprovedForMerge {
        handoff: SpecHandoff,
        approved_digest: SpecDigest,
    },
    Merged {
        commit: CommitOid,
    },
    CompletedReadOnly {
        summary: String,
    },
    Rejected {
        reason: RejectionReason,
    },
}

impl TaskState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskState::Merged { .. }
                | TaskState::CompletedReadOnly { .. }
                | TaskState::Rejected { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEvent {
    StartAttempt,
    FinishReadOnly {
        summary: String,
    },
    SubmitHandoff {
        handoff: SpecHandoff,
    },
    ApproveSpec {
        approved_digest: SpecDigest,
    },
    CompleteMerge {
        merged_commit: CommitOid,
        verified_digest: SpecDigest,
    },
    Reject {
        reason: RejectionReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    AlreadyTerminal,
    InvalidStateForEvent,
    ModeMismatch,
    AttemptBudgetExhausted,
    ScopeViolation(ScopeError),
    GateFailed(GateError),
    DigestMismatch,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransitionError::AlreadyTerminal => write!(f, "task is already in a terminal state"),
            TransitionError::InvalidStateForEvent => write!(f, "event is invalid for current state"),
            TransitionError::ModeMismatch => write!(f, "event does not match task mode"),
            TransitionError::AttemptBudgetExhausted => write!(f, "attempt budget exhausted"),
            TransitionError::ScopeViolation(e) => write!(f, "scope violation: {}", e),
            TransitionError::GateFailed(e) => write!(f, "gate failed: {}", e),
            TransitionError::DigestMismatch => write!(f, "spec digest mismatch"),
        }
    }
}

impl std::error::Error for TransitionError {}

pub fn can_approve(handoff: &SpecHandoff) -> Result<(), GateError> {
    can_approve_gate(&handoff.hygiene, &handoff.verdict)
}

pub fn transition(
    mode: &TaskMode,
    state: TaskState,
    event: TaskEvent,
    max_attempts: u8,
) -> Result<TaskState, TransitionError> {
    if state.is_terminal() {
        return Err(TransitionError::AlreadyTerminal);
    }

    match (state, event) {
        (TaskState::Queued, TaskEvent::StartAttempt) => {
            if max_attempts == 0 {
                return Err(TransitionError::AttemptBudgetExhausted);
            }
            Ok(TaskState::Running { attempt: 1 })
        }
        (TaskState::Running { attempt }, TaskEvent::StartAttempt) => {
            if attempt >= max_attempts {
                return Err(TransitionError::AttemptBudgetExhausted);
            }
            Ok(TaskState::Running {
                attempt: attempt + 1,
            })
        }
        (TaskState::Running { .. }, TaskEvent::FinishReadOnly { summary }) => {
            if !mode.is_read_only() {
                return Err(TransitionError::ModeMismatch);
            }
            Ok(TaskState::CompletedReadOnly { summary })
        }
        (TaskState::Running { .. }, TaskEvent::SubmitHandoff { handoff }) => {
            let (write_scope, forbidden) = match mode {
                TaskMode::Write {
                    write_scope,
                    forbidden,
                    ..
                } => (write_scope, forbidden),
                TaskMode::ReadOnly { .. } => return Err(TransitionError::ModeMismatch),
            };
            check_scope(&handoff.changed_paths, write_scope, forbidden)
                .map_err(TransitionError::ScopeViolation)?;
            can_approve(&handoff).map_err(TransitionError::GateFailed)?;
            Ok(TaskState::AwaitingSpecReview { handoff })
        }
        (
            TaskState::AwaitingSpecReview { handoff },
            TaskEvent::ApproveSpec { approved_digest },
        ) => {
            if mode.is_read_only() {
                return Err(TransitionError::ModeMismatch);
            }
            if handoff.spec_digest != approved_digest {
                return Err(TransitionError::DigestMismatch);
            }
            can_approve(&handoff).map_err(TransitionError::GateFailed)?;
            Ok(TaskState::ApprovedForMerge {
                handoff,
                approved_digest,
            })
        }
        (
            TaskState::ApprovedForMerge {
                approved_digest, ..
            },
            TaskEvent::CompleteMerge {
                merged_commit,
                verified_digest,
            },
        ) => {
            if mode.is_read_only() {
                return Err(TransitionError::ModeMismatch);
            }
            if approved_digest != verified_digest {
                return Err(TransitionError::DigestMismatch);
            }
            Ok(TaskState::Merged {
                commit: merged_commit,
            })
        }
        (_, TaskEvent::Reject { reason }) => Ok(TaskState::Rejected { reason }),
        _ => Err(TransitionError::InvalidStateForEvent),
    }
}
