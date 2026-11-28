# Tasks

- [x] Inspect the provider setup flow and the verification-min-cover procedure.
- [x] Run baseline workspace tests and frontend checks.
- [x] Compare draft-settings scanning with saved-provider-only scanning.
- [x] Blocking first steps: verify the baseline, command boundaries, and credential rules before implementation.
- [x] Independent workstreams: run read-only auditing separately from the model-scan implementation.
- [x] Shared mutable state: isolate implementation in its own Git worktree and integrate serially.
- [x] Smallest safe decomposition: one implementation owner for the coupled Rust command, TypeScript API, and provider form.
- [x] Implement Scan for models and focused boundary and UI checks.
- [x] Verify provider saving, failures, stale scan results, and secret handling.
- [x] Complete the app audit and independently check confirmed findings.
- [x] Repair confirmed handoff schema, Windows URL opening, and non-main merge-target defects in an isolated worktree.
- [x] Run local workspace, desktop, frontend, Verus, TLC and production trace checks; record their scope and gaps.
- [x] Embed the Common Controls v6 manifest in Windows test binaries and rerun native tests.
- [ ] Run the existing Linux, Windows and macOS CI jobs on a disposable verification branch, including the unfinished Kani proofs.
- [x] Review and integrate verified work, then clean up the finished worktrees.
- [ ] Right now the agents cannot run subagents, we need to plan that properly. Research and plan
