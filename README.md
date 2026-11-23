# Verity v2

Verity is a Rust controller and Tauri v2 Material Design 3 desktop app for running coding subagents under machine-checked contracts. Read-only subagents run without worktrees and return text summaries. Write subagents run in locked `git worktree` checkouts, formalize and prove their work, and hand back a stripped spec-only review bundle before lock-free `git merge-tree` integration.

## Workspace layout

- `crates/verity-core`: Zero-IO pure Rust domain types, task state machine, write-scope rules, Kani verdict classifier, and JSON parser.
- `crates/verity-runtime`: Native `git worktree` and `git merge-tree`, rootless Linux `bubblewrap` and macOS `sandbox-exec` sandboxing, `O_NOFOLLOW` workspace path jail, Kani and Lean 4 hygiene lints, Pi JSONL RPC client, and spec-only handoff extractor.
- `crates/verity-cli`: Headless CLI (`run-readonly`, `merge`, `log`) and local STDIO MCP server (`verity mcp`).
- `formal/`: Kani `#[kani::proof]` proof drivers with `kani::cover!` reachability probes verifying `verity-core` directly.
- `desktop/`: Tauri v2 Rust shell (`src-tauri/`) and Material Design 3 React frontend (`ui/`).

## Commands

Run the Rust unit, formal finite-model, and integration test suite:

```sh
cargo test --workspace
```

Run the symbolic Kani proofs on `verity-core` (requires Kani 0.68.0):

```sh
cargo kani -p verity-formal --enable-unstable
```

Start the local STDIO MCP server:

```sh
cargo run -p verity-cli -- mcp
```
