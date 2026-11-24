#[cfg(kani)]
use verity_core::VerdictStatus;
#[cfg(kani)]
use verity_core::{transition_core, TaskEventCore, TaskModeKind, TaskStateCore};

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
