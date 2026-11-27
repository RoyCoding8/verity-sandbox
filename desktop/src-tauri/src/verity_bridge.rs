use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use verity_core::{RelPath, TaskId, VerdictStatus};
use verity_runtime::handoff::RunLog;
use verity_runtime::orchestrator::{
    approve_and_merge, load_handoff, parse_verifier_kind, spawn_write_task, verifier_kind_str,
    MergeResult, PersistedHandoff, WriteTaskRequest,
};
use verity_runtime::pi_rpc::{run_readonly_pi_prompt, PiRpcConfig};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HygieneViolationView {
    pub rule: String,
    pub path: String,
    pub line: usize,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerdictView {
    pub status: String,
    pub reason: String,
    pub checks_passed: usize,
    pub probes_satisfied: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HandoffView {
    pub task_id: String,
    pub base_commit: String,
    pub verifier: String,
    pub candidate_commit: String,
    pub changed_paths: Vec<String>,
    pub extracted_spec: String,
    pub back_translation: Option<String>,
    pub hygiene_clean: bool,
    pub hygiene_violations: Vec<HygieneViolationView>,
    pub verdict: VerdictView,
    pub spec_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadOnlyResponse {
    pub summary: String,
    pub tool_calls: usize,
    pub settled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergeResponse {
    pub status: String,
    pub commit: Option<String>,
    pub already_merged: bool,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpawnWriteTaskInput {
    pub repo_path: String,
    pub task_id: String,
    pub base_ref: String,
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub forbidden: Vec<String>,
    pub verifier: String,
    pub goal: String,
}

fn verdict_status_str(status: VerdictStatus) -> &'static str {
    match status {
        VerdictStatus::Pass => "pass",
        VerdictStatus::Counterexample => "counterexample",
        VerdictStatus::Inconclusive => "inconclusive",
        VerdictStatus::InvalidReport => "invalid_report",
    }
}

impl From<PersistedHandoff> for HandoffView {
    fn from(record: PersistedHandoff) -> Self {
        let h = record.handoff;
        let hygiene_clean = h.hygiene.is_clean();
        Self {
            task_id: record.task_id.as_str().to_string(),
            base_commit: record.base_commit.as_str().to_string(),
            verifier: verifier_kind_str(record.verifier).to_string(),
            candidate_commit: h.candidate_commit.as_str().to_string(),
            changed_paths: h
                .changed_paths
                .into_iter()
                .map(|p| p.as_str().to_string())
                .collect(),
            extracted_spec: h.extracted_spec,
            back_translation: h.back_translation,
            hygiene_clean,
            hygiene_violations: h
                .hygiene
                .violations
                .into_iter()
                .map(|v| HygieneViolationView {
                    rule: v.rule,
                    path: v.path,
                    line: v.line,
                    detail: v.detail,
                })
                .collect(),
            verdict: VerdictView {
                status: verdict_status_str(h.verdict.status).to_string(),
                reason: h.verdict.reason,
                checks_passed: h.verdict.checks_passed,
                probes_satisfied: h.verdict.probes_satisfied,
            },
            spec_digest: h.spec_digest.to_hex(),
        }
    }
}

pub fn load_handoff_view(repo_dir: &Path, task_id: &str) -> Result<HandoffView, String> {
    let parsed_id = TaskId::parse(task_id).map_err(|e| e.to_string())?;
    let record = load_handoff(repo_dir, &parsed_id).map_err(|e| e.to_string())?;
    Ok(HandoffView::from(record))
}

/// Lists all persisted handoff JSON files under `<repo>/.verity/handoffs/`,
/// ignoring stale temp files or non-`.json` artifacts.
pub fn list_persisted_handoffs(repo_dir: &Path) -> Result<Vec<HandoffView>, String> {
    let dir = repo_dir.join(".verity").join("handoffs");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&dir)
        .map_err(|e| format!("failed to read handoffs directory {}: {}", dir.display(), e))?;

    let mut handoffs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let task_id = TaskId::parse(stem).map_err(|e| e.to_string())?;
        let record = load_handoff(repo_dir, &task_id).map_err(|e| e.to_string())?;
        handoffs.push(HandoffView::from(record));
    }

    handoffs.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    Ok(handoffs)
}

pub fn spawn_write_task_view(input: SpawnWriteTaskInput) -> Result<HandoffView, String> {
    let task_id = TaskId::parse(&input.task_id).map_err(|e| e.to_string())?;
    let verifier = parse_verifier_kind(&input.verifier)?;
    let write_scope: Vec<RelPath> = input
        .write_scope
        .iter()
        .map(|p| RelPath::parse(p).map_err(|e| format!("invalid write_scope '{}': {}", p, e)))
        .collect::<Result<_, _>>()?;
    let forbidden: Vec<RelPath> = input
        .forbidden
        .iter()
        .map(|p| RelPath::parse(p).map_err(|e| format!("invalid forbidden '{}': {}", p, e)))
        .collect::<Result<_, _>>()?;

    let req = WriteTaskRequest {
        repo_dir: PathBuf::from(input.repo_path),
        task_id,
        base_ref: input.base_ref,
        write_scope,
        forbidden,
        verifier,
        goal: input.goal,
        pi_config: None,
    };

    let record = spawn_write_task(req).map_err(|e| e.to_string())?;
    Ok(HandoffView::from(record))
}

pub fn approve_and_merge_view(
    repo_dir: &Path,
    task_id: &str,
    target_ref: &str,
    approved_digest: &str,
) -> Result<MergeResponse, String> {
    let parsed_id = TaskId::parse(task_id).map_err(|e| e.to_string())?;
    match approve_and_merge(repo_dir, &parsed_id, target_ref, approved_digest)
        .map_err(|e| e.to_string())?
    {
        MergeResult::Merged {
            commit,
            already_merged,
        } => Ok(MergeResponse {
            status: "merged".to_string(),
            commit: Some(commit.as_str().to_string()),
            already_merged,
            conflicts: Vec::new(),
        }),
        MergeResult::Conflict { conflicted_paths } => Ok(MergeResponse {
            status: "conflict".to_string(),
            commit: None,
            already_merged: false,
            conflicts: conflicted_paths
                .into_iter()
                .map(|p| p.as_str().to_string())
                .collect(),
        }),
    }
}

#[tauri::command]
pub async fn list_run_log(repo_path: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let log = RunLog::open(&PathBuf::from(repo_path)).map_err(|e| e.to_string())?;
        log.read_lines().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn run_readonly_subagent(
    repo_path: String,
    goal: String,
) -> Result<ReadOnlyResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = PathBuf::from(&repo_path);
        let cfg = PiRpcConfig::default_pi(repo.clone(), true);
        let res =
            run_readonly_pi_prompt(&cfg, "desktop-readonly", &goal).map_err(|e| e.to_string())?;
        if let Ok(log) = RunLog::open(&repo) {
            let _ = log.append_event("desktop-readonly", "completed_readonly", &res.final_text);
        }
        Ok(ReadOnlyResponse {
            summary: res.final_text,
            tool_calls: res.tool_calls,
            settled: res.settled,
        })
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn spawn_write_subagent(input: SpawnWriteTaskInput) -> Result<HandoffView, String> {
    tauri::async_runtime::spawn_blocking(move || spawn_write_task_view(input))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn list_handoffs(repo_path: String) -> Result<Vec<HandoffView>, String> {
    tauri::async_runtime::spawn_blocking(move || list_persisted_handoffs(&PathBuf::from(repo_path)))
        .await
        .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn get_handoff(repo_path: String, task_id: String) -> Result<HandoffView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        load_handoff_view(&PathBuf::from(repo_path), &task_id)
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[tauri::command]
pub async fn approve_and_merge_handoff(
    repo_path: String,
    task_id: String,
    target_ref: String,
    approved_digest: String,
) -> Result<MergeResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        approve_and_merge_view(
            &PathBuf::from(repo_path),
            &task_id,
            &target_ref,
            &approved_digest,
        )
    })
    .await
    .map_err(|e| format!("blocking task failed: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};
    use verity_core::{CheckerVerdict, CommitOid, VerifierKind};
    use verity_runtime::handoff::build_spec_handoff;
    use verity_runtime::orchestrator::{handoff_path_for, handoff_to_json};

    fn unique_temp_repo(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "verity-bridge-{}-{}-{}",
            label,
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_sample_handoff(repo_dir: &Path, task_id: &str, path: &str, code: &str) -> String {
        let id = TaskId::parse(task_id).unwrap();
        let base = CommitOid::parse(&"1".repeat(40)).unwrap();
        let candidate = CommitOid::parse(&"2".repeat(40)).unwrap();
        let rel = RelPath::parse(path).unwrap();
        let handoff = build_spec_handoff(
            candidate,
            &[(rel, code.to_string())],
            VerifierKind::RustKani,
            CheckerVerdict {
                status: VerdictStatus::Pass,
                reason: "all checks passed".to_string(),
                checks_passed: 2,
                probes_satisfied: 1,
            },
            Some("Safe addition checked.".to_string()),
        );
        let digest = handoff.spec_digest.to_hex();
        let record = PersistedHandoff {
            task_id: id.clone(),
            base_commit: base,
            verifier: VerifierKind::RustKani,
            handoff,
        };
        let target = handoff_path_for(repo_dir, &id);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, handoff_to_json(&record)).unwrap();
        digest
    }

    #[test]
    fn lists_and_loads_persisted_handoffs_and_ignores_temp_files() {
        let repo = unique_temp_repo("handoffs");

        // Empty repo without .verity/handoffs returns empty list.
        assert!(list_persisted_handoffs(&repo).unwrap().is_empty());

        let digest_b = write_sample_handoff(
            &repo,
            "task-b",
            "src/b.rs",
            "pub fn b() -> u32 { 1 }\nassert!(true);\n",
        );
        let digest_a = write_sample_handoff(
            &repo,
            "task-a",
            "src/a.rs",
            "pub fn a() -> u32 { 2 }\nkani::cover!(true);\n",
        );

        // Stale temp file from an interrupted write must be ignored.
        let stale_tmp = repo
            .join(".verity")
            .join("handoffs")
            .join("task-c.json.tmp.9999");
        fs::write(&stale_tmp, "{half written").unwrap();

        let listed = list_persisted_handoffs(&repo).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].task_id, "task-a");
        assert_eq!(listed[0].spec_digest, digest_a);
        assert_eq!(listed[1].task_id, "task-b");
        assert_eq!(listed[1].spec_digest, digest_b);

        let single = load_handoff_view(&repo, "task-a").unwrap();
        assert_eq!(single.task_id, "task-a");
        assert_eq!(single.verifier, "kani");
        assert!(single.hygiene_clean);
        assert_eq!(single.verdict.status, "pass");
        assert_eq!(single.changed_paths, vec!["src/a.rs"]);
        assert_eq!(
            single.back_translation.as_deref(),
            Some("Safe addition checked.")
        );

        // Invalid task IDs are rejected at the boundary.
        assert!(load_handoff_view(&repo, "../escape").is_err());

        let _ = fs::remove_dir_all(&repo);
    }
}
