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

/// Allocation-free view of a Kani check for zero-heap verification and core classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KaniCheckRef<'a> {
    pub kind: CheckKind,
    pub description: &'a str,
    pub status: CheckStatus,
}

pub trait CheckView {
    fn kind(&self) -> CheckKind;
    fn description(&self) -> &str;
    fn status(&self) -> CheckStatus;
}

impl CheckView for KaniCheck {
    fn kind(&self) -> CheckKind {
        self.kind
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn status(&self) -> CheckStatus {
        self.status
    }
}

impl CheckView for KaniCheckRef<'_> {
    fn kind(&self) -> CheckKind {
        self.kind
    }

    fn description(&self) -> &str {
        self.description
    }

    fn status(&self) -> CheckStatus {
        self.status
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictReason {
    InvalidMetadataOrShouldPanic,
    SolverError,
    CheckFailed {
        check_index: usize,
    },
    CheckUndetermined {
        check_index: usize,
    },
    ProbeUnreachable {
        probe_index: usize,
    },
    ProbeUndetermined {
        probe_index: usize,
    },
    ProbeMatchCountMismatch {
        probe_index: usize,
        matched_count: usize,
    },
    AllPassed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerdictCore {
    pub status: VerdictStatus,
    pub reason: VerdictReason,
    pub checks_passed: usize,
    pub probes_satisfied: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckerVerdict {
    pub status: VerdictStatus,
    pub reason: String,
    pub checks_passed: usize,
    pub probes_satisfied: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Allocation-free core classifier verified directly by Kani.
/// Accepts standard CBMC safety checks alongside assertions and covers, while
/// failing closed on any failure, undetermined check, solver error, or
/// missing/unsatisfiable `kani::cover!` reachability probe.
pub fn classify_kani_core<C: CheckView>(
    has_harness_name: bool,
    should_panic: bool,
    solver_error: bool,
    checks: &[C],
    expected_probes: &[&str],
) -> VerdictCore {
    if !has_harness_name || should_panic {
        return VerdictCore {
            status: VerdictStatus::InvalidReport,
            reason: VerdictReason::InvalidMetadataOrShouldPanic,
            checks_passed: 0,
            probes_satisfied: 0,
        };
    }
    if solver_error {
        return VerdictCore {
            status: VerdictStatus::Inconclusive,
            reason: VerdictReason::SolverError,
            checks_passed: 0,
            probes_satisfied: 0,
        };
    }

    let mut checks_passed = 0usize;
    let mut check_index = 0usize;
    while check_index < checks.len() {
        let check = &checks[check_index];
        match check.kind() {
            CheckKind::Assertion | CheckKind::Safety => match check.status() {
                CheckStatus::Success => checks_passed += 1,
                CheckStatus::Unreachable => {}
                CheckStatus::Failure => {
                    return VerdictCore {
                        status: VerdictStatus::Counterexample,
                        reason: VerdictReason::CheckFailed { check_index },
                        checks_passed,
                        probes_satisfied: 0,
                    };
                }
                CheckStatus::Undetermined | CheckStatus::Satisfied | CheckStatus::Unsatisfiable => {
                    return VerdictCore {
                        status: VerdictStatus::Inconclusive,
                        reason: VerdictReason::CheckUndetermined { check_index },
                        checks_passed,
                        probes_satisfied: 0,
                    };
                }
            },
            CheckKind::Cover => {}
        }
        check_index += 1;
    }

    let mut probes_satisfied = 0usize;
    let mut probe_index = 0usize;
    while probe_index < expected_probes.len() {
        let expected = expected_probes[probe_index].as_bytes();
        let mut matched_count = 0usize;
        let mut i = 0usize;
        while i < checks.len() {
            let check = &checks[i];
            if check.kind() == CheckKind::Cover
                && bytes_eq(check.description().as_bytes(), expected)
            {
                matched_count += 1;
                match check.status() {
                    CheckStatus::Satisfied => {}
                    CheckStatus::Unsatisfiable
                    | CheckStatus::Unreachable
                    | CheckStatus::Failure => {
                        return VerdictCore {
                            status: VerdictStatus::Counterexample,
                            reason: VerdictReason::ProbeUnreachable { probe_index },
                            checks_passed,
                            probes_satisfied,
                        };
                    }
                    CheckStatus::Undetermined | CheckStatus::Success => {
                        return VerdictCore {
                            status: VerdictStatus::Inconclusive,
                            reason: VerdictReason::ProbeUndetermined { probe_index },
                            checks_passed,
                            probes_satisfied,
                        };
                    }
                }
            }
            i += 1;
        }
        if matched_count != 1 {
            return VerdictCore {
                status: VerdictStatus::InvalidReport,
                reason: VerdictReason::ProbeMatchCountMismatch {
                    probe_index,
                    matched_count,
                },
                checks_passed,
                probes_satisfied,
            };
        }
        probes_satisfied += 1;
        probe_index += 1;
    }

    VerdictCore {
        status: VerdictStatus::Pass,
        reason: VerdictReason::AllPassed,
        checks_passed,
        probes_satisfied,
    }
}

/// Ported and fixed from archive/verity-prototype/src/kani-judge.mjs (lines 56-200).
/// Delegates classification to `classify_kani_core` and formats human-readable reasons.
pub fn classify_kani_report(report: &KaniReport, expected_probes: &[&str]) -> CheckerVerdict {
    let core = classify_kani_core(
        !report.harness_name.is_empty(),
        report.should_panic,
        report.solver_error,
        &report.checks,
        expected_probes,
    );
    let reason = match core.reason {
        VerdictReason::InvalidMetadataOrShouldPanic => {
            "invalid harness metadata or should_panic is enabled".to_string()
        }
        VerdictReason::SolverError => "solver error reported by Kani".to_string(),
        VerdictReason::CheckFailed { check_index } => {
            format!("check failed: {}", report.checks[check_index].description)
        }
        VerdictReason::CheckUndetermined { check_index } => {
            format!(
                "check undetermined: {}",
                report.checks[check_index].description
            )
        }
        VerdictReason::ProbeUnreachable { probe_index } => {
            format!(
                "reachability probe unreachable: {}",
                expected_probes[probe_index]
            )
        }
        VerdictReason::ProbeUndetermined { probe_index } => {
            format!(
                "reachability probe undetermined: {}",
                expected_probes[probe_index]
            )
        }
        VerdictReason::ProbeMatchCountMismatch {
            probe_index,
            matched_count,
        } => {
            format!(
                "expected probe '{}' once, saw {} match(es)",
                expected_probes[probe_index], matched_count
            )
        }
        VerdictReason::AllPassed => {
            "all checks passed and all reachability probes satisfied".to_string()
        }
    };

    CheckerVerdict {
        status: core.status,
        reason,
        checks_passed: core.checks_passed,
        probes_satisfied: core.probes_satisfied,
    }
}

/// Allocation-free core approval gate verified directly by Kani.
pub fn can_approve_gate_core(
    hygiene_violations: usize,
    verdict_status: VerdictStatus,
) -> Result<(), GateError> {
    if hygiene_violations > 0 {
        return Err(GateError::HygieneFailed(hygiene_violations));
    }
    if verdict_status != VerdictStatus::Pass {
        return Err(GateError::VerdictNotPass(verdict_status));
    }
    Ok(())
}

pub fn can_approve_gate(
    hygiene: &HygieneReport,
    verdict: &CheckerVerdict,
) -> Result<(), GateError> {
    can_approve_gate_core(hygiene.violations.len(), verdict.status)
}
