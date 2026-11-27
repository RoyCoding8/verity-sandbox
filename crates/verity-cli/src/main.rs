mod mcp;

use std::env;
use std::path::PathBuf;
use std::process;
use verity_core::json::escape_json_string;
use verity_core::{RelPath, TaskId};
use verity_runtime::handoff::RunLog;
use verity_runtime::orchestrator::{
    approve_and_merge, handoff_to_json, load_handoff, parse_verifier_kind, spawn_write_task,
    MergeResult, WriteTaskRequest,
};
use verity_runtime::pi_rpc::{run_readonly_pi_prompt, PiRpcConfig};

fn usage() {
    eprintln!(
        "Verity v{}\n\
         Commands:\n\
         \tverity run-readonly <repo> <goal> [pi-bin]\n\
         \tverity spawn <repo> <task-id> <base-ref> <kani|lean> <write-scope-csv> <goal> [pi-bin]\n\
         \tverity review <repo> <task-id>\n\
         \tverity merge <repo> <task-id> <target-ref> <approved-digest>\n\
         \tverity log <repo>\n\
         \tverity mcp",
        env!("CARGO_PKG_VERSION")
    );
}

fn parse_csv_paths(raw: &str) -> Result<Vec<RelPath>, String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let rel =
            RelPath::parse(trimmed).map_err(|e| format!("invalid path '{}': {}", trimmed, e))?;
        out.push(rel);
    }
    Ok(out)
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        process::exit(2);
    }

    match args[0].as_str() {
        "mcp" => {
            if let Err(e) = mcp::run_mcp_stdio() {
                eprintln!("mcp error: {}", e);
                process::exit(1);
            }
        }
        "run-readonly" => {
            if !(3..=4).contains(&args.len()) {
                usage();
                process::exit(2);
            }
            let repo = PathBuf::from(&args[1]);
            let goal = &args[2];
            let mut cfg = PiRpcConfig::default_pi(repo.clone(), true);
            if let Some(bin) = args.get(3) {
                cfg.program = bin.clone();
            }
            match run_readonly_pi_prompt(&cfg, "readonly-1", goal) {
                Ok(res) => {
                    if let Ok(log) = RunLog::open(&repo) {
                        let _ =
                            log.append_event("readonly-1", "completed_readonly", &res.final_text);
                    }
                    println!(
                        "{{\"status\":\"completed_readonly\",\"settled\":{},\"tool_calls\":{},\"summary\":{}}}",
                        res.settled,
                        res.tool_calls,
                        escape_json_string(&res.final_text)
                    );
                }
                Err(e) => {
                    eprintln!("run-readonly failed: {}", e);
                    process::exit(1);
                }
            }
        }
        "spawn" => {
            if !(7..=8).contains(&args.len()) {
                usage();
                process::exit(2);
            }
            let repo_dir = PathBuf::from(&args[1]);
            let task_id = TaskId::parse(&args[2]).unwrap_or_else(|e| {
                eprintln!("invalid task id: {}", e);
                process::exit(2);
            });
            let base_ref = args[3].clone();
            let verifier = parse_verifier_kind(&args[4]).unwrap_or_else(|e| {
                eprintln!("{}", e);
                process::exit(2);
            });
            let write_scope = parse_csv_paths(&args[5]).unwrap_or_else(|e| {
                eprintln!("invalid write scope: {}", e);
                process::exit(2);
            });
            let goal = args[6].clone();
            let pi_config = args.get(7).map(|bin| {
                let mut cfg = PiRpcConfig::default_pi(repo_dir.clone(), false);
                cfg.program = bin.clone();
                cfg
            });

            let req = WriteTaskRequest {
                repo_dir,
                task_id,
                base_ref,
                write_scope,
                forbidden: Vec::new(),
                verifier,
                goal,
                pi_config,
            };

            match spawn_write_task(req) {
                Ok(record) => {
                    println!("{}", handoff_to_json(&record));
                }
                Err(e) => {
                    eprintln!("spawn failed: {}", e);
                    process::exit(1);
                }
            }
        }
        "review" => {
            if args.len() != 3 {
                usage();
                process::exit(2);
            }
            let repo_dir = PathBuf::from(&args[1]);
            let task_id = TaskId::parse(&args[2]).unwrap_or_else(|e| {
                eprintln!("invalid task id: {}", e);
                process::exit(2);
            });
            match load_handoff(&repo_dir, &task_id) {
                Ok(record) => {
                    println!("{}", handoff_to_json(&record));
                }
                Err(e) => {
                    eprintln!("review failed: {}", e);
                    process::exit(1);
                }
            }
        }
        "merge" => {
            if args.len() == 5 {
                let repo = PathBuf::from(&args[1]);
                let task_id = TaskId::parse(&args[2]).unwrap_or_else(|e| {
                    eprintln!("invalid task id: {}", e);
                    process::exit(2);
                });
                let target_ref = &args[3];
                let approved_digest = &args[4];
                match approve_and_merge(&repo, &task_id, target_ref, approved_digest) {
                    Ok(MergeResult::Merged {
                        commit,
                        already_merged,
                    }) => {
                        println!(
                            "{{\"status\":\"merged\",\"commit\":{},\"already_merged\":{}}}",
                            escape_json_string(commit.as_str()),
                            already_merged
                        );
                    }
                    Ok(MergeResult::Conflict { conflicted_paths }) => {
                        let paths: Vec<String> = conflicted_paths
                            .iter()
                            .map(|p| escape_json_string(p.as_str()))
                            .collect();
                        println!(
                            "{{\"status\":\"conflict\",\"paths\":[{}]}}",
                            paths.join(",")
                        );
                        process::exit(1);
                    }
                    Err(e) => {
                        eprintln!("merge failed: {}", e);
                        process::exit(1);
                    }
                }
            } else {
                usage();
                process::exit(2);
            }
        }
        "log" => {
            if args.len() != 2 {
                usage();
                process::exit(2);
            }
            let repo = PathBuf::from(&args[1]);
            match RunLog::open(&repo).and_then(|l| l.read_lines()) {
                Ok(lines) => {
                    for line in lines {
                        println!("{}", line);
                    }
                }
                Err(e) => {
                    eprintln!("log error: {}", e);
                    process::exit(1);
                }
            }
        }
        _ => {
            usage();
            process::exit(2);
        }
    }
}
