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
}

pub mod pi_wire;
