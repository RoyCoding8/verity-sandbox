# Tasks

Model routing follows the user instruction. Use `codex` with GPT 6.1 Sol Medium for smaller tasks and High for tougher tasks. Use `antigravity_2` with Gemini Hydrogen for independent reviews. Do not substitute Muse or skill model defaults.

## Active repair. Bug fix playbook

- [x] 1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables), even when a debug or instrumentation protocol says to ask the user to reproduce. Ask the user only with a stated, specific reason the control surface cannot reach the target, and only after driving it as far as it goes. If it won't reproduce directly, synthesize the trigger, tighten conditions, or instrument until it fires.
- [x] 2. Binary-search the cause. Form the candidate hypotheses, then rule them out until one survives. Seed them with `how` over the affected subsystem and the **why** skill for regression history. Each pass, take the split that cuts the most remaining problem space, get runtime evidence, eliminate. When program state is unclear, add instrumentation or logging and read it as the code runs. Don't guess. Drive a long or stubborn hunt with Cursor's `/loop` command. Confirm the surviving *mechanism* with runtime evidence before the step-3 architect/interrogate fan-out.
- [ ] 3. Plan the fix. If it crosses a function boundary, `architect` first. Use your configured bug-fix line as a suggest→verify pipeline (default `deepai/deepseek-v4.1-flash@high` -> `deepai/mimo-2.6-pro@high`): the suggest model (read-only, never edits) proposes the root cause and fix; the verify model audits the suggestion against your runtime evidence and rejects what it cannot ground; the verify model then implements the accepted fix as a subagent with a specific scope.
- [ ] 4. Verify on the same surface. The original repro now passes. "Inconclusive" or wrong-surface is not a pass. Flag it. Unit tests show branch behavior, not bug absence.
- [ ] 5. Stage the commits so the failing repro lands before the fix in git history. See the **tdd** skill for the failing-test-first cadence when the bug has a cheap local test path. Skip it when the test would be expensive, integration-heavy, or unclear.
- [ ] 6. Run **Opening a PR**.

## Repair throughput checkpoint

- [x] Blocking first steps. Confirm the running main session has the extension, an empty CLI path, and no delegation tools.
- [ ] Independent workstreams. Keep design review read-only while one isolated implementation owner writes the launcher and packaging fix.
- [ ] Shared mutable state. Parent owns tasks.md and the decision ledger. The implementation owner writes only its worktree.
- [ ] Smallest safe decomposition. Keep CLI preparation, discovery, extension diagnostics and their coupled regression checks under one owner.

## Existing task history

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
- [x] Run the existing Linux, Windows and macOS CI jobs on a disposable verification branch, including all nine Kani proofs and live verifier integration.
- [x] Review and integrate verified work, then clean up the finished worktrees.
- [ ] Make the desktop main session reliably locate its Verity CLI and expose missing setup instead of silently omitting delegation tools.
- [ ] Verify the main agent's registered tool catalog and a real read-only delegation from the desktop session.
- [ ] Right now the agents cannot run subagents, we need to plan that properly. Research and plan
