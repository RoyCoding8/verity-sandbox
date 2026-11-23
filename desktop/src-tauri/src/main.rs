#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use std::path::PathBuf;
use verity_core::CommitOid;
use verity_runtime::git::{commit_tree, merge_tree, update_ref_cas, MergeOutcome};
use verity_runtime::handoff::RunLog;
use verity_runtime::pi_rpc::{run_pi_prompt, PiRpcConfig};

#[derive(Serialize)]
struct ReadOnlyResponse {
    summary: String,
    tool_calls: usize,
    settled: bool,
}

#[derive(Serialize)]
struct MergeResponse {
    status: String,
    commit: Option<String>,
    conflicts: Vec<String>,
}

#[tauri::command]
fn list_run_log(repo_path: String) -> Result<Vec<String>, String> {
    let log = RunLog::open(&PathBuf::from(repo_path)).map_err(|e| e.to_string())?;
    log.read_lines().map_err(|e| e.to_string())
}

#[tauri::command]
fn run_readonly_subagent(repo_path: String, goal: String) -> Result<ReadOnlyResponse, String> {
    let repo = PathBuf::from(&repo_path);
    let cfg = PiRpcConfig::default_pi(repo.clone(), true);
    let res = run_pi_prompt(&cfg, "desktop-readonly", &goal).map_err(|e| e.to_string())?;
    if let Ok(log) = RunLog::open(&repo) {
        let _ = log.append_event("desktop-readonly", "completed_readonly", &res.final_text);
    }
    Ok(ReadOnlyResponse {
        summary: res.final_text,
        tool_calls: res.tool_calls,
        settled: res.settled,
    })
}

#[tauri::command]
fn merge_approved_handoff(
    repo_path: String,
    ref_name: String,
    base_oid: String,
    candidate_oid: String,
    approved_digest: String,
    verified_digest: String,
) -> Result<MergeResponse, String> {
    if approved_digest != verified_digest {
        return Err("approved digest does not match verified integration digest".to_string());
    }
    let repo = PathBuf::from(&repo_path);
    let base = CommitOid::parse(&base_oid).map_err(|e| e.to_string())?;
    let candidate = CommitOid::parse(&candidate_oid).map_err(|e| e.to_string())?;
    match merge_tree(&repo, &base, &candidate).map_err(|e| e.to_string())? {
        MergeOutcome::Clean { tree_oid } => {
            let merged = commit_tree(&repo, &tree_oid, &[&base, &candidate], "verity merge")
                .map_err(|e| e.to_string())?;
            update_ref_cas(&repo, &ref_name, &merged, Some(&base)).map_err(|e| e.to_string())?;
            if let Ok(log) = RunLog::open(&repo) {
                let _ = log.append_event("desktop-merge", "merged", merged.as_str());
            }
            Ok(MergeResponse {
                status: "merged".to_string(),
                commit: Some(merged.as_str().to_string()),
                conflicts: Vec::new(),
            })
        }
        MergeOutcome::Conflict { conflicted_paths } => Ok(MergeResponse {
            status: "conflict".to_string(),
            commit: None,
            conflicts: conflicted_paths
                .into_iter()
                .map(|p| p.as_str().to_string())
                .collect(),
        }),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_run_log,
            run_readonly_subagent,
            merge_approved_handoff
        ])
        .run(tauri::generate_context!())
        .expect("error while running Verity desktop");
}
