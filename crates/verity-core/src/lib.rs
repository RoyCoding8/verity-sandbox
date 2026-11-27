pub mod handoff;
pub mod json;
pub mod provider;
pub mod scope;
pub mod session;
pub mod types;
pub mod verdict;

pub use handoff::{can_approve, SpecHandoff};
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
pub use types::{sha256_bytes, CommitOid, RelPath, SpecDigest, TaskId, TypeError, VerifierKind};
pub use verdict::{
    can_approve_gate, can_approve_gate_core, classify_kani_core, classify_kani_report, CheckKind,
    CheckStatus, CheckView, CheckerVerdict, GateError, HygieneReport, HygieneViolation, KaniCheck,
    KaniCheckRef, KaniReport, VerdictCore, VerdictReason, VerdictStatus,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_reference_at_padding_and_block_boundaries() {
        let digest = SpecDigest::of_spec("abc");
        assert_eq!(
            digest.to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        for (len, expected) in [
            (
                0,
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                55,
                "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
            ),
            (
                56,
                "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
            ),
            (
                64,
                "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
            ),
            (
                65,
                "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0",
            ),
        ] {
            assert_eq!(
                SpecDigest::of_spec(&"a".repeat(len)).to_hex(),
                expected,
                "{len} bytes"
            );
        }
    }
}

pub mod pi_wire;
