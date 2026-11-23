use crate::scope::{check_scope, ScopeError};
use crate::types::{CommitOid, RelPath, SpecDigest, TaskMode};
use crate::verdict::{
    can_approve_gate, can_approve_gate_core, CheckerVerdict, GateError, HygieneReport,
    VerdictStatus,
};
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
        self.to_core().is_terminal()
    }

    pub fn to_core(&self) -> TaskStateCore<SpecDigest> {
        match self {
            TaskState::Queued => TaskStateCore::Queued,
            TaskState::Running { attempt } => TaskStateCore::Running { attempt: *attempt },
            TaskState::AwaitingSpecReview { handoff } => TaskStateCore::AwaitingSpecReview {
                spec_digest: handoff.spec_digest,
                hygiene_violations: handoff.hygiene.violations.len(),
                verdict_status: handoff.verdict.status,
            },
            TaskState::ApprovedForMerge {
                approved_digest, ..
            } => TaskStateCore::ApprovedForMerge {
                approved_digest: *approved_digest,
            },
            TaskState::Merged { .. } => TaskStateCore::Merged,
            TaskState::CompletedReadOnly { .. } => TaskStateCore::CompletedReadOnly,
            TaskState::Rejected { .. } => TaskStateCore::Rejected,
        }
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

/// Allocation-free mode tag for core state-machine verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskModeKind {
    ReadOnly,
    Write,
}

impl TaskModeKind {
    pub fn from_mode(mode: &TaskMode) -> Self {
        if mode.is_read_only() {
            Self::ReadOnly
        } else {
            Self::Write
        }
    }
}

/// Allocation-free core task state verified directly by Kani.
/// Generic over the digest token `D` (instantiated with `SpecDigest` at runtime
/// and compact `Copy + Eq` tokens in bounded model checking).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStateCore<D = SpecDigest> {
    Queued,
    Running {
        attempt: u8,
    },
    AwaitingSpecReview {
        spec_digest: D,
        hygiene_violations: usize,
        verdict_status: VerdictStatus,
    },
    ApprovedForMerge {
        approved_digest: D,
    },
    Merged,
    CompletedReadOnly,
    Rejected,
}

impl<D> TaskStateCore<D> {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStateCore::Merged | TaskStateCore::CompletedReadOnly | TaskStateCore::Rejected
        )
    }
}

/// Allocation-free core task event verified directly by Kani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEventCore<D = SpecDigest> {
    StartAttempt,
    FinishReadOnly,
    SubmitHandoff {
        scope_ok: bool,
        hygiene_violations: usize,
        verdict_status: VerdictStatus,
        spec_digest: D,
    },
    ApproveSpec {
        approved_digest: D,
    },
    CompleteMerge {
        verified_digest: D,
    },
    Reject,
}

/// Allocation-free core transition error verified directly by Kani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionErrorCore {
    AlreadyTerminal,
    InvalidStateForEvent,
    ModeMismatch,
    AttemptBudgetExhausted,
    ScopeViolation,
    GateFailed(GateError),
    DigestMismatch,
}

pub fn can_approve(handoff: &SpecHandoff) -> Result<(), GateError> {
    can_approve_gate(&handoff.hygiene, &handoff.verdict)
}

/// Allocation-free core state transition function verified directly by Kani.
pub fn transition_core<D: Copy + Eq>(
    mode: TaskModeKind,
    state: TaskStateCore<D>,
    event: TaskEventCore<D>,
    max_attempts: u8,
) -> Result<TaskStateCore<D>, TransitionErrorCore> {
    if state.is_terminal() {
        return Err(TransitionErrorCore::AlreadyTerminal);
    }

    match (state, event) {
        (TaskStateCore::Queued, TaskEventCore::StartAttempt) => {
            if max_attempts == 0 {
                return Err(TransitionErrorCore::AttemptBudgetExhausted);
            }
            Ok(TaskStateCore::Running { attempt: 1 })
        }
        (TaskStateCore::Running { attempt }, TaskEventCore::StartAttempt) => {
            if attempt >= max_attempts {
                return Err(TransitionErrorCore::AttemptBudgetExhausted);
            }
            Ok(TaskStateCore::Running {
                attempt: attempt + 1,
            })
        }
        (TaskStateCore::Running { .. }, TaskEventCore::FinishReadOnly) => {
            if mode != TaskModeKind::ReadOnly {
                return Err(TransitionErrorCore::ModeMismatch);
            }
            Ok(TaskStateCore::CompletedReadOnly)
        }
        (
            TaskStateCore::Running { .. },
            TaskEventCore::SubmitHandoff {
                scope_ok,
                hygiene_violations,
                verdict_status,
                spec_digest,
            },
        ) => {
            if mode != TaskModeKind::Write {
                return Err(TransitionErrorCore::ModeMismatch);
            }
            if !scope_ok {
                return Err(TransitionErrorCore::ScopeViolation);
            }
            can_approve_gate_core(hygiene_violations, verdict_status)
                .map_err(TransitionErrorCore::GateFailed)?;
            Ok(TaskStateCore::AwaitingSpecReview {
                spec_digest,
                hygiene_violations,
                verdict_status,
            })
        }
        (
            TaskStateCore::AwaitingSpecReview {
                spec_digest,
                hygiene_violations,
                verdict_status,
            },
            TaskEventCore::ApproveSpec { approved_digest },
        ) => {
            if mode != TaskModeKind::Write {
                return Err(TransitionErrorCore::ModeMismatch);
            }
            if spec_digest != approved_digest {
                return Err(TransitionErrorCore::DigestMismatch);
            }
            can_approve_gate_core(hygiene_violations, verdict_status)
                .map_err(TransitionErrorCore::GateFailed)?;
            Ok(TaskStateCore::ApprovedForMerge { approved_digest })
        }
        (
            TaskStateCore::ApprovedForMerge { approved_digest },
            TaskEventCore::CompleteMerge { verified_digest },
        ) => {
            if mode != TaskModeKind::Write {
                return Err(TransitionErrorCore::ModeMismatch);
            }
            if approved_digest != verified_digest {
                return Err(TransitionErrorCore::DigestMismatch);
            }
            Ok(TaskStateCore::Merged)
        }
        (_, TaskEventCore::Reject) => Ok(TaskStateCore::Rejected),
        _ => Err(TransitionErrorCore::InvalidStateForEvent),
    }
}

pub fn transition(
    mode: &TaskMode,
    state: TaskState,
    event: TaskEvent,
    max_attempts: u8,
) -> Result<TaskState, TransitionError> {
    let mode_kind = TaskModeKind::from_mode(mode);
    let state_core = state.to_core();

    let (event_core, scope_err) = match &event {
        TaskEvent::StartAttempt => (TaskEventCore::StartAttempt, None),
        TaskEvent::FinishReadOnly { .. } => (TaskEventCore::FinishReadOnly, None),
        TaskEvent::SubmitHandoff { handoff } => {
            let scope_res = match mode {
                TaskMode::Write {
                    write_scope,
                    forbidden,
                    ..
                } => check_scope(&handoff.changed_paths, write_scope, forbidden),
                TaskMode::ReadOnly { .. } => Ok(()),
            };
            let scope_ok = scope_res.is_ok();
            (
                TaskEventCore::SubmitHandoff {
                    scope_ok,
                    hygiene_violations: handoff.hygiene.violations.len(),
                    verdict_status: handoff.verdict.status,
                    spec_digest: handoff.spec_digest,
                },
                scope_res.err(),
            )
        }
        TaskEvent::ApproveSpec { approved_digest } => (
            TaskEventCore::ApproveSpec {
                approved_digest: *approved_digest,
            },
            None,
        ),
        TaskEvent::CompleteMerge {
            verified_digest, ..
        } => (
            TaskEventCore::CompleteMerge {
                verified_digest: *verified_digest,
            },
            None,
        ),
        TaskEvent::Reject { .. } => (TaskEventCore::Reject, None),
    };

    let next_core = transition_core(mode_kind, state_core, event_core, max_attempts).map_err(
        |err| match err {
            TransitionErrorCore::AlreadyTerminal => TransitionError::AlreadyTerminal,
            TransitionErrorCore::InvalidStateForEvent => TransitionError::InvalidStateForEvent,
            TransitionErrorCore::ModeMismatch => TransitionError::ModeMismatch,
            TransitionErrorCore::AttemptBudgetExhausted => TransitionError::AttemptBudgetExhausted,
            TransitionErrorCore::ScopeViolation => {
                TransitionError::ScopeViolation(scope_err.unwrap_or(ScopeError::EmptyDiff))
            }
            TransitionErrorCore::GateFailed(g) => TransitionError::GateFailed(g),
            TransitionErrorCore::DigestMismatch => TransitionError::DigestMismatch,
        },
    )?;

    match (next_core, state, event) {
        (TaskStateCore::Running { attempt }, _, _) => Ok(TaskState::Running { attempt }),
        (TaskStateCore::CompletedReadOnly, _, TaskEvent::FinishReadOnly { summary }) => {
            Ok(TaskState::CompletedReadOnly { summary })
        }
        (TaskStateCore::AwaitingSpecReview { .. }, _, TaskEvent::SubmitHandoff { handoff }) => {
            Ok(TaskState::AwaitingSpecReview { handoff })
        }
        (
            TaskStateCore::ApprovedForMerge { approved_digest },
            TaskState::AwaitingSpecReview { handoff },
            TaskEvent::ApproveSpec { .. },
        ) => Ok(TaskState::ApprovedForMerge {
            handoff,
            approved_digest,
        }),
        (
            TaskStateCore::Merged,
            _,
            TaskEvent::CompleteMerge { merged_commit, .. },
        ) => Ok(TaskState::Merged {
            commit: merged_commit,
        }),
        (TaskStateCore::Rejected, _, TaskEvent::Reject { reason }) => {
            Ok(TaskState::Rejected { reason })
        }
        _ => unreachable!("transition_core returned a state inconsistent with its input event"),
    }
}
