pub mod json;
pub mod provider;
pub mod scope;
pub mod session;
pub mod state_machine;
pub mod types;
pub mod verdict;

pub use provider::{
    validate_external_sign_in_url, validate_provider_url, validate_provider_url_bytes,
    ExternalSignInUrl, ProviderApi, ProviderError, ProviderProfile, ProviderUrl,
};
pub use scope::{
    check_scope, check_scope_core, within, within_str, ScopeError, ScopeViolationKind,
};
pub use session::{
    transition_session, SessionActivity, SessionCommand, SessionError, SessionTransition,
    MAX_PENDING_QUEUE,
};
pub use state_machine::{
    can_approve, transition, transition_core, RejectionReason, SpecHandoff, TaskEvent,
    TaskEventCore, TaskModeKind, TaskState, TaskStateCore, TransitionError, TransitionErrorCore,
};
pub use types::{
    sha256_bytes, CommitOid, RelPath, SpecDigest, TaskId, TaskMode, TypeError, VerifierKind,
};
pub use verdict::{
    can_approve_gate, can_approve_gate_core, classify_kani_core, classify_kani_report, CheckKind,
    CheckStatus, CheckView, CheckerVerdict, GateError, HygieneReport, HygieneViolation, KaniCheck,
    KaniCheckRef, KaniReport, VerdictCore, VerdictReason, VerdictStatus,
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
        assert_eq!(
            RelPath::parse("src/lib.rs/").unwrap().as_str(),
            "src/lib.rs"
        );
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

    #[test]
    fn provider_url_validation_covers_schemes_and_rejections() {
        assert!(validate_provider_url("https://api.openai.com/v1").is_ok());
        assert!(validate_provider_url("http://localhost:11434/v1").is_ok());
        assert!(validate_provider_url("http://[::1]:8080/v1").is_ok());
        assert_eq!(
            validate_provider_url("ftp://example.com/v1"),
            Err(ProviderError::InvalidScheme)
        );
        assert_eq!(
            validate_provider_url("https://user:pass@api.example.com/v1"),
            Err(ProviderError::EmbeddedCredentials)
        );
        assert_eq!(
            validate_provider_url("https://api.example.com/v1?key=1"),
            Err(ProviderError::QueryNotAllowed)
        );
        assert_eq!(
            validate_provider_url("https://api.example.com/v1#frag"),
            Err(ProviderError::FragmentNotAllowed)
        );
        assert_eq!(
            validate_provider_url("https://api.example.com/v1 "),
            Err(ProviderError::WhitespaceNotAllowed)
        );
        assert_eq!(
            validate_provider_url("https:///v1"),
            Err(ProviderError::EmptyHost)
        );
        assert_eq!(
            validate_external_sign_in_url("http://auth.example.com/login"),
            Err(ProviderError::HttpsRequired)
        );
        assert!(validate_external_sign_in_url("https://auth.example.com/login").is_ok());
        assert!(ProviderUrl::parse("https://api.anthropic.com/v1").is_ok());
        assert!(ExternalSignInUrl::parse("https://auth.example.com/oauth").is_ok());
    }

    #[test]
    fn session_state_machine_enforces_exclusivity_and_deferred_reload() {
        let s1 = transition_session(SessionActivity::Idle, SessionCommand::SendPrompt).unwrap();
        assert_eq!(
            s1.activity,
            SessionActivity::Streaming {
                pending_queue: 0,
                reload_deferred: false,
            }
        );
        assert!(!s1.run_provider_reload);

        assert_eq!(
            transition_session(s1.activity, SessionCommand::SendPrompt),
            Err(SessionError::NotIdle)
        );
        assert_eq!(
            transition_session(s1.activity, SessionCommand::Compact),
            Err(SessionError::NotIdle)
        );
        assert_eq!(
            transition_session(s1.activity, SessionCommand::SwitchSession),
            Err(SessionError::NotIdle)
        );

        let s2 = transition_session(s1.activity, SessionCommand::Steer).unwrap();
        let s3 = transition_session(s2.activity, SessionCommand::FollowUp).unwrap();
        assert_eq!(s3.activity.pending_queue(), 2);

        let s4 = transition_session(s3.activity, SessionCommand::RequestProviderReload).unwrap();
        assert!(!s4.run_provider_reload);
        assert!(s4.activity.reload_deferred());
        assert_eq!(s4.activity.pending_queue(), 2);

        let s5 = transition_session(s4.activity, SessionCommand::AgentSettled).unwrap();
        assert_eq!(s5.activity, SessionActivity::Idle);
        assert!(s5.run_provider_reload);
    }
}
