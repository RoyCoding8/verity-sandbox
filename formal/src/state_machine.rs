#[cfg(kani)]
use verity_core::{transition_core, TaskEventCore, TaskModeKind, TaskStateCore};
#[cfg(test)]
use verity_core::{
    transition, CheckerVerdict, CommitOid, HygieneReport, HygieneViolation, RelPath, SpecHandoff,
    TaskEvent, TaskMode, TaskState, TransitionError, VerifierKind,
};
#[cfg(test)]
use verity_core::SpecDigest;
use verity_core::VerdictStatus;

#[cfg(test)]
fn sample_oid(ch: char) -> CommitOid {
    let hex: String = std::iter::repeat_n(ch, 40).collect();
    CommitOid::parse(&hex).expect("valid hex oid")
}

#[cfg(test)]
fn sample_write_mode() -> TaskMode {
    TaskMode::write(
        sample_oid('a'),
        vec![RelPath::parse("src").unwrap()],
        vec![RelPath::parse("src/secret.rs").unwrap()],
        VerifierKind::RustKani,
    )
    .expect("non-empty write scope")
}

#[cfg(test)]
fn sample_handoff(hygiene_ok: bool, pass_verdict: bool, spec_byte: u8) -> SpecHandoff {
    let hygiene = if hygiene_ok {
        HygieneReport::default()
    } else {
        HygieneReport {
            violations: vec![HygieneViolation {
                rule: "no-assume".to_string(),
                path: "src/lib.rs".to_string(),
                line: 10,
                detail: "unexpected kani::assume".to_string(),
            }],
        }
    };
    let verdict = CheckerVerdict {
        status: if pass_verdict {
            VerdictStatus::Pass
        } else {
            VerdictStatus::Counterexample
        },
        reason: "checked".to_string(),
        checks_passed: 1,
        probes_satisfied: 1,
    };
    SpecHandoff {
        candidate_commit: sample_oid('b'),
        changed_paths: vec![RelPath::parse("src/lib.rs").unwrap()],
        extracted_spec: "ensures(true)".to_string(),
        back_translation: Some("Returns true.".to_string()),
        hygiene,
        verdict,
        spec_digest: SpecDigest::from_bytes([spec_byte; 32]),
    }
}

/// Law 1 & Law 2 & Law 5:
/// - No write task reaches Merged without ApprovedForMerge and matching digest.
/// - Read-only tasks never reach AwaitingSpecReview, ApprovedForMerge, or Merged.
/// - Terminal states reject all further events.
/// - Attempts never exceed max_attempts.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
pub fn prove_state_machine_safety() {
    let is_read_only: bool = kani::any();
    let max_attempts: u8 = kani::any();
    kani::assume(max_attempts <= 3);

    let mode = if is_read_only {
        TaskModeKind::ReadOnly
    } else {
        TaskModeKind::Write
    };

    let mut state: TaskStateCore<u8> = TaskStateCore::Queued;
    let mut approved_seen = false;
    let steps: u8 = kani::any();
    kani::assume(steps <= 4);

    for _ in 0..steps {
        let ev_kind: u8 = kani::any();
        kani::assume(ev_kind <= 5);
        let event = match ev_kind {
            0 => TaskEventCore::StartAttempt,
            1 => TaskEventCore::FinishReadOnly,
            2 => {
                let scope_ok: bool = kani::any();
                let hygiene_ok: bool = kani::any();
                let pass_verdict: bool = kani::any();
                TaskEventCore::SubmitHandoff {
                    scope_ok,
                    hygiene_violations: if hygiene_ok { 0 } else { 1 },
                    verdict_status: if pass_verdict {
                        VerdictStatus::Pass
                    } else {
                        VerdictStatus::Counterexample
                    },
                    spec_digest: 7u8,
                }
            }
            3 => {
                let digest_byte: u8 = kani::any();
                kani::assume(digest_byte == 7 || digest_byte == 9);
                TaskEventCore::ApproveSpec {
                    approved_digest: digest_byte,
                }
            }
            4 => {
                let digest_byte: u8 = kani::any();
                kani::assume(digest_byte == 7 || digest_byte == 9);
                TaskEventCore::CompleteMerge {
                    verified_digest: digest_byte,
                }
            }
            _ => TaskEventCore::Reject,
        };

        let was_terminal = state.is_terminal();
        if let Ok(next) = transition_core(mode, state, event, max_attempts) {
            assert!(!was_terminal, "terminal state must never transition");
            if let TaskStateCore::Running { attempt } = next {
                assert!(attempt >= 1 && attempt <= max_attempts);
            }
            if matches!(next, TaskStateCore::ApprovedForMerge { .. }) {
                approved_seen = true;
            }
            if is_read_only {
                assert!(
                    !matches!(
                        next,
                        TaskStateCore::AwaitingSpecReview { .. }
                            | TaskStateCore::ApprovedForMerge { .. }
                            | TaskStateCore::Merged
                    ),
                    "read-only task entered write review/merge state"
                );
            }
            if matches!(next, TaskStateCore::Merged) {
                assert!(approved_seen, "merged without prior approval");
            }
            state = next;
        }
    }

    // Law 6: Non-vacuity reachability probes.
    kani::cover!(matches!(state, TaskStateCore::Merged), "reach_merged");
    kani::cover!(
        matches!(state, TaskStateCore::CompletedReadOnly),
        "reach_completed_read_only"
    );
    kani::cover!(matches!(state, TaskStateCore::Rejected), "reach_rejected");
}

#[cfg(test)]
mod exhaustive_tests {
    use super::*;

    #[test]
    fn write_task_requires_review_and_matching_digest_before_merge() {
        let mode = sample_write_mode();
        let handoff = sample_handoff(true, true, 7);
        let s1 = transition(&mode, TaskState::Queued, TaskEvent::StartAttempt, 2).unwrap();
        // Direct merge from Running is forbidden.
        let direct_merge = transition(
            &mode,
            s1.clone(),
            TaskEvent::CompleteMerge {
                merged_commit: sample_oid('c'),
                verified_digest: SpecDigest::from_bytes([7; 32]),
            },
            2,
        );
        assert_eq!(direct_merge, Err(TransitionError::InvalidStateForEvent));

        let s2 = transition(&mode, s1, TaskEvent::SubmitHandoff { handoff }, 2).unwrap();
        // Digest mismatch on approval is rejected.
        let bad_approve = transition(
            &mode,
            s2.clone(),
            TaskEvent::ApproveSpec {
                approved_digest: SpecDigest::from_bytes([9; 32]),
            },
            2,
        );
        assert_eq!(bad_approve, Err(TransitionError::DigestMismatch));

        let s3 = transition(
            &mode,
            s2,
            TaskEvent::ApproveSpec {
                approved_digest: SpecDigest::from_bytes([7; 32]),
            },
            2,
        )
        .unwrap();
        // Digest mismatch on merge is rejected.
        let bad_merge = transition(
            &mode,
            s3.clone(),
            TaskEvent::CompleteMerge {
                merged_commit: sample_oid('c'),
                verified_digest: SpecDigest::from_bytes([9; 32]),
            },
            2,
        );
        assert_eq!(bad_merge, Err(TransitionError::DigestMismatch));

        let s4 = transition(
            &mode,
            s3,
            TaskEvent::CompleteMerge {
                merged_commit: sample_oid('c'),
                verified_digest: SpecDigest::from_bytes([7; 32]),
            },
            2,
        )
        .unwrap();
        assert!(matches!(s4, TaskState::Merged { .. }));
        // Terminal state refuses any further transition.
        assert_eq!(
            transition(&mode, s4, TaskEvent::StartAttempt, 2),
            Err(TransitionError::AlreadyTerminal)
        );
    }

    #[test]
    fn read_only_task_never_enters_write_states_or_requests_worktree() {
        let mode = TaskMode::read_only(vec![RelPath::parse("src").unwrap()]);
        assert!(!mode.needs_worktree());
        let s1 = transition(&mode, TaskState::Queued, TaskEvent::StartAttempt, 1).unwrap();
        let handoff_err = transition(
            &mode,
            s1.clone(),
            TaskEvent::SubmitHandoff {
                handoff: sample_handoff(true, true, 1),
            },
            1,
        );
        assert_eq!(handoff_err, Err(TransitionError::ModeMismatch));

        let done = transition(
            &mode,
            s1,
            TaskEvent::FinishReadOnly {
                summary: "audited".to_string(),
            },
            1,
        )
        .unwrap();
        assert!(matches!(done, TaskState::CompletedReadOnly { .. }));
    }

    #[test]
    fn attempt_budget_is_strictly_bounded() {
        let mode = sample_write_mode();
        assert_eq!(
            transition(&mode, TaskState::Queued, TaskEvent::StartAttempt, 0),
            Err(TransitionError::AttemptBudgetExhausted)
        );
        let s1 = transition(&mode, TaskState::Queued, TaskEvent::StartAttempt, 2).unwrap();
        let s2 = transition(&mode, s1, TaskEvent::StartAttempt, 2).unwrap();
        assert_eq!(s2, TaskState::Running { attempt: 2 });
        assert_eq!(
            transition(&mode, s2, TaskEvent::StartAttempt, 2),
            Err(TransitionError::AttemptBudgetExhausted)
        );
    }
}
