use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use verity_core::json::{escape_json_string, parse_json, JsonValue};
use verity_core::{CommitOid, RelPath, TaskId};
use verity_runtime::git::{commit_tree, merge_tree, update_ref_cas, MergeOutcome};
use verity_runtime::handoff::RunLog;
use verity_runtime::orchestrator::{
    approve_and_merge, handoff_to_json, parse_verifier_kind, spawn_write_task, MergeResult,
    WriteTaskRequest,
};
use verity_runtime::pi_rpc::{run_pi_prompt, PiRpcConfig};

fn jsonrpc_result(id_str: &str, result_json: &str) -> String {
    format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{}}}\n", id_str, result_json)
}

fn jsonrpc_error(id_str: &str, code: i32, message: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":{},\"message\":{}}}}}\n",
        id_str,
        code,
        escape_json_string(message)
    )
}

fn tool_text_response(id_str: &str, text: &str, is_error: bool) -> String {
    let result = format!(
        "{{\"content\":[{{\"type\":\"text\",\"text\":{}}}],\"isError\":{}}}",
        escape_json_string(text),
        is_error
    );
    jsonrpc_result(id_str, &result)
}

fn arg_str<'a>(args: &'a JsonValue, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing required string argument '{}'", key))
}

fn parse_optional_string_array(args: &JsonValue, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(v) => {
            let arr = v
                .as_array()
                .ok_or_else(|| format!("argument '{}' must be an array of strings", key))?;
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item
                    .as_str()
                    .ok_or_else(|| format!("argument '{}' elements must be strings", key))?;
                out.push(s.to_string());
            }
            Ok(out)
        }
    }
}

fn parse_rel_paths_array(args: &JsonValue, key: &str, required: bool) -> Result<Vec<RelPath>, String> {
    if required && args.get(key).is_none() {
        return Err(format!("missing required array argument '{}'", key));
    }
    let raw = parse_optional_string_array(args, key)?;
    let mut out = Vec::with_capacity(raw.len());
    for s in raw {
        let rel = RelPath::parse(&s).map_err(|e| format!("invalid path '{}': {}", s, e))?;
        out.push(rel);
    }
    Ok(out)
}

fn build_pi_config(args: &JsonValue, cwd: PathBuf, read_only: bool) -> Result<Option<PiRpcConfig>, String> {
    let Some(pi_bin) = args.get("pi_bin").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let mut cfg = PiRpcConfig::default_pi(cwd, read_only);
    cfg.program = pi_bin.to_string();
    cfg.args = parse_optional_string_array(args, "pi_args")?;
    Ok(Some(cfg))
}

fn exec_run_readonly(args: &JsonValue) -> Result<String, String> {
    let repo = PathBuf::from(arg_str(args, "repo")?);
    let goal = arg_str(args, "goal")?;
    let cfg = match build_pi_config(args, repo.clone(), true)? {
        Some(c) => c,
        None => PiRpcConfig::default_pi(repo.clone(), true),
    };
    let res = run_pi_prompt(&cfg, "readonly-mcp", goal).map_err(|e| e.to_string())?;
    if let Ok(log) = RunLog::open(&repo) {
        let _ = log.append_event("readonly-mcp", "completed_readonly", &res.final_text);
    }
    Ok(format!(
        "{{\"status\":\"completed_readonly\",\"settled\":{},\"tool_calls\":{},\"summary\":{}}}",
        res.settled,
        res.tool_calls,
        escape_json_string(&res.final_text)
    ))
}

fn exec_spawn_write(args: &JsonValue) -> Result<String, String> {
    let repo_dir = PathBuf::from(arg_str(args, "repo")?);
    let task_id = TaskId::parse(arg_str(args, "task_id")?).map_err(|e| e.to_string())?;
    let write_scope = parse_rel_paths_array(args, "write_scope", true)?;
    let forbidden = parse_rel_paths_array(args, "forbidden", false)?;
    let verifier = parse_verifier_kind(arg_str(args, "verifier")?)?;
    let goal = arg_str(args, "goal")?.to_string();
    let base_ref = args
        .get("base_ref")
        .and_then(|v| v.as_str())
        .unwrap_or("HEAD")
        .to_string();
    let pi_config = build_pi_config(args, repo_dir.clone(), false)?;

    let req = WriteTaskRequest {
        repo_dir,
        task_id,
        base_ref,
        write_scope,
        forbidden,
        verifier,
        goal,
        pi_config,
    };

    let record = spawn_write_task(req).map_err(|e| e.to_string())?;
    Ok(handoff_to_json(&record))
}

fn exec_merge_approved(args: &JsonValue) -> Result<(String, bool), String> {
    let repo = PathBuf::from(arg_str(args, "repo")?);
    let approved_digest = arg_str(args, "approved_digest")?;

    // Persisted-handoff path: repo + task_id + (target_ref | ref_name) + approved_digest.
    if let Some(task_id_raw) = args.get("task_id").and_then(|v| v.as_str()) {
        let task_id = TaskId::parse(task_id_raw).map_err(|e| e.to_string())?;
        let target_ref = args
            .get("target_ref")
            .or_else(|| args.get("ref_name"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required string argument 'target_ref' or 'ref_name'".to_string())?;
        return match approve_and_merge(&repo, &task_id, target_ref, approved_digest) {
            Ok(MergeResult::Merged {
                commit,
                already_merged,
            }) => Ok((
                format!(
                    "{{\"status\":\"merged\",\"commit\":{},\"already_merged\":{}}}",
                    escape_json_string(commit.as_str()),
                    already_merged
                ),
                false,
            )),
            Ok(MergeResult::Conflict { conflicted_paths }) => {
                let paths: Vec<String> = conflicted_paths
                    .iter()
                    .map(|p| escape_json_string(p.as_str()))
                    .collect();
                Ok((
                    format!("{{\"status\":\"conflict\",\"paths\":[{}]}}", paths.join(",")),
                    true,
                ))
            }
            Err(e) => Err(e.to_string()),
        };
    }

    // Direct OID fallback path: repo + ref_name + base + candidate + approved_digest + verified_digest.
    let ref_name = arg_str(args, "ref_name")?;
    let base = CommitOid::parse(arg_str(args, "base")?).map_err(|e| e.to_string())?;
    let candidate = CommitOid::parse(arg_str(args, "candidate")?).map_err(|e| e.to_string())?;
    let verified_digest = arg_str(args, "verified_digest")?;
    if approved_digest != verified_digest {
        return Err("refusing merge: approved_digest does not match verified_digest".to_string());
    }
    match merge_tree(&repo, &base, &candidate).map_err(|e| e.to_string())? {
        MergeOutcome::Clean { tree_oid } => {
            let merged = commit_tree(&repo, &tree_oid, &[&base, &candidate], "verity merge")
                .map_err(|e| e.to_string())?;
            update_ref_cas(&repo, ref_name, &merged, Some(&base)).map_err(|e| e.to_string())?;
            if let Ok(log) = RunLog::open(&repo) {
                let _ = log.append_event("merge-mcp", "merged", merged.as_str());
            }
            Ok((
                format!(
                    "{{\"status\":\"merged\",\"commit\":{},\"already_merged\":false}}",
                    escape_json_string(merged.as_str())
                ),
                false,
            ))
        }
        MergeOutcome::Conflict { conflicted_paths } => {
            let paths: Vec<String> = conflicted_paths
                .iter()
                .map(|p| escape_json_string(p.as_str()))
                .collect();
            Ok((
                format!("{{\"status\":\"conflict\",\"paths\":[{}]}}", paths.join(",")),
                true,
            ))
        }
    }
}

/// Handles a single parsed MCP JSON-RPC 2.0 request. Returns `None` for
/// notifications that require no response.
pub fn handle_mcp_request(req: &JsonValue) -> Option<String> {
    let id_str = match req.get("id") {
        Some(JsonValue::Number(n)) => format!("{}", *n as i64),
        Some(JsonValue::String(s)) => escape_json_string(s),
        _ => "null".to_string(),
    };
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");

    match method {
        "initialize" => Some(jsonrpc_result(
            &id_str,
            "{\"protocolVersion\":\"2025-03-26\",\"capabilities\":{\"tools\":{}},\"serverInfo\":{\"name\":\"verity-mcp\",\"version\":\"0.2.0\"}}",
        )),
        "tools/list" => Some(jsonrpc_result(
            &id_str,
            "{\"tools\":[\
            {\"name\":\"verity_run_readonly\",\"description\":\"Run a read-only subagent without a worktree and return its summary.\",\"inputSchema\":{\"type\":\"object\",\"properties\":{\"repo\":{\"type\":\"string\"},\"goal\":{\"type\":\"string\"},\"pi_bin\":{\"type\":\"string\"},\"pi_args\":{\"type\":\"array\",\"items\":{\"type\":\"string\"}}},\"required\":[\"repo\",\"goal\"]}},\
            {\"name\":\"verity_spawn_write\",\"description\":\"Spawn a write subagent in an isolated git worktree to formalize, implement, and prove a contract.\",\"inputSchema\":{\"type\":\"object\",\"properties\":{\"repo\":{\"type\":\"string\"},\"task_id\":{\"type\":\"string\"},\"base_ref\":{\"type\":\"string\"},\"write_scope\":{\"type\":\"array\",\"items\":{\"type\":\"string\"}},\"forbidden\":{\"type\":\"array\",\"items\":{\"type\":\"string\"}},\"verifier\":{\"type\":\"string\",\"enum\":[\"kani\",\"lean\"]},\"goal\":{\"type\":\"string\"},\"pi_bin\":{\"type\":\"string\"},\"pi_args\":{\"type\":\"array\",\"items\":{\"type\":\"string\"}}},\"required\":[\"repo\",\"task_id\",\"write_scope\",\"verifier\",\"goal\"]}},\
            {\"name\":\"verity_merge_approved\",\"description\":\"Verify approved spec digest, run lock-free git merge-tree, and compare-and-swap update the target ref.\",\"inputSchema\":{\"type\":\"object\",\"properties\":{\"repo\":{\"type\":\"string\"},\"task_id\":{\"type\":\"string\"},\"target_ref\":{\"type\":\"string\"},\"ref_name\":{\"type\":\"string\"},\"base\":{\"type\":\"string\"},\"candidate\":{\"type\":\"string\"},\"approved_digest\":{\"type\":\"string\"},\"verified_digest\":{\"type\":\"string\"}},\"required\":[\"repo\",\"approved_digest\"]}}\
            ]}",
        )),
        "tools/call" => {
            let Some(params) = req.get("params") else {
                return Some(jsonrpc_error(&id_str, -32602, "missing params"));
            };
            let Some(tool_name) = params.get("name").and_then(|v| v.as_str()) else {
                return Some(jsonrpc_error(&id_str, -32602, "missing tool name"));
            };
            let empty_args = JsonValue::Object(Default::default());
            let tool_args = params.get("arguments").unwrap_or(&empty_args);

            match tool_name {
                "verity_run_readonly" => match exec_run_readonly(tool_args) {
                    Ok(payload) => Some(tool_text_response(&id_str, &payload, false)),
                    Err(e) => Some(tool_text_response(&id_str, &e, true)),
                },
                "verity_spawn_write" => match exec_spawn_write(tool_args) {
                    Ok(payload) => Some(tool_text_response(&id_str, &payload, false)),
                    Err(e) => Some(tool_text_response(&id_str, &e, true)),
                },
                "verity_merge_approved" => match exec_merge_approved(tool_args) {
                    Ok((payload, is_err)) => Some(tool_text_response(&id_str, &payload, is_err)),
                    Err(e) => Some(tool_text_response(&id_str, &e, true)),
                },
                _ => Some(jsonrpc_error(
                    &id_str,
                    -32602,
                    &format!("unknown tool '{}'", tool_name),
                )),
            }
        }
        "notifications/initialized" => None,
        _ => Some(jsonrpc_error(&id_str, -32601, "method not found")),
    }
}

/// Minimal local STDIO MCP server (JSON-RPC 2.0) exposing Verity's
/// verified-subagent workflow to any MCP-compatible host coding agent.
pub fn run_mcp_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line_res in stdin.lock().lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req = match parse_json(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = jsonrpc_error("null", -32700, &e.to_string());
                stdout.write_all(err_resp.as_bytes())?;
                stdout.flush()?;
                continue;
            }
        };

        if let Some(response) = handle_mcp_request(&req) {
            stdout.write_all(response.as_bytes())?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn init_repo(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("verity_mcp_{}_{}", label, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(Command::new("git")
            .current_dir(&dir)
            .args(["init", "-b", "main"])
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .current_dir(&dir)
            .args(["config", "user.name", "Test"])
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .current_dir(&dir)
            .args(["config", "user.email", "test@local"])
            .status()
            .unwrap()
            .success());
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "pub fn init() {}\n").unwrap();
        assert!(Command::new("git")
            .current_dir(&dir)
            .args(["add", "-A"])
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .current_dir(&dir)
            .args(["commit", "-m", "initial"])
            .status()
            .unwrap()
            .success());
        dir
    }

    #[test]
    fn mcp_tools_call_executes_readonly_spawn_and_merge() {
        let repo = init_repo("full_flow");
        let repo_json = escape_json_string(repo.to_str().unwrap());

        // 1. verity_run_readonly
        let ro_req = parse_json(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"verity_run_readonly\",\"arguments\":{{\"repo\":{},\"goal\":\"summarize\",\"pi_bin\":\"/bin/sh\",\"pi_args\":[\"-c\",\"read line; printf '{{\\\"type\\\":\\\"message_update\\\",\\\"delta\\\":\\\"clean repo\\\"}}\\n'; printf '{{\\\"type\\\":\\\"agent_settled\\\"}}\\n'\"]}}}}}}",
            repo_json
        ))
        .unwrap();
        let ro_resp = parse_json(&handle_mcp_request(&ro_req).unwrap()).unwrap();
        let ro_res = ro_resp.get("result").unwrap();
        assert_eq!(ro_res.get("isError").and_then(|v| v.as_bool()), Some(false));
        let ro_text = ro_res.get("content").unwrap().as_array().unwrap()[0].get("text").unwrap().as_str().unwrap();
        assert!(ro_text.contains("clean repo"));

        // 2. verity_spawn_write
        let worker_script = "read line; mkdir -p src; cat <<'EOF' > src/calc.rs\n\
pub fn safe_inc(x: u32) -> u32 {\n\
    x.saturating_add(1)\n\
}\n\
#[kani::proof]\n\
fn verify_inc() {\n\
    let x: u32 = kani::any();\n\
    assert!(safe_inc(x) >= x);\n\
}\n\
EOF\n\
printf '{\"type\":\"message_update\",\"delta\":\"safe_inc never decreases x\"}\\n'\n\
printf '{\"type\":\"agent_settled\"}\\n'";
        let spawn_req = parse_json(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"verity_spawn_write\",\"arguments\":{{\"repo\":{},\"task_id\":\"mcp-t1\",\"base_ref\":\"refs/heads/main\",\"write_scope\":[\"src\"],\"verifier\":\"kani\",\"goal\":\"add safe_inc\",\"pi_bin\":\"/bin/sh\",\"pi_args\":[\"-c\",{}]}}}}}}",
            repo_json,
            escape_json_string(worker_script)
        ))
        .unwrap();
        let spawn_resp = parse_json(&handle_mcp_request(&spawn_req).unwrap()).unwrap();
        let spawn_res = spawn_resp.get("result").unwrap();
        assert_eq!(spawn_res.get("isError").and_then(|v| v.as_bool()), Some(false));
        let handoff_text = spawn_res.get("content").unwrap().as_array().unwrap()[0].get("text").unwrap().as_str().unwrap();
        let handoff_val = parse_json(handoff_text).unwrap();
        let digest = handoff_val.get("spec_digest").unwrap().as_str().unwrap();

        // 3. verity_merge_approved
        let merge_req = parse_json(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"verity_merge_approved\",\"arguments\":{{\"repo\":{},\"task_id\":\"mcp-t1\",\"target_ref\":\"refs/heads/main\",\"approved_digest\":{}}}}}}}",
            repo_json,
            escape_json_string(digest)
        ))
        .unwrap();
        let merge_resp = parse_json(&handle_mcp_request(&merge_req).unwrap()).unwrap();
        let merge_res = merge_resp.get("result").unwrap();
        assert_eq!(merge_res.get("isError").and_then(|v| v.as_bool()), Some(false));
        let merge_text = merge_res.get("content").unwrap().as_array().unwrap()[0].get("text").unwrap().as_str().unwrap();
        assert!(merge_text.contains("\"status\":\"merged\""));

        let _ = fs::remove_dir_all(&repo);
    }

}
