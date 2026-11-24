#[cfg(kani)]
use verity_core::provider::{
    is_loopback_host_bytes, validate_external_sign_in_url_bytes, validate_provider_url_bytes,
    ProviderError,
};
#[cfg(kani)]
use verity_core::verdict::can_approve_handoff_gate_core;
#[cfg(kani)]
use verity_core::{
    can_approve_gate_core, classify_kani_core, GateError, KaniCheckRef, VerdictStatus,
};
#[cfg(kani)]
use verity_core::{CheckKind, CheckStatus};

#[cfg(kani)]
fn any_check_kind() -> CheckKind {
    let tag: u8 = kani::any();
    kani::assume(tag < 3);
    match tag {
        0 => CheckKind::Assertion,
        1 => CheckKind::Cover,
        _ => CheckKind::Safety,
    }
}

#[cfg(kani)]
fn any_check_status() -> CheckStatus {
    let tag: u8 = kani::any();
    kani::assume(tag < 6);
    match tag {
        0 => CheckStatus::Success,
        1 => CheckStatus::Failure,
        2 => CheckStatus::Unreachable,
        3 => CheckStatus::Undetermined,
        4 => CheckStatus::Satisfied,
        _ => CheckStatus::Unsatisfiable,
    }
}

#[cfg(kani)]
fn any_check_ref() -> KaniCheckRef<'static> {
    let kind = any_check_kind();
    let status = any_check_status();
    let use_probe_name: bool = kani::any();
    let description = if use_probe_name { "p" } else { "c" };
    KaniCheckRef {
        kind,
        description,
        status,
    }
}

/// Law 4: Empty check sets, failed/undetermined/unreachable assertions,
/// unsatisfied `kani::cover!` probes, hygiene findings, and unmet
/// `spec_digest` / `blind_back_translation` requirements must always fail
/// classification or the approval gate.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(5)]
pub fn prove_verdict_and_hygiene_gate() {
    let checks_buf = [any_check_ref(), any_check_ref(), any_check_ref()];
    let len: usize = kani::any();
    kani::assume(len <= 3);
    let checks = &checks_buf[..len];

    let has_harness_name: bool = kani::any();
    let should_panic: bool = kani::any();
    let solver_error: bool = kani::any();
    let require_probe: bool = kani::any();
    let hygiene_violations: usize = kani::any();
    let spec_digest_present: bool = kani::any();
    let blind_back_translation_verified: bool = kani::any();

    let expected_probes: &[&str] = if require_probe { &["p"] } else { &[] };

    let verdict = classify_kani_core(
        has_harness_name,
        should_panic,
        solver_error,
        checks,
        expected_probes,
    );
    let gate = can_approve_gate_core(hygiene_violations, verdict.status);
    let handoff_gate = can_approve_handoff_gate_core(
        hygiene_violations,
        verdict.status,
        spec_digest_present,
        blind_back_translation_verified,
    );

    let mut any_pass_check = false;
    let mut any_fail_check = false;
    let mut any_unreachable_assertion = false;
    let mut any_undetermined_check = false;
    let mut any_cover_unsatisfied = false;
    let mut any_cover_undetermined = false;
    let mut i = 0usize;
    while i < len {
        let c = checks[i];
        match c.kind {
            CheckKind::Assertion => match c.status {
                CheckStatus::Success => any_pass_check = true,
                CheckStatus::Failure => any_fail_check = true,
                CheckStatus::Unreachable => any_unreachable_assertion = true,
                CheckStatus::Undetermined | CheckStatus::Satisfied | CheckStatus::Unsatisfiable => {
                    any_undetermined_check = true
                }
            },
            CheckKind::Safety => match c.status {
                CheckStatus::Success => any_pass_check = true,
                CheckStatus::Unreachable => {}
                CheckStatus::Failure => any_fail_check = true,
                CheckStatus::Undetermined | CheckStatus::Satisfied | CheckStatus::Unsatisfiable => {
                    any_undetermined_check = true
                }
            },
            CheckKind::Cover => match c.status {
                CheckStatus::Satisfied => {}
                CheckStatus::Unsatisfiable | CheckStatus::Unreachable | CheckStatus::Failure => {
                    any_cover_unsatisfied = true
                }
                CheckStatus::Undetermined | CheckStatus::Success => any_cover_undetermined = true,
            },
        }
        i += 1;
    }

    // Vacuous-empty rejection: empty checks or zero passing checks can never Pass.
    if len == 0 || !any_pass_check {
        assert_ne!(verdict.status, VerdictStatus::Pass);
        assert!(gate.is_err());
        assert!(handoff_gate.is_err());
    }

    // Failure-overrides-pass soundness: any assertion/safety failure yields Counterexample
    // whenever metadata and solver are valid, even if other checks succeeded or were undetermined.
    if has_harness_name && !should_panic && !solver_error && any_fail_check {
        assert_eq!(verdict.status, VerdictStatus::Counterexample);
        assert!(gate.is_err());
        assert!(handoff_gate.is_err());
    }

    // Unreachable assertion, undetermined check, or any unsatisfied cover must never Pass.
    if any_unreachable_assertion
        || any_undetermined_check
        || any_cover_unsatisfied
        || any_cover_undetermined
    {
        assert_ne!(verdict.status, VerdictStatus::Pass);
        assert!(gate.is_err());
        assert!(handoff_gate.is_err());
    }

    // Hygiene violations always reject both core and handoff gates.
    if hygiene_violations > 0 {
        assert_eq!(gate, Err(GateError::HygieneFailed(hygiene_violations)));
        assert_eq!(
            handoff_gate,
            Err(GateError::HygieneFailed(hygiene_violations))
        );
    }

    // Unmet spec_digest or blind_back_translation always rejects handoff gate.
    if !spec_digest_present || !blind_back_translation_verified {
        assert!(handoff_gate.is_err());
    }

    // Exact gate soundness.
    assert_eq!(
        gate.is_ok(),
        hygiene_violations == 0 && verdict.status == VerdictStatus::Pass
    );
    assert_eq!(
        handoff_gate.is_ok(),
        hygiene_violations == 0
            && verdict.status == VerdictStatus::Pass
            && spec_digest_present
            && blind_back_translation_verified
    );

    // Non-vacuity reachability probes.
    kani::cover!(
        gate.is_ok() && handoff_gate.is_ok(),
        "gate_accept_reachable"
    );
    kani::cover!(
        len == 0 && verdict.status == VerdictStatus::InvalidReport,
        "vacuous_empty_rejected"
    );
    kani::cover!(
        has_harness_name
            && !should_panic
            && !solver_error
            && any_cover_unsatisfied
            && verdict.status == VerdictStatus::Counterexample,
        "cover_unsatisfied_rejected"
    );
    kani::cover!(
        has_harness_name
            && !should_panic
            && !solver_error
            && any_pass_check
            && any_fail_check
            && verdict.status == VerdictStatus::Counterexample,
        "failure_overrides_pass_reachable"
    );
    kani::cover!(
        has_harness_name
            && !should_panic
            && !solver_error
            && any_unreachable_assertion
            && verdict.status == VerdictStatus::Counterexample,
        "unreachable_assertion_rejected"
    );
    kani::cover!(
        matches!(gate, Err(GateError::HygieneFailed(_))),
        "gate_hygiene_reject_reachable"
    );
    kani::cover!(
        matches!(handoff_gate, Err(GateError::MissingSpecDigest)),
        "missing_spec_digest_rejected"
    );
    kani::cover!(
        matches!(handoff_gate, Err(GateError::BlindBackTranslationUnverified)),
        "blind_back_translation_rejected"
    );
}

#[cfg(kani)]
fn starts_with(haystack: &[u8], prefix: &[u8]) -> bool {
    if haystack.len() < prefix.len() {
        return false;
    }
    let mut i = 0usize;
    while i < prefix.len() {
        if haystack[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(kani)]
fn extract_host_from_http_url(slice: &[u8]) -> &[u8] {
    let rest = &slice[7..];
    let mut auth_end = rest.len();
    let mut i = 0usize;
    while i < rest.len() {
        if rest[i] == b'/' {
            auth_end = i;
            break;
        }
        i += 1;
    }
    let auth = &rest[..auth_end];
    if !auth.is_empty() && auth[0] == b'[' {
        let mut k = 1usize;
        while k < auth.len() {
            if auth[k] == b']' {
                return &auth[..=k];
            }
            k += 1;
        }
        return auth;
    }
    let mut k = 0usize;
    while k < auth.len() {
        if auth[k] == b':' {
            return &auth[..k];
        }
        k += 1;
    }
    auth
}

/// Law 5: Canonical Provider & External Sign-In URL validation laws (len <= 16).
///
/// Proves that byte validators are panic-free and never accept:
/// - embedded credentials (`@`), query (`?`), fragment (`#`), whitespace/CRLF/control bytes,
/// - `javascript:`/`file:`/`data:` schemes, empty hosts, or non-loopback `http://` URLs.
/// Also proves `validate_external_sign_in_url_bytes` strictly requires `https://` and implies
/// `validate_provider_url_bytes` acceptance.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(18)]
pub fn prove_provider_url_injection_and_loopback_laws() {
    let buf: [u8; 16] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 16);
    let slice = &buf[..len];

    let provider_res = validate_provider_url_bytes(slice, false);
    let signin_res = validate_external_sign_in_url_bytes(slice);

    let mut has_at = false;
    let mut has_ws = false;
    let mut has_ctrl = false;
    let mut has_query = false;
    let mut has_fragment = false;
    let mut i = 0usize;
    while i < len {
        let b = slice[i];
        if b == b'@' {
            has_at = true;
        }
        if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            has_ws = true;
        }
        if b < 0x20 || b == 0x7f {
            has_ctrl = true;
        }
        if b == b'?' {
            has_query = true;
        }
        if b == b'#' {
            has_fragment = true;
        }
        i += 1;
    }

    let is_js = starts_with(slice, b"javascript:");
    let is_file = starts_with(slice, b"file:");
    let is_data = starts_with(slice, b"data:");
    let is_https = starts_with(slice, b"https://");
    let is_http = starts_with(slice, b"http://");

    if has_at || has_ws || has_ctrl || has_query || has_fragment || is_js || is_file || is_data {
        assert!(provider_res.is_err());
        assert!(signin_res.is_err());
    }

    if signin_res.is_ok() {
        assert!(len >= 9);
        assert!(is_https);
        assert!(provider_res.is_ok());
    }

    if provider_res.is_ok() {
        assert!(is_https || is_http);
        if is_http {
            let host = extract_host_from_http_url(slice);
            assert!(is_loopback_host_bytes(host));
            assert_eq!(signin_res, Err(ProviderError::HttpsRequired));
        }
    }

    if is_http && len > 7 {
        let host = extract_host_from_http_url(slice);
        if !is_loopback_host_bytes(host) {
            assert!(provider_res.is_err());
        }
    }

    // Non-vacuity reachability probes.
    kani::cover!(signin_res.is_ok(), "https_accept_reachable");
    kani::cover!(
        provider_res.is_ok() && is_http,
        "loopback_http_accept_reachable"
    );
    kani::cover!(
        is_http && provider_res == Err(ProviderError::HttpsRequired),
        "remote_http_reject_reachable"
    );
    kani::cover!(
        provider_res == Err(ProviderError::EmbeddedCredentials),
        "userinfo_reject_reachable"
    );
    kani::cover!(
        provider_res == Err(ProviderError::QueryNotAllowed),
        "query_reject_reachable"
    );
    kani::cover!(
        provider_res == Err(ProviderError::FragmentNotAllowed),
        "fragment_reject_reachable"
    );
    kani::cover!(
        provider_res == Err(ProviderError::EmptyHost),
        "empty_host_reject_reachable"
    );
    kani::cover!(
        provider_res == Err(ProviderError::ControlCharNotAllowed),
        "control_char_reject_reachable"
    );
    kani::cover!(
        provider_res == Err(ProviderError::WhitespaceNotAllowed),
        "whitespace_crlf_reject_reachable"
    );
}
