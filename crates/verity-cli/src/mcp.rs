use std::io::{self, BufRead, Write};
use verity_core::json::{escape_json_string, parse_json, JsonValue};

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
                let err_resp = format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{{\"code\":-32700,\"message\":{}}}}}\n",
                    escape_json_string(&e.to_string())
                );
                stdout.write_all(err_resp.as_bytes())?;
                stdout.flush()?;
                continue;
            }
        };

        let id_str = match req.get("id") {
            Some(JsonValue::Number(n)) => format!("{}", *n as i64),
            Some(JsonValue::String(s)) => escape_json_string(s),
            _ => "null".to_string(),
        };
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":\"2025-03-26\",\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"verity-mcp\",\"version\":\"0.2.0\"}}}}}}\n",
                id_str
            ),
            "tools/list" => format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"tools\":[\
                {{\"name\":\"verity_run_readonly\",\"description\":\"Run a read-only subagent without a worktree and return its summary.\",\"inputSchema\":{{\"type\":\"object\",\"properties\":{{\"repo\":{{\"type\":\"string\"}},\"goal\":{{\"type\":\"string\"}}}},\"required\":[\"repo\",\"goal\"]}}}},\
                {{\"name\":\"verity_spawn_write\",\"description\":\"Spawn a write subagent in an isolated git worktree to formalize, implement, and prove a contract.\",\"inputSchema\":{{\"type\":\"object\",\"properties\":{{\"repo\":{{\"type\":\"string\"}},\"task_id\":{{\"type\":\"string\"}},\"write_scope\":{{\"type\":\"array\",\"items\":{{\"type\":\"string\"}}}},\"verifier\":{{\"type\":\"string\",\"enum\":[\"kani\",\"lean\"]}},\"goal\":{{\"type\":\"string\"}}}},\"required\":[\"repo\",\"task_id\",\"write_scope\",\"verifier\",\"goal\"]}}}},\
                {{\"name\":\"verity_merge_approved\",\"description\":\"Re-check digest, run lock-free git merge-tree, and compare-and-swap update the target ref.\",\"inputSchema\":{{\"type\":\"object\",\"properties\":{{\"repo\":{{\"type\":\"string\"}},\"ref_name\":{{\"type\":\"string\"}},\"base\":{{\"type\":\"string\"}},\"candidate\":{{\"type\":\"string\"}},\"approved_digest\":{{\"type\":\"string\"}},\"verified_digest\":{{\"type\":\"string\"}}}},\"required\":[\"repo\",\"ref_name\",\"base\",\"candidate\",\"approved_digest\",\"verified_digest\"]}}}}\
                ]}}}}\n",
                id_str
            ),
            "notifications/initialized" => continue,
            _ => format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":-32601,\"message\":\"method not found\"}}}}\n",
                id_str
            ),
        };

        stdout.write_all(response.as_bytes())?;
        stdout.flush()?;
    }
    Ok(())
}
