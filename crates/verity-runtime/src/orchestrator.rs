use crate::git::{
    changed_paths, commit_tree, commit_worktree_changes, committed_text, create_worktree,
    git_common_dir, integrate_branch, merge_tree, read_ref, remove_worktree, run_git, GitError,
    MergeOutcome,
};
use crate::handoff::{build_spec_handoff, candidate_spec_digest, RunLog};
use crate::pi_rpc::{run_sandboxed_pi_prompt, PiRpcConfig, PiRpcError};
use crate::sandbox::default_offline_config;
use crate::verifiers::{
    discover_kani_proofs, run_kani_proof_sandboxed, run_lean_comparator_sandboxed,
};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use verity_core::json::{escape_json_string, parse_json, JsonValue};
use verity_core::{
    can_approve, check_scope, CheckerVerdict, CommitOid, GateError, HygieneReport,
    HygieneViolation, RelPath, ScopeError, SpecDigest, SpecHandoff, TaskId, VerdictStatus,
    VerifierKind,
};

#[derive(Debug, Clone)]
pub struct WriteTaskRequest {
    pub repo_dir: PathBuf,
    pub task_id: TaskId,
    pub base_ref: String,
    pub write_scope: Vec<RelPath>,
    pub forbidden: Vec<RelPath>,
    pub verifier: VerifierKind,
    pub goal: String,
    pub pi_config: Option<PiRpcConfig>,
}

/// Optional execution controls for `spawn_write_task_with_options`, keeping
/// `WriteTaskRequest` backwards-compatible for existing CLI and desktop callers.
#[derive(Debug, Clone)]
pub struct WriteTaskOptions {
    /// Runs the controller's checker. Disabling it produces a review-only handoff.
    pub run_live_verifier: bool,
    /// Optional path to a Lean 4 challenge export file for
    /// `VerifierKind::Lean4Comparator`. Falls back to the source checkout's challenge.export.
    pub lean_challenge_export: Option<PathBuf>,
}

impl Default for WriteTaskOptions {
    fn default() -> Self {
        Self {
            run_live_verifier: true,
            lean_challenge_export: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedHandoff {
    pub task_id: TaskId,
    pub base_commit: CommitOid,
    pub verifier: VerifierKind,
    pub handoff: SpecHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeResult {
    Merged {
        commit: CommitOid,
        already_merged: bool,
    },
    Conflict {
        conflicted_paths: Vec<RelPath>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchestratorError {
    EmptyWriteScope,
    RefNotFound(String),
    TargetMoved,
    Git(GitError),
    PiRpc(PiRpcError),
    ScopeViolation(ScopeError),
    GateFailed(GateError),
    DigestMismatch { expected: String, actual: String },
    InvalidDigest(String),
    HandoffNotFound(PathBuf),
    CorruptHandoff(String),
    Io(String),
}

impl fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrchestratorError::EmptyWriteScope => {
                write!(f, "write task requires at least one write_scope path")
            }
            OrchestratorError::RefNotFound(r) => write!(f, "git ref not found: {}", r),
            OrchestratorError::TargetMoved => write!(f, "target branch differs from the verified base; rerun verification on the current target"),
            OrchestratorError::Git(e) => write!(f, "git error: {}", e),
            OrchestratorError::PiRpc(e) => write!(f, "pi rpc error: {}", e),
            OrchestratorError::ScopeViolation(e) => write!(f, "scope violation: {}", e),
            OrchestratorError::GateFailed(e) => write!(f, "gate failed: {}", e),
            OrchestratorError::DigestMismatch { expected, actual } => write!(
                f,
                "approved digest mismatch: expected {}, got {}",
                expected, actual
            ),
            OrchestratorError::InvalidDigest(msg) => write!(f, "invalid spec digest: {}", msg),
            OrchestratorError::HandoffNotFound(p) => {
                write!(f, "persisted handoff not found at {}", p.display())
            }
            OrchestratorError::CorruptHandoff(msg) => write!(f, "corrupt handoff json: {}", msg),
            OrchestratorError::Io(msg) => write!(f, "io error: {}", msg),
        }
    }
}

impl std::error::Error for OrchestratorError {}

impl From<GitError> for OrchestratorError {
    fn from(err: GitError) -> Self {
        OrchestratorError::Git(err)
    }
}

impl From<PiRpcError> for OrchestratorError {
    fn from(err: PiRpcError) -> Self {
        OrchestratorError::PiRpc(err)
    }
}

/// Ensures the isolated git worktree is removed on every return path.
struct WorktreeGuard<'a> {
    repo_dir: &'a Path,
    worktree_path: PathBuf,
}

impl Drop for WorktreeGuard<'_> {
    fn drop(&mut self) {
        let _ = remove_worktree(self.repo_dir, &self.worktree_path);
    }
}

impl<'a> WorktreeGuard<'a> {
    fn create(
        repo_dir: &'a Path,
        worktree_path: PathBuf,
        task_id: &TaskId,
        commit: &CommitOid,
    ) -> Result<Self, GitError> {
        create_worktree(repo_dir, &worktree_path, task_id, commit)?;
        Ok(Self {
            repo_dir,
            worktree_path,
        })
    }
}

pub fn worktree_path_for(repo_dir: &Path, task_id: &TaskId) -> PathBuf {
    repo_dir
        .join(".verity")
        .join("worktrees")
        .join(task_id.as_str())
}

pub fn handoff_path_for(repo_dir: &Path, task_id: &TaskId) -> PathBuf {
    repo_dir
        .join(".verity")
        .join("handoffs")
        .join(format!("{}.json", task_id.as_str()))
}

pub fn parse_verifier_kind(raw: &str) -> Result<VerifierKind, String> {
    match raw {
        "kani" | "rust-kani" | "rust" => Ok(VerifierKind::RustKani),
        "lean" | "lean4" | "lean4-comparator" => Ok(VerifierKind::Lean4Comparator),
        other => Err(format!(
            "unsupported verifier '{}' (expected 'kani' or 'lean')",
            other
        )),
    }
}

pub fn verifier_kind_str(verifier: VerifierKind) -> &'static str {
    match verifier {
        VerifierKind::RustKani => "kani",
        VerifierKind::Lean4Comparator => "lean",
    }
}

pub fn parse_spec_digest_hex(hex: &str) -> Result<SpecDigest, OrchestratorError> {
    if hex.len() != 64 {
        return Err(OrchestratorError::InvalidDigest(format!(
            "expected 64 lowercase hex chars, got {}",
            hex.len()
        )));
    }
    let bytes = hex.as_bytes();
    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = decode_hex_nibble(bytes[i * 2])?;
        let lo = decode_hex_nibble(bytes[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(SpecDigest::from_bytes(out))
}

fn decode_hex_nibble(b: u8) -> Result<u8, OrchestratorError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => Err(OrchestratorError::InvalidDigest(
            "digest must be lowercase hex".to_string(),
        )),
    }
}

fn verdict_status_str(status: VerdictStatus) -> &'static str {
    match status {
        VerdictStatus::Pass => "pass",
        VerdictStatus::Counterexample => "counterexample",
        VerdictStatus::Inconclusive => "inconclusive",
        VerdictStatus::InvalidReport => "invalid_report",
    }
}

fn parse_verdict_status(raw: &str) -> Result<VerdictStatus, OrchestratorError> {
    match raw {
        "pass" => Ok(VerdictStatus::Pass),
        "counterexample" => Ok(VerdictStatus::Counterexample),
        "inconclusive" => Ok(VerdictStatus::Inconclusive),
        "invalid_report" => Ok(VerdictStatus::InvalidReport),
        other => Err(OrchestratorError::CorruptHandoff(format!(
            "unknown verdict status '{}'",
            other
        ))),
    }
}

/// Serializes a `PersistedHandoff` to deterministic JSON without external deps.
pub fn handoff_to_json(record: &PersistedHandoff) -> String {
    let h = &record.handoff;
    let paths_json: Vec<String> = h
        .changed_paths
        .iter()
        .map(|p| escape_json_string(p.as_str()))
        .collect();
    let violations_json: Vec<String> = h
        .hygiene
        .violations
        .iter()
        .map(|v| {
            format!(
                "{{\"rule\":{},\"path\":{},\"line\":{},\"detail\":{}}}",
                escape_json_string(&v.rule),
                escape_json_string(&v.path),
                v.line,
                escape_json_string(&v.detail)
            )
        })
        .collect();
    let back_translation_json = match &h.back_translation {
        Some(text) => escape_json_string(text),
        None => "null".to_string(),
    };

    format!(
        "{{\"task_id\":{},\"base_commit\":{},\"verifier\":{},\"candidate_commit\":{},\
         \"changed_paths\":[{}],\"extracted_spec\":{},\"back_translation\":{},\
         \"hygiene\":{{\"clean\":{},\"violations\":[{}]}},\
         \"verdict\":{{\"status\":{},\"reason\":{},\"checks_passed\":{},\"probes_satisfied\":{}}},\
         \"spec_digest\":{}}}",
        escape_json_string(record.task_id.as_str()),
        escape_json_string(record.base_commit.as_str()),
        escape_json_string(verifier_kind_str(record.verifier)),
        escape_json_string(h.candidate_commit.as_str()),
        paths_json.join(","),
        escape_json_string(&h.extracted_spec),
        back_translation_json,
        h.hygiene.is_clean(),
        violations_json.join(","),
        escape_json_string(verdict_status_str(h.verdict.status)),
        escape_json_string(&h.verdict.reason),
        h.verdict.checks_passed,
        h.verdict.probes_satisfied,
        escape_json_string(&h.spec_digest.to_hex()),
    )
}

fn require_str<'a>(obj: &'a JsonValue, key: &str) -> Result<&'a str, OrchestratorError> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| OrchestratorError::CorruptHandoff(format!("missing string field '{}'", key)))
}

fn require_usize(obj: &JsonValue, key: &str) -> Result<usize, OrchestratorError> {
    match obj.get(key) {
        Some(JsonValue::Number(n)) if *n >= 0.0 && n.fract() == 0.0 => Ok(*n as usize),
        _ => Err(OrchestratorError::CorruptHandoff(format!(
            "missing integer field '{}'",
            key
        ))),
    }
}

/// Parses a persisted handoff JSON document into validated domain types.
pub fn parse_handoff_json(raw: &str) -> Result<PersistedHandoff, OrchestratorError> {
    let root = parse_json(raw).map_err(|e| OrchestratorError::CorruptHandoff(e.to_string()))?;
    let task_id = TaskId::parse(require_str(&root, "task_id")?)
        .map_err(|e| OrchestratorError::CorruptHandoff(e.to_string()))?;
    let base_commit = CommitOid::parse(require_str(&root, "base_commit")?)
        .map_err(|e| OrchestratorError::CorruptHandoff(e.to_string()))?;
    let verifier = parse_verifier_kind(require_str(&root, "verifier")?)
        .map_err(OrchestratorError::CorruptHandoff)?;
    let candidate_commit = CommitOid::parse(require_str(&root, "candidate_commit")?)
        .map_err(|e| OrchestratorError::CorruptHandoff(e.to_string()))?;

    let paths_arr = root
        .get("changed_paths")
        .and_then(|v| v.as_array())
        .ok_or_else(|| OrchestratorError::CorruptHandoff("missing changed_paths array".into()))?;
    let mut changed_paths = Vec::with_capacity(paths_arr.len());
    for item in paths_arr {
        let s = item.as_str().ok_or_else(|| {
            OrchestratorError::CorruptHandoff("non-string entry in changed_paths".into())
        })?;
        let rel =
            RelPath::parse(s).map_err(|e| OrchestratorError::CorruptHandoff(e.to_string()))?;
        changed_paths.push(rel);
    }

    let extracted_spec = require_str(&root, "extracted_spec")?.to_string();
    let back_translation = match root.get("back_translation") {
        Some(JsonValue::String(s)) => Some(s.clone()),
        Some(JsonValue::Null) | None => None,
        _ => {
            return Err(OrchestratorError::CorruptHandoff(
                "invalid back_translation field".into(),
            ))
        }
    };

    let hygiene_obj = root
        .get("hygiene")
        .ok_or_else(|| OrchestratorError::CorruptHandoff("missing hygiene object".into()))?;
    let viol_arr = hygiene_obj
        .get("violations")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            OrchestratorError::CorruptHandoff("missing hygiene.violations array".into())
        })?;
    let mut violations = Vec::with_capacity(viol_arr.len());
    for v in viol_arr {
        violations.push(HygieneViolation {
            rule: require_str(v, "rule")?.to_string(),
            path: require_str(v, "path")?.to_string(),
            line: require_usize(v, "line")?,
            detail: require_str(v, "detail")?.to_string(),
        });
    }

    let verdict_obj = root
        .get("verdict")
        .ok_or_else(|| OrchestratorError::CorruptHandoff("missing verdict object".into()))?;
    let mut verdict = CheckerVerdict {
        status: parse_verdict_status(require_str(verdict_obj, "status")?)?,
        reason: require_str(verdict_obj, "reason")?.to_string(),
        checks_passed: require_usize(verdict_obj, "checks_passed")?,
        probes_satisfied: require_usize(verdict_obj, "probes_satisfied")?,
    };

    let spec_digest = parse_spec_digest_hex(require_str(&root, "spec_digest")?)
        .map_err(|e| OrchestratorError::CorruptHandoff(e.to_string()))?;

    if candidate_spec_digest(&candidate_commit, &extracted_spec) != spec_digest {
        verdict = inconclusive("handoff is missing a current candidate-bound approval digest");
    }

    Ok(PersistedHandoff {
        task_id,
        base_commit,
        verifier,
        handoff: SpecHandoff {
            candidate_commit,
            changed_paths,
            extracted_spec,
            back_translation,
            hygiene: HygieneReport { violations },
            verdict,
            spec_digest,
        },
    })
}

/// Atomically writes a persisted handoff JSON file via temp-file + rename so
/// interrupted runs never leave half-written records.
fn write_handoff_atomic(
    repo_dir: &Path,
    record: &PersistedHandoff,
) -> Result<PathBuf, OrchestratorError> {
    let target = handoff_path_for(repo_dir, &record.task_id);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| OrchestratorError::Io(e.to_string()))?;
    }
    let tmp = target.with_extension(format!("json.tmp.{}", std::process::id()));
    let json = handoff_to_json(record);
    fs::write(&tmp, json.as_bytes()).map_err(|e| OrchestratorError::Io(e.to_string()))?;
    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        OrchestratorError::Io(e.to_string())
    })?;
    Ok(target)
}

fn inconclusive(reason: impl Into<String>) -> CheckerVerdict {
    CheckerVerdict {
        status: VerdictStatus::Inconclusive,
        reason: reason.into(),
        checks_passed: 0,
        probes_satisfied: 0,
    }
}

/// Executes the configured live machine checker inside the isolated worktree
/// before worktree teardown, failing closed on missing targets or missing tools.
fn execute_live_verifier(
    worktree_dir: &Path,
    verifier: VerifierKind,
    changed_files: &[(RelPath, String)],
    lean_challenge_export: &Path,
) -> CheckerVerdict {
    match verifier {
        VerifierKind::RustKani => {
            let targets = discover_kani_proofs(changed_files);
            if targets.is_empty() {
                return CheckerVerdict {
                    status: VerdictStatus::InvalidReport,
                    reason: "no #[kani::proof] harness discovered in changed Rust files"
                        .to_string(),
                    checks_passed: 0,
                    probes_satisfied: 0,
                };
            }
            let mut result = CheckerVerdict {
                status: VerdictStatus::Pass,
                reason: format!("Kani passed all {} discovered harnesses", targets.len()),
                checks_passed: 0,
                probes_satisfied: 0,
            };
            for target in &targets {
                match run_kani_proof_sandboxed(worktree_dir, target) {
                    Ok(verdict) if verdict.status == VerdictStatus::Pass => {
                        result.checks_passed += verdict.checks_passed;
                        result.probes_satisfied += verdict.probes_satisfied;
                    }
                    Ok(verdict) => return verdict,
                    Err(err) => {
                        return inconclusive(format!("Kani check for '{}': {err}", target.name))
                    }
                }
            }
            result
        }
        VerifierKind::Lean4Comparator => {
            if !lean_challenge_export.is_file() {
                return inconclusive("no controller challenge.export configured for Lean 4");
            }
            match run_lean_comparator_sandboxed(worktree_dir, lean_challenge_export) {
                Ok(verdict) => verdict,
                Err(err) => inconclusive(format!("Lean comparator: {err}")),
            }
        }
    }
}

/// Runs the full write-subagent pipeline with controller verification enabled.
pub fn spawn_write_task(req: WriteTaskRequest) -> Result<PersistedHandoff, OrchestratorError> {
    spawn_write_task_with_options(req, WriteTaskOptions::default())
}

/// Runs the full write-subagent pipeline:
/// 1. Creates a locked detached git worktree at `<repo>/.verity/worktrees/<task_id>`.
/// 2. Runs the Pi RPC worker inside that worktree.
/// 3. Commits candidate changes and verifies the diff against `verity_core::check_scope`.
/// 4. Reads committed files, optionally runs the live Kani or Lean 4 verifier in a fresh
///    checkout of the candidate, builds the spec-only `SpecHandoff` (running
///    hygiene lints), atomically persists `<repo>/.verity/handoffs/<task_id>.json`, and logs
///    to `RunLog`.
/// 5. Cleans up the worktree on every exit path.
pub fn spawn_write_task_with_options(
    mut req: WriteTaskRequest,
    options: WriteTaskOptions,
) -> Result<PersistedHandoff, OrchestratorError> {
    if req.write_scope.is_empty() {
        return Err(OrchestratorError::EmptyWriteScope);
    }
    req.repo_dir =
        std::path::absolute(&req.repo_dir).map_err(|err| OrchestratorError::Io(err.to_string()))?;

    let base_commit = read_ref(&req.repo_dir, &format!("{}^{{commit}}", req.base_ref))?
        .ok_or_else(|| OrchestratorError::RefNotFound(req.base_ref.clone()))?;

    let worker = WorktreeGuard::create(
        &req.repo_dir,
        worktree_path_for(&req.repo_dir, &req.task_id),
        &req.task_id,
        &base_commit,
    )?;
    let wt_path = &worker.worktree_path;

    let mut pi_cfg = req
        .pi_config
        .unwrap_or_else(|| PiRpcConfig::default_pi(wt_path.clone(), false));
    pi_cfg.cwd = wt_path.clone();

    let mut sandbox = default_offline_config(wt_path, Some(git_common_dir(&req.repo_dir)?));
    sandbox.allow_network = true;
    sandbox.env_passthrough = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "OPENROUTER_API_KEY",
    ]
    .into_iter()
    .filter_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| (name.to_string(), value))
    })
    .collect();
    let pi_res = run_sandboxed_pi_prompt(&pi_cfg, &sandbox, req.task_id.as_str(), &req.goal)?;

    let commit_msg = format!("verity candidate {}", req.task_id.as_str());
    let candidate_commit = commit_worktree_changes(wt_path, &commit_msg)?;

    let diff_paths = changed_paths(&req.repo_dir, &base_commit, &candidate_commit)?;
    if let Err(scope_err) = check_scope(&diff_paths, &req.write_scope, &req.forbidden) {
        if let Ok(log) = RunLog::open(&req.repo_dir) {
            let _ = log.append_event(
                req.task_id.as_str(),
                "rejected_scope",
                &scope_err.to_string(),
            );
        }
        return Err(OrchestratorError::ScopeViolation(scope_err));
    }

    let mut changed_files = Vec::with_capacity(diff_paths.len());
    for rel in &diff_paths {
        let content = committed_text(&req.repo_dir, &candidate_commit, rel)?;
        changed_files.push((rel.clone(), content));
    }

    let verdict = if !pi_res.settled {
        inconclusive("worker exited before emitting agent_settled")
    } else if options.run_live_verifier {
        let verifier = WorktreeGuard::create(
            &req.repo_dir,
            req.repo_dir
                .join(".verity/checker-worktrees")
                .join(req.task_id.as_str()),
            &req.task_id,
            &candidate_commit,
        )?;
        let default_export = req.repo_dir.join("challenge.export");
        execute_live_verifier(
            &verifier.worktree_path,
            req.verifier,
            &changed_files,
            options
                .lean_challenge_export
                .as_deref()
                .unwrap_or(&default_export),
        )
    } else {
        inconclusive("controller verification was disabled")
    };

    let back_translation = if pi_res.final_text.trim().is_empty() {
        None
    } else {
        Some(pi_res.final_text)
    };

    let handoff = build_spec_handoff(
        candidate_commit,
        &changed_files,
        req.verifier,
        verdict,
        back_translation,
    );

    let record = PersistedHandoff {
        task_id: req.task_id.clone(),
        base_commit,
        verifier: req.verifier,
        handoff,
    };

    write_handoff_atomic(&req.repo_dir, &record)?;

    if let Ok(log) = RunLog::open(&req.repo_dir) {
        let _ = log.append_event(
            record.task_id.as_str(),
            "handoff_ready",
            &record.handoff.spec_digest.to_hex(),
        );
    }

    Ok(record)
}

/// Loads a persisted `SpecHandoff` record from `<repo>/.verity/handoffs/<task_id>.json`.
pub fn load_handoff(
    repo_dir: &Path,
    task_id: &TaskId,
) -> Result<PersistedHandoff, OrchestratorError> {
    let path = handoff_path_for(repo_dir, task_id);
    if !path.exists() {
        return Err(OrchestratorError::HandoffNotFound(path));
    }
    let raw = fs::read_to_string(&path).map_err(|e| OrchestratorError::Io(e.to_string()))?;
    let record = parse_handoff_json(&raw)?;
    if record.task_id != *task_id {
        return Err(OrchestratorError::CorruptHandoff(
            "task ID does not match the handoff filename".into(),
        ));
    }
    Ok(record)
}

fn is_ancestor(
    repo_dir: &Path,
    ancestor: &CommitOid,
    descendant: &CommitOid,
) -> Result<bool, GitError> {
    let out = run_git(
        repo_dir,
        &[
            "merge-base",
            "--is-ancestor",
            ancestor.as_str(),
            descendant.as_str(),
        ],
    )?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        code => Err(GitError::CommandFailed {
            args: vec!["merge-base".to_string(), "--is-ancestor".to_string()],
            code,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }),
    }
}

/// Merges the approved candidate if the target still points to the verified base.
/// A repeat approval succeeds when the target already includes the candidate.
pub fn approve_and_merge(
    repo_dir: &Path,
    task_id: &TaskId,
    target_ref: &str,
    approved_digest: &str,
) -> Result<MergeResult, OrchestratorError> {
    let record = load_handoff(repo_dir, task_id)?;
    let approved = parse_spec_digest_hex(approved_digest)?;

    if record.handoff.spec_digest != approved {
        return Err(OrchestratorError::DigestMismatch {
            expected: record.handoff.spec_digest.to_hex(),
            actual: approved_digest.to_string(),
        });
    }

    can_approve(&record.handoff).map_err(OrchestratorError::GateFailed)?;

    let target_ref = if target_ref.starts_with("refs/heads/") {
        target_ref.to_string()
    } else {
        format!("refs/heads/{target_ref}")
    };
    if !run_git(repo_dir, &["check-ref-format", &target_ref])?
        .status
        .success()
    {
        return Err(OrchestratorError::RefNotFound(target_ref));
    }
    let target_oid = read_ref(repo_dir, &target_ref)?
        .ok_or_else(|| OrchestratorError::RefNotFound(target_ref.clone()))?;

    if is_ancestor(repo_dir, &record.handoff.candidate_commit, &target_oid)? {
        return Ok(MergeResult::Merged {
            commit: target_oid,
            already_merged: true,
        });
    }

    if target_oid != record.base_commit {
        return Err(OrchestratorError::TargetMoved);
    }

    match merge_tree(repo_dir, &target_oid, &record.handoff.candidate_commit)? {
        MergeOutcome::Clean { tree_oid } => {
            let msg = format!("verity merge {}", task_id.as_str());
            let merged = commit_tree(
                repo_dir,
                &tree_oid,
                &[&target_oid, &record.handoff.candidate_commit],
                &msg,
            )?;
            integrate_branch(repo_dir, &target_ref, &merged, &target_oid)?;
            if let Ok(log) = RunLog::open(repo_dir) {
                let _ = log.append_event(task_id.as_str(), "merged", merged.as_str());
            }
            Ok(MergeResult::Merged {
                commit: merged,
                already_merged: false,
            })
        }
        MergeOutcome::Conflict { conflicted_paths } => {
            if let Ok(log) = RunLog::open(repo_dir) {
                let joined: Vec<&str> = conflicted_paths.iter().map(RelPath::as_str).collect();
                let _ = log.append_event(task_id.as_str(), "merge_conflict", &joined.join(","));
            }
            Ok(MergeResult::Conflict { conflicted_paths })
        }
    }
}
