use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use verity_core::{
    transition, CheckerVerdict, CommitOid, RelPath, TaskEvent, TaskMode, TaskState, VerdictStatus,
    VerifierKind,
};
use verity_runtime::handoff::{build_spec_handoff, extract_spec_only, RunLog};
use verity_runtime::pi_rpc::{run_pi_prompt, PiRpcConfig, PiRpcError};

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "verity_p3_{}_{}",
        label,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn pi_rpc_handles_jsonl_stream_and_enforces_tool_budget_and_timeout() {
    let dir = temp_dir("pi_rpc");

    // 1. Happy path mock Pi RPC responder.
    let ok_cfg = PiRpcConfig {
        program: "/bin/sh".to_string(),
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
        program: "/bin/sh".to_string(),
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
        program: "/bin/sh".to_string(),
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
fn spec_only_handoff_strips_impl_and_drives_full_state_machine() {
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

    let oid_a = CommitOid::parse(&"a".repeat(40)).unwrap();
    let oid_b = CommitOid::parse(&"b".repeat(40)).unwrap();
    let oid_c = CommitOid::parse(&"c".repeat(40)).unwrap();

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

    let mode = TaskMode::write(
        oid_a,
        vec![RelPath::parse("src").unwrap()],
        vec![],
        VerifierKind::RustKani,
    )
    .unwrap();
    let s1 = transition(&mode, TaskState::Queued, TaskEvent::StartAttempt, 2).unwrap();
    let s2 = transition(&mode, s1, TaskEvent::SubmitHandoff { handoff }, 2).unwrap();
    let s3 = transition(
        &mode,
        s2,
        TaskEvent::ApproveSpec {
            approved_digest: digest,
        },
        2,
    )
    .unwrap();
    let s4 = transition(
        &mode,
        s3,
        TaskEvent::CompleteMerge {
            merged_commit: oid_c,
            verified_digest: digest,
        },
        2,
    )
    .unwrap();
    assert!(matches!(s4, TaskState::Merged { .. }));

    let log = RunLog::open(&dir).unwrap();
    log.append_event("math-1", "merged", &digest.to_hex()).unwrap();
    assert_eq!(log.read_lines().unwrap().len(), 1);

    let _ = fs::remove_dir_all(&dir);
}
