import { useCallback, useEffect, useMemo, useState } from "react";
import "./md3-tokens.css";
import "./components/AppShell.css";
import {
  isTauriRuntime,
  tauriApi,
  type ExtensionUiAnswer,
  type PiSessionStats,
} from "./api/tauri";
import { ChatTranscript, STARTER_SUGGESTIONS } from "./components/ChatTranscript";
import { Composer } from "./components/Composer";
import {
  ExtensionToastLayer,
  type ExtensionToastItem as ToastLayerItem,
} from "./components/ExtensionToastLayer";
import { ExtensionUiLayer } from "./components/ExtensionUiLayer";
import { ForkModal } from "./components/ForkModal";
import { HistoryTreeModal } from "./components/HistoryTreeModal";
import {
  AlertIcon,
  CloseIcon,
  CopyIcon,
  GitMergeIcon,
  SettingsIcon,
  ShieldCheckIcon,
  TerminalIcon,
} from "./components/Icons";
import { MergeQueueModal } from "./components/MergeQueueModal";
import { ProviderSettingsModal } from "./components/ProviderSettingsModal";
import { RunLogDrawer } from "./components/RunLogDrawer";
import {
  SessionSettingsModal,
  type SessionStatsView,
  type SlashCommandDirectoryItem,
} from "./components/SessionSettingsModal";
import { Sidebar } from "./components/Sidebar";
import { SpecReviewDrawer } from "./components/SpecReviewDrawer";
import { SubagentRail } from "./components/SubagentRail";
import { displaySlashName, mergeSlashCommands } from "./lib/slashCommands";
import { usePiWorkspace } from "./state/piStore";
import type {
  HandoffView,
  VerdictStatus,
  MergeQueueEntry,
  SpawnSubagentRequest,
  SubagentTaskItem,
} from "./types/verity";

function parseVerdictStatus(raw: string): VerdictStatus {
  const lower = raw.toLowerCase();
  if (lower === "pass") return "pass";
  if (lower === "counterexample") return "counterexample";
  if (lower === "inconclusive") return "inconclusive";
  return "invalid_report";
}

function toSessionStatsView(stats: PiSessionStats | null): SessionStatsView | null {
  if (!stats) return null;
  return {
    sessionId: stats.sessionId ?? undefined,
    userMessages: stats.userMessages,
    assistantMessages: stats.assistantMessages,
    toolCalls: stats.toolCalls,
    totalMessages: stats.totalMessages,
    inputTokens: stats.tokens?.input,
    outputTokens: stats.tokens?.output,
    cacheReadTokens: stats.tokens?.cacheRead,
    cacheWriteTokens: stats.tokens?.cacheWrite,
    totalTokens: stats.tokens?.total,
    cost: stats.cost,
    contextWindow: stats.contextWindow ?? undefined,
    contextUsagePercent: stats.contextUsagePercent ?? undefined,
  };
}

export function App() {
  const { state, projectSessions, activeSession, actions } = usePiWorkspace();
  const desktopRuntime = isTauriRuntime();

  const [verityPanelOpen, setVerityPanelOpen] = useState(false);
  const [mergeQueueOpen, setMergeQueueOpen] = useState(false);
  const [runLogOpen, setRunLogOpen] = useState(false);
  const [pendingComposerInsert, setPendingComposerInsert] = useState<string | null>(
    null
  );

  const [subagents, setSubagents] = useState<SubagentTaskItem[]>([]);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [approvedIds, setApprovedIds] = useState<Record<string, boolean>>({});
  const [mergeEntries, setMergeEntries] = useState<MergeQueueEntry[]>([]);
  const [rawLogLines, setRawLogLines] = useState<string[]>([]);
  const [spawning, setSpawning] = useState(false);
  const [shellError, setShellError] = useState<string | null>(null);
  const [dismissedStoreError, setDismissedStoreError] = useState<string | null>(
    null
  );

  useEffect(() => {
    if (state.error === null) {
      setDismissedStoreError(null);
    }
  }, [state.error]);

  const activeProject = useMemo(
    () => state.projects.find((p) => p.id === state.activeProjectId) ?? null,
    [state.projects, state.activeProjectId]
  );
  const activeProjectId = activeProject?.id ?? null;
  const activeProjectPath = activeProject?.path ?? null;

  useEffect(() => {
    setMergeEntries([]);
    setApprovedIds({});
  }, [activeProjectId]);

  useEffect(() => {
    if (!desktopRuntime || !activeProjectPath) return;
    let cancelled = false;
    Promise.all([
      tauriApi.listHandoffs(activeProjectPath).catch(() => []),
      tauriApi.listRunLog(activeProjectPath).catch(() => []),
    ]).then(([handoffs, logs]) => {
      if (cancelled) return;
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
      setRawLogLines([...logs]);
    });
    return () => {
      cancelled = true;
    };
  }, [desktopRuntime, activeProjectId, activeProjectPath]);

  const selectedTask = useMemo(
    () => subagents.find((t) => t.taskId === selectedTaskId) ?? null,
    [subagents, selectedTaskId]
  );

  const pendingHandoffCount = useMemo(
    () => subagents.filter((t) => t.status === "awaiting_spec_review").length,
    [subagents]
  );

  const sessionStatsView = useMemo(
    () => toSessionStatsView(state.sessionStats),
    [state.sessionStats]
  );

  const contextUsagePercent = useMemo(() => {
    const raw = state.sessionStats?.contextUsagePercent;
    if (typeof raw !== "number" || !Number.isFinite(raw)) return null;
    return Math.max(0, Math.min(100, Math.round(raw)));
  }, [state.sessionStats]);

  const commandDirectoryItems = useMemo<SlashCommandDirectoryItem[]>(
    () =>
      mergeSlashCommands(state.commands).map((cmd) => ({
        name: cmd.normalizedName,
        displayName: displaySlashName(cmd.name),
        description: cmd.description || undefined,
        source: cmd.source,
        location: cmd.location,
        usage: cmd.usage,
      })),
    [state.commands]
  );

  const toastLayerItems = useMemo<ToastLayerItem[]>(
    () =>
      state.extensionToasts.map((toast, idx) => ({
        id: toast.id,
        message: toast.message,
        level: toast.notifyType,
        createdAt: idx,
      })),
    [state.extensionToasts]
  );

  const toastLayerWidgets = useMemo<Record<string, string[]>>(() => {
    const out: Record<string, string[]> = {};
    for (const [key, lines] of Object.entries(state.extensionWidgetLines)) {
      out[key] = [...lines];
    }
    return out;
  }, [state.extensionWidgetLines]);

  const extensionStatusEntries = useMemo(
    () => Object.entries(state.extensionStatus),
    [state.extensionStatus]
  );

  const effectiveEditorPrefill = pendingComposerInsert ?? state.editorPrefillText;

  const handleConsumeEditorPrefill = useCallback((): string | null => {
    if (pendingComposerInsert !== null) {
      const next = pendingComposerInsert;
      setPendingComposerInsert(null);
      return next;
    }
    return actions.consumeEditorPrefill();
  }, [pendingComposerInsert, actions.consumeEditorPrefill]);

  const handleInsertCommand = useCallback(
    (commandText: string) => {
      setPendingComposerInsert(commandText);
      actions.closeSettingsModal();
    },
    [actions.closeSettingsModal]
  );

  const handleRespondExtensionDialog = useCallback(
    (id: string, ans: ExtensionUiAnswer) => {
      void actions.respondExtensionDialog(id, ans);
    },
    [actions.respondExtensionDialog]
  );

  function handleToggleRunLog() {
    setRunLogOpen((open) => !open);
  }

  async function handleSpawnSubagent(req: SpawnSubagentRequest) {
    setSpawning(true);
    setShellError(null);
    try {
      const repoPath = activeProjectPath ?? ".";
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
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      setShellError(`Failed to spawn subagent: ${message}`);
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
          targetRef: "refs/heads/main",
          worktreePath: `.verity/worktrees/${handoff.task_id}`,
          handoff,
          approvedDigest: handoff.spec_digest,
          mergeTree: { kind: "unchecked" },
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
    const repoPath = activeProjectPath ?? ".";
    setMergeEntries((prev) =>
      prev.map((e) =>
        e.taskId === entry.taskId
          ? { ...e, mergeState: "merging", errorMessage: undefined }
          : e
      )
    );
    try {
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
                errorMessage: undefined,
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
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      setMergeEntries((prev) =>
        prev.map((e) =>
          e.taskId === entry.taskId
            ? { ...e, mergeState: "error", errorMessage: message }
            : e
        )
      );
    }
  }

  const visibleError =
    shellError ??
    (state.error !== null && state.error !== dismissedStoreError
      ? state.error
      : null);

  function handleDismissError() {
    if (shellError !== null) {
      setShellError(null);
    } else if (state.error !== null) {
      setDismissedStoreError(state.error);
    }
  }

  const hasMessages = state.messages.length > 0;
  const isHeroEmpty = !hasMessages && state.liveTools.length === 0;
  const rightRailOpen = verityPanelOpen || runLogOpen;

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
      bashRunning={state.bashRunning}
      editorPrefillText={effectiveEditorPrefill}
      variant={variant}
      onDraftChange={actions.setDraft}
      onConsumeEditorPrefill={handleConsumeEditorPrefill}
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
      onAbortBash={() => void actions.abortBash()}
    />
  );

  return (
    <div
      className={
        rightRailOpen
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
        onSelectProject={(path) => void actions.selectProject(path)}
        onPickAndAddProject={() => void actions.pickAndAddProject()}
        onAddProject={(path) => void actions.addProject(path)}
        onRemoveProject={(path) => void actions.removeProject(path)}
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
            {state.isCompacting && isHeroEmpty && (
              <span className="verity-status-pill verity-status-pill--compacting" role="status">
                <span className="verity-status-dot" aria-hidden="true" />
                Compacting...
              </span>
            )}
            {extensionStatusEntries.length > 0 && (
              <div className="verity-ext-status-strip" aria-label="Extension status">
                {extensionStatusEntries.map(([key, text]) => (
                  <span
                    key={key}
                    className="verity-ext-status-pill"
                    title={`${key}: ${text}`}
                  >
                    <span className="verity-ext-status-key">{key}</span>
                    <span className="verity-ext-status-text">{text}</span>
                  </span>
                ))}
              </div>
            )}
          </div>

          <div className="verity-topbar-actions">
            {contextUsagePercent !== null && (
              <button
                type="button"
                className="verity-topbar-pill verity-topbar-pill--context"
                onClick={() => actions.openSettingsModal("stats")}
                title="View session statistics and context usage"
                aria-label={`Context usage ${contextUsagePercent}%`}
              >
                <span>{contextUsagePercent}% ctx</span>
              </button>
            )}

            <button
              type="button"
              className="verity-topbar-pill"
              disabled={state.messages.length === 0}
              onClick={() => void actions.copyFullChat()}
              title="Copy full chat as Markdown"
            >
              <CopyIcon size={15} />
              <span className="verity-topbar-btn-label">Copy chat</span>
            </button>

            <button
              type="button"
              className={
                state.settingsModalOpen
                  ? "verity-topbar-pill verity-topbar-pill--active"
                  : "verity-topbar-pill"
              }
              aria-pressed={state.settingsModalOpen}
              onClick={() => actions.openSettingsModal("behavior")}
              title="Open session settings, stats, and skills"
            >
              <SettingsIcon size={15} />
              <span className="verity-topbar-btn-label">Session &amp; Skills</span>
            </button>

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
                runLogOpen
                  ? "verity-topbar-pill verity-topbar-pill--icon verity-topbar-pill--active"
                  : "verity-topbar-pill verity-topbar-pill--icon"
              }
              aria-pressed={runLogOpen}
              aria-label={runLogOpen ? "Hide run log" : "Show run log"}
              onClick={handleToggleRunLog}
              title={runLogOpen ? "Hide run log" : "Show run log"}
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

        {visibleError && (
          <div className="verity-error-banner" role="alert">
            <span className="verity-error-banner-content">
              <AlertIcon size={15} className="verity-error-banner-icon" />
              <span>{visibleError}</span>
            </span>
            <button
              type="button"
              className="md3-icon-btn md3-icon-btn-sm"
              aria-label="Dismiss error"
              onClick={handleDismissError}
            >
              <CloseIcon size={14} />
            </button>
          </div>
        )}

        {state.loading ? (
          <div className="verity-loading-stage" role="status" aria-live="polite">
            <div className="verity-loading-indicator">
              <span className="verity-loading-dot" aria-hidden="true" />
              <span>Loading workspace...</span>
            </div>
          </div>
        ) : isHeroEmpty ? (
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
                isCompacting={state.isCompacting}
                canRedoRevert={state.canRedoRevert}
                lastRevertedFiles={state.lastRevertedFiles}
                onAbortRetry={() => void actions.abortRetry()}
                onPickStarterPrompt={(prompt) => actions.setDraft(prompt)}
                onRevertTurn={(entryId, revertCode, fallbackPromptText) =>
                  void actions.revertTurn(entryId, revertCode, fallbackPromptText)
                }
                onRedoRevert={() => void actions.redoRevert()}
              />
            </div>

            {composerElement("docked")}
          </>
        )}
      </main>

      {rightRailOpen && (
        <aside
          className="verity-right-rail"
          aria-label={
            verityPanelOpen ? "Verification workspace" : "Run log"
          }
        >
          {runLogOpen && (
            <RunLogDrawer
              rawLines={rawLogLines}
              open={runLogOpen}
              onSelectTask={(taskId) => {
                setSelectedTaskId(taskId);
                setVerityPanelOpen(true);
              }}
              onClose={() => setRunLogOpen(false)}
            />
          )}
          {verityPanelOpen && (
            <>
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
            </>
          )}
        </aside>
      )}

      <HistoryTreeModal
        open={state.modal.kind === "history_tree"}
        nodes={state.modal.kind === "history_tree" ? state.modal.nodes : []}
        onForkFromNode={actions.forkAtEntry}
        onClose={actions.closeModal}
      />
      <ForkModal
        open={state.modal.kind === "fork"}
        candidates={state.modal.kind === "fork" ? state.modal.candidates : []}
        onFork={actions.forkAtEntry}
        onClose={actions.closeModal}
      />
      <ProviderSettingsModal
        open={state.modal.kind === "providers"}
        providers={state.providers}
        onSave={actions.upsertProvider}
        onDelete={actions.deleteProvider}
        onClose={actions.closeModal}
      />
      <SessionSettingsModal
        open={state.settingsModalOpen}
        initialTab={state.settingsModalTab}
        onClose={actions.closeSettingsModal}
        autoCompactionEnabled={state.autoCompactionEnabled}
        autoRetryEnabled={state.autoRetryEnabled}
        steeringMode={state.steeringMode}
        followUpMode={state.followUpMode}
        onToggleAutoCompaction={(enabled) => void actions.setAutoCompaction(enabled)}
        onToggleAutoRetry={(enabled) => void actions.setAutoRetry(enabled)}
        onChangeSteeringMode={(mode) => void actions.setSteeringMode(mode)}
        onChangeFollowUpMode={(mode) => void actions.setFollowUpMode(mode)}
        onCycleModel={() => void actions.cycleModel()}
        onCycleThinkingLevel={() => void actions.cycleThinkingLevel()}
        stats={sessionStatsView}
        onRefreshStats={() => void actions.refreshSessionStats()}
        commands={commandDirectoryItems}
        onInsertCommand={handleInsertCommand}
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
        onRespond={handleRespondExtensionDialog}
        onDismissSnackbar={actions.dismissSnackbar}
        onDismissWidget={actions.dismissWidget}
      />
      <ExtensionToastLayer
        toasts={toastLayerItems}
        onDismissToast={actions.dismissToast}
        widgets={toastLayerWidgets}
      />
    </div>
  );
}
