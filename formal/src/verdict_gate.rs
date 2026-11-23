#[cfg(test)]
use verity_core::{
    can_approve_gate, classify_kani_report, HygieneReport, HygieneViolation, KaniCheck, KaniReport,
};
#[cfg(kani)]
use verity_core::{
    can_approve_gate_core, classify_kani_core, GateError, KaniCheckRef, VerdictStatus,
};
use verity_core::{CheckKind, CheckStatus};

/// Law 4: Hygiene violations, failed/undetermined checks, or unsatisfiable
/// `kani::cover!` reachability probes must always fail the approval gate.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(16)]
pub fn prove_verdict_and_hygiene_gate() {
    let hygiene_clean: bool = kani::any();
    let assertion_ok: bool = kani::any();
    let probe_satisfied: bool = kani::any();
    let solver_error: bool = kani::any();

    let checks = [
        KaniCheckRef {
            kind: CheckKind::Assertion,
            description: "main_invariant",
            status: if assertion_ok {
                CheckStatus::Success
            } else {
                CheckStatus::Failure
            },
        },
        KaniCheckRef {
            kind: CheckKind::Cover,
            description: "reachable_path",
            status: if probe_satisfied {
                CheckStatus::Satisfied
            } else {
                CheckStatus::Unsatisfiable
            },
        },
    ];

    let verdict = classify_kani_core(true, false, solver_error, &checks, &["reachable_path"]);
    let hygiene_violations = if hygiene_clean { 0 } else { 1 };

    let gate = can_approve_gate_core(hygiene_violations, verdict.status);
    let expected_ok = hygiene_clean && assertion_ok && probe_satisfied && !solver_error;
    assert_eq!(gate.is_ok(), expected_ok);

    kani::cover!(gate.is_ok(), "gate_accept_reachable");
    kani::cover!(
        matches!(gate, Err(GateError::HygieneFailed(_))),
        "gate_hygiene_reject_reachable"
    );
    kani::cover!(
        matches!(
            gate,
            Err(GateError::VerdictNotPass(VerdictStatus::Counterexample))
        ),
        "gate_counterexample_reject_reachable"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustive_gate_truth_table() {
        for hygiene_clean in [false, true] {
            for assertion_ok in [false, true] {
                for probe_satisfied in [false, true] {
                    for solver_error in [false, true] {
                        let report = KaniReport {
                            harness_name: "verify_invariant".to_string(),
                            should_panic: false,
                            solver_error,
                            checks: vec![
                                KaniCheck {
                                    kind: CheckKind::Assertion,
                                    description: "main_invariant".to_string(),
                                    status: if assertion_ok {
                                        CheckStatus::Success
                                    } else {
                                        CheckStatus::Failure
                                    },
                                },
                                KaniCheck {
                                    kind: CheckKind::Cover,
                                    description: "reachable_path".to_string(),
                                    status: if probe_satisfied {
                                        CheckStatus::Satisfied
                                    } else {
                                        CheckStatus::Unsatisfiable
                                    },
                                },
                            ],
                        };
                        let verdict = classify_kani_report(&report, &["reachable_path"]);
                        let hygiene = if hygiene_clean {
                            HygieneReport::default()
                        } else {
                            HygieneReport {
                                violations: vec![HygieneViolation {
                                    rule: "no-stub".to_string(),
                                    path: "src/lib.rs".to_string(),
                                    line: 1,
                                    detail: "forbidden stub".to_string(),
                                }],
                            }
                        };
                        let gate = can_approve_gate(&hygiene, &verdict);
                        let expected_ok =
                            hygiene_clean && assertion_ok && probe_satisfied && !solver_error;
                        assert_eq!(gate.is_ok(), expected_ok);
                    }
                }
            }
        }
    }
}
