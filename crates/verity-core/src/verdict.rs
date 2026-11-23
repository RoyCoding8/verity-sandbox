use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HygieneViolation {
    pub rule: String,
    pub path: String,
    pub line: usize,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HygieneReport {
    pub violations: Vec<HygieneViolation>,
}

impl HygieneReport {
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Assertion,
    Cover,
    Safety,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Success,
    Failure,
    Unreachable,
    Undetermined,
    Satisfied,
    Unsatisfiable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaniCheck {
    pub kind: CheckKind,
    pub description: String,
    pub status: CheckStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaniReport {
    pub harness_name: String,
    pub should_panic: bool,
    pub solver_error: bool,
    pub checks: Vec<KaniCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictStatus {
    Pass,
    Counterexample,
    Inconclusive,
    InvalidReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckerVerdict {
    pub status: VerdictStatus,
    pub reason: String,
    pub checks_passed: usize,
    pub probes_satisfied: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateError {
    HygieneFailed(usize),
    VerdictNotPass(VerdictStatus),
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateError::HygieneFailed(n) => {
                write!(f, "spec hygiene check failed with {} violation(s)", n)
            }
            GateError::VerdictNotPass(status) => {
                write!(f, "machine checker verdict is {:?}, not Pass", status)
            }
        }
    }
}

impl std::error::Error for GateError {}

/// Ported and fixed from archive/verity-prototype/src/kani-judge.mjs (lines 56-200).
/// Accepts standard CBMC safety checks alongside assertions and covers, while
/// failing closed on any failure, undetermined check, solver error, or
/// missing/unsatisfiable `kani::cover!` reachability probe.
pub fn classify_kani_report(report: &KaniReport, expected_probes: &[&str]) -> CheckerVerdict {
    if report.harness_name.is_empty() || report.should_panic {
        return CheckerVerdict {
            status: VerdictStatus::InvalidReport,
            reason: "invalid harness metadata or should_panic is enabled".to_string(),
            checks_passed: 0,
            probes_satisfied: 0,
        };
    }
    if report.solver_error {
        return CheckerVerdict {
            status: VerdictStatus::Inconclusive,
            reason: "solver error reported by Kani".to_string(),
            checks_passed: 0,
            probes_satisfied: 0,
        };
    }

    let mut checks_passed = 0usize;
    let mut cover_hits: Vec<(&str, CheckStatus)> = Vec::new();

    for check in &report.checks {
        match check.kind {
            CheckKind::Assertion | CheckKind::Safety => match check.status {
                CheckStatus::Success => checks_passed += 1,
                CheckStatus::Unreachable => {}
                CheckStatus::Failure => {
                    return CheckerVerdict {
                        status: VerdictStatus::Counterexample,
                        reason: format!("check failed: {}", check.description),
                        checks_passed,
                        probes_satisfied: 0,
                    };
                }
                CheckStatus::Undetermined | CheckStatus::Satisfied | CheckStatus::Unsatisfiable => {
                    return CheckerVerdict {
                        status: VerdictStatus::Inconclusive,
                        reason: format!("check undetermined: {}", check.description),
                        checks_passed,
                        probes_satisfied: 0,
                    };
                }
            },
            CheckKind::Cover => {
                cover_hits.push((check.description.as_str(), check.status));
            }
        }
    }

    let mut probes_satisfied = 0usize;
    for expected in expected_probes {
        let mut matched_count = 0usize;
        for (desc, status) in &cover_hits {
            if desc == expected {
                matched_count += 1;
                match status {
                    CheckStatus::Satisfied => {}
                    CheckStatus::Unsatisfiable | CheckStatus::Unreachable | CheckStatus::Failure => {
                        return CheckerVerdict {
                            status: VerdictStatus::Counterexample,
                            reason: format!("reachability probe unreachable: {}", expected),
                            checks_passed,
                            probes_satisfied,
                        };
                    }
                    CheckStatus::Undetermined | CheckStatus::Success => {
                        return CheckerVerdict {
                            status: VerdictStatus::Inconclusive,
                            reason: format!("reachability probe undetermined: {}", expected),
                            checks_passed,
                            probes_satisfied,
                        };
                    }
                }
            }
        }
        if matched_count != 1 {
            return CheckerVerdict {
                status: VerdictStatus::InvalidReport,
                reason: format!(
                    "expected probe '{}' once, saw {} match(es)",
                    expected, matched_count
                ),
                checks_passed,
                probes_satisfied,
            };
        }
        probes_satisfied += 1;
    }

    CheckerVerdict {
        status: VerdictStatus::Pass,
        reason: "all checks passed and all reachability probes satisfied".to_string(),
        checks_passed,
        probes_satisfied,
    }
}

pub fn can_approve_gate(hygiene: &HygieneReport, verdict: &CheckerVerdict) -> Result<(), GateError> {
    if !hygiene.is_clean() {
        return Err(GateError::HygieneFailed(hygiene.violations.len()));
    }
    if verdict.status != VerdictStatus::Pass {
        return Err(GateError::VerdictNotPass(verdict.status));
    }
    Ok(())
}
