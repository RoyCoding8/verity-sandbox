mod mcp;

use std::env;
use std::path::PathBuf;
use std::process;
use verity_core::json::escape_json_string;
use verity_core::CommitOid;
use verity_runtime::git::{commit_tree, merge_tree, update_ref_cas, MergeOutcome};
use verity_runtime::handoff::RunLog;
use verity_runtime::pi_rpc::{run_pi_prompt, PiRpcConfig};

fn usage() {
    eprintln!(
        "Verity v0.2.0\n\
         Commands:\n\
         \tverity run-readonly <repo> <goal> [pi-bin]\n\
         \tverity merge <repo> <ref> <base-oid> <candidate-oid> <approved-digest> <verified-digest>\n\
         \tverity log <repo>\n\
         \tverity mcp"
    );
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
            if args.len() < 3 {
                usage();
                process::exit(2);
            }
            let repo = PathBuf::from(&args[1]);
            let goal = &args[2];
            let mut cfg = PiRpcConfig::default_pi(repo.clone(), true);
            if let Some(bin) = args.get(3) {
                cfg.program = bin.clone();
                cfg.args.clear();
            }
            match run_pi_prompt(&cfg, "readonly-1", goal) {
                Ok(res) => {
                    if let Ok(log) = RunLog::open(&repo) {
                        let _ = log.append_event("readonly-1", "completed_readonly", &res.final_text);
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
        "merge" => {
            if args.len() != 7 {
                usage();
                process::exit(2);
            }
            let repo = PathBuf::from(&args[1]);
            let ref_name = &args[2];
            let base = CommitOid::parse(&args[3]).unwrap_or_else(|e| {
                eprintln!("invalid base oid: {}", e);
                process::exit(2);
            });
            let candidate = CommitOid::parse(&args[4]).unwrap_or_else(|e| {
                eprintln!("invalid candidate oid: {}", e);
                process::exit(2);
            });
            if args[5] != args[6] {
                eprintln!("refusing merge: approved digest does not match verified digest");
                process::exit(1);
            }
            match merge_tree(&repo, &base, &candidate) {
                Ok(MergeOutcome::Clean { tree_oid }) => {
                    let merged = commit_tree(&repo, &tree_oid, &[&base, &candidate], "verity merge")
                        .unwrap_or_else(|e| {
                            eprintln!("commit-tree failed: {}", e);
                            process::exit(1);
                        });
                    update_ref_cas(&repo, ref_name, &merged, Some(&base)).unwrap_or_else(|e| {
                        eprintln!("cas update-ref failed: {}", e);
                        process::exit(1);
                    });
                    if let Ok(log) = RunLog::open(&repo) {
                        let _ = log.append_event("merge", "merged", merged.as_str());
                    }
                    println!(
                        "{{\"status\":\"merged\",\"commit\":{}}}",
                        escape_json_string(merged.as_str())
                    );
                }
                Ok(MergeOutcome::Conflict { conflicted_paths }) => {
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
                    eprintln!("merge-tree failed: {}", e);
                    process::exit(1);
                }
            }
        }
        "log" => {
            if args.len() < 2 {
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
