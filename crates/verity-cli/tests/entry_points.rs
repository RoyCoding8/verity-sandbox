use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use verity_core::json::{escape_json_string, parse_json, JsonValue};
use verity_core::{CheckerVerdict, RelPath, TaskId, VerdictStatus, VerifierKind};
use verity_runtime::git::{
    commit_worktree_changes, create_worktree, read_ref, remove_worktree, run_git,
};
use verity_runtime::handoff::build_spec_handoff;
use verity_runtime::orchestrator::{handoff_path_for, handoff_to_json, PersistedHandoff};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!("verity-cli-{label}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn verity(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verity"))
        .args(args)
        .output()
        .unwrap()
}

fn git(repo: &Path, args: &[&str]) {
    let out = run_git(repo, args).unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn expect_failure(out: Output, exit_code: i32, diagnostic: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(exit_code), "{stderr}");
    assert!(stderr.contains(diagnostic), "{stderr}");
    assert!(out.stdout.is_empty());
}

fn stdout_json(out: Output) -> JsonValue {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    parse_json(std::str::from_utf8(&out.stdout).unwrap()).unwrap()
}

#[test]
fn cli_routes_spawn_validation_and_readonly_start_errors() {
    let temp = TempDir::new("errors");
    let repo = temp.0.to_str().unwrap();
    expect_failure(
        verity(&["spawn", repo, "fixture", "HEAD", "other", "src", "verify"]),
        2,
        "unsupported verifier",
    );
    expect_failure(
        verity(&["spawn", repo, "fixture", "HEAD", "kani", " , ", "verify"]),
        1,
        "requires at least one write_scope path",
    );
    let worker = temp.0.join("missing-worker");
    expect_failure(
        verity(&["run-readonly", repo, "summarize", worker.to_str().unwrap()]),
        1,
        "run-readonly failed",
    );
}

#[cfg(target_os = "linux")]
#[test]
fn cli_returns_readonly_worker_summary_and_logs_completion() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new("readonly");
    let repo = temp.0.join("repo");
    fs::create_dir(&repo).unwrap();
    fs::write(repo.join("overview.txt"), "Project overview\n").unwrap();
    let worker = temp.0.join("worker");
    fs::write(
        &worker,
        r#"#!/bin/sh
read -r prompt
case "$prompt" in *"Summarize the overview"*) ;; *) exit 1 ;; esac
read -r title < overview.txt
printf '%s\n' '{"type":"tool_call","name":"read"}'
printf '{"type":"message_update","delta":"%s"}\n' "$title"
printf '%s\n' '{"type":"agent_settled"}'
"#,
    )
    .unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();
    let repo = repo.to_str().unwrap();
    let result = stdout_json(verity(&[
        "run-readonly",
        repo,
        "Summarize the overview",
        worker.to_str().unwrap(),
    ]));
    assert_eq!(
        result.get("status").and_then(JsonValue::as_str),
        Some("completed_readonly")
    );
    assert_eq!(
        result.get("summary").and_then(JsonValue::as_str),
        Some("Project overview")
    );
    assert_eq!(
        result.get("settled").and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(result.get("tool_calls"), Some(&JsonValue::Number(1.0)));
    let event = stdout_json(verity(&["log", repo]));
    assert_eq!(
        event.get("kind").and_then(JsonValue::as_str),
        Some("completed_readonly")
    );
    assert_eq!(
        event.get("detail").and_then(JsonValue::as_str),
        Some("Project overview")
    );
}

#[test]
fn cli_routes_review_approval_and_checkout_errors() {
    let temp = TempDir::new("approval");
    let repo = temp.0.join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@local"]);
    fs::write(repo.join("math.rs"), "pub fn zero() -> u8 { 0 }\n").unwrap();
    let base = commit_worktree_changes(&repo, "base").unwrap();

    let task = "fixture-review";
    let task_id = TaskId::parse(task).unwrap();
    let worktree = temp.0.join("candidate");
    create_worktree(&repo, &worktree, &task_id, &base).unwrap();
    let source = "pub fn one() -> u8 { 1 }\n";
    fs::write(worktree.join("math.rs"), source).unwrap();
    let candidate = commit_worktree_changes(&worktree, "candidate").unwrap();
    remove_worktree(&repo, &worktree).unwrap();
    let verifier = VerifierKind::RustKani;
    let handoff = build_spec_handoff(
        candidate.clone(),
        &[(RelPath::parse("math.rs").unwrap(), source.into())],
        verifier,
        CheckerVerdict {
            status: VerdictStatus::Pass,
            reason: "fixture controller verdict".into(),
            checks_passed: 1,
            probes_satisfied: 0,
        },
        None,
    );
    let digest = handoff.spec_digest.to_hex();
    let path = handoff_path_for(&repo, &task_id);
    let record = PersistedHandoff {
        task_id,
        base_commit: base.clone(),
        verifier,
        handoff,
    };
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, handoff_to_json(&record)).unwrap();
    let repo_arg = repo.to_str().unwrap();

    expect_failure(verity(&["review", repo_arg]), 2, "verity review");
    expect_failure(
        verity(&["review", repo_arg, "absent"]),
        1,
        "handoff not found",
    );
    let reviewed = stdout_json(verity(&["review", repo_arg, task]));
    assert_eq!(
        reviewed.get("spec_digest").and_then(JsonValue::as_str),
        Some(digest.as_str())
    );
    assert_eq!(
        reviewed.get("candidate_commit").and_then(JsonValue::as_str),
        Some(candidate.as_str())
    );

    let merge = |branch: &str, approved: &str| verity(&["merge", repo_arg, task, branch, approved]);
    expect_failure(merge("main", "bad-digest"), 1, "invalid spec digest");
    expect_failure(
        merge("main", &"0".repeat(64)),
        1,
        "approved digest mismatch",
    );
    expect_failure(merge("missing", &digest), 1, "git ref not found");
    git(&repo, &["branch", "other-checkout"]);
    let other = temp.0.join("other");
    git(
        &repo,
        &["worktree", "add", other.to_str().unwrap(), "other-checkout"],
    );
    expect_failure(merge("other-checkout", &digest), 1, "checked out elsewhere");
    assert_eq!(read_ref(&other, "HEAD").unwrap(), Some(base.clone()));
    remove_worktree(&repo, &other).unwrap();

    let dirty = "pub fn two() -> u8 { 2 }\n";
    fs::write(repo.join("math.rs"), dirty).unwrap();
    expect_failure(merge("main", &digest), 1, "contains tracked changes");
    assert_eq!(fs::read_to_string(repo.join("math.rs")).unwrap(), dirty);
    assert_eq!(read_ref(&repo, "HEAD").unwrap(), Some(base));
    git(&repo, &["restore", "math.rs"]);

    let merged = stdout_json(merge("main", &digest));
    let commit = read_ref(&repo, "HEAD").unwrap().unwrap();
    assert_eq!(
        merged.get("commit").and_then(JsonValue::as_str),
        Some(commit.as_str())
    );
    assert_eq!(
        merged.get("already_merged").and_then(JsonValue::as_bool),
        Some(false)
    );
    assert_eq!(fs::read_to_string(repo.join("math.rs")).unwrap(), source);

    let reply = mcp_exchange(&[call(
        1,
        "verity_merge_approved",
        &format!(
            r#"{{"repo":{},"task_id":"{task}","ref_name":"main","approved_digest":"{digest}"}}"#,
            escape_json_string(repo_arg)
        ),
    )]);
    let result = tool_text(&reply[0], false);
    let retry = parse_json(result).unwrap();
    assert_eq!(
        retry.get("already_merged").and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(
        retry.get("commit").and_then(JsonValue::as_str),
        Some(commit.as_str())
    );
    let log = verity(&["log", repo_arg]);
    assert!(log.status.success());
    let lines = String::from_utf8(log.stdout).unwrap();
    assert_eq!(lines.lines().count(), 1);
    let event = parse_json(lines.trim()).unwrap();
    assert_eq!(
        event.get("kind").and_then(JsonValue::as_str),
        Some("merged")
    );
}

fn call(id: i32, name: &str, arguments: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{arguments}}}}}"#
    )
}

fn mcp_exchange(requests: &[String]) -> Vec<JsonValue> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_verity"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut input = child.stdin.take().unwrap();
        for request in requests {
            writeln!(input, "{request}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| parse_json(line).unwrap())
        .collect()
}

fn tool_text(reply: &JsonValue, is_error: bool) -> &str {
    let result = reply.get("result").unwrap();
    assert_eq!(
        result.get("isError").and_then(JsonValue::as_bool),
        Some(is_error)
    );
    result.get("content").unwrap().as_array().unwrap()[0]
        .get("text")
        .and_then(JsonValue::as_str)
        .unwrap()
}

#[test]
fn mcp_stdio_routes_protocol_and_tool_errors_and_continues_after_invalid_input() {
    let temp = TempDir::new("mcp");
    let repo = escape_json_string(temp.0.to_str().unwrap());
    let requests = [
        r#"{"jsonrpc":"2.0","method":"initialize","id":"init"}"#.into(),
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#.into(),
        "{".into(),
        r#"{"jsonrpc":"1.0","method":"ping","id":1}"#.into(),
        r#"{"jsonrpc":"2.0","method":"ping","id":null}"#.into(),
        r#"{"jsonrpc":"2.0","method":"ping","id":9007199254740993}"#.into(),
        r#"{"jsonrpc":"2.0","method":"other","id":2}"#.into(),
        call(3, "verity_run_readonly", "[]"),
        call(
            4,
            "verity_run_readonly",
            r#"{"goal":"summarize","pi_bin":"other-pi"}"#,
        ),
        call(5, "verity_spawn_write", r#"{"goal":"verify","pi_args":[]}"#),
        call(
            6,
            "verity_spawn_write",
            &format!(
                r#"{{"repo":{repo},"task_id":"fixture","write_scope":["src"],"verifier":"kani","goal":"verify","base_ref":null}}"#
            ),
        ),
        call(
            7,
            "verity_merge_approved",
            &format!(
                r#"{{"repo":{repo},"task_id":"absent","target_ref":"main","approved_digest":"{}"}}"#,
                "0".repeat(64)
            ),
        ),
        r#"{"jsonrpc":"2.0","method":"tools/list","id":8}"#.into(),
        r#"{"jsonrpc":"2.0","method":"ping","id":"check"}"#.into(),
    ];
    let replies = mcp_exchange(&requests);
    assert_eq!(replies.len(), requests.len() - 1);
    assert_eq!(
        replies[0].get("id").and_then(JsonValue::as_str),
        Some("init")
    );
    assert!(replies[0]
        .get("result")
        .unwrap()
        .get("capabilities")
        .unwrap()
        .get("tools")
        .is_some());
    for (reply, expected) in replies[1..7]
        .iter()
        .zip([-32700, -32600, -32600, -32600, -32601, -32602])
    {
        assert_eq!(
            reply.get("error").unwrap().get("code"),
            Some(&JsonValue::Number(f64::from(expected)))
        );
    }
    for (reply, expected) in replies[7..11].iter().zip([
        "controller's Pi command",
        "controller's Pi command",
        "base_ref must be a string",
        "handoff not found",
    ]) {
        let message = tool_text(reply, true);
        assert!(message.contains(expected), "{message}");
    }
    let tools = replies[11]
        .get("result")
        .unwrap()
        .get("tools")
        .unwrap()
        .as_array()
        .unwrap();
    let names: Vec<_> = tools
        .iter()
        .map(|tool| tool.get("name").unwrap().as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "verity_run_readonly",
            "verity_spawn_write",
            "verity_merge_approved"
        ]
    );
    assert_eq!(
        replies[12].get("id").and_then(JsonValue::as_str),
        Some("check")
    );
    assert!(replies[12].get("result").is_some());
}
