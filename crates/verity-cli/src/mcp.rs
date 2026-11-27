use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use verity_core::json::{escape_json_string, parse_json, JsonValue};
use verity_core::{RelPath, TaskId};
use verity_runtime::handoff::RunLog;
use verity_runtime::orchestrator::{
    approve_and_merge, handoff_to_json, parse_verifier_kind, spawn_write_task, MergeResult,
    WriteTaskRequest,
};
use verity_runtime::pi_rpc::{run_readonly_pi_prompt, PiRpcConfig};

fn jsonrpc_result(id_str: &str, result_json: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{}}}\n",
        id_str, result_json
    )
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

fn parse_rel_paths_array(
    args: &JsonValue,
    key: &str,
    required: bool,
) -> Result<Vec<RelPath>, String> {
    if required && args.get(key).and_then(JsonValue::as_array).is_none() {
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

fn require_controller_pi(args: &JsonValue) -> Result<(), String> {
    if args.get("pi_bin").is_some() || args.get("pi_args").is_some() {
        return Err("MCP requests use the controller's Pi command".to_string());
    }
    Ok(())
}

fn exec_run_readonly(args: &JsonValue) -> Result<String, String> {
    require_controller_pi(args)?;
    let repo = PathBuf::from(arg_str(args, "repo")?);
    let goal = arg_str(args, "goal")?;
    let cfg = PiRpcConfig::default_pi(repo.clone(), true);
    let res = run_readonly_pi_prompt(&cfg, "readonly-mcp", goal).map_err(|e| e.to_string())?;
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
    require_controller_pi(args)?;
    let repo_dir = PathBuf::from(arg_str(args, "repo")?);
    let task_id = TaskId::parse(arg_str(args, "task_id")?).map_err(|e| e.to_string())?;
    let write_scope = parse_rel_paths_array(args, "write_scope", true)?;
    let forbidden = parse_rel_paths_array(args, "forbidden", false)?;
    let verifier = parse_verifier_kind(arg_str(args, "verifier")?)?;
    let goal = arg_str(args, "goal")?.to_string();
    let base_ref = match args.get("base_ref") {
        None => "HEAD".to_string(),
        Some(JsonValue::String(value)) => value.clone(),
        _ => return Err("base_ref must be a string".to_string()),
    };
    let req = WriteTaskRequest {
        repo_dir,
        task_id,
        base_ref,
        write_scope,
        forbidden,
        verifier,
        goal,
        pi_config: None,
    };

    let record = spawn_write_task(req).map_err(|e| e.to_string())?;
    Ok(handoff_to_json(&record))
}

fn exec_merge_approved(args: &JsonValue) -> Result<(String, bool), String> {
    let repo = PathBuf::from(arg_str(args, "repo")?);
    let approved_digest = arg_str(args, "approved_digest")?;

    let task_id = TaskId::parse(arg_str(args, "task_id")?).map_err(|e| e.to_string())?;
    let target_ref = args
        .get("target_ref")
        .or_else(|| args.get("ref_name"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "missing required string argument 'target_ref'".to_string())?;
    match approve_and_merge(&repo, &task_id, target_ref, approved_digest) {
        Ok(MergeResult::Merged {
            commit,
            already_merged,
        }) => Ok((
            format!(
                "{{\"status\":\"merged\",\"commit\":{},\"already_merged\":{already_merged}}}",
                escape_json_string(commit.as_str()),
            ),
            false,
        )),
        Ok(MergeResult::Conflict { conflicted_paths }) => {
            let paths: Vec<String> = conflicted_paths
                .iter()
                .map(|p| escape_json_string(p.as_str()))
                .collect();
            Ok((
                format!(
                    "{{\"status\":\"conflict\",\"paths\":[{}]}}",
                    paths.join(",")
                ),
                true,
            ))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Handles a single parsed MCP JSON-RPC 2.0 request. Returns `None` for
/// notifications that require no response.
pub fn handle_mcp_request(req: &JsonValue) -> Option<String> {
    let Some(method) = req.get("method").and_then(JsonValue::as_str) else {
        return Some(jsonrpc_error("null", -32600, "invalid request"));
    };
    if req.get("jsonrpc").and_then(JsonValue::as_str) != Some("2.0") {
        return Some(jsonrpc_error("null", -32600, "invalid JSON-RPC version"));
    }
    let id_str = match req.get("id") {
        Some(JsonValue::Number(n)) if n.fract() == 0.0 && n.abs() < (1_u64 << 53) as f64 => {
            (*n as i64).to_string()
        }
        Some(JsonValue::String(s)) => escape_json_string(s),
        None => return None,
        _ => return Some(jsonrpc_error("null", -32600, "invalid request id")),
    };

    match method {
        "initialize" => Some(jsonrpc_result(
            &id_str,
            "{\"protocolVersion\":\"2025-03-26\",\"capabilities\":{\"tools\":{}},\"serverInfo\":{\"name\":\"verity-mcp\",\"version\":\"0.2.0\"}}",
        )),
        "ping" => Some(jsonrpc_result(&id_str, "{}")),
        "tools/list" => Some(jsonrpc_result(
            &id_str,
            "{\"tools\":[\
            {\"name\":\"verity_run_readonly\",\"description\":\"Run a read-only subagent without a worktree and return its summary.\",\"inputSchema\":{\"type\":\"object\",\"properties\":{\"repo\":{\"type\":\"string\"},\"goal\":{\"type\":\"string\"}},\"required\":[\"repo\",\"goal\"]}},\
            {\"name\":\"verity_spawn_write\",\"description\":\"Spawn a write subagent in an isolated git worktree to formalize, implement, and prove a contract.\",\"inputSchema\":{\"type\":\"object\",\"properties\":{\"repo\":{\"type\":\"string\"},\"task_id\":{\"type\":\"string\"},\"base_ref\":{\"type\":\"string\"},\"write_scope\":{\"type\":\"array\",\"items\":{\"type\":\"string\"}},\"forbidden\":{\"type\":\"array\",\"items\":{\"type\":\"string\"}},\"verifier\":{\"type\":\"string\",\"enum\":[\"kani\",\"lean\"]},\"goal\":{\"type\":\"string\"}},\"required\":[\"repo\",\"task_id\",\"write_scope\",\"verifier\",\"goal\"]}},\
            {\"name\":\"verity_merge_approved\",\"description\":\"Verify approved spec digest, run lock-free git merge-tree, and compare-and-swap update the target ref.\",\"inputSchema\":{\"type\":\"object\",\"properties\":{\"repo\":{\"type\":\"string\"},\"task_id\":{\"type\":\"string\"},\"target_ref\":{\"type\":\"string\"},\"approved_digest\":{\"type\":\"string\"}},\"required\":[\"repo\",\"task_id\",\"target_ref\",\"approved_digest\"]}}\
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
            if !matches!(tool_args, JsonValue::Object(_)) {
                return Some(jsonrpc_error(&id_str, -32602, "arguments must be an object"));
            }

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
