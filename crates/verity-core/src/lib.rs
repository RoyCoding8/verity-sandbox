pub mod json;
pub mod scope;
pub mod state_machine;
pub mod types;
pub mod verdict;

pub use scope::{check_scope, within, ScopeError};
pub use state_machine::{
    can_approve, transition, RejectionReason, SpecHandoff, TaskEvent, TaskState, TransitionError,
};
pub use types::{sha256_bytes, CommitOid, RelPath, SpecDigest, TaskId, TaskMode, TypeError, VerifierKind};
pub use verdict::{
    can_approve_gate, classify_kani_report, CheckKind, CheckStatus, CheckerVerdict, GateError,
    HygieneReport, HygieneViolation, KaniCheck, KaniReport, VerdictStatus,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_standard_vector() {
        let digest = SpecDigest::of_spec("abc");
        assert_eq!(
            digest.to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn rel_path_blocks_traversal_and_dot_git() {
        assert!(RelPath::parse("src/lib.rs").is_ok());
        assert_eq!(RelPath::parse("src/lib.rs/").unwrap().as_str(), "src/lib.rs");
        assert!(RelPath::parse("../secret").is_err());
        assert!(RelPath::parse(".git/hooks/pre-commit").is_err());
        assert!(RelPath::parse("sub/.GIT/config").is_err());
        assert!(RelPath::parse("CON.txt").is_err());
        assert!(RelPath::parse("/etc/passwd").is_err());
    }

    #[test]
    fn kani_classifier_accepts_safety_checks_and_enforces_cover_probes() {
        let report = KaniReport {
            harness_name: "verify_add".to_string(),
            should_panic: false,
            solver_error: false,
            checks: vec![
                KaniCheck {
                    kind: CheckKind::Safety,
                    description: "arithmetic overflow".to_string(),
                    status: CheckStatus::Success,
                },
                KaniCheck {
                    kind: CheckKind::Assertion,
                    description: "postcondition".to_string(),
                    status: CheckStatus::Success,
                },
                KaniCheck {
                    kind: CheckKind::Cover,
                    description: "happy_path".to_string(),
                    status: CheckStatus::Satisfied,
                },
            ],
        };
        let verdict = classify_kani_report(&report, &["happy_path"]);
        assert_eq!(verdict.status, VerdictStatus::Pass);
        assert_eq!(verdict.checks_passed, 2);
        assert_eq!(verdict.probes_satisfied, 1);

        let missing = classify_kani_report(&report, &["happy_path", "missing_probe"]);
        assert_eq!(missing.status, VerdictStatus::InvalidReport);
    }
}
