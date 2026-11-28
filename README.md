# Verity

Verity is a desktop app and command-line tool for running Pi coding agents in isolated Git worktrees and checking their changes with Kani or Lean 4 before merging into your branch.

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue)](https://www.apache.org/licenses/LICENSE-2.0)

Coding agents can leave messy working trees or skip checks when they edit your main checkout directly. Verity isolates write tasks in detached Git worktrees, verifies candidate commits in a fresh checkout, and gates merges on passing formal checks and explicit `spec_digest` approval.

## Quick start

### Prerequisites

Install these tools before building or running Verity:

- Rust (stable toolchain) and Node.js 22 or newer
- Git 2.38 or newer (required for `git merge-tree --write-tree`)
- Pi CLI (`@earendil-works/pi-coding-agent`, providing `pi` on your `PATH`, or `%APPDATA%\npm\pi.cmd` on Windows)
- On Ubuntu or Debian, Tauri v2 packages: `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, and `patchelf`
- On Linux for write subagents: `bubblewrap` (`bwrap`) with unprivileged user and PID namespaces enabled, plus Kani `0.68.0` or Lean 4 (`lake`) if you run formal checks

### Run the desktop app

Clone the repository, install dependencies, and start the Tauri v2 app from the repository root:

```sh
npm run setup
npm run tauri:dev
```

Run `npm run tauri:build` to build a release bundle, or `npm run dev` to preview only the React UI in a browser without Tauri.

The desktop dev and release builds automatically build and bundle the Verity CLI, which enables delegation tools in Pi chat sessions. Configure model providers from **Provider settings** in the desktop sidebar, or set provider API keys in your shell environment. To use a different CLI, set `VERITY_MCP_BIN` to its absolute path. Pi reports a startup warning and persistent status when the CLI is missing or cannot run. Set `VERITY_DISABLE_SUBAGENT_TOOLS=1` to intentionally disable these tools.

Before running desktop Cargo commands directly, run `npm run sidecar:prepare` from the repository root. Use `-- --release` for a release CLI or `-- --target <triple>` for a cross build. The helper also honors Tauri's target and debug settings and Cargo's `CARGO_BUILD_TARGET` and `CARGO_TARGET_DIR` environment variables.

## Limitations

- **Write subagents require Linux.** Desktop chat and read-only subagents run on Linux, macOS, and Windows. Write subagents require Linux with `bubblewrap`.
- **Kani checks bounded properties.** Verity requires at least one passing assertion check rather than implicit safety or `cover!` checks alone. Review the extracted spec, unwind bounds, and any `kani::assume` lines in the handoff before approving.
- **Branch changes affect approval.** A new merge requires the target branch to still point to the task's base commit. If the target already includes the candidate, approval reports that it was merged earlier. Otherwise, rerun against the new base.
- **Turn revert and redo are separate.** Reverting a turn restores files only when a matching snapshot checkpoint exists. Redo restores only the files from the last revert in that session and refuses to run if affected paths changed afterward.

## Architecture and documentation

Read [ARCHITECTURE.md](ARCHITECTURE.md) for the system diagram, feature overview, and detailed reference:

- [System architecture](ARCHITECTURE.md#system-architecture): component diagram and desktop feature summary
- [Workspace layout](ARCHITECTURE.md#workspace-layout): crates, Tauri shell, UI, and formal verification directories
- [Write task lifecycle and merge approval](ARCHITECTURE.md#write-task-lifecycle-and-merge-approval): sandboxing, checked Git diff scope, read-only fallback, branch merge rules, and desktop snapshots
- [CLI and MCP usage](ARCHITECTURE.md#cli-and-mcp-usage): subcommands, `verity mcp` tools, and example workflows
- [Configuring Kani and Lean 4 checkers](ARCHITECTURE.md#configuring-kani-and-lean-4-checkers): environment variables, standalone or Cargo Kani, Lean comparator setup, and hygiene scanners
- [Project verification and CI](ARCHITECTURE.md#project-verification-and-ci): local test, lint, Kani, Verus, and TLA+ checks for Verity itself

## License

Licensed under [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0).
