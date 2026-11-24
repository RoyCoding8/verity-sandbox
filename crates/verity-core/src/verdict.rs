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
    EmptyChecks,
    CheckFailed {
        check_index: usize,
    },
    AssertionUnreachable {
        check_index: usize,
    },
    CheckUndetermined {
        check_index: usize,
    },
    CoverUnsatisfied {
        check_index: usize,
    },
    CoverUndetermined {
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
    MissingSpecDigest,
    BlindBackTranslationUnverified,
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
            GateError::MissingSpecDigest => {
                write!(f, "spec_digest is missing or unverified")
            }
            GateError::BlindBackTranslationUnverified => {
                write!(f, "blind back-translation requirement is unmet")
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
/// failing closed on empty check sets, any failure, unreachable assertion,
/// undetermined check, solver error, or any unsatisfied/missing `kani::cover!` probe.
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
    if checks.is_empty() {
        return VerdictCore {
            status: VerdictStatus::InvalidReport,
            reason: VerdictReason::EmptyChecks,
            checks_passed: 0,
            probes_satisfied: 0,
        };
    }

    let mut checks_passed = 0usize;
    let mut first_unreachable_assertion: Option<usize> = None;
    let mut first_cover_unsatisfied: Option<usize> = None;
    let mut first_undetermined: Option<(usize, bool)> = None;

    let mut check_index = 0usize;
    while check_index < checks.len() {
        let check = &checks[check_index];
        match check.kind() {
            CheckKind::Assertion => match check.status() {
                CheckStatus::Success => checks_passed += 1,
                CheckStatus::Failure => {
                    return VerdictCore {
                        status: VerdictStatus::Counterexample,
                        reason: VerdictReason::CheckFailed { check_index },
                        checks_passed,
                        probes_satisfied: 0,
                    };
                }
                CheckStatus::Unreachable => {
                    if first_unreachable_assertion.is_none() {
                        first_unreachable_assertion = Some(check_index);
                    }
                }
                CheckStatus::Undetermined | CheckStatus::Satisfied | CheckStatus::Unsatisfiable => {
                    if first_undetermined.is_none() {
                        first_undetermined = Some((check_index, false));
                    }
                }
            },
            CheckKind::Safety => match check.status() {
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
                    if first_undetermined.is_none() {
                        first_undetermined = Some((check_index, false));
                    }
                }
            },
            CheckKind::Cover => match check.status() {
                CheckStatus::Satisfied => {}
                CheckStatus::Unsatisfiable | CheckStatus::Unreachable | CheckStatus::Failure => {
                    if first_cover_unsatisfied.is_none() {
                        first_cover_unsatisfied = Some(check_index);
                    }
                }
                CheckStatus::Undetermined | CheckStatus::Success => {
                    if first_undetermined.is_none() {
                        first_undetermined = Some((check_index, true));
                    }
                }
            },
        }
        check_index += 1;
    }

    if let Some(check_index) = first_unreachable_assertion {
        return VerdictCore {
            status: VerdictStatus::Counterexample,
            reason: VerdictReason::AssertionUnreachable { check_index },
            checks_passed,
            probes_satisfied: 0,
        };
    }

    if let Some(check_index) = first_cover_unsatisfied {
        return VerdictCore {
            status: VerdictStatus::Counterexample,
            reason: VerdictReason::CoverUnsatisfied { check_index },
            checks_passed,
            probes_satisfied: 0,
        };
    }

    if let Some((check_index, is_cover)) = first_undetermined {
        return VerdictCore {
            status: VerdictStatus::Inconclusive,
            reason: if is_cover {
                VerdictReason::CoverUndetermined { check_index }
            } else {
                VerdictReason::CheckUndetermined { check_index }
            },
            checks_passed,
            probes_satisfied: 0,
        };
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

    if checks_passed == 0 {
        return VerdictCore {
            status: VerdictStatus::InvalidReport,
            reason: VerdictReason::EmptyChecks,
            checks_passed: 0,
            probes_satisfied,
        };
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
        VerdictReason::EmptyChecks => {
            "report contains no passing assertion or safety checks".to_string()
        }
        VerdictReason::CheckFailed { check_index } => {
            format!("check failed: {}", report.checks[check_index].description)
        }
        VerdictReason::AssertionUnreachable { check_index } => {
            format!(
                "assertion unreachable: {}",
                report.checks[check_index].description
            )
        }
        VerdictReason::CheckUndetermined { check_index } => {
            format!(
                "check undetermined: {}",
                report.checks[check_index].description
            )
        }
        VerdictReason::CoverUnsatisfied { check_index } => {
            format!(
                "cover property unsatisfied: {}",
                report.checks[check_index].description
            )
        }
        VerdictReason::CoverUndetermined { check_index } => {
            format!(
                "cover property undetermined: {}",
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

/// Allocation-free handoff approval gate enforcing hygiene, checker verdict,
/// `spec_digest`, and `blind_back_translation` requirements.
pub fn can_approve_handoff_gate_core(
    hygiene_violations: usize,
    verdict_status: VerdictStatus,
    spec_digest_present: bool,
    blind_back_translation_verified: bool,
) -> Result<(), GateError> {
    can_approve_gate_core(hygiene_violations, verdict_status)?;
    if !spec_digest_present {
        return Err(GateError::MissingSpecDigest);
    }
    if !blind_back_translation_verified {
        return Err(GateError::BlindBackTranslationUnverified);
    }
    Ok(())
}

pub fn can_approve_gate(
    hygiene: &HygieneReport,
    verdict: &CheckerVerdict,
) -> Result<(), GateError> {
    can_approve_gate_core(hygiene.violations.len(), verdict.status)
}
