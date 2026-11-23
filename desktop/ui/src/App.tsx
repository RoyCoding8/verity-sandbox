import { useEffect, useMemo, useState } from "react";
import "./md3-tokens.css";
import "./components/AppShell.css";
import { isTauriRuntime, tauriApi } from "./api/tauri";
import { ChatTranscript, STARTER_SUGGESTIONS } from "./components/ChatTranscript";
import { Composer } from "./components/Composer";
import { ExtensionUiLayer } from "./components/ExtensionUiLayer";
import { ForkModal } from "./components/ForkModal";
import { HistoryTreeModal } from "./components/HistoryTreeModal";
import {
  AlertIcon,
  GitMergeIcon,
  ShieldCheckIcon,
  TerminalIcon,
} from "./components/Icons";
import { MergeQueueModal } from "./components/MergeQueueModal";
import { ProviderSettingsModal } from "./components/ProviderSettingsModal";
import { RunLogDrawer } from "./components/RunLogDrawer";
import { Sidebar } from "./components/Sidebar";
import { SpecReviewDrawer } from "./components/SpecReviewDrawer";
import { SubagentRail } from "./components/SubagentRail";
import { usePiWorkspace } from "./state/piStore";
import { applyMd3Theme } from "./theme/md3";
import type {
  HandoffView,
  VerdictStatus,
  MergeQueueEntry,
  SpawnSubagentRequest,
  SubagentTaskItem,
} from "./types/verity";

const INITIAL_HANDOFF: HandoffView = {
  task_id: "task-session-laws",
  base_commit: "961b030c0a00bef3fe708972fffe56368abb2499",
  verifier: "kani",
  candidate_commit: "880ece1e7691f6c58c4a4f53285bd7741c518897",
  changed_paths: [
    "crates/verity-core/src/session.rs",
    "formal/src/session_laws.rs",
  ],
  extracted_spec: [
    "// crates/verity-core/src/session.rs",
    "18: pub enum SessionActivity { Idle, Streaming { pending_queue: u16, reload_deferred: bool }, Compacting, SwitchingSession }",
    "41: pub fn transition_session(activity: SessionActivity, cmd: SessionCommand) -> Result<SessionTransition, SessionError>",
    "// formal/src/session_laws.rs",
    "64: #[kani::proof]",
    "65: pub fn prove_session_exclusivity_and_queue_laws()",
    "92: assert!(next.activity == SessionActivity::Idle || !next.run_provider_reload);",
    "95: kani::cover!(next.run_provider_reload, \"deferred_reload_fires_on_settle\");",
  ].join("\n"),
  back_translation:
    "Prompts, compaction, and session switches are accepted only while the session is idle. Steering and follow-up messages are accepted only while streaming and stay bounded without integer overflow. Provider reloads requested mid-turn are deferred and fire only when agent_settled returns the session to idle.",
  hygiene_clean: true,
  hygiene_violations: [],
  verdict: {
    status: "pass",
    reason: "0 of 271 checks failed; 6 of 6 kani::cover! probes satisfied",
    checks_passed: 271,
    probes_satisfied: 6,
  },
  spec_digest:
    "7c9e2f4b18d3a50692f1e4c8b7a6d5e4f3c2b1a0987654321fedcba012345678",
};

const INITIAL_SUBAGENTS: SubagentTaskItem[] = [
  {
    taskId: "task-session-laws",
    goal: "Prove Pi session exclusivity and deferred reload laws",
    mode: "write",
    status: "awaiting_spec_review",
    verifier: "kani",
    worktreePath: ".worktrees/core-session-laws",
    writeScope: ["crates/verity-core/src/session.rs", "formal/src/session_laws.rs"],
    budget: {
      toolCallsUsed: 12,
      toolCallsLimit: 64,
      wallClockSecsUsed: 42,
      wallClockSecsLimit: 300,
    },
    handoff: INITIAL_HANDOFF,
  },
  {
    taskId: "audit-sandbox-binds",
    goal: "Audit bubblewrap read-only $GIT_COMMON_DIR mount",
    mode: "readonly",
    status: "completed_readonly",
    verifier: null,
    worktreePath: null,
    writeScope: [],
    budget: {
      toolCallsUsed: 5,
      toolCallsLimit: 32,
      wallClockSecsUsed: 11,
      wallClockSecsLimit: 120,
    },
    readonlySummary:
      "Verified sandbox.rs binds $GIT_COMMON_DIR read-only and passes --unshare-net on Linux.",
  },
];

const INITIAL_LOG_LINES: string[] = [
  '{"task_id":"task-session-laws","kind":"awaiting_spec_review","detail":"Kani 6/6 proofs verified, 22/22 cover! probes satisfied"}',
  '{"task_id":"audit-sandbox-binds","kind":"completed_readonly","detail":"Verified read-only .git bind mount under bubblewrap"}',
];

function parseVerdictStatus(raw: string): VerdictStatus {
  const lower = raw.toLowerCase();
  if (lower === "pass") return "pass";
  if (lower === "counterexample") return "counterexample";
  if (lower === "inconclusive") return "inconclusive";
  return "invalid_report";
}

export function App() {
  const { state, projectSessions, activeSession, actions } = usePiWorkspace();
  const desktopRuntime = isTauriRuntime();

  const [verityPanelOpen, setVerityPanelOpen] = useState(false);
  const [mergeQueueOpen, setMergeQueueOpen] = useState(false);
  const [runLogOpen, setRunLogOpen] = useState(false);

  const [subagents, setSubagents] = useState<SubagentTaskItem[]>(INITIAL_SUBAGENTS);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>("task-session-laws");
  const [approvedIds, setApprovedIds] = useState<Record<string, boolean>>({});
  const [mergeEntries, setMergeEntries] = useState<MergeQueueEntry[]>([]);
  const [rawLogLines, setRawLogLines] = useState<string[]>(INITIAL_LOG_LINES);
  const [spawning, setSpawning] = useState(false);

  useEffect(() => {
    applyMd3Theme(state.theme);
  }, [state.theme]);

  const activeProject = useMemo(
    () => state.projects.find((p) => p.id === state.activeProjectId) ?? null,
    [state.projects, state.activeProjectId]
  );

  useEffect(() => {
    if (!desktopRuntime || !activeProject) return;
    let cancelled = false;
    Promise.all([
      tauriApi.listHandoffs(activeProject.path).catch(() => []),
      tauriApi.listRunLog(activeProject.path).catch(() => []),
    ]).then(([handoffs, logs]) => {
      if (cancelled) return;
      if (handoffs.length > 0) {
        const mapped: SubagentTaskItem[] = handoffs.map((h) => ({
          taskId: h.task_id,
          goal: `Write task ${h.task_id}`,
          mode: "write",
          status: "awaiting_spec_review",
          verifier: h.verifier === "lean" ? "lean" : "kani",
          worktreePath: `.verity/worktrees/${h.task_id}`,
          writeScope: [...h.changed_paths],
          budget: {
            toolCallsUsed: 0,
            toolCallsLimit: 64,
            wallClockSecsUsed: 0,
            wallClockSecsLimit: 300,
          },
          handoff: {
            ...h,
            verifier: h.verifier === "lean" ? "lean" : "kani",
            changed_paths: [...h.changed_paths],
            hygiene_violations: [...h.hygiene_violations],
            verdict: { ...h.verdict, status: parseVerdictStatus(h.verdict.status) },
          },
        }));
        setSubagents(mapped);
        setSelectedTaskId(mapped[0]?.taskId ?? null);
      }
      if (logs.length > 0) {
        setRawLogLines([...logs]);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [desktopRuntime, activeProject]);

  const selectedTask = useMemo(
    () => subagents.find((t) => t.taskId === selectedTaskId) ?? null,
    [subagents, selectedTaskId]
  );

  const pendingHandoffCount = useMemo(
    () => subagents.filter((t) => t.status === "awaiting_spec_review").length,
    [subagents]
  );

  function handleToggleRunLog() {
    setRunLogOpen((open) => {
      const next = !open;
      if (next) setVerityPanelOpen(true);
      return next;
    });
  }

  async function handleSpawnSubagent(req: SpawnSubagentRequest) {
    setSpawning(true);
    try {
      const repoPath = activeProject?.path ?? ".";
      if (req.mode === "readonly") {
        const res = await tauriApi.runReadonlySubagent(repoPath, req.goal);
        const taskId = `ro-${Date.now().toString(36)}`;
        const item: SubagentTaskItem = {
          taskId,
          goal: req.goal,
          mode: "readonly",
          status: "completed_readonly",
          verifier: null,
          worktreePath: null,
          writeScope: [],
          budget: {
            toolCallsUsed: res.tool_calls,
            toolCallsLimit: 32,
            wallClockSecsUsed: 3,
            wallClockSecsLimit: 120,
          },
          readonlySummary: res.summary,
        };
        setSubagents((prev) => [item, ...prev]);
        setSelectedTaskId(taskId);
        setRawLogLines((prev) => [
          JSON.stringify({ task_id: taskId, kind: "completed_readonly", detail: res.summary }),
          ...prev,
        ]);
      } else {
        const handoffRaw = await tauriApi.spawnWriteSubagent({
          repo_path: repoPath,
          task_id: req.taskId,
          base_ref: "HEAD",
          write_scope: [...req.writeScope],
          forbidden: [],
          verifier: req.verifier,
          goal: req.goal,
        });
        const handoff: HandoffView = {
          ...handoffRaw,
          verifier: handoffRaw.verifier === "lean" ? "lean" : "kani",
          changed_paths: [...handoffRaw.changed_paths],
          hygiene_violations: [...handoffRaw.hygiene_violations],
          verdict: { ...handoffRaw.verdict, status: parseVerdictStatus(handoffRaw.verdict.status) },
        };
        const item: SubagentTaskItem = {
          taskId: handoff.task_id,
          goal: req.goal,
          mode: "write",
          status: "awaiting_spec_review",
          verifier: handoff.verifier,
          worktreePath: `.verity/worktrees/${handoff.task_id}`,
          writeScope: [...handoff.changed_paths],
          budget: {
            toolCallsUsed: 8,
            toolCallsLimit: 64,
            wallClockSecsUsed: 14,
            wallClockSecsLimit: 300,
          },
          handoff,
        };
        setSubagents((prev) => [item, ...prev]);
        setSelectedTaskId(item.taskId);
        setRawLogLines((prev) => [
          JSON.stringify({
            task_id: item.taskId,
            kind: "awaiting_spec_review",
            detail: handoff.spec_digest,
          }),
          ...prev,
        ]);
      }
    } finally {
      setSpawning(false);
    }
  }

  function handleApproveSpec(handoff: HandoffView) {
    setApprovedIds((prev) => ({ ...prev, [handoff.task_id]: true }));
    setSubagents((prev) =>
      prev.map((t) =>
        t.taskId === handoff.task_id ? { ...t, status: "approved_for_merge" } : t
      )
    );
    setMergeEntries((prev) => {
      if (prev.some((e) => e.taskId === handoff.task_id)) return prev;
      return [
        {
          taskId: handoff.task_id,
          targetRef: "refs/heads/master",
          worktreePath: `.verity/worktrees/${handoff.task_id}`,
          handoff,
          approvedDigest: handoff.spec_digest,
          mergeTree: { kind: "clean" },
          mergeState: "ready",
        },
        ...prev,
      ];
    });
    setMergeQueueOpen(true);
  }

  function handleRejectSpec(handoff: HandoffView, reason?: string) {
    setSubagents((prev) =>
      prev.map((t) =>
        t.taskId === handoff.task_id ? { ...t, status: "rejected" } : t
      )
    );
    setRawLogLines((prev) => [
      JSON.stringify({
        task_id: handoff.task_id,
        kind: "rejected",
        detail: reason || "Spec rejected by reviewer",
      }),
      ...prev,
    ]);
  }

  async function handleTriggerMerge(entry: MergeQueueEntry) {
    const repoPath = activeProject?.path ?? ".";
    setMergeEntries((prev) =>
      prev.map((e) =>
        e.taskId === entry.taskId ? { ...e, mergeState: "merging" } : e
      )
    );
    const res = await tauriApi.approveAndMergeHandoff({
      repoPath,
      taskId: entry.taskId,
      targetRef: entry.targetRef,
      approvedDigest: entry.approvedDigest,
    });
    setMergeEntries((prev) =>
      prev.map((e) =>
        e.taskId === entry.taskId
          ? {
              ...e,
              mergeState: res.status === "merged" ? "merged" : "conflict",
              mergedCommit: res.commit ?? undefined,
              alreadyMerged: res.already_merged,
              mergeTree:
                res.conflicts.length > 0
                  ? { kind: "conflict", conflictedPaths: [...res.conflicts] }
                  : { kind: "clean" },
            }
          : e
      )
    );
    if (res.status === "merged") {
      setSubagents((prev) =>
        prev.map((t) =>
          t.taskId === entry.taskId ? { ...t, status: "merged" } : t
        )
      );
      setRawLogLines((prev) => [
        JSON.stringify({
          task_id: entry.taskId,
          kind: "merged",
          detail: res.commit ?? entry.approvedDigest,
        }),
        ...prev,
      ]);
    }
  }

  const hasMessages = state.messages.length > 0;
  const isHeroEmpty = !hasMessages && state.liveTools.length === 0;
  const runLogVisible = verityPanelOpen && runLogOpen;

  const composerElement = (variant: "hero" | "docked") => (
    <Composer
      draft={state.draft}
      attachments={state.attachments}
      models={state.models}
      selectedModel={state.selectedModel}
      thinkingLevels={state.thinkingLevels}
      thinkingLevel={state.thinkingLevel}
      commands={state.commands}
      isStreaming={state.isStreaming}
      pendingQueueCount={state.pendingQueueCount}
      variant={variant}
      onDraftChange={actions.setDraft}
      onRecallUp={actions.recallUp}
      onRecallDown={actions.recallDown}
      onAddAttachment={actions.addAttachment}
      onRemoveAttachment={actions.removeAttachment}
      onSelectModel={(p, m) => void actions.selectModel(p, m)}
      onSelectThinkingLevel={(l) => void actions.selectThinkingLevel(l)}
      onSend={() => void actions.sendPrompt()}
      onSteer={() => void actions.steerPrompt()}
      onFollowUp={() => void actions.followUpPrompt()}
      onClearQueue={() => void actions.clearQueue()}
      onAbort={() => void actions.abortTurn()}
    />
  );

  return (
    <div
      className={
        verityPanelOpen
          ? "verity-shell verity-shell--panel-open"
          : "verity-shell"
      }
    >
      <Sidebar
        projects={state.projects}
        activeProjectId={state.activeProjectId}
        sessions={projectSessions}
        activeSessionId={state.activeSessionId}
        theme={state.theme}
        isDesktop={desktopRuntime}
        onSelectProject={(id) => void actions.selectProject(id)}
        onSelectSession={(s) => void actions.switchSession(s)}
        onCreateSession={() => void actions.createSession()}
        onRenameSession={(id, name) => void actions.renameSession(id, name)}
        onOpenHistoryTree={() => void actions.openHistoryTree()}
        onOpenFork={() => void actions.openForkModal()}
        onCloneSession={() => void actions.cloneCurrentSession()}
        onExportHtml={() => void actions.exportSessionHtml()}
        onOpenProviders={() => void actions.openProvidersModal()}
        onToggleTheme={actions.toggleTheme}
      />

      <main className="verity-main">
        <header className="verity-topbar">
          <div className="verity-topbar-left">
            {hasMessages && (
              <>
                <h1 className="verity-topbar-title">
                  {activeSession?.name ?? "Verity"}
                </h1>
                {state.isStreaming && (
                  <span className="verity-status-pill" role="status">
                    <span className="verity-status-dot" aria-hidden="true" />
                    Running
                  </span>
                )}
              </>
            )}
          </div>

          <div className="verity-topbar-actions">
            <button
              type="button"
              className={
                mergeEntries.length > 0
                  ? "verity-topbar-pill verity-topbar-pill--highlighted"
                  : "verity-topbar-pill"
              }
              onClick={() => setMergeQueueOpen(true)}
              title="Open merge queue"
            >
              <GitMergeIcon size={15} />
              <span className="verity-topbar-btn-label">Merge Queue</span>
              {mergeEntries.length > 0 && (
                <span className="verity-topbar-badge">
                  {mergeEntries.length}
                </span>
              )}
            </button>

            <button
              type="button"
              className={
                runLogVisible
                  ? "verity-topbar-pill verity-topbar-pill--icon verity-topbar-pill--active"
                  : "verity-topbar-pill verity-topbar-pill--icon"
              }
              aria-pressed={runLogVisible}
              aria-label={runLogVisible ? "Hide run log" : "Show run log"}
              onClick={handleToggleRunLog}
              title={runLogVisible ? "Hide run log" : "Show run log"}
            >
              <TerminalIcon size={15} />
            </button>

            <button
              type="button"
              className={
                verityPanelOpen
                  ? "verity-topbar-pill verity-topbar-pill--active"
                  : "verity-topbar-pill"
              }
              aria-pressed={verityPanelOpen}
              onClick={() => setVerityPanelOpen((v) => !v)}
              title={
                verityPanelOpen
                  ? "Hide verification panel"
                  : "Show verification panel"
              }
            >
              <ShieldCheckIcon size={15} />
              <span className="verity-topbar-btn-label">Verification</span>
              {pendingHandoffCount > 0 ? (
                <span className="verity-topbar-badge">
                  {pendingHandoffCount}
                </span>
              ) : (
                subagents.length > 0 && (
                  <span className="verity-topbar-dot" aria-hidden="true" />
                )
              )}
            </button>
          </div>
        </header>

        {isHeroEmpty ? (
          <div className="verity-hero-stage">
            <div className="verity-hero-content">
              <h1 className="verity-hero-headline">Where should we start?</h1>

              {state.retryBanner && (
                <div className="pi-retry-banner" role="status">
                  <span className="pi-retry-banner-text">
                    <AlertIcon size={15} />
                    <span>
                      Auto-retrying request (attempt {state.retryBanner.attempt} of{" "}
                      {state.retryBanner.maxAttempts},{" "}
                      {Math.ceil(state.retryBanner.delayMs / 1000)}s backoff)...
                    </span>
                  </span>
                  <button
                    type="button"
                    className="md3-btn-outlined"
                    onClick={() => void actions.abortRetry()}
                  >
                    Abort retry
                  </button>
                </div>
              )}

              {composerElement("hero")}

              <div className="verity-hero-suggestions">
                {STARTER_SUGGESTIONS.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className="verity-suggestion-line"
                    onClick={() => actions.setDraft(item.prompt)}
                  >
                    <span className="verity-suggestion-icon" aria-hidden="true">
                      {item.icon}
                    </span>
                    <span>{item.label}</span>
                  </button>
                ))}
              </div>
            </div>
          </div>
        ) : (
          <>
            <div className="verity-transcript-stage">
              <ChatTranscript
                messages={state.messages}
                isStreaming={state.isStreaming}
                liveTools={state.liveTools}
                retryBanner={state.retryBanner}
                onAbortRetry={() => void actions.abortRetry()}
                onPickStarterPrompt={(prompt) => actions.setDraft(prompt)}
              />
            </div>

            {composerElement("docked")}
          </>
        )}
      </main>

      {verityPanelOpen && (
        <aside
          className="verity-right-rail"
          aria-label="Verification workspace"
        >
          <SubagentRail
            tasks={subagents}
            selectedTaskId={selectedTaskId}
            onSelectTask={setSelectedTaskId}
            onSpawnSubagent={handleSpawnSubagent}
            spawning={spawning}
          />
          <SpecReviewDrawer
            handoff={selectedTask?.handoff ?? null}
            approved={
              selectedTask ? Boolean(approvedIds[selectedTask.taskId]) : false
            }
            onApprove={handleApproveSpec}
            onReject={handleRejectSpec}
          />
          {runLogOpen && (
            <RunLogDrawer
              rawLines={rawLogLines}
              open={runLogOpen}
              onSelectTask={setSelectedTaskId}
              onClose={() => setRunLogOpen(false)}
            />
          )}
        </aside>
      )}

      <HistoryTreeModal
        open={state.modal.kind === "history_tree"}
        nodes={state.modal.kind === "history_tree" ? state.modal.nodes : []}
        onForkFromNode={(id) => void actions.forkAtEntry(id)}
        onClose={actions.closeModal}
      />
      <ForkModal
        open={state.modal.kind === "fork"}
        candidates={state.modal.kind === "fork" ? state.modal.candidates : []}
        onFork={(id) => void actions.forkAtEntry(id)}
        onClose={actions.closeModal}
      />
      <ProviderSettingsModal
        open={state.modal.kind === "providers"}
        providers={state.providers}
        onSave={actions.upsertProvider}
        onDelete={actions.deleteProvider}
        onClose={actions.closeModal}
      />
      <MergeQueueModal
        open={mergeQueueOpen}
        entries={mergeEntries}
        onTriggerMerge={handleTriggerMerge}
        onInspectSpec={(id) => {
          setSelectedTaskId(id);
          setMergeQueueOpen(false);
          setVerityPanelOpen(true);
        }}
        onClose={() => setMergeQueueOpen(false)}
      />
      <ExtensionUiLayer
        dialogs={state.extensionDialogs}
        snackbars={state.extensionSnackbars}
        widgets={state.extensionWidgets}
        onRespond={(id, ans) => void actions.respondExtensionDialog(id, ans)}
        onDismissSnackbar={actions.dismissSnackbar}
        onDismissWidget={actions.dismissWidget}
      />
    </div>
  );
}
