#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod verity_bridge;

use tauri::Manager;
use verity_desktop::pi_ipc::{self, PiAppState};
use verity_desktop::providers;

fn main() {
    let app = tauri::Builder::default()
        .manage(PiAppState::new_default())
        .invoke_handler(tauri::generate_handler![
            providers::list_providers,
            providers::upsert_provider,
            providers::delete_provider,
            providers::validate_provider_sign_in_url,
            verity_bridge::list_run_log,
            verity_bridge::run_readonly_subagent,
            verity_bridge::spawn_write_subagent,
            verity_bridge::list_handoffs,
            verity_bridge::get_handoff,
            verity_bridge::approve_and_merge_handoff,
            pi_ipc::pi_bootstrap,
            pi_ipc::pi_switch_session,
            pi_ipc::pi_new_session,
            pi_ipc::pi_set_session_name,
            pi_ipc::pi_get_tree,
            pi_ipc::pi_get_fork_messages,
            pi_ipc::pi_fork,
            pi_ipc::pi_clone,
            pi_ipc::pi_export_html,
            pi_ipc::pi_set_model,
            pi_ipc::pi_set_thinking_level,
            pi_ipc::pi_prompt,
            pi_ipc::pi_compact,
            pi_ipc::pi_steer,
            pi_ipc::pi_follow_up,
            pi_ipc::pi_clear_queue,
            pi_ipc::pi_abort,
            pi_ipc::pi_abort_retry,
            pi_ipc::pi_extension_ui_response,
            pi_ipc::pi_open_external_url,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Verity desktop");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = app_handle.try_state::<PiAppState>() {
                state.shutdown();
            }
        }
    });
}
