use std::fs;
use std::path::PathBuf;
use std::process::Command;
use verity_core::classify_kani_report;
use verity_core::{RelPath, TaskId, VerdictStatus};
use verity_runtime::git::{
    changed_paths, commit_tree, commit_worktree_changes, create_worktree, git_common_dir,
    merge_tree, read_ref, remove_worktree, update_ref_cas, MergeOutcome,
};
use verity_runtime::jail::{JailError, WorkspaceJail};
use verity_runtime::sandbox::{bwrap_available, default_offline_config, run_sandboxed};
use verity_runtime::verifiers::{hygiene_lint_lean, hygiene_lint_rust, parse_kani_json};

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("verity_test_{}_{}", label, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_test_repo(dir: &PathBuf) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(["init", "-b", "main"])
        .status()
        .unwrap();
    assert!(status.success());
    Command::new("git")
        .current_dir(dir)
        .args(["config", "user.name", "Test"])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(dir)
        .args(["config", "user.email", "test@local"])
        .status()
        .unwrap();
    fs::write(
        dir.join("lib.rs"),
        "pub fn add(a: u32, b: u32) -> u32 { a + b }\n",
    )
    .unwrap();
    Command::new("git")
        .current_dir(dir)
        .args(["add", "-A"])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(dir)
        .args(["commit", "-m", "initial"])
        .status()
        .unwrap();
}

#[test]
fn git_worktree_merge_tree_and_cas_ref_lifecycle() {
    let root = temp_dir("git_lifecycle");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_test_repo(&repo);

    let base = read_ref(&repo, "refs/heads/main").unwrap().unwrap();
    let wt_path = root.join("worktrees/task-1");
    let task_id = TaskId::parse("task-1").unwrap();

    create_worktree(&repo, &wt_path, &task_id, &base).unwrap();

    let change = "pub fn add(a: u32, b: u32) -> u32 { a.saturating_add(b) }\n";
    fs::write(wt_path.join("lib.rs"), change).unwrap();
    assert!(create_worktree(&repo, &wt_path, &task_id, &base).is_err());
    assert_eq!(fs::read_to_string(wt_path.join("lib.rs")).unwrap(), change);
    let cand = commit_worktree_changes(&wt_path, "task-1 candidate").unwrap();
    let diffs = changed_paths(&repo, &base, &cand).unwrap();
    assert_eq!(diffs, vec![RelPath::parse("lib.rs").unwrap()]);

    // Clean lock-free merge-tree + commit-tree + CAS ref update.
    let merged_tree = match merge_tree(&repo, &base, &cand).unwrap() {
        MergeOutcome::Clean { tree_oid } => tree_oid,
        MergeOutcome::Conflict { .. } => panic!("expected clean merge"),
    };
    let merged_commit = commit_tree(&repo, &merged_tree, &[&base, &cand], "merge task-1").unwrap();
    update_ref_cas(&repo, "refs/heads/main", &merged_commit, Some(&base)).unwrap();

    // Stale CAS update must fail.
    assert!(update_ref_cas(&repo, "refs/heads/main", &cand, Some(&base)).is_err());

    // Conflict detection test: edit same line from base in a second worktree.
    let wt2 = root.join("worktrees/task-2");
    create_worktree(&repo, &wt2, &TaskId::parse("task-2").unwrap(), &base).unwrap();
    fs::write(
        wt2.join("lib.rs"),
        "pub fn add(a: u32, b: u32) -> u32 { a.wrapping_add(b) }\n",
    )
    .unwrap();
    let cand2 = commit_worktree_changes(&wt2, "task-2 conflicting").unwrap();
    match merge_tree(&repo, &merged_commit, &cand2).unwrap() {
        MergeOutcome::Conflict { conflicted_paths } => {
            assert_eq!(conflicted_paths, vec![RelPath::parse("lib.rs").unwrap()]);
        }
        MergeOutcome::Clean { .. } => panic!("expected conflict"),
    }

    remove_worktree(&repo, &wt_path).unwrap();
    remove_worktree(&repo, &wt2).unwrap();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sandbox_enforces_readonly_git_dir_and_writable_worktree() {
    if !bwrap_available() {
        return;
    }
    let root = temp_dir("sandbox");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_test_repo(&repo);
    let base = read_ref(&repo, "refs/heads/main").unwrap().unwrap();
    let wt = root.join("wt");
    create_worktree(&repo, &wt, &TaskId::parse("sb-1").unwrap(), &base).unwrap();
    let common_git = git_common_dir(&repo).unwrap();

    let cfg = default_offline_config(&wt, Some(common_git.clone()));
    // Writing inside worktree succeeds.
    let ok = run_sandboxed(&cfg, "/bin/sh", &["-c", "echo hi > sandbox_ok.txt"]).unwrap();
    assert!(ok.status.success());

    // Writing to $GIT_COMMON_DIR/hooks is blocked by read-only bind mount.
    let hook_target = format!("{}/hooks/evil", common_git.display());
    let blocked = run_sandboxed(
        &cfg,
        "/bin/sh",
        &["-c", &format!("echo evil > '{}'", hook_target)],
    )
    .unwrap();
    assert!(!blocked.status.success());

    remove_worktree(&repo, &wt).unwrap();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn workspace_jail_blocks_symlinks_and_skips_oversized_files_on_search() {
    let root = temp_dir("jail");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/good.rs"), "fn target_symbol() {}\n").unwrap();
    // Oversized file (>256 KB) should be skipped during search, not abort it.
    fs::write(root.join("src/big.bin"), "x".repeat(300 * 1024)).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/hosts", root.join("src/escape")).unwrap();
    }

    let jail = WorkspaceJail::new(
        root.clone(),
        vec![RelPath::parse("src").unwrap()],
        vec![RelPath::parse("src").unwrap()],
        vec![RelPath::parse("src/locked.rs").unwrap()],
    )
    .unwrap();

    #[cfg(unix)]
    assert!(matches!(
        jail.read_text("src/escape"),
        Err(JailError::SymlinkForbidden(_))
    ));
    assert!(matches!(
        jail.write_text("src/locked.rs", "nope"),
        Err(JailError::ScopeDenied(_))
    ));
    let hits = jail.search_substring("target_symbol", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "src/good.rs");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn hygiene_lints_and_kani_json_parser_catch_cheats() {
    let bad_rust = "fn f() { kani::assume(false); }\n#[kani::stub(a, b)]\n";
    let report = hygiene_lint_rust("src/lib.rs", bad_rust);
    assert_eq!(report.violations.len(), 2);

    let ok_rust = "fn f() { kani::assume(x < 10); // verity:trusted-assume\n}\n";
    assert!(hygiene_lint_rust("src/lib.rs", ok_rust).is_clean());

    let bad_lean = "partial def loop (n : Nat) : Nat := loop n\ntheorem t : True := by sorry\n";
    assert_eq!(hygiene_lint_lean("Spec.lean", bad_lean).violations.len(), 2);

    let json = include_str!("fixtures/kani-0.68.0-pass.json");
    let parsed = parse_kani_json(json).unwrap();
    let verdict = classify_kani_report(&parsed, &["forty_two"]);
    assert_eq!(verdict.status, VerdictStatus::Pass);
    assert_eq!(verdict.checks_passed, 1);
    assert_eq!(verdict.probes_satisfied, 1);
}
