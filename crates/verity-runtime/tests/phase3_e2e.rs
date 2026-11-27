use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use verity_core::{
    CheckerVerdict, CommitOid, GateError, RelPath, ScopeError, TaskId, VerdictStatus, VerifierKind,
};
use verity_runtime::git::{
    commit_tree, commit_worktree_changes, create_worktree, read_ref, remove_worktree, run_git,
    update_ref_cas,
};
use verity_runtime::handoff::{build_spec_handoff, extract_spec_only, RunLog};
use verity_runtime::orchestrator::{
    approve_and_merge, handoff_path_for, handoff_to_json, load_handoff, spawn_write_task,
    spawn_write_task_with_options, worktree_path_for, MergeResult, OrchestratorError,
    PersistedHandoff, WriteTaskOptions, WriteTaskRequest,
};
use verity_runtime::pi_rpc::{run_pi_prompt, PiRpcConfig, PiRpcError};
use verity_runtime::sandbox::pi_sandbox_available;
use verity_runtime::verifiers::discover_kani_targets;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("verity_p3_{}_{}", label, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_git_repo(dir: &Path) {
    assert!(Command::new("git")
        .current_dir(dir)
        .args(["init", "-b", "main"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .current_dir(dir)
        .args(["config", "user.name", "Test"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .current_dir(dir)
        .args(["config", "user.email", "test@local"])
        .status()
        .unwrap()
        .success());
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn base() -> u32 { 0 }\n").unwrap();
    fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    assert!(Command::new("git")
        .current_dir(dir)
        .args(["add", "-A"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .current_dir(dir)
        .args(["commit", "-m", "initial"])
        .status()
        .unwrap()
        .success());
}

fn mock_pi_config(script: &str) -> PiRpcConfig {
    PiRpcConfig {
        program: "sh".to_string(),
        args: vec!["-c".to_string(), script.to_string()],
        cwd: PathBuf::new(),
        timeout: Duration::from_secs(10),
        max_tool_calls: 16,
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Kani 0.68.0 launchers and working Linux bubblewrap"]
fn live_kani_approves_standalone_and_cargo_candidates() {
    assert!(pi_sandbox_available(), "Pi sandbox is required");
    assert!(std::env::var_os("VERITY_KANI").is_some());
    assert!(std::env::var_os("VERITY_CARGO_KANI").is_some());
    let repo = temp_dir("live_kani");
    init_git_repo(&repo);
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    commit_worktree_changes(&repo, "configure fixture package").unwrap();

    for (id, path, offset) in [
        ("standalone", "proof.rs", 0),
        ("cargo", "src/lib.rs", 0),
        ("counterexample", "proof.rs", 1),
    ] {
        let task_id = TaskId::parse(id).unwrap();
        let source = format!(
            r#"mod proofs {{
    #[kani::proof]
    fn verify_addition_preserves_input() {{
        let n: u8 = kani::any();
        assert!(n.saturating_add({offset}) == n);
        kani::cover!(n == 42, "forty_two");
    }}
}}
"#
        );
        let worker = format!(
            "read -r prompt\ncat > '{path}' <<'RUST'\n{source}RUST\n\
             printf '%s\\n' '{{\"type\":\"agent_settled\"}}'"
        );
        let record = spawn_write_task(WriteTaskRequest {
            repo_dir: repo.clone(),
            task_id: task_id.clone(),
            base_ref: "HEAD".into(),
            write_scope: vec![RelPath::parse(path).unwrap()],
            forbidden: Vec::new(),
            verifier: VerifierKind::RustKani,
            goal: "Verify lossless addition of zero".into(),
            pi_config: Some(mock_pi_config(&worker)),
        })
        .unwrap();
        let persisted = load_handoff(&repo, &task_id).unwrap();
        assert_eq!(persisted, record);
        let verdict = &persisted.handoff.verdict;
        assert!(!worktree_path_for(&repo, &task_id).exists());
        assert!(!repo.join(".verity/checker-worktrees").join(id).exists());

        let merged = approve_and_merge(
            &repo,
            &task_id,
            "main",
            &persisted.handoff.spec_digest.to_hex(),
        );
        if offset > 0 {
            assert_eq!(
                verdict.status,
                VerdictStatus::Counterexample,
                "{}",
                verdict.reason
            );
            assert!(matches!(merged, Err(OrchestratorError::GateFailed(_))));
            assert_eq!(
                read_ref(&repo, "HEAD").unwrap(),
                Some(persisted.base_commit)
            );
            continue;
        }
        assert_eq!(
            verdict.status,
            VerdictStatus::Pass,
            "{id}: {}",
            verdict.reason
        );
        assert_eq!(verdict.checks_passed, 1);
        assert_eq!(verdict.probes_satisfied, 1);
        assert!(
            matches!(
                merged,
                Ok(MergeResult::Merged {
                    already_merged: false,
                    ..
                })
            ),
            "{merged:?}"
        );
        assert_eq!(fs::read_to_string(repo.join(path)).unwrap(), source);
    }

    fs::remove_dir_all(&repo).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Lean 4.35 Lake, a trusted smoke challenge export/config, and Linux bubblewrap"]
fn live_lean_approves_matching_proof_and_rejects_different_statement() {
    assert!(pi_sandbox_available(), "Pi sandbox is required");
    assert!(std::env::var_os("VERITY_LEAN_COMPARATOR").is_some());
    assert!(std::env::var_os("VERITY_LEAN_COMPARATOR_CONFIG").is_some());
    let export = std::env::var_os("VERITY_LEAN_CHALLENGE_EXPORT").unwrap();
    let repo = temp_dir("live_lean");
    init_git_repo(&repo);
    fs::write(
        repo.join("lakefile.toml"),
        "name = \"smoke\"\n[[lean_lib]]\nname = \"Solution\"\n",
    )
    .unwrap();
    fs::write(
        repo.join("lake-manifest.json"),
        r#"{"version":"1.3.0","packagesDir":".lake/packages","packages":[],"name":"smoke","lakeDir":".lake","fixedToolchain":false}"#,
    )
    .unwrap();
    commit_worktree_changes(&repo, "prepare Lean workspace").unwrap();
    fs::copy(export, repo.join("challenge.export")).unwrap();
    let base = read_ref(&repo, "HEAD").unwrap().unwrap();

    for (id, source, status) in [
        (
            "different",
            "theorem smoke : 1 = 1 := rfl\n",
            VerdictStatus::Inconclusive,
        ),
        (
            "matching",
            "theorem smoke : True := by trivial\n",
            VerdictStatus::Pass,
        ),
    ] {
        let task_id = TaskId::parse(id).unwrap();
        let worker = format!(
            "read -r prompt\ncat > Solution.lean <<'LEAN'\n{source}LEAN\n\
             printf '%s\\n' '{{\"type\":\"agent_settled\"}}'"
        );
        let record = spawn_write_task(WriteTaskRequest {
            repo_dir: repo.clone(),
            task_id: task_id.clone(),
            base_ref: "HEAD".into(),
            write_scope: vec![RelPath::parse("Solution.lean").unwrap()],
            forbidden: Vec::new(),
            verifier: VerifierKind::Lean4Comparator,
            goal: "Prove the trusted smoke theorem".into(),
            pi_config: Some(mock_pi_config(&worker)),
        })
        .unwrap();
        let verdict = &record.handoff.verdict;
        assert_eq!(verdict.status, status, "{}", verdict.reason);
        let approved = approve_and_merge(
            &repo,
            &task_id,
            "main",
            &record.handoff.spec_digest.to_hex(),
        );
        match status {
            VerdictStatus::Pass => {
                assert_eq!(verdict.checks_passed, 1);
                assert!(
                    matches!(approved, Ok(MergeResult::Merged { .. })),
                    "{approved:?}"
                );
                assert_eq!(
                    fs::read_to_string(repo.join("Solution.lean")).unwrap(),
                    source
                );
            }
            VerdictStatus::Inconclusive => {
                assert!(
                    verdict.reason.contains("theorem statement do not match"),
                    "{}",
                    verdict.reason
                );
                assert!(matches!(approved, Err(OrchestratorError::GateFailed(_))));
                assert_eq!(read_ref(&repo, "HEAD").unwrap(), Some(base.clone()));
            }
            _ => unreachable!(),
        }
    }
    fs::remove_dir_all(&repo).unwrap();
}

#[test]
fn pi_rpc_handles_jsonl_stream_and_enforces_tool_budget_and_timeout() {
    let dir = temp_dir("pi_rpc");

    // 1. Happy path mock Pi RPC responder.
    let ok_cfg = PiRpcConfig {
        program: "sh".to_string(),
        args: vec![
            "-c".to_string(),
            r#"read line
printf '{"type":"message_update","delta":"Found 2 invariants."}\n'
printf '{"type":"tool_call","name":"read"}\n'
printf '{"type":"agent_settled"}\n'"#
                .to_string(),
        ],
        cwd: dir.clone(),
        timeout: Duration::from_secs(5),
        max_tool_calls: 5,
    };
    let res = run_pi_prompt(&ok_cfg, "t-1", "inspect src/").unwrap();
    assert!(res.settled);
    assert_eq!(res.tool_calls, 1);
    assert_eq!(res.final_text, "Found 2 invariants.");

    // 2. Tool call cap enforcement kills runaway worker.
    let cap_cfg = PiRpcConfig {
        program: "sh".to_string(),
        args: vec![
            "-c".to_string(),
            r#"read line
printf '{"type":"tool_call","name":"read"}\n'
printf '{"type":"tool_call","name":"read"}\n'
printf '{"type":"tool_call","name":"read"}\n'"#
                .to_string(),
        ],
        cwd: dir.clone(),
        timeout: Duration::from_secs(5),
        max_tool_calls: 2,
    };
    let err = run_pi_prompt(&cap_cfg, "t-2", "loop").unwrap_err();
    assert_eq!(
        err,
        PiRpcError::ToolCallBudgetExceeded { used: 3, limit: 2 }
    );

    // 3. Hard wall-clock timeout kills hung worker.
    let timeout_cfg = PiRpcConfig {
        program: "sh".to_string(),
        args: vec!["-c".to_string(), "read line; sleep 10".to_string()],
        cwd: dir.clone(),
        timeout: Duration::from_millis(120),
        max_tool_calls: 5,
    };
    assert_eq!(
        run_pi_prompt(&timeout_cfg, "t-3", "hang").unwrap_err(),
        PiRpcError::Timeout
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn spec_only_handoff_preserves_contract_and_records_candidate_digest() {
    let dir = temp_dir("handoff");
    let src = r#"
pub fn safe_sub(a: u32, b: u32) -> u32 {
    let secret_internal_body = a.saturating_sub(b);
    secret_internal_body
}
#[kani::proof]
fn verify_safe_sub() {
    let a: u32 = kani::any();
    let b: u32 = kani::any();
    let r = safe_sub(a, b);
    assert!(r <= a);
    kani::cover!(a > b, "positive_diff");
}
"#;
    let files = vec![(RelPath::parse("src/math.rs").unwrap(), src.to_string())];
    let spec = extract_spec_only(VerifierKind::RustKani, &files);
    assert!(spec.contains("pub fn safe_sub"));
    assert!(spec.contains("assert!(r <= a);"));
    assert!(spec.contains("kani::cover!(a > b"));
    // Implementation body line must NOT leak into the main agent's spec view.
    assert!(!spec.contains("secret_internal_body"));

    let oid_b = CommitOid::parse(&"b".repeat(40)).unwrap();

    let handoff = build_spec_handoff(
        oid_b,
        &files,
        VerifierKind::RustKani,
        CheckerVerdict {
            status: VerdictStatus::Pass,
            reason: "ok".to_string(),
            checks_passed: 2,
            probes_satisfied: 1,
        },
        Some("safe_sub never exceeds its first argument.".to_string()),
    );
    let digest = handoff.spec_digest;

    assert_eq!(handoff.extracted_spec, spec);
    assert_eq!(handoff.candidate_commit.as_str(), "b".repeat(40));

    let log = RunLog::open(&dir).unwrap();
    log.append_event("math-1", "merged", &digest.to_hex())
        .unwrap();
    let lines = log.read_lines().unwrap();
    let [line] = lines.as_slice() else {
        panic!("expected one run-log entry: {lines:?}");
    };
    let event = verity_core::json::parse_json(line).unwrap();
    assert_eq!(
        event
            .get("kind")
            .and_then(verity_core::json::JsonValue::as_str),
        Some("merged")
    );
    assert_eq!(
        event
            .get("detail")
            .and_then(verity_core::json::JsonValue::as_str),
        Some(digest.to_hex().as_str())
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_subagent_pipeline_blocks_merge_without_checker_evidence() {
    if !pi_sandbox_available() {
        return;
    }
    let repo = temp_dir("write_pipeline");
    init_git_repo(&repo);
    let base_oid = read_ref(&repo, "refs/heads/main").unwrap().unwrap();
    let task_id = TaskId::parse("math-safe-add").unwrap();

    // Mock Pi worker writes a Rust implementation + Kani proof, including a
    // nested block comment mentioning kani::assume(false) to prove no false positive.
    let worker_script = r#"read line
mkdir -p src
cat <<'EOF' > src/math.rs
/* nested /* kani::assume(false) */ comment must not trigger lint */
use crate::base;
pub fn safe_add(a: u32, b: u32) -> u32 {
    let internal_secret_impl = a.saturating_add(b).saturating_add(base());
    internal_secret_impl
}

#[kani::proof]
fn verify_safe_add() {
    let a: u32 = kani::any();
    let b: u32 = kani::any();
    let r = safe_add(a, b);
    assert!(r >= a && r >= b);
    kani::cover!(a > 0 && b > 0, "both_positive");
}
EOF
printf '{"type":"tool_call","name":"write"}\n'
printf '{"type":"message_update","delta":"safe_add saturates and is bounded below by both inputs."}\n'
printf '{"type":"agent_settled"}\n'"#;

    let spawned = spawn_write_task(WriteTaskRequest {
        repo_dir: repo.clone(),
        task_id: task_id.clone(),
        base_ref: "refs/heads/main".to_string(),
        write_scope: vec![RelPath::parse("src").unwrap()],
        forbidden: vec![RelPath::parse("Cargo.toml").unwrap()],
        verifier: VerifierKind::RustKani,
        goal: "implement and verify safe_add".to_string(),
        pi_config: Some(mock_pi_config(worker_script)),
    })
    .unwrap();

    // Worktree must be cleaned up after spawn returns.
    assert!(!worktree_path_for(&repo, &task_id).exists());
    // Persisted handoff JSON must exist and round-trip identically via load_handoff.
    assert!(handoff_path_for(&repo, &task_id).exists());
    let loaded = load_handoff(&repo, &task_id).unwrap();
    assert_eq!(loaded, spawned);

    // Spec-only extraction includes public signature + Kani proof checks, never impl body.
    assert!(loaded.handoff.hygiene.is_clean());
    assert!(loaded.handoff.extracted_spec.contains("pub fn safe_add"));
    assert!(loaded.handoff.extracted_spec.contains("#[kani::proof]"));
    assert!(loaded
        .handoff
        .extracted_spec
        .contains("assert!(r >= a && r >= b);"));
    assert!(loaded
        .handoff
        .extracted_spec
        .contains("kani::cover!(a > 0 && b > 0"));
    assert!(!loaded
        .handoff
        .extracted_spec
        .contains("internal_secret_impl"));

    // Wrong digest must be rejected before touching target ref.
    let bad_digest = "0".repeat(64);
    assert!(matches!(
        approve_and_merge(&repo, &task_id, "refs/heads/main", &bad_digest),
        Err(OrchestratorError::DigestMismatch { .. })
    ));
    assert_eq!(
        read_ref(&repo, "refs/heads/main").unwrap().unwrap(),
        base_oid
    );

    let digest_hex = loaded.handoff.spec_digest.to_hex();
    assert!(matches!(
        approve_and_merge(&repo, &task_id, "refs/heads/main", &digest_hex),
        Err(OrchestratorError::GateFailed(GateError::VerdictNotPass(_)))
    ));
    assert_eq!(
        read_ref(&repo, "refs/heads/main").unwrap().as_ref(),
        Some(&base_oid)
    );

    // RunLog recorded the handoff but no merge.
    let log_lines = RunLog::open(&repo).unwrap().read_lines().unwrap();
    assert!(log_lines.iter().any(|l| l.contains("\"handoff_ready\"")));
    assert!(!log_lines.iter().any(|l| l.contains("\"merged\"")));

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn approval_rejects_changed_candidate_and_advances_clean_checkout_once() {
    let repo = temp_dir("approve");
    init_git_repo(&repo);
    let task_id = TaskId::parse("approve").unwrap();
    let base_oid = read_ref(&repo, "HEAD").unwrap().unwrap();
    let worktree = worktree_path_for(&repo, &task_id);
    create_worktree(&repo, &worktree, &task_id, &base_oid).unwrap();
    let source = "pub fn safe_add(a: u32, b: u32) -> u32 { a.saturating_add(b) }\n";
    fs::write(worktree.join("src/math.rs"), source).unwrap();
    let candidate = commit_worktree_changes(&worktree, "verified candidate").unwrap();
    remove_worktree(&repo, &worktree).unwrap();
    let verifier = VerifierKind::RustKani;
    let handoff = build_spec_handoff(
        candidate,
        &[(RelPath::parse("src/math.rs").unwrap(), source.to_string())],
        verifier,
        CheckerVerdict {
            status: VerdictStatus::Pass,
            reason: "simulated controller verdict".to_string(),
            checks_passed: 1,
            probes_satisfied: 0,
        },
        None,
    );
    let digest_hex = handoff.spec_digest.to_hex();
    let mut verified = PersistedHandoff {
        task_id: task_id.clone(),
        base_commit: base_oid.clone(),
        verifier,
        handoff,
    };
    let handoff_path = handoff_path_for(&repo, &task_id);
    fs::create_dir_all(handoff_path.parent().unwrap()).unwrap();
    let verified_json = handoff_to_json(&verified);
    verified.handoff.candidate_commit = base_oid.clone();
    fs::write(&handoff_path, handoff_to_json(&verified)).unwrap();
    assert!(matches!(
        approve_and_merge(&repo, &task_id, "refs/heads/main", &digest_hex),
        Err(OrchestratorError::CorruptHandoff(ref msg)) if msg.contains("spec_digest integrity check failed")
    ));
    assert_eq!(
        read_ref(&repo, "refs/heads/main").unwrap().as_ref(),
        Some(&base_oid)
    );

    fs::write(&handoff_path, verified_json).unwrap();
    let base_tree = read_ref(&repo, "HEAD^{tree}").unwrap().unwrap();
    let advanced = commit_tree(&repo, &base_tree, &[&base_oid], "advanced target").unwrap();
    update_ref_cas(&repo, "refs/heads/advanced", &advanced, None).unwrap();
    assert!(matches!(
        approve_and_merge(&repo, &task_id, "advanced", &digest_hex),
        Err(OrchestratorError::TargetMoved)
    ));
    assert_eq!(
        read_ref(&repo, "refs/heads/advanced").unwrap(),
        Some(advanced)
    );
    let MergeResult::Merged {
        commit,
        already_merged: false,
    } = approve_and_merge(&repo, &task_id, "refs/heads/main", &digest_hex).unwrap()
    else {
        panic!("expected clean merge");
    };
    let show = run_git(&repo, &["show", "refs/heads/main:src/math.rs"]).unwrap();
    assert!(show.status.success());
    assert!(String::from_utf8_lossy(&show.stdout).contains("pub fn safe_add"));
    assert_eq!(
        fs::read_to_string(repo.join("src/math.rs"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        source.trim_end()
    );
    assert_eq!(
        approve_and_merge(&repo, &task_id, "refs/heads/main", &digest_hex).unwrap(),
        MergeResult::Merged {
            commit,
            already_merged: true
        }
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn write_subagent_enforces_scope_hygiene_and_cleans_up_worktree_on_failure() {
    if !pi_sandbox_available() {
        return;
    }
    let repo = temp_dir("write_guards");
    init_git_repo(&repo);

    // 1. Scope violation (touching forbidden Cargo.toml) fails and cleans up worktree.
    let scope_task = TaskId::parse("bad-scope").unwrap();
    let escape_script = r##"read line
echo "# hacked" >> Cargo.toml
printf '{"type":"agent_settled"}\n'"##;
    let err = spawn_write_task(WriteTaskRequest {
        repo_dir: repo.clone(),
        task_id: scope_task.clone(),
        base_ref: "refs/heads/main".to_string(),
        write_scope: vec![RelPath::parse("src").unwrap()],
        forbidden: vec![RelPath::parse("Cargo.toml").unwrap()],
        verifier: VerifierKind::RustKani,
        goal: "edit src".to_string(),
        pi_config: Some(mock_pi_config(escape_script)),
    })
    .unwrap_err();
    assert!(matches!(
        err,
        OrchestratorError::ScopeViolation(ScopeError::ForbiddenPath(_))
    ));
    assert!(!worktree_path_for(&repo, &scope_task).exists());

    // 2. Block-comment-smuggled kani::assume is caught by hygiene lint and blocks merge.
    let hygiene_task = TaskId::parse("bad-hygiene").unwrap();
    let cheat_script = r#"read line
cat <<'EOF' > src/cheat.rs
pub fn f(x: u32) -> u32 {
    /* innocent */ kani::assume(false);
    x
}
EOF
printf '{"type":"agent_settled"}\n'"#;
    let persisted = spawn_write_task(WriteTaskRequest {
        repo_dir: repo.clone(),
        task_id: hygiene_task.clone(),
        base_ref: "refs/heads/main".to_string(),
        write_scope: vec![RelPath::parse("src").unwrap()],
        forbidden: vec![],
        verifier: VerifierKind::RustKani,
        goal: "add f".to_string(),
        pi_config: Some(mock_pi_config(cheat_script)),
    })
    .unwrap();
    assert!(!worktree_path_for(&repo, &hygiene_task).exists());
    assert!(!persisted.handoff.hygiene.is_clean());
    assert_eq!(
        persisted.handoff.hygiene.violations[0].rule,
        "unreviewed-kani-assume"
    );

    let merge_err = approve_and_merge(
        &repo,
        &hygiene_task,
        "refs/heads/main",
        &persisted.handoff.spec_digest.to_hex(),
    )
    .unwrap_err();
    assert!(matches!(
        merge_err,
        OrchestratorError::GateFailed(GateError::HygieneFailed(1))
    ));

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn discover_kani_targets_and_live_verifier_fail_closed_in_spawn_write_task() {
    // 1. Direct check on discover_kani_targets ignoring commented-out proofs/covers.
    let files = vec![(
        RelPath::parse("src/check.rs").unwrap(),
        r#"
// #[kani::proof]
// fn ignored_proof() { kani::cover!(true, "ignored_probe"); }
#[kani::proof]
fn verify_clamped() {
    kani::cover!(true, "nonzero_branch");
}
"#
        .to_string(),
    )];
    let (harness, probes) = discover_kani_targets(&files);
    assert_eq!(harness.as_deref(), Some("verify_clamped"));
    assert_eq!(probes, vec!["nonzero_branch".to_string()]);

    if !pi_sandbox_available() {
        return;
    }

    let repo = temp_dir("live_verifier");
    init_git_repo(&repo);

    // 2. Live Kani verifier when NO #[kani::proof] harness exists fails closed with InvalidReport.
    let missing_proof_task = TaskId::parse("live-no-proof").unwrap();
    let no_proof_script = r#"read line
cat <<'EOF' > src/no_proof.rs
// #[kani::proof]
// fn commented_out() {}
pub fn inc(x: u32) -> u32 { x.saturating_add(1) }
EOF
printf '{"type":"agent_settled"}\n'"#;
    let no_proof_handoff = spawn_write_task_with_options(
        WriteTaskRequest {
            repo_dir: repo.clone(),
            task_id: missing_proof_task.clone(),
            base_ref: "refs/heads/main".to_string(),
            write_scope: vec![RelPath::parse("src").unwrap()],
            forbidden: vec![],
            verifier: VerifierKind::RustKani,
            goal: "implement inc".to_string(),
            pi_config: Some(mock_pi_config(no_proof_script)),
        },
        WriteTaskOptions {
            run_live_verifier: true,
            lean_challenge_export: None,
        },
    )
    .unwrap();
    assert!(!worktree_path_for(&repo, &missing_proof_task).exists());
    assert_eq!(
        no_proof_handoff.handoff.verdict.status,
        VerdictStatus::InvalidReport
    );
    assert!(no_proof_handoff
        .handoff
        .verdict
        .reason
        .contains("no #[kani::proof]"));
    let merge_err = approve_and_merge(
        &repo,
        &missing_proof_task,
        "refs/heads/main",
        &no_proof_handoff.handoff.spec_digest.to_hex(),
    )
    .unwrap_err();
    assert!(matches!(
        merge_err,
        OrchestratorError::GateFailed(GateError::VerdictNotPass(VerdictStatus::InvalidReport))
    ));

    // 3. Live Kani verifier when a #[kani::proof] harness IS discovered runs inside the
    //    isolated worktree before teardown and returns a structured verdict (never panics).
    let live_kani_task = TaskId::parse("live-kani-proof").unwrap();
    let kani_script = r#"read line
cat <<'EOF' > src/proved.rs
pub fn safe_add(a: u32, b: u32) -> u32 { a.saturating_add(b) }

#[kani::proof]
fn verify_safe_add() {
    let a: u32 = kani::any();
    let b: u32 = kani::any();
    assert!(safe_add(a, b) >= a);
    kani::cover!(a > 0 && b > 0, "both_positive");
}
EOF
printf '{"type":"agent_settled"}\n'"#;
    let live_handoff = spawn_write_task_with_options(
        WriteTaskRequest {
            repo_dir: repo.clone(),
            task_id: live_kani_task.clone(),
            base_ref: "refs/heads/main".to_string(),
            write_scope: vec![RelPath::parse("src").unwrap()],
            forbidden: vec![],
            verifier: VerifierKind::RustKani,
            goal: "implement and prove safe_add".to_string(),
            pi_config: Some(mock_pi_config(kani_script)),
        },
        WriteTaskOptions {
            run_live_verifier: true,
            lean_challenge_export: None,
        },
    )
    .unwrap();
    assert!(!worktree_path_for(&repo, &live_kani_task).exists());
    assert!(handoff_path_for(&repo, &live_kani_task).exists());
    let verdict = &live_handoff.handoff.verdict;
    if verdict.status == VerdictStatus::Pass {
        assert!(std::env::var_os("VERITY_KANI").is_some());
        assert_eq!((verdict.checks_passed, verdict.probes_satisfied), (1, 1));
    } else {
        assert!(matches!(
            verdict.status,
            VerdictStatus::InvalidReport | VerdictStatus::Inconclusive
        ));
    }
    assert!(!live_handoff.handoff.verdict.reason.is_empty());

    // 4. Live Lean 4 verifier returns Inconclusive with actionable reason when export/lake is missing.
    let lean_task = TaskId::parse("live-lean-proof").unwrap();
    let lean_script = r#"read line
cat <<'EOF' > src/Spec.lean
theorem add_zero (n : Nat) : n + 0 = n := Nat.add_zero n
EOF
printf '{"type":"agent_settled"}\n'"#;
    let lean_handoff = spawn_write_task_with_options(
        WriteTaskRequest {
            repo_dir: repo.clone(),
            task_id: lean_task.clone(),
            base_ref: "refs/heads/main".to_string(),
            write_scope: vec![RelPath::parse("src").unwrap()],
            forbidden: vec![],
            verifier: VerifierKind::Lean4Comparator,
            goal: "prove add_zero".to_string(),
            pi_config: Some(mock_pi_config(lean_script)),
        },
        WriteTaskOptions {
            run_live_verifier: true,
            lean_challenge_export: None,
        },
    )
    .unwrap();
    assert!(!worktree_path_for(&repo, &lean_task).exists());
    assert_eq!(
        lean_handoff.handoff.verdict.status,
        VerdictStatus::Inconclusive
    );
    assert!(lean_handoff
        .handoff
        .verdict
        .reason
        .contains("challenge.export"));

    // 5. A worker report cannot grant approval when the controller checker is disabled.
    let report_task = TaskId::parse("report-with-probes").unwrap();
    let report_script = r#"read line
cat <<'EOF' > src/checked.rs
#[kani::proof]
fn verify_nonzero() {
    kani::cover!(true, "reached_ok");
}
EOF
cat <<'EOF' > .verity-kani-report.json
{"harness":"verify_nonzero","should_panic":false,"solver_error":false,"checks":[{"property_class":"assertion","description":"ok","status":"SUCCESS"},{"property_class":"cover","description":"reached_ok","status":"SATISFIED"}]}
EOF
printf '{"type":"agent_settled"}\n'"#;
    let report_handoff = spawn_write_task_with_options(
        WriteTaskRequest {
            repo_dir: repo.clone(),
            task_id: report_task.clone(),
            base_ref: "refs/heads/main".to_string(),
            write_scope: vec![
                RelPath::parse("src").unwrap(),
                RelPath::parse(".verity-kani-report.json").unwrap(),
            ],
            forbidden: vec![],
            verifier: VerifierKind::RustKani,
            goal: "verify nonzero".to_string(),
            pi_config: Some(mock_pi_config(report_script)),
        },
        WriteTaskOptions {
            run_live_verifier: false,
            lean_challenge_export: None,
        },
    )
    .unwrap();
    assert_eq!(
        report_handoff.handoff.verdict.status,
        VerdictStatus::Inconclusive
    );
    assert!(matches!(
        approve_and_merge(
            &repo,
            &report_task,
            "refs/heads/main",
            &report_handoff.handoff.spec_digest.to_hex(),
        ),
        Err(OrchestratorError::GateFailed(GateError::VerdictNotPass(_)))
    ));

    let _ = fs::remove_dir_all(&repo);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn write_task_requires_a_supported_worker_sandbox() {
    let repo = temp_dir("unsupported_worker_sandbox");
    init_git_repo(&repo);
    let task_id = TaskId::parse("sandbox-required").unwrap();
    let result = spawn_write_task(WriteTaskRequest {
        repo_dir: repo.clone(),
        task_id: task_id.clone(),
        base_ref: "HEAD".to_string(),
        write_scope: vec![RelPath::parse("src").unwrap()],
        forbidden: vec![],
        verifier: VerifierKind::RustKani,
        goal: "inspect src".to_string(),
        pi_config: Some(mock_pi_config("exit 1")),
    });
    assert!(matches!(
        result,
        Err(OrchestratorError::PiRpc(PiRpcError::Sandbox(_)))
    ));
    assert!(!worktree_path_for(&repo, &task_id).exists());
    assert!(!handoff_path_for(&repo, &task_id).exists());
    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn approve_and_merge_rejects_tampered_verdict_and_hygiene_on_disk() {
    let repo = temp_dir("tamper_handoff");
    init_git_repo(&repo);
    let base_oid = read_ref(&repo, "refs/heads/main").unwrap().unwrap();

    // 1. Write a handoff with VerdictStatus::Counterexample, tamper on disk to flip status -> "pass".
    let task_id = TaskId::parse("tampered-verdict").unwrap();
    let worktree = worktree_path_for(&repo, &task_id);
    create_worktree(&repo, &worktree, &task_id, &base_oid).unwrap();
    let source = "pub fn safe_add(a: u32, b: u32) -> u32 { a.saturating_add(b) }\n";
    fs::write(worktree.join("src/math.rs"), source).unwrap();
    let candidate = commit_worktree_changes(&worktree, "candidate with counterexample").unwrap();
    remove_worktree(&repo, &worktree).unwrap();

    let handoff = build_spec_handoff(
        candidate.clone(),
        &[(RelPath::parse("src/math.rs").unwrap(), source.to_string())],
        VerifierKind::RustKani,
        CheckerVerdict {
            status: VerdictStatus::Counterexample,
            reason: "assertion failed".to_string(),
            checks_passed: 0,
            probes_satisfied: 0,
        },
        None,
    );
    let digest_hex = handoff.spec_digest.to_hex();
    let persisted = PersistedHandoff {
        task_id: task_id.clone(),
        base_commit: base_oid.clone(),
        verifier: VerifierKind::RustKani,
        handoff,
    };
    let handoff_path = handoff_path_for(&repo, &task_id);
    fs::create_dir_all(handoff_path.parent().unwrap()).unwrap();
    let raw_json = handoff_to_json(&persisted);
    let tampered_json = raw_json.replacen(r#""status":"counterexample""#, r#""status":"pass""#, 1);
    assert_ne!(raw_json, tampered_json);
    fs::write(&handoff_path, tampered_json).unwrap();

    let err = approve_and_merge(&repo, &task_id, "refs/heads/main", &digest_hex).unwrap_err();
    assert!(
        matches!(err, OrchestratorError::CorruptHandoff(ref msg) if msg.contains("spec_digest integrity check failed")),
        "expected CorruptHandoff integrity error, got: {err:?}"
    );
    assert_eq!(
        read_ref(&repo, "refs/heads/main").unwrap().as_ref(),
        Some(&base_oid)
    );

    // 2. Write a handoff with a hygiene violation, tamper on disk to clear violations & set clean=true.
    let hygiene_task = TaskId::parse("tampered-hygiene").unwrap();
    let dirty_source = "pub fn cheat(x: u32) -> u32 { kani::assume(false); x }\n";
    let dirty_handoff = build_spec_handoff(
        candidate,
        &[(
            RelPath::parse("src/cheat.rs").unwrap(),
            dirty_source.to_string(),
        )],
        VerifierKind::RustKani,
        CheckerVerdict {
            status: VerdictStatus::Pass,
            reason: "passed".to_string(),
            checks_passed: 1,
            probes_satisfied: 0,
        },
        None,
    );
    assert!(!dirty_handoff.hygiene.is_clean());
    let dirty_digest = dirty_handoff.spec_digest.to_hex();
    let dirty_persisted = PersistedHandoff {
        task_id: hygiene_task.clone(),
        base_commit: base_oid.clone(),
        verifier: VerifierKind::RustKani,
        handoff: dirty_handoff,
    };
    let dirty_path = handoff_path_for(&repo, &hygiene_task);
    let dirty_json = handoff_to_json(&dirty_persisted);
    let start = dirty_json.find(r#""hygiene":{"clean":false,"#).unwrap();
    let end = dirty_json[start..].find(r#","verdict":"#).unwrap() + start;
    let tampered_hygiene = format!(
        "{}{}{}",
        &dirty_json[..start],
        r#""hygiene":{"clean":true,"violations":[]}"#,
        &dirty_json[end..]
    );
    fs::write(&dirty_path, tampered_hygiene).unwrap();

    let err =
        approve_and_merge(&repo, &hygiene_task, "refs/heads/main", &dirty_digest).unwrap_err();
    assert!(
        matches!(err, OrchestratorError::CorruptHandoff(ref msg) if msg.contains("spec_digest integrity check failed")),
        "expected CorruptHandoff integrity error, got: {err:?}"
    );
    assert_eq!(
        read_ref(&repo, "refs/heads/main").unwrap().as_ref(),
        Some(&base_oid)
    );

    let _ = fs::remove_dir_all(&repo);
}
