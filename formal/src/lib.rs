#[cfg(not(kani))]
use verity_core as _;

#[cfg(kani)]
pub mod scope_policy;
#[cfg(kani)]
pub mod session_laws;
#[cfg(kani)]
pub mod state_machine;
#[cfg(kani)]
pub mod verdict_gate;
pub mod command_and_project_laws;
pub mod cycle_and_revert_laws;
pub mod json_laws;
pub mod types_laws;
pub mod pi_event_laws;
