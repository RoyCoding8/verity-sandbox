#[cfg(test)]
use verity_core::provider::{
    is_loopback_host_bytes, validate_external_sign_in_url_bytes, validate_provider_url_bytes,
    ProviderError,
};
#[cfg(test)]
use verity_core::verdict::can_approve_handoff_gate_core;
#[cfg(test)]
use verity_core::{
    can_approve_gate, classify_kani_report, GateError, HygieneReport, HygieneViolation, KaniCheck,
    KaniReport, VerdictStatus,
};
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
                CheckStatus::Unsatisfiable
                | CheckStatus::Unreachable
                | CheckStatus::Failure => any_cover_unsatisfied = true,
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

/// Law 5: Provider and external sign-in URL byte validators are panic-free and
/// never accept embedded credentials (`@`), whitespace/CRLF/control bytes,
/// `javascript:`/`file:`/`data:` schemes, or non-loopback `http://` URLs.
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
        i += 1;
    }

    let is_js = starts_with(slice, b"javascript:");
    let is_file = starts_with(slice, b"file:");
    let is_data = starts_with(slice, b"data:");
    let is_https = starts_with(slice, b"https://");
    let is_http = starts_with(slice, b"http://");

    if has_at || has_ws || has_ctrl || is_js || is_file || is_data {
        assert!(provider_res.is_err());
        assert!(signin_res.is_err());
    }

    if signin_res.is_ok() {
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
        provider_res == Err(ProviderError::ControlCharNotAllowed),
        "control_char_reject_reachable"
    );
    kani::cover!(
        provider_res == Err(ProviderError::WhitespaceNotAllowed),
        "whitespace_crlf_reject_reachable"
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

    #[test]
    fn verdict_rejects_empty_unreachable_unlisted_cover_and_handoff_gaps() {
        let empty = KaniReport {
            harness_name: "empty_harness".to_string(),
            should_panic: false,
            solver_error: false,
            checks: vec![],
        };
        assert_eq!(
            classify_kani_report(&empty, &[]).status,
            VerdictStatus::InvalidReport
        );

        let unreachable_assert = KaniReport {
            harness_name: "unreachable_assert".to_string(),
            should_panic: false,
            solver_error: false,
            checks: vec![KaniCheck {
                kind: CheckKind::Assertion,
                description: "dead_assert".to_string(),
                status: CheckStatus::Unreachable,
            }],
        };
        assert_eq!(
            classify_kani_report(&unreachable_assert, &[]).status,
            VerdictStatus::Counterexample
        );

        let unlisted_cover_unsat = KaniReport {
            harness_name: "unlisted_cover".to_string(),
            should_panic: false,
            solver_error: false,
            checks: vec![
                KaniCheck {
                    kind: CheckKind::Assertion,
                    description: "ok".to_string(),
                    status: CheckStatus::Success,
                },
                KaniCheck {
                    kind: CheckKind::Cover,
                    description: "extra_cover".to_string(),
                    status: CheckStatus::Unsatisfiable,
                },
            ],
        };
        assert_eq!(
            classify_kani_report(&unlisted_cover_unsat, &[]).status,
            VerdictStatus::Counterexample
        );

        let failure_overrides_undetermined = KaniReport {
            harness_name: "precedence".to_string(),
            should_panic: false,
            solver_error: false,
            checks: vec![
                KaniCheck {
                    kind: CheckKind::Assertion,
                    description: "pass".to_string(),
                    status: CheckStatus::Success,
                },
                KaniCheck {
                    kind: CheckKind::Assertion,
                    description: "undetermined".to_string(),
                    status: CheckStatus::Undetermined,
                },
                KaniCheck {
                    kind: CheckKind::Assertion,
                    description: "fail".to_string(),
                    status: CheckStatus::Failure,
                },
            ],
        };
        assert_eq!(
            classify_kani_report(&failure_overrides_undetermined, &[]).status,
            VerdictStatus::Counterexample
        );

        assert_eq!(
            can_approve_handoff_gate_core(0, VerdictStatus::Pass, false, true),
            Err(GateError::MissingSpecDigest)
        );
        assert_eq!(
            can_approve_handoff_gate_core(0, VerdictStatus::Pass, true, false),
            Err(GateError::BlindBackTranslationUnverified)
        );
        assert!(can_approve_handoff_gate_core(0, VerdictStatus::Pass, true, true).is_ok());
    }

    #[test]
    fn provider_url_rejects_ssrf_injection_and_allows_loopback_http() {
        assert!(validate_provider_url_bytes(b"https://api.openai.com/v1", false).is_ok());
        assert!(validate_provider_url_bytes(b"http://localhost:11434/v1", false).is_ok());
        assert!(validate_provider_url_bytes(b"http://127.0.0.1:8080/v1", false).is_ok());
        assert!(validate_provider_url_bytes(b"http://[::1]:8080/v1", false).is_ok());

        assert_eq!(
            validate_provider_url_bytes(b"http://evil.com/v1", false),
            Err(ProviderError::HttpsRequired)
        );
        assert_eq!(
            validate_provider_url_bytes(b"http://localhost.evil.com/v1", false),
            Err(ProviderError::HttpsRequired)
        );
        assert_eq!(
            validate_provider_url_bytes(b"http://[::2]:8080/v1", false),
            Err(ProviderError::HttpsRequired)
        );
        assert_eq!(
            validate_provider_url_bytes(b"https://user:pass@evil.com/v1", false),
            Err(ProviderError::EmbeddedCredentials)
        );
        assert_eq!(
            validate_provider_url_bytes(b"https://api.example.com/v1\r\nX-Injected: 1", false),
            Err(ProviderError::WhitespaceNotAllowed)
        );
        assert_eq!(
            validate_provider_url_bytes(b"https://api.example.com/v1\0", false),
            Err(ProviderError::ControlCharNotAllowed)
        );
        assert_eq!(
            validate_provider_url_bytes(b"javascript:alert(1)", false),
            Err(ProviderError::InvalidScheme)
        );
        assert_eq!(
            validate_provider_url_bytes(b"file:///etc/passwd", false),
            Err(ProviderError::InvalidScheme)
        );
        assert_eq!(
            validate_provider_url_bytes(b"data:text/html,hi", false),
            Err(ProviderError::InvalidScheme)
        );
        assert_eq!(
            validate_provider_url_bytes(b"https://example.com:99999/v1", false),
            Err(ProviderError::InvalidHost)
        );
        assert_eq!(
            validate_external_sign_in_url_bytes(b"http://localhost:3000/cb"),
            Err(ProviderError::HttpsRequired)
        );
        assert!(validate_external_sign_in_url_bytes(b"https://auth.example.com/cb").is_ok());

        assert!(is_loopback_host_bytes(b"localhost"));
        assert!(is_loopback_host_bytes(b"LOCALHOST"));
        assert!(is_loopback_host_bytes(b"127.0.0.1"));
        assert!(is_loopback_host_bytes(b"[::1]"));
        assert!(!is_loopback_host_bytes(b"127.0.0.1.evil.com"));
        assert!(!is_loopback_host_bytes(b"[::2]"));
    }
}
