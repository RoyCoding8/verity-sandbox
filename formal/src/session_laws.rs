#[cfg(kani)]
use verity_core::{
    transition_session, SessionActivity, SessionCommand, SessionError, MAX_PENDING_QUEUE,
};

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
    kani::assume(tag <= 14);
    match tag {
        0 => SessionCommand::SendPrompt,
        1 => SessionCommand::Steer,
        2 => SessionCommand::FollowUp,
        3 => SessionCommand::ClearQueue,
        4 => SessionCommand::Compact,
        5 => SessionCommand::SwitchSession,
        6 => SessionCommand::NewSession,
        7 => SessionCommand::Fork,
        8 => SessionCommand::CloneSession,
        9 => SessionCommand::RequestProviderReload,
        10 => SessionCommand::AgentSettled,
        11 => SessionCommand::FinishCompacting,
        12 => SessionCommand::FinishSwitch,
        13 => SessionCommand::Abort,
        _ => SessionCommand::ProcessExited,
    }
}

/// Session Exclusivity, Queue, Deferred-Reload, & Universal Idempotent Recovery Laws
/// (all 4 states x all 15 commands x all u16 queue depths):
/// - `SendPrompt` is rejected if Streaming, Compacting, SwitchingSession, or `pending_queue > 0`.
/// - `Compact`, `SwitchSession`, `NewSession`, `Fork`, and `CloneSession` are rejected unless Idle.
/// - `Steer` and `FollowUp` are accepted only while Streaming, increment `pending_queue` without
///   overflow, and preserve `reload_deferred`.
/// - `ClearQueue` preserves `reload_deferred`.
/// - `RequestProviderReload` while Streaming sets `reload_deferred = true` without interrupting
///   the turn; `AgentSettled` and `Abort` return whether a deferred reload must now run.
/// - `Abort` and `ProcessExited` always succeed from any state, converge to `Idle` with
///   `pending_queue == 0` and `!reload_deferred`, and are idempotent on repetition.
#[cfg(kani)]
#[kani::proof]
pub fn prove_session_exclusivity_and_queue_laws() {
    let activity = any_activity();
    assert_eq!(
        activity.is_idle(),
        matches!(activity, SessionActivity::Idle)
    );
    match activity {
        SessionActivity::Streaming {
            pending_queue,
            reload_deferred,
        } => {
            assert!(activity.is_streaming());
            assert_eq!(activity.pending_queue(), pending_queue);
            assert_eq!(activity.reload_deferred(), reload_deferred);
        }
        _ => {
            assert!(!activity.is_streaming());
            assert_eq!(activity.pending_queue(), 0);
            assert!(!activity.reload_deferred());
        }
    }
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
        SessionCommand::SwitchSession
        | SessionCommand::NewSession
        | SessionCommand::Fork
        | SessionCommand::CloneSession => {
            if activity.is_idle() {
                let t = res.expect("Idle must accept exclusive session lifecycle command");
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
        SessionCommand::Abort => {
            let t = res.expect("Abort must succeed from any state");
            assert_eq!(t.activity, SessionActivity::Idle);
            assert_eq!(t.run_provider_reload, activity.reload_deferred());
        }
        SessionCommand::ProcessExited => {
            let t = res.expect("ProcessExited must succeed from any state");
            assert_eq!(t.activity, SessionActivity::Idle);
            assert!(!t.run_provider_reload);
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
            (
                SessionCommand::NewSession | SessionCommand::Fork | SessionCommand::CloneSession,
                Ok(t)
            ) if t.activity == SessionActivity::SwitchingSession
        ),
        "lifecycle_switch_accept_reachable"
    );
    kani::cover!(
        matches!(
            (command, res),
            (
                SessionCommand::NewSession | SessionCommand::Fork | SessionCommand::CloneSession,
                Err(SessionError::NotIdle)
            )
        ),
        "lifecycle_switch_reject_reachable"
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
    kani::cover!(
        matches!(
            (command, res),
            (SessionCommand::Abort, Ok(t)) if t.activity == SessionActivity::Idle && t.run_provider_reload
        ),
        "abort_deferred_reload_dispatch_reachable"
    );
    kani::cover!(
        matches!(
            (activity, command, res),
            (SessionActivity::Streaming { .. }, SessionCommand::ProcessExited, Ok(t)) if t.activity == SessionActivity::Idle
        ),
        "process_exited_streaming_recovery_reachable"
    );
    kani::cover!(
        matches!(
            (activity, command),
            (
                SessionActivity::Compacting,
                SessionCommand::Abort | SessionCommand::ProcessExited
            )
        ),
        "abort_and_exit_from_compacting"
    );
    kani::cover!(
        matches!(
            (activity, command),
            (
                SessionActivity::SwitchingSession,
                SessionCommand::Abort | SessionCommand::ProcessExited
            )
        ),
        "abort_and_exit_from_switching"
    );
}
