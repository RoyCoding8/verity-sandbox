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
                    assert_eq!(t.activity.reload_deferred(), reload_deferred);
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
                assert_eq!(t.activity.reload_deferred(), reload_deferred);
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
            assert_eq!(t.activity.pending_queue(), 0);
            assert!(!t.activity.reload_deferred());
            assert_eq!(t.run_provider_reload, activity.reload_deferred());

            // Idempotence: a second Abort from Idle stays Idle without re-triggering reload.
            let again = transition_session(t.activity, SessionCommand::Abort)
                .expect("Abort from Idle must be idempotent");
            assert_eq!(again.activity, SessionActivity::Idle);
            assert_eq!(again.activity.pending_queue(), 0);
            assert!(!again.run_provider_reload);
        }
        SessionCommand::ProcessExited => {
            let t = res.expect("ProcessExited must succeed from any state");
            assert_eq!(t.activity, SessionActivity::Idle);
            assert_eq!(t.activity.pending_queue(), 0);
            assert!(!t.activity.reload_deferred());
            assert!(!t.run_provider_reload);

            // Idempotence: a second ProcessExited stays Idle.
            let again = transition_session(t.activity, SessionCommand::ProcessExited)
                .expect("ProcessExited from Idle must be idempotent");
            assert_eq!(again.activity, SessionActivity::Idle);
            assert_eq!(again.activity.pending_queue(), 0);
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

/// Bounded Multi-Step Trace Law:
/// Over an arbitrary symbolic sequence of up to 5 commands starting from `Idle`,
/// every session invariant holds at every step:
/// - `pending_queue == 0` and `!reload_deferred` whenever not `Streaming`
/// - Exclusive commands (`SendPrompt`, `Compact`, `SwitchSession`, `NewSession`, `Fork`, `CloneSession`)
///   are never accepted when not `Idle`
/// - `reload_deferred` is never lost while `Streaming` and fires on `AgentSettled` or `Abort`
/// - `Abort` and `ProcessExited` always succeed and recover to `Idle`
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(7)]
pub fn prove_bounded_multi_step_session_trace() {
    let mut activity = SessionActivity::Idle;
    let mut reload_requested_while_streaming = false;
    let mut deferred_reload_on_settle = false;
    let mut deferred_reload_on_abort = false;
    let mut process_exited_from_busy = false;
    let mut exclusive_rejected_while_busy = false;

    let steps: u8 = kani::any();
    kani::assume(steps <= 5);

    for _ in 0..steps {
        let cmd = any_command();
        let was_idle = activity.is_idle();
        let was_streaming = activity.is_streaming();
        let is_exclusive = matches!(
            cmd,
            SessionCommand::SendPrompt
                | SessionCommand::Compact
                | SessionCommand::SwitchSession
                | SessionCommand::NewSession
                | SessionCommand::Fork
                | SessionCommand::CloneSession
        );

        match transition_session(activity, cmd) {
            Ok(t) => {
                if is_exclusive {
                    assert!(was_idle, "exclusive command must only succeed from Idle");
                }
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
                        deferred_reload_on_settle = true;
                    }
                    reload_requested_while_streaming = false;
                }
                if matches!(cmd, SessionCommand::Abort) {
                    assert_eq!(t.activity, SessionActivity::Idle);
                    assert_eq!(t.activity.pending_queue(), 0);
                    assert_eq!(t.run_provider_reload, reload_requested_while_streaming);
                    if t.run_provider_reload {
                        deferred_reload_on_abort = true;
                    }
                    reload_requested_while_streaming = false;
                }
                if matches!(cmd, SessionCommand::ProcessExited) {
                    assert_eq!(t.activity, SessionActivity::Idle);
                    assert_eq!(t.activity.pending_queue(), 0);
                    if !was_idle {
                        process_exited_from_busy = true;
                    }
                    reload_requested_while_streaming = false;
                }
                if t.activity.is_streaming() && reload_requested_while_streaming {
                    assert!(
                        t.activity.reload_deferred(),
                        "deferred reload flag must persist until AgentSettled or Abort"
                    );
                }
                if !t.activity.is_streaming() {
                    assert_eq!(t.activity.pending_queue(), 0);
                    assert!(!t.activity.reload_deferred());
                }
                activity = t.activity;
            }
            Err(err) => {
                assert!(
                    !matches!(cmd, SessionCommand::Abort | SessionCommand::ProcessExited),
                    "Abort and ProcessExited must never fail"
                );
                if is_exclusive && !was_idle {
                    assert_eq!(err, SessionError::NotIdle);
                    exclusive_rejected_while_busy = true;
                }
            }
        }
    }

    kani::cover!(
        deferred_reload_on_settle,
        "multi_step_deferred_reload_on_settle"
    );
    kani::cover!(
        deferred_reload_on_abort,
        "multi_step_deferred_reload_on_abort"
    );
    kani::cover!(
        process_exited_from_busy,
        "multi_step_process_exited_recovery"
    );
    kani::cover!(
        exclusive_rejected_while_busy,
        "multi_step_exclusive_rejected_while_busy"
    );
}
