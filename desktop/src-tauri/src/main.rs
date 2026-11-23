#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod providers;
mod verity_bridge;

fn main() {
    tauri::Builder::default()
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Verity desktop");
}
