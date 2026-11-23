# Verity v2

Verity is a Rust controller and Tauri v2 Material Design 3 desktop app for running coding subagents under machine-checked contracts. Read-only subagents run without worktrees and return text summaries. Write subagents run in locked `git worktree` checkouts, formalize and prove their work, and hand back a stripped spec-only review bundle before lock-free `git merge-tree` integration.

## Quick start

Requires Rust (stable), Node.js 22+, Git 2.38+, and Tauri v2 OS libraries (on Ubuntu/Debian: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`).

Install dependencies once from the repository root:

```sh
npm run setup
```

Launch the native Tauri v2 desktop app:

```sh
npm run tauri:dev
```

Or preview the Material Design 3 UI in a browser without Tauri:

```sh
npm run dev
```

Build a production desktop binary:

```sh
npm run tauri:build
```

## Workspace layout

- `crates/verity-core`: Zero-IO pure Rust domain types, task state machine, Pi session exclusivity laws, provider URL validation, write-scope rules, Kani verdict classifier, and JSON parser.
- `crates/verity-runtime`: Native `git worktree` and `git merge-tree`, rootless Linux `bubblewrap` and macOS `sandbox-exec` sandboxing, `O_NOFOLLOW` workspace path jail, comment-aware Kani and Lean 4 hygiene lints, Pi JSONL RPC client, write-subagent orchestrator, and spec-only handoff extractor.
- `crates/verity-cli`: Headless CLI (`run-readonly`, `spawn`, `review`, `merge`, `log`) and local STDIO MCP server (`verity mcp`).
- `formal/`: Kani 0.68.0 `#[kani::proof]` proof drivers with `kani::cover!` reachability probes verifying `verity-core` directly.
- `desktop/`: Tauri v2 Rust shell (`src-tauri/`) and Material Design 3 React frontend (`ui/`).

## Verification commands

Run all Rust workspace and Tauri backend tests:

```sh
npm test
```

Run the 6 symbolic Kani formal proofs on `verity-core` (requires Kani 0.68.0):

```sh
npm run formal
```
