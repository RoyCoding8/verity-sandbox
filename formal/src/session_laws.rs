#[cfg(kani)]
use verity_core::validate_provider_url_bytes;
use verity_core::{
    transition_session, ProviderError, SessionActivity, SessionCommand, SessionError,
    MAX_PENDING_QUEUE,
};
#[cfg(test)]
use verity_core::{validate_external_sign_in_url, validate_provider_url};

#[cfg(kani)]
fn any_activity() -> SessionActivity {
    let tag: u8 = kani::any();
    kani::assume(tag <= 3);
    match tag {
        0 => SessionActivity::Idle,
        1 => SessionActivity::Streaming {
            pending_queue: kani::any(),
            reload_deferred: kani::any(),
        },
        2 => SessionActivity::Compacting,
        _ => SessionActivity::SwitchingSession,
    }
}

#[cfg(kani)]
fn any_command() -> SessionCommand {
    let tag: u8 = kani::any();
    kani::assume(tag <= 9);
    match tag {
        0 => SessionCommand::SendPrompt,
        1 => SessionCommand::Steer,
        2 => SessionCommand::FollowUp,
        3 => SessionCommand::ClearQueue,
        4 => SessionCommand::Compact,
        5 => SessionCommand::SwitchSession,
        6 => SessionCommand::RequestProviderReload,
        7 => SessionCommand::AgentSettled,
        8 => SessionCommand::FinishCompacting,
        _ => SessionCommand::FinishSwitch,
    }
}

/// Session Exclusivity & Queue Laws (all 4 states x all 10 commands x all u16 queue depths):
/// - `SendPrompt` is rejected if Streaming, Compacting, SwitchingSession, or `pending_queue > 0`.
/// - `Compact` and `SwitchSession` are rejected unless Idle.
/// - `Steer` and `FollowUp` are accepted only while Streaming and increment `pending_queue` without overflow.
/// - `RequestProviderReload` while Streaming sets `reload_deferred = true` without interrupting the turn;
///   `AgentSettled` returns whether a deferred reload must now run.
#[cfg(kani)]
#[kani::proof]
pub fn prove_session_exclusivity_and_queue_laws() {
    let activity = any_activity();
    let command = any_command();
    let res = transition_session(activity, command);

    match command {
        SessionCommand::SendPrompt => {
            if activity.is_idle() && activity.pending_queue() == 0 {
                let t = res.expect("Idle must accept SendPrompt");
                assert_eq!(
                    t.activity,
                    SessionActivity::Streaming {
                        pending_queue: 0,
                        reload_deferred: false,
                    }
                );
                assert!(!t.run_provider_reload);
            } else {
                assert_eq!(res, Err(SessionError::NotIdle));
            }
        }
        SessionCommand::Compact => {
            if activity.is_idle() {
                let t = res.expect("Idle must accept Compact");
                assert_eq!(t.activity, SessionActivity::Compacting);
                assert!(!t.run_provider_reload);
            } else {
                assert_eq!(res, Err(SessionError::NotIdle));
            }
        }
        SessionCommand::SwitchSession => {
            if activity.is_idle() {
                let t = res.expect("Idle must accept SwitchSession");
                assert_eq!(t.activity, SessionActivity::SwitchingSession);
                assert!(!t.run_provider_reload);
            } else {
                assert_eq!(res, Err(SessionError::NotIdle));
            }
        }
        SessionCommand::Steer | SessionCommand::FollowUp => match activity {
            SessionActivity::Streaming {
                pending_queue,
                reload_deferred,
            } => {
                if pending_queue < MAX_PENDING_QUEUE {
                    let t = res.expect("Streaming with queue capacity must accept Steer/FollowUp");
                    assert_eq!(
                        t.activity,
                        SessionActivity::Streaming {
                            pending_queue: pending_queue + 1,
                            reload_deferred,
                        }
                    );
                    assert!(!t.run_provider_reload);
                } else {
                    assert_eq!(res, Err(SessionError::QueueFull));
                }
            }
            _ => assert_eq!(res, Err(SessionError::NotStreaming)),
        },
        SessionCommand::ClearQueue => match activity {
            SessionActivity::Streaming {
                reload_deferred, ..
            } => {
                let t = res.expect("Streaming must accept ClearQueue");
                assert_eq!(
                    t.activity,
                    SessionActivity::Streaming {
                        pending_queue: 0,
                        reload_deferred,
                    }
                );
                assert!(!t.run_provider_reload);
            }
            _ => assert_eq!(res, Err(SessionError::NotStreaming)),
        },
        SessionCommand::RequestProviderReload => match activity {
            SessionActivity::Streaming { pending_queue, .. } => {
                let t = res.expect("Streaming must defer RequestProviderReload");
                assert_eq!(
                    t.activity,
                    SessionActivity::Streaming {
                        pending_queue,
                        reload_deferred: true,
                    }
                );
                assert!(!t.run_provider_reload);
            }
            SessionActivity::Idle => {
                let t = res.expect("Idle must execute RequestProviderReload immediately");
                assert_eq!(t.activity, SessionActivity::Idle);
                assert!(t.run_provider_reload);
            }
            SessionActivity::Compacting | SessionActivity::SwitchingSession => {
                assert_eq!(res, Err(SessionError::Busy));
            }
        },
        SessionCommand::AgentSettled => match activity {
            SessionActivity::Streaming {
                reload_deferred, ..
            } => {
                let t = res.expect("Streaming must accept AgentSettled");
                assert_eq!(t.activity, SessionActivity::Idle);
                assert_eq!(t.run_provider_reload, reload_deferred);
            }
            _ => assert_eq!(res, Err(SessionError::NotStreaming)),
        },
        SessionCommand::FinishCompacting => {
            if matches!(activity, SessionActivity::Compacting) {
                let t = res.expect("Compacting must accept FinishCompacting");
                assert_eq!(t.activity, SessionActivity::Idle);
                assert!(!t.run_provider_reload);
            } else {
                assert_eq!(res, Err(SessionError::NotCompacting));
            }
        }
        SessionCommand::FinishSwitch => {
            if matches!(activity, SessionActivity::SwitchingSession) {
                let t = res.expect("SwitchingSession must accept FinishSwitch");
                assert_eq!(t.activity, SessionActivity::Idle);
                assert!(!t.run_provider_reload);
            } else {
                assert_eq!(res, Err(SessionError::NotSwitching));
            }
        }
    }

    // Non-vacuity reachability probes.
    kani::cover!(
        matches!(
            (command, res),
            (
                SessionCommand::SendPrompt,
                Ok(t)
            ) if matches!(t.activity, SessionActivity::Streaming { pending_queue: 0, reload_deferred: false })
        ),
        "send_prompt_accept_reachable"
    );
    kani::cover!(
        matches!(
            (command, res),
            (SessionCommand::SendPrompt, Err(SessionError::NotIdle))
        ),
        "send_prompt_reject_reachable"
    );
    kani::cover!(
        matches!(
            (command, res),
            (SessionCommand::Steer | SessionCommand::FollowUp, Ok(_))
        ),
        "queue_increment_reachable"
    );
    kani::cover!(
        matches!(
            (command, res),
            (
                SessionCommand::Steer | SessionCommand::FollowUp,
                Err(SessionError::QueueFull)
            )
        ),
        "queue_full_reject_reachable"
    );
    kani::cover!(
        matches!(
            (command, res),
            (
                SessionCommand::RequestProviderReload,
                Ok(t)
            ) if matches!(t.activity, SessionActivity::Streaming { reload_deferred: true, .. }) && !t.run_provider_reload
        ),
        "deferred_reload_mark_reachable"
    );
    kani::cover!(
        matches!(
            (command, res),
            (SessionCommand::AgentSettled, Ok(t)) if t.activity == SessionActivity::Idle && t.run_provider_reload
        ),
        "deferred_reload_dispatch_reachable"
    );
}

/// Multi-step trace proving deferred provider reload is never lost while streaming
/// and fires when the agent settles back to Idle.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(6)]
pub fn prove_deferred_reload_survives_streaming_turns() {
    let mut activity = SessionActivity::Idle;
    let mut reload_requested_while_streaming = false;
    let mut deferred_reload_dispatched = false;

    let steps: u8 = kani::any();
    kani::assume(steps <= 4);

    for _ in 0..steps {
        let cmd = any_command();
        let was_streaming = activity.is_streaming();
        if let Ok(t) = transition_session(activity, cmd) {
            if was_streaming && matches!(cmd, SessionCommand::RequestProviderReload) {
                reload_requested_while_streaming = true;
                assert!(t.activity.is_streaming());
                assert!(t.activity.reload_deferred());
                assert!(!t.run_provider_reload);
            }
            if matches!(cmd, SessionCommand::AgentSettled) {
                assert_eq!(t.activity, SessionActivity::Idle);
                assert_eq!(t.run_provider_reload, reload_requested_while_streaming);
                if t.run_provider_reload {
                    deferred_reload_dispatched = true;
                }
                reload_requested_while_streaming = false;
            }
            if t.activity.is_streaming() && reload_requested_while_streaming {
                assert!(
                    t.activity.reload_deferred(),
                    "deferred reload flag must persist until AgentSettled"
                );
            }
            activity = t.activity;
        }
    }

    kani::cover!(
        deferred_reload_dispatched,
        "multi_step_deferred_reload_dispatched"
    );
}

/// Provider URL & External Sign-In URL validation laws:
/// - Any URL containing whitespace, `@`, `?`, or `#` is always rejected.
/// - `validate_external_sign_in_url` strictly requires `https://` and implies `validate_provider_url` acceptance.
/// - Empty host is always rejected.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(11)]
pub fn prove_provider_url_validation_laws() {
    let buf: [u8; 10] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 10);
    let slice = &buf[..len];

    let provider_res = validate_provider_url_bytes(slice, false);
    let signin_res = validate_provider_url_bytes(slice, true);

    let mut has_whitespace = false;
    let mut has_at = false;
    let mut has_query = false;
    let mut has_fragment = false;
    let mut i = 0usize;
    while i < len {
        let b = slice[i];
        if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            has_whitespace = true;
        }
        if b == b'@' {
            has_at = true;
        }
        if b == b'?' {
            has_query = true;
        }
        if b == b'#' {
            has_fragment = true;
        }
        i += 1;
    }

    if has_whitespace || has_at || has_query || has_fragment {
        assert!(provider_res.is_err());
        assert!(signin_res.is_err());
    }

    if signin_res.is_ok() {
        assert!(len >= 9);
        assert_eq!(&slice[..8], b"https://");
        assert!(provider_res.is_ok());
    }

    if provider_res.is_ok() {
        let is_https = len >= 9 && &slice[..8] == b"https://";
        let is_http = len >= 8 && &slice[..7] == b"http://";
        assert!(is_https || is_http);
        if is_http {
            assert_eq!(signin_res, Err(ProviderError::HttpsRequired));
        }
    }

    // Non-vacuity reachability probes.
    kani::cover!(
        provider_res.is_ok() && signin_res == Err(ProviderError::HttpsRequired),
        "provider_http_ok_signin_rejected"
    );
    kani::cover!(signin_res.is_ok(), "signin_https_ok_reachable");
    kani::cover!(
        provider_res == Err(ProviderError::EmbeddedCredentials),
        "provider_credentials_rejected"
    );
    kani::cover!(
        provider_res == Err(ProviderError::QueryNotAllowed),
        "provider_query_rejected"
    );
    kani::cover!(
        provider_res == Err(ProviderError::FragmentNotAllowed),
        "provider_fragment_rejected"
    );
    kani::cover!(
        provider_res == Err(ProviderError::EmptyHost),
        "provider_empty_host_rejected"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_queue_overflow_is_bounded_without_panic() {
        let full = SessionActivity::Streaming {
            pending_queue: MAX_PENDING_QUEUE,
            reload_deferred: true,
        };
        assert_eq!(
            transition_session(full, SessionCommand::Steer),
            Err(SessionError::QueueFull)
        );
        assert_eq!(
            transition_session(full, SessionCommand::FollowUp),
            Err(SessionError::QueueFull)
        );
        let cleared = transition_session(full, SessionCommand::ClearQueue).unwrap();
        assert_eq!(
            cleared.activity,
            SessionActivity::Streaming {
                pending_queue: 0,
                reload_deferred: true,
            }
        );
    }

    #[test]
    fn compact_and_switch_are_exclusive_with_streaming_and_each_other() {
        let compacting = transition_session(SessionActivity::Idle, SessionCommand::Compact)
            .unwrap()
            .activity;
        assert_eq!(
            transition_session(compacting, SessionCommand::SendPrompt),
            Err(SessionError::NotIdle)
        );
        assert_eq!(
            transition_session(compacting, SessionCommand::SwitchSession),
            Err(SessionError::NotIdle)
        );
        assert_eq!(
            transition_session(compacting, SessionCommand::RequestProviderReload),
            Err(SessionError::Busy)
        );
        let idle = transition_session(compacting, SessionCommand::FinishCompacting)
            .unwrap()
            .activity;
        assert_eq!(idle, SessionActivity::Idle);

        let switching = transition_session(idle, SessionCommand::SwitchSession)
            .unwrap()
            .activity;
        assert_eq!(
            transition_session(switching, SessionCommand::Compact),
            Err(SessionError::NotIdle)
        );
        assert_eq!(
            transition_session(switching, SessionCommand::SendPrompt),
            Err(SessionError::NotIdle)
        );
        let back_idle = transition_session(switching, SessionCommand::FinishSwitch)
            .unwrap()
            .activity;
        assert_eq!(back_idle, SessionActivity::Idle);
    }

    #[test]
    fn provider_and_external_sign_in_validation_edges() {
        assert!(validate_provider_url("https://api.example.com").is_ok());
        assert!(validate_provider_url("http://127.0.0.1:8000/v1").is_ok());
        assert_eq!(validate_provider_url(""), Err(ProviderError::EmptyUrl));
        assert_eq!(
            validate_provider_url("https://"),
            Err(ProviderError::EmptyHost)
        );
        assert_eq!(
            validate_provider_url("https://:443/v1"),
            Err(ProviderError::EmptyHost)
        );
        assert_eq!(
            validate_provider_url("https://api.example.com:abc"),
            Err(ProviderError::InvalidHost)
        );
        assert_eq!(
            validate_external_sign_in_url("http://localhost:3000/callback"),
            Err(ProviderError::HttpsRequired)
        );
        assert!(validate_external_sign_in_url("https://auth.example.com/sign-in").is_ok());
    }
}
