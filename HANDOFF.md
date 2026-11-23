# Session Compaction & Handoff Summary (Verity v2)

Structured using [OpenCode's compaction summary template](https://gist.github.com/rmk40/cde7a98c1c90614a27478216cc01551f) (`Goal`, `Instructions`, `Discoveries`, `Accomplished`, `Relevant files`) with exact paths, commit SHAs, versions, and behavioral constraints preserved verbatim ([OpenCode #16512](https://github.com/anomalyco/opencode/issues/16512)).

## Goal

Build **Verity v2** from scratch as a full-fledged native desktop coding-agent harness (comparable to Claude Code, Codex, or Antigravity) built on:
- **Backend:** Rust Cargo workspace (`verity-core`, `verity-runtime`, `verity-cli`) + formal-proof-driven development in `formal/` verified by Kani 0.68.0 (`cargo kani -p verity-formal`).
- **Agent loop:** `@earendil-works/pi-coding-agent` driven over LF-delimited JSONL RPC (`pi --mode rpc --no-session`), preserving full Pi desktop UX parity (projects, sessions, tree, fork, clone, compact, streaming chat, steering/follow-up queues, model/thinking pickers, image attachments, `/` & `$` completions, provider settings, FIFO Extension UI).
- **Formal subagent workflow:** Main orchestrator spawns read-only subagents without worktrees and write subagents in locked `git worktree` checkouts. Write subagents formalize contracts, implement code, write proofs, and hand back a stripped spec-only `SpecHandoff` (extracted spec, blind back-translation, AST hygiene report, `kani::cover!` reachability probes, canonical `SpecDigest`) before lock-free `git merge-tree --write-tree` and CAS `git update-ref --no-deref`.
- **Desktop app:** Tauri v2 (`desktop/src-tauri/`) with a sleek Google Material Design 3 React + TypeScript frontend (`desktop/ui/`).

## Instructions

User-stated workflow and operating constraints to preserve in the next session:
1. **Operate in `@poteto-mode`:** Concise, unslopped prose; cite every skill and principle applied at each step (`Foundational Thinking`, `Type System Discipline`, `Model the Domain`, `Boundary Discipline`, `Make Operations Idempotent`, `Fix Root Causes`, `Subtract Before You Add`, `Minimize Reader Load`, `Experience First`, `Guard the Context Window`, `Prove It Works`).
2. **Formal-proof-driven development:** Pure domain invariants live in `crates/verity-core` and are proved symbolically in `formal/` using Kani (`#[kani::proof]` + `kani::cover!` reachability probes). Formal contracts are skipped where overkill (UI layout, OS syscall glue).
3. **Strict `git-worktree-discipline` for subagents:**
   - Never run parallel subagents on the same checkout.
   - Assign disjoint file ownership per worktree under `.worktrees/<job-id>` on branch `wt/<job-id>`.
   - Verify each subagent's formal proof or test/lint gate before merging serially (`git merge --no-ff wt/<job-id>`) onto `main`.
   - Re-run gates on the merged tip, then delete the worktree and branch (`git worktree remove --force .worktrees/<job-id> && git branch -d wt/<job-id>`). Confirm `git worktree list` shows only the main checkout.
4. **Repository & GitHub workflow:**
   - Active repo: `https://github.com/RoyCoding8/verity-sandbox.git` (branch `main`).
   - Do **not** store any PAT in `.git/config` or commit secrets; authenticate ephemeral HTTPS pushes via `git -c "http.extraHeader=Authorization: Basic <b64>"` (SSH ports 22 and 443 to GitHub are blocked by the sandbox proxy).
   - Respect the user's `9dfd201` (`frontend overhaul`, 25 files) CSS and component redesign in `desktop/ui/`.

## Discoveries

- **Old prototype was archived, not extended:** The original JS/Electron Verity prototype had an inverted architecture (pre-approved static JSON oracles, auto-promotion without spec review, read-only subagents blocked in 4 places, blob-copy fake worktrees breaking `.lake/`/`target/`, 11-table SQLite journal). Verity v2 was rebuilt cleanly in Rust + Tauri v2 + MD3.
- **Kani 0.68.0 CBMC OOM root cause & fix:** Running `cargo kani` directly on `String`/`Vec`/`format!()` functions OOMs CBMC in `std::fmt`/`memchr_aligned`. Separating allocation-free `Copy`/enum/slice core predicates in `verity-core` (delegated to by the String-bearing public APIs) makes all 6 Kani harnesses verify in 0.2s–9.7s each with 22/22 `kani::cover!` probes satisfied. Also note `--enable-unstable` is obsolete in Kani 0.68.0.
- **Tauri v2 path & Windows specifics:**
  - `beforeDevCommand` / `beforeBuildCommand` in `desktop/src-tauri/tauri.conf.json` execute with `cwd = desktop/` (parent of `src-tauri/`), so they must run `npm --prefix ui run dev` and `npm --prefix ui run build`.
  - Windows `tauri-build` requires `desktop/src-tauri/icons/icon.ico` (multi-size 16/32/48/256 RGBA ICO) in `bundle.icon`.
  - Windows process teardown uses `taskkill /PID <pid> /T /F` under `#[cfg(windows)]`.
  - Windows global npm shims for `pi` are `.cmd` files (`pi.cmd`), handled by `resolve_pi_program()` in `pi_session.rs`.
- **Material Design 3 library choice:** Google's `@material/web` is in maintenance mode, so the UI computes MD3 light/dark tonal palettes dynamically via `@material/material-color-utilities` (`desktop/ui/src/theme/md3.ts`).
- **Sandbox VM tips (if rebuilding in a new container):** `/tmp` is mounted `noexec`, so set `export TMPDIR="$HOME/.tmp"` for Cargo/Kani builds, and set `export NODE_EXTRA_CA_CERTS=/etc/ssl/certs/ca-certificates.crt` for `npm install` on Node 22.

## Accomplished

Current `main` HEAD: **`7de2fc7`** on `https://github.com/RoyCoding8/verity-sandbox`. All temporary worktrees and `wt/*` branches are cleaned up.

- **Pure verifiable core (`crates/verity-core`) + Kani proofs (`formal/`):**
  - Branded types (`TaskId`, `CommitOid`, `RelPath`, `SpecDigest`), zero-dep SHA-256 and JSON parser, write-scope containment, Kani report + `cover!` verdict classifier, task lifecycle state machine (`transition`), Pi session exclusivity/queue/deferred-reload state machine (`transition_session`), and custom provider URL validation (`validate_provider_url`, `validate_external_sign_in_url`).
  - 6 symbolic Kani 0.68.0 harnesses (22/22 `kani::cover!` probes satisfied) passing in CI (`formal.yml`).
- **Runtime & CLI (`crates/verity-runtime`, `crates/verity-cli`):**
  - Scrubbed native `git worktree add --detach --lock`, lock-free `git merge-tree --write-tree` with structured conflict detection, CAS `git update-ref --no-deref`, rootless Linux `bubblewrap` / macOS `sandbox-exec` offline runner keeping `$GIT_COMMON_DIR` read-only, `O_NOFOLLOW` `WorkspaceJail`, comment/string-aware Rust & Lean hygiene lints, automatic `#[kani::proof]` / `kani::cover!` target discovery (`discover_kani_targets`), live sandboxed Kani/Lean verifier execution in `spawn_write_task_with_options`, spec-only `SpecHandoff` extraction, append-only `.verity/run-log.jsonl`, CLI (`run-readonly`, `spawn`, `review`, `merge`, `log`), and live STDIO MCP server (`verity mcp`).
- **Tauri v2 desktop backend (`desktop/src-tauri/`):**
  - Long-lived `PiSessionSupervisor` covering all 24 Pi JSONL RPC stdin commands and 10 stdout events, 10 MB image attachment validator, Extension UI FIFO tracker, atomic `0o600` custom provider store (never leaking API keys to WebView, injecting `VERITY_DESKTOP_PROVIDERS` + `VERITY_DESKTOP_KEY_*` + `VERITY_MCP_BIN` into Pi child env), and all 28 Tauri IPC commands registered in `main.rs` (18 `pi_*` + 4 `providers::*` + 6 `verity_bridge::*`).
  - Fixed empty `"default"` model dropdown and stuck `"Medium"` thinking level (`91254e4`, `a7b4ad3`): added multi-source model catalog merging + fallback catalog, degraded-mode model/thinking persistence with replay on live Pi start, Windows `pi.cmd` resolution, optimistic UI updates with rollback/error snackbars, and provider-to-model dropdown syncing.
- **Material Design 3 frontend (`desktop/ui/`):**
  - User completed a major frontend overhaul (`9dfd201`, 25 files) on top of the three-pane MD3 workspace (Navigation Drawer, streaming markdown chat, Composer with `Md3Select`, History Tree, Fork, Provider Settings, FIFO Extension UI, Subagent Rail, Spec Review Drawer, Merge Queue, Run Log).
- **CI pipelines (`.github/workflows/`):**
  - `formal.yml` (Kani 0.68.0), `tests.yml` (Rust fmt/clippy/tests + UI typecheck/build), and `desktop.yml` (`ubuntu-latest`, `macos-latest`, `windows-latest`).

### Remaining / Next Up
- Verify the user's live Windows run of `git pull && npm run tauri:dev` on `7de2fc7` (confirming Model and Thinking dropdowns now behave cleanly).
- Test a live end-to-end prompt against an installed `pi` CLI + real provider API key, and wire live `pi_bootstrap` project switching / session listing if needed.
- Wire live `run_live_verifier: true` toggle in `desktop/src-tauri/src/verity_bridge.rs` + real blind back-translation LLM call in `handoff.rs` when testing full write-subagent formal proof loops.

## Relevant files

- `Cargo.toml`, `package.json`, `desktop/package.json`, `README.md` — Root workspace and npm quick-start (`npm run setup`, `npm run tauri:dev`, `npm run dev`, `npm test`, `npm run formal`).
- `crates/verity-core/src/{types,scope,verdict,state_machine,session,provider,json}.rs` — Zero-IO pure Rust domain core.
- `formal/src/{state_machine,scope_policy,verdict_gate,session_laws}.rs` — 6 Kani 0.68.0 proof harnesses + exhaustive unit tests.
- `crates/verity-runtime/src/{git,sandbox,jail,verifiers,pi_rpc,handoff,orchestrator}.rs` — Worktrees, OS sandbox, hygiene lints, Kani/Lean discovery & runners, Pi RPC, write-task orchestrator.
- `crates/verity-cli/src/{main,mcp}.rs` — Headless CLI and local STDIO MCP server.
- `desktop/src-tauri/src/{main,lib,pi_session,pi_ipc,attachments,extension_ui,providers,verity_bridge}.rs` — Tauri v2 backend, Pi RPC supervisor, 28 IPC commands, `0o600` provider store.
- `desktop/src-tauri/{Cargo.toml,tauri.conf.json,icons/icon.ico,icons/icon.png}` — Tauri v2 config and cross-platform icons.
- `desktop/ui/src/{App.tsx,md3-tokens.css,theme/md3.ts,api/tauri.ts,state/piStore.ts,types/verity.ts}` and `desktop/ui/src/components/*.tsx` — Overhauled Google Material Design 3 React frontend.
- `.github/workflows/{formal,tests,desktop}.yml` — Kani, Rust/UI tests, and Linux/macOS/Windows Tauri CI.
