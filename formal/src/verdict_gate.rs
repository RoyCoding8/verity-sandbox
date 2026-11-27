use verity_core::provider::{
    is_loopback_host_bytes, validate_external_sign_in_url_bytes, validate_provider_url_bytes,
    ProviderError,
};
use verity_core::{
    can_approve_gate_core, classify_kani_core, CheckKind, CheckStatus, GateError, KaniCheckRef,
    VerdictReason, VerdictStatus,
};

fn any_check_ref() -> KaniCheckRef<'static> {
    let kinds = [CheckKind::Assertion, CheckKind::Cover, CheckKind::Safety];
    let statuses = [
        CheckStatus::Success,
        CheckStatus::Failure,
        CheckStatus::Unreachable,
        CheckStatus::Undetermined,
        CheckStatus::Satisfied,
        CheckStatus::Unsatisfiable,
    ];
    let names = ["p", "c", "other"];
    let kind: usize = kani::any();
    let status: usize = kani::any();
    let name: usize = kani::any();
    kani::assume(kind < kinds.len() && status < statuses.len() && name < names.len());
    KaniCheckRef {
        kind: kinds[kind],
        description: names[name],
        status: statuses[status],
    }
}

/// All report flags and lists of up to three checks and two required probes.
/// Check names range over the two required names and one unmatched name.
#[kani::proof]
#[kani::unwind(5)]
pub fn prove_verdict_and_hygiene_gate() {
    let checks_buf = [any_check_ref(), any_check_ref(), any_check_ref()];
    let len: usize = kani::any();
    kani::assume(len <= checks_buf.len());
    let checks = &checks_buf[..len];
    let probe_names = [
        if kani::any() { "p" } else { "c" },
        if kani::any() { "p" } else { "c" },
    ];
    let probe_count: usize = kani::any();
    kani::assume(probe_count <= probe_names.len());
    let expected_probes = &probe_names[..probe_count];
    let named: bool = kani::any();
    let should_panic: bool = kani::any();
    let solver_error: bool = kani::any();
    let hygiene_violations: usize = kani::any();
    let verdict = classify_kani_core(named, should_panic, solver_error, checks, expected_probes);
    let gate = can_approve_gate_core(hygiene_violations, verdict.status);

    let successes = checks
        .iter()
        .filter(|check| {
            matches!(check.kind, CheckKind::Assertion | CheckKind::Safety)
                && check.status == CheckStatus::Success
        })
        .count();
    let all_checks_acceptable = checks.iter().all(|check| {
        matches!(
            (check.kind, check.status),
            (
                CheckKind::Assertion | CheckKind::Safety,
                CheckStatus::Success
            ) | (CheckKind::Safety, CheckStatus::Unreachable)
                | (CheckKind::Cover, CheckStatus::Satisfied)
        )
    });
    let unique_expected_covers = expected_probes.iter().all(|expected| {
        checks
            .iter()
            .filter(|check| check.kind == CheckKind::Cover && check.description == *expected)
            .count()
            == 1
    });
    let metadata_valid = named && !should_panic && !solver_error;
    let should_pass =
        metadata_valid && successes > 0 && all_checks_acceptable && unique_expected_covers;
    assert_eq!(verdict.status == VerdictStatus::Pass, should_pass);

    let first_failure = checks.iter().position(|check| {
        matches!(check.kind, CheckKind::Assertion | CheckKind::Safety)
            && check.status == CheckStatus::Failure
    });
    if !named || should_panic {
        assert_eq!(verdict.status, VerdictStatus::InvalidReport);
        assert_eq!(verdict.reason, VerdictReason::InvalidMetadataOrShouldPanic);
    } else if solver_error {
        assert_eq!(verdict.status, VerdictStatus::Inconclusive);
        assert_eq!(verdict.reason, VerdictReason::SolverError);
    } else if let Some(check_index) = first_failure {
        assert_eq!(verdict.status, VerdictStatus::Counterexample);
        assert_eq!(verdict.reason, VerdictReason::CheckFailed { check_index });
    } else if all_checks_acceptable && !unique_expected_covers && successes > 0 {
        assert_eq!(verdict.status, VerdictStatus::InvalidReport);
        assert!(matches!(
            verdict.reason,
            VerdictReason::ProbeMatchCountMismatch { .. }
        ));
    }
    if should_pass {
        assert_eq!(verdict.checks_passed, successes);
        assert_eq!(verdict.probes_satisfied, probe_count);
        assert_eq!(verdict.reason, VerdictReason::AllPassed);
    }

    if hygiene_violations > 0 {
        assert_eq!(gate, Err(GateError::HygieneFailed(hygiene_violations)));
    } else if should_pass {
        assert_eq!(gate, Ok(()));
    } else {
        assert_eq!(gate, Err(GateError::VerdictNotPass(verdict.status)));
    }

    kani::cover!(
        gate.is_ok() && probe_count == 2 && probe_names[0] != probe_names[1],
        "gate_accept_two_distinct_probes"
    );
    kani::cover!(
        metadata_valid && len == 0 && verdict.status == VerdictStatus::InvalidReport,
        "vacuous_empty_rejected"
    );
    kani::cover!(
        matches!(verdict.reason, VerdictReason::CoverUnsatisfied { .. }),
        "cover_unsatisfied_rejected"
    );
    kani::cover!(
        successes > 0 && matches!(verdict.reason, VerdictReason::CheckFailed { .. }),
        "failure_overrides_pass"
    );
    kani::cover!(
        matches!(verdict.reason, VerdictReason::AssertionUnreachable { .. }),
        "unreachable_assertion_rejected"
    );
    kani::cover!(
        matches!(
            verdict.reason,
            VerdictReason::ProbeMatchCountMismatch {
                matched_count: 0,
                ..
            }
        ) && successes > 0,
        "missing_required_probe_rejected"
    );
    kani::cover!(
        matches!(
            verdict.reason,
            VerdictReason::ProbeMatchCountMismatch {
                matched_count: 2,
                ..
            }
        ) && successes > 0,
        "duplicate_required_probe_rejected"
    );
    kani::cover!(
        matches!(gate, Err(GateError::HygieneFailed(_))) && should_pass,
        "gate_hygiene_reject_reachable"
    );
}
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
