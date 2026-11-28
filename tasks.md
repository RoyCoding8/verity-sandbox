# Tasks

Model routing follows the user instruction. Use `codex` with GPT 6.1 Sol Medium for smaller tasks and High for tougher tasks. Use `antigravity_2` with Gemini Hydrogen for independent reviews. Do not substitute Muse or skill model defaults.

## Active repair. Bug fix playbook

- [x] 1. Reproduce it yourself on the matching surface via the control skill (Non-negotiables), even when a debug or instrumentation protocol says to ask the user to reproduce. Ask the user only with a stated, specific reason the control surface cannot reach the target, and only after driving it as far as it goes. If it won't reproduce directly, synthesize the trigger, tighten conditions, or instrument until it fires.
- [x] 2. Binary-search the cause. Form the candidate hypotheses, then rule them out until one survives. Seed them with `how` over the affected subsystem and the **why** skill for regression history. Each pass, take the split that cuts the most remaining problem space, get runtime evidence, eliminate. When program state is unclear, add instrumentation or logging and read it as the code runs. Don't guess. Drive a long or stubborn hunt with Cursor's `/loop` command. Confirm the surviving *mechanism* with runtime evidence before the step-3 architect/interrogate fan-out.
- [x] 3. Plan the fix. If it crosses a function boundary, `architect` first. Use your configured bug-fix line as a suggest→verify pipeline (default `deepai/deepseek-v4.1-flash@high` -> `deepai/mimo-2.6-pro@high`): the suggest model (read-only, never edits) proposes the root cause and fix; the verify model audits the suggestion against your runtime evidence and rejects what it cannot ground; the verify model then implements the accepted fix as a subagent with a specific scope.
- [ ] 4. Verify on the same surface. The original repro now passes. "Inconclusive" or wrong-surface is not a pass. Flag it. Unit tests show branch behavior, not bug absence.
- [x] 5. Stage the commits so the failing repro lands before the fix in git history. See the **tdd** skill for the failing-test-first cadence when the bug has a cheap local test path. Skip it when the test would be expensive, integration-heavy, or unclear.
- [x] 6. Run **Opening a PR**.

## Repair throughput checkpoint

- [x] Blocking first steps. Confirm the running main session has the extension, an empty CLI path, and no delegation tools.
- [x] Independent workstreams. Keep design review read-only while one isolated implementation owner writes the launcher and packaging fix.
- [x] Shared mutable state. Parent owns tasks.md and the decision ledger. The implementation owner writes only its worktree.
- [x] Smallest safe decomposition. Keep CLI preparation, discovery, extension diagnostics and their coupled regression checks under one owner.

## Delegation follow-up plan

The reported defect concerns the main agent. Nested worker delegation is a separate capability. The current runtime smoke uses real Pi and CLI processes with an isolated fixture model. It does not verify desktop provider propagation or native WebView interaction.

1. Finish the current CLI packaging repair. On Linux, Windows and macOS, verify the staged executable, all three active main-agent tools, visible missing-CLI diagnostics, and a real read-only child result.
2. Pass an explicit provider and model to each worker. Transfer only its required endpoint and credential through a trusted worker configuration, with no project extension loading or persistent repository secrets. Verify a desktop-configured loopback provider reaches read-only workers and Linux write workers without a pre-existing Pi models file. Verify unrelated credentials are absent. Write workers currently refuse to run on Windows and macOS because they require Linux bubblewrap. Keep that refusal explicit until a separate isolation design is implemented.
3. Preserve worker authority. Read-only workers retain read, grep, find and ls. Write workers keep declared scope, isolated Git worktrees and verifier gates. Check actual read-only filesystem effects on each platform, the Linux write sandbox and the Windows and macOS write-worker refusal. State where enforcement relies on tool restrictions rather than an OS sandbox.
4. Define task identity and lifecycle before asynchronous delegation. Persist started, running, cancelled, failed and completed states. Check cancellation stops the child process tree, restart does not duplicate work or publish twice, and unmerged work survives recovery.
5. Keep worker-to-worker delegation disabled until authority is explicit. A child must receive a bounded depth, shared task budget, cancellation ownership and scope no broader than its parent. Verify denied nesting, exhausted budgets and attempts to widen scope before enabling the capability.

Production points to change or retain are `pi_ipc.rs::build_pi_launch_env`, the embedded extension's tool definitions, `PiRpcConfig::default_pi`, `run_sandboxed_pi_prompt`, `run_readonly_pi_prompt`, `sandbox.rs::configure_environment`, `verity_bridge.rs` and `orchestrator.rs`. The sandbox clears its environment and Pi workers suppress extensions, so main-agent model registration alone does not configure those workers.

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
- [x] Make the desktop main session reliably locate its Verity CLI and expose missing setup instead of silently omitting delegation tools.
- [x] Verify the main agent's registered tool catalog and a real read-only delegation from the desktop session. Checked through the production Pi extension and freshly staged CLI; native WebView interaction remains untested.
- [x] Right now the agents cannot run subagents, we need to plan that properly. Research and plan
