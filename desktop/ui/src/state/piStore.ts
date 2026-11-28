import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import {
  tauriApi,
  type DialogKind,
  type ExtensionToastItem,
  type ExtensionUiAnswer,
  type ForkCandidate,
  type HistoryTreeNode,
  type ModelOption,
  type PendingAttachment,
  type PiBashResult,
  type PiBootstrapData,
  type PiChatMessage,
  type PiProject,
  type PiQueueMode,
  type PiRetryBanner,
  type PiSessionStats,
  type PiSessionSummary,
  type PiToolCallState,
  type PiUiEvent,
  type ProviderProfileInput,
  type PublicProviderProfile,
  type SessionWithMessages,
  type SlashOrSkillCommand,
  type ThinkingLevel,
} from "../api/tauri";
import {
  parseBashPrefix,
  parseBuiltinSlashInvocation,
} from "../lib/slashCommands";
import { applyMd3Theme, detectPreferredTheme, type ThemeMode } from "../theme/md3";

// ============================================================
// Pure State Types
// ============================================================

export type ConnectionState = "connecting" | "connected" | "degraded" | "disconnected";

export interface RetryBannerState extends PiRetryBanner {}

export interface QueuedExtensionDialog {
  readonly id: string;
  readonly kind: DialogKind;
  readonly title: string;
  readonly options: readonly string[];
  readonly timeoutMs: number | null;
}

export interface ExtensionSnackbar {
  readonly id: string;
  readonly message: string;
  readonly tone: "info" | "error";
}

export interface ExtensionWidget {
  readonly id: string;
  readonly title: string;
  readonly body: string;
  readonly lines?: readonly string[];
}

export type SettingsModalTab = "behavior" | "stats" | "commands";

export type ActiveModal =
  | { readonly kind: "none" }
  | { readonly kind: "history_tree"; readonly nodes: readonly HistoryTreeNode[] }
  | { readonly kind: "fork"; readonly candidates: readonly ForkCandidate[] }
  | { readonly kind: "providers" };

export interface ComposerRecallState {
  readonly history: readonly string[];
  /** `null` when editing fresh draft; `0..history.length-1` when browsing recalled entries. */
  readonly index: number | null;
  readonly draftSnapshot: string;
}

export interface PiWorkspaceState {
  readonly theme: ThemeMode;
  readonly connectionState: ConnectionState;
  readonly loading: boolean;
  readonly error: string | null;
  readonly projects: readonly PiProject[];
  readonly sessions: readonly PiSessionSummary[];
  readonly activeProjectId: string;
  readonly activeSessionId: string;
  readonly messages: readonly PiChatMessage[];
  readonly isStreaming: boolean;
  readonly ignoreStaleTurnEvents: boolean;
  readonly liveTools: readonly PiToolCallState[];
  readonly retryBanner: RetryBannerState | null;
  readonly pendingQueueCount: number;
  readonly models: readonly ModelOption[];
  readonly selectedModel: ModelOption;
  readonly thinkingLevels: readonly ThinkingLevel[];
  readonly thinkingLevel: ThinkingLevel;
  readonly commands: readonly SlashOrSkillCommand[];
  readonly providers: readonly PublicProviderProfile[];
  readonly draft: string;
  readonly attachments: readonly PendingAttachment[];
  readonly recall: ComposerRecallState;
  readonly modal: ActiveModal;
  readonly extensionDialogs: readonly QueuedExtensionDialog[];
  readonly extensionSnackbars: readonly ExtensionSnackbar[];
  readonly extensionWidgets: readonly ExtensionWidget[];
  readonly extensionWidgetLines: Readonly<Record<string, readonly string[]>>;
  readonly extensionToasts: readonly ExtensionToastItem[];
  readonly extensionStatus: Readonly<Record<string, string>>;
  readonly editorPrefillText: string | null;
  readonly autoCompactionEnabled: boolean;
  readonly autoRetryEnabled: boolean;
  readonly steeringMode: PiQueueMode;
  readonly followUpMode: PiQueueMode;
  readonly sessionStats: PiSessionStats | null;
  readonly isCompacting: boolean;
  readonly bashRunning: boolean;
  readonly bashOutput: string;
  readonly settingsModalOpen: boolean;
  readonly settingsModalTab: SettingsModalTab;
  readonly canRedoRevert: boolean;
  readonly lastRevertedFiles: readonly string[];
}

const THEME_STORAGE_KEY = "verity.desktop.theme";

const FALLBACK_MODEL: ModelOption = {
  provider: "anthropic",
  modelId: "claude-sonnet-5-5",
  label: "Claude Sonnet 5.5",
};

const DEFAULT_THINKING_LEVELS: readonly ThinkingLevel[] = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
];

let clientIdCounter = 0;

function nextClientId(prefix: string): string {
  clientIdCounter += 1;
  return `${prefix}-${clientIdCounter}`;
}

function readStoredTheme(): ThemeMode {
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
      if (stored === "light" || stored === "dark") {
        return stored;
      }
    }
  } catch {
    // Ignore storage access failures in restricted environments.
  }
  return detectPreferredTheme();
}

function persistTheme(theme: ThemeMode): void {
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    }
  } catch {
    // Ignore storage access failures in restricted environments.
  }
}

function modelKey(model: Pick<ModelOption, "provider" | "modelId">): string {
  return `${model.provider}::${model.modelId}`;
}

function providerProfilesToModels(
  providers: readonly PublicProviderProfile[]
): ModelOption[] {
  const result: ModelOption[] = [];
  for (const provider of providers) {
    const providerId = provider.id.trim();
    if (!providerId) continue;
    for (const rawModelId of provider.models) {
      const modelId = rawModelId.trim();
      if (!modelId) continue;
      result.push({
        provider: providerId,
        modelId,
        label: modelId,
      });
    }
  }
  return result;
}

function collectModels(
  models: readonly ModelOption[],
  providers: readonly PublicProviderProfile[] = []
): ModelOption[] {
  const byKey = new Map<string, ModelOption>();

  for (const model of models) {
    const key = modelKey(model);
    const existing = byKey.get(key);
    if (!existing || (existing.label === existing.modelId && model.label !== model.modelId)) {
      byKey.set(key, model);
    }
  }

  for (const providerModel of providerProfilesToModels(providers)) {
    const key = modelKey(providerModel);
    if (!byKey.has(key)) {
      byKey.set(key, providerModel);
    }
  }

  return Array.from(byKey.values());
}

export function normalizeModels(
  models: readonly ModelOption[],
  selectedModel: ModelOption,
  providers: readonly PublicProviderProfile[] = []
): readonly ModelOption[] {
  const collected = collectModels(models, providers);
  const selectedKey = modelKey(selectedModel);
  if (!collected.some((m) => modelKey(m) === selectedKey)) {
    collected.push(selectedModel);
  }
  return collected.length > 0 ? collected : [selectedModel];
}

export function normalizeThinkingLevels(
  levels: readonly ThinkingLevel[],
  selectedLevel: ThinkingLevel
): readonly ThinkingLevel[] {
  const source = levels.length > 0 ? levels : DEFAULT_THINKING_LEVELS;
  const seen = new Set<ThinkingLevel>();
  const result: ThinkingLevel[] = [];
  for (const level of source) {
    if (!seen.has(level)) {
      seen.add(level);
      result.push(level);
    }
  }
  if (!seen.has(selectedLevel)) {
    result.push(selectedLevel);
  }
  return result;
}

function formatErrorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readNonEmptyString(obj: Record<string, unknown>, key: string): string | null {
  const value = obj[key];
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function parseThinkingLevelValue(raw: string | null): ThinkingLevel | undefined {
  if (
    raw === "off" ||
    raw === "minimal" ||
    raw === "low" ||
    raw === "medium" ||
    raw === "high" ||
    raw === "xhigh"
  ) {
    return raw;
  }
  return undefined;
}

export type ResolveCycledModelResult =
  | {
      readonly kind: "cycled";
      readonly model: ModelOption;
      readonly thinkingLevel?: ThinkingLevel;
    }
  | {
      readonly kind: "fallback_set_model";
      readonly model: ModelOption;
    }
  | {
      readonly kind: "single_model";
    };

export function resolveCycledModel(
  models: readonly ModelOption[],
  selectedModel: ModelOption,
  wirePayload: unknown
): ResolveCycledModelResult {
  let parsedProvider: string | null = null;
  let parsedModelId: string | null = null;
  let parsedLabel: string | null = null;
  let parsedThinkingLevel: ThinkingLevel | undefined;

  if (isPlainRecord(wirePayload)) {
    const nested = isPlainRecord(wirePayload["model"]) ? wirePayload["model"] : null;
    parsedProvider =
      (nested ? readNonEmptyString(nested, "provider") : null) ??
      readNonEmptyString(wirePayload, "provider");
    parsedModelId =
      (nested
        ? readNonEmptyString(nested, "modelId") ?? readNonEmptyString(nested, "id")
        : null) ??
      readNonEmptyString(wirePayload, "modelId") ??
      readNonEmptyString(wirePayload, "id");
    parsedLabel =
      (nested
        ? readNonEmptyString(nested, "label") ?? readNonEmptyString(nested, "name")
        : null) ??
      readNonEmptyString(wirePayload, "label") ??
      readNonEmptyString(wirePayload, "name");
    const rawThinking =
      readNonEmptyString(wirePayload, "thinkingLevel") ??
      readNonEmptyString(wirePayload, "thinking_level") ??
      (nested
        ? readNonEmptyString(nested, "thinkingLevel") ??
          readNonEmptyString(nested, "thinking_level")
        : null);
    parsedThinkingLevel = parseThinkingLevelValue(rawThinking);
  }

  if (parsedModelId && !parsedProvider) {
    const byId = models.find((m) => m.modelId === parsedModelId);
    parsedProvider = byId?.provider ?? selectedModel.provider;
  }

  if (
    !parsedProvider ||
    !parsedModelId ||
    (parsedProvider === selectedModel.provider && parsedModelId === selectedModel.modelId)
  ) {
    if (models.length >= 2) {
      const currentIdx = models.findIndex(
        (m) => m.provider === selectedModel.provider && m.modelId === selectedModel.modelId
      );
      const nextIdx = (currentIdx + 1) % models.length;
      return { kind: "fallback_set_model", model: models[nextIdx] };
    }
    return { kind: "single_model" };
  }

  const existing = models.find(
    (m) => m.provider === parsedProvider && m.modelId === parsedModelId
  );
  const matchedOrNewModel: ModelOption = existing ?? {
    provider: parsedProvider,
    modelId: parsedModelId,
    label: parsedLabel ?? parsedModelId,
  };

  return parsedThinkingLevel
    ? { kind: "cycled", model: matchedOrNewModel, thinkingLevel: parsedThinkingLevel }
    : { kind: "cycled", model: matchedOrNewModel };
}

export type ResolveCycledThinkingResult =
  | { readonly kind: "cycled"; readonly level: ThinkingLevel }
  | { readonly kind: "fallback_set_thinking"; readonly level: ThinkingLevel }
  | { readonly kind: "single_level" };

export function resolveCycledThinkingLevel(
  thinkingLevels: readonly ThinkingLevel[],
  currentLevel: ThinkingLevel,
  wirePayload: unknown
): ResolveCycledThinkingResult {
  let parsedLevel: ThinkingLevel | undefined;
  if (isPlainRecord(wirePayload)) {
    const raw =
      readNonEmptyString(wirePayload, "level") ??
      readNonEmptyString(wirePayload, "thinkingLevel") ??
      readNonEmptyString(wirePayload, "thinking_level");
    parsedLevel = parseThinkingLevelValue(raw);
  }

  if (!parsedLevel || parsedLevel === currentLevel) {
    if (thinkingLevels.length >= 2) {
      const currentIdx = thinkingLevels.indexOf(currentLevel);
      const nextIdx = (currentIdx + 1) % thinkingLevels.length;
      return { kind: "fallback_set_thinking", level: thinkingLevels[nextIdx] };
    }
    return { kind: "single_level" };
  }

  return { kind: "cycled", level: parsedLevel };
}

export function formatChatTranscriptMarkdown(messages: readonly PiChatMessage[]): string {
  const sections: string[] = [];
  for (const msg of messages) {
    const content = msg.content.trim();
    if (!content) continue;
    const heading =
      msg.role === "user"
        ? "## You"
        : msg.role === "assistant"
          ? "## Verity"
          : "## System";
    sections.push(`${heading}
${content}`);
  }
  return sections.join("\n\n");
}

export function createInitialPiState(): PiWorkspaceState {
  return {
    theme: readStoredTheme(),
    connectionState: "connecting",
    loading: true,
    error: null,
    projects: [],
    sessions: [],
    activeProjectId: "",
    activeSessionId: "",
    messages: [],
    isStreaming: false,
    ignoreStaleTurnEvents: false,
    liveTools: [],
    retryBanner: null,
    pendingQueueCount: 0,
    models: [FALLBACK_MODEL],
    selectedModel: FALLBACK_MODEL,
    thinkingLevels: DEFAULT_THINKING_LEVELS,
    thinkingLevel: "medium",
    commands: [],
    providers: [],
    draft: "",
    attachments: [],
    recall: { history: [], index: null, draftSnapshot: "" },
    modal: { kind: "none" },
    extensionDialogs: [],
    extensionSnackbars: [],
    extensionWidgets: [],
    extensionWidgetLines: {},
    extensionToasts: [],
    extensionStatus: {},
    editorPrefillText: null,
    autoCompactionEnabled: true,
    autoRetryEnabled: true,
    steeringMode: "one-at-a-time",
    followUpMode: "one-at-a-time",
    sessionStats: null,
    isCompacting: false,
    bashRunning: false,
    bashOutput: "",
    settingsModalOpen: false,
    settingsModalTab: "behavior",
    canRedoRevert: false,
    lastRevertedFiles: [],
  };
}

// ============================================================
// Pure ComposerRecall & Completion Helpers
// ============================================================

export function recallUp(
  recall: ComposerRecallState,
  currentDraft: string
): { recall: ComposerRecallState; nextDraft: string } {
  if (recall.history.length === 0) return { recall, nextDraft: currentDraft };
  if (recall.index === null) {
    const idx = recall.history.length - 1;
    return {
      recall: { ...recall, index: idx, draftSnapshot: currentDraft },
      nextDraft: recall.history[idx],
    };
  }
  const nextIdx = Math.max(0, recall.index - 1);
  return {
    recall: { ...recall, index: nextIdx },
    nextDraft: recall.history[nextIdx],
  };
}

export function recallDown(
  recall: ComposerRecallState,
  currentDraft: string
): { recall: ComposerRecallState; nextDraft: string } {
  if (recall.index === null) return { recall, nextDraft: currentDraft };
  if (recall.index >= recall.history.length - 1) {
    return {
      recall: { ...recall, index: null },
      nextDraft: recall.draftSnapshot,
    };
  }
  const nextIdx = recall.index + 1;
  return {
    recall: { ...recall, index: nextIdx },
    nextDraft: recall.history[nextIdx],
  };
}

// ============================================================
// Discriminated Reducer Actions
// ============================================================

export type PiAction =
  | { readonly type: "bootstrap_start" }
  | { readonly type: "bootstrap_loaded"; readonly data: PiBootstrapData }
  | { readonly type: "set_error"; readonly error: string | null }
  | { readonly type: "event_stream_failed"; readonly error: string }
  | { readonly type: "set_connection_state"; readonly connectionState: ConnectionState }
  | { readonly type: "toggle_theme" }
  | { readonly type: "select_project"; readonly projectId: string }
  | {
      readonly type: "session_switched" | "session_created";
      readonly session: PiSessionSummary;
      readonly messages: readonly PiChatMessage[];
    }
  | { readonly type: "session_renamed"; readonly session: PiSessionSummary }
  | { readonly type: "set_draft"; readonly draft: string }
  | { readonly type: "clear_composer" }
  | {
      readonly type: "restore_composer";
      readonly draft: string;
      readonly attachments: readonly PendingAttachment[];
    }
  | { readonly type: "recall_up" }
  | { readonly type: "recall_down" }
  | { readonly type: "add_attachment"; readonly attachment: PendingAttachment }
  | { readonly type: "remove_attachment"; readonly filename: string }
  | { readonly type: "prompt_submitted"; readonly message: PiChatMessage }
  | { readonly type: "prompt_failed" }
  | { readonly type: "turn_aborted" }
  | { readonly type: "model_selected"; readonly model: ModelOption }
  | { readonly type: "thinking_selected"; readonly level: ThinkingLevel }
  | { readonly type: "set_modal"; readonly modal: ActiveModal }
  | {
      readonly type: "providers_updated";
      readonly providers: readonly PublicProviderProfile[];
      readonly refreshedModels?: readonly ModelOption[];
      readonly deletedProviderId?: string;
    }
  | { readonly type: "pi_event"; readonly event: PiUiEvent }
  | { readonly type: "shift_extension_dialog"; readonly dialogId: string }
  | { readonly type: "push_snackbar"; readonly snackbar: ExtensionSnackbar }
  | { readonly type: "dismiss_snackbar"; readonly id: string }
  | { readonly type: "push_toast"; readonly toast: ExtensionToastItem }
  | { readonly type: "dismiss_toast"; readonly id: string }
  | { readonly type: "dismiss_widget"; readonly id: string }
  | { readonly type: "set_auto_compaction"; readonly enabled: boolean }
  | { readonly type: "set_auto_retry"; readonly enabled: boolean }
  | { readonly type: "set_steering_mode"; readonly mode: PiQueueMode }
  | { readonly type: "set_follow_up_mode"; readonly mode: PiQueueMode }
  | { readonly type: "set_session_stats"; readonly stats: PiSessionStats | null }
  | { readonly type: "bash_started"; readonly command: string }
  | {
      readonly type: "bash_finished";
      readonly command: string;
      readonly result: PiBashResult;
    }
  | { readonly type: "bash_aborted" }
  | { readonly type: "open_settings_modal"; readonly tab?: SettingsModalTab }
  | { readonly type: "close_settings_modal" }
  | { readonly type: "consume_editor_prefill" }
  | {
      readonly type: "turn_reverted";
      readonly session: PiSessionSummary;
      readonly messages: readonly PiChatMessage[];
      readonly restoredPrompt: string | null;
      readonly canRedo: boolean;
      readonly revertedFiles: readonly string[];
    }
  | { readonly type: "revert_redone" };

function nowTimeLabel(): string {
  return new Date().toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
  });
}

const TERMINAL_ASSISTANT_STOP_REASONS = new Set<string>([
  "stop",
  "end_turn",
  "length",
  "error",
  "aborted",
]);

function isNonAssistantMessageRole(role: string | null | undefined): boolean {
  return (
    role === "user" ||
    role === "system" ||
    role === "developer" ||
    role === "toolResult" ||
    role === "tool_result" ||
    role === "tool"
  );
}

function isTurnScopedEvent(event: PiUiEvent): boolean {
  switch (event.type) {
    case "agent_start":
    case "turn_start":
    case "turn_end":
    case "message_start":
    case "message_update":
    case "message_end":
    case "tool_execution_start":
    case "tool_execution_update":
    case "tool_execution_end":
    case "queue_update":
    case "auto_retry_start":
    case "auto_retry_end":
    case "agent_settled":
      return true;
    case "compaction_start":
    case "compaction_end":
    case "bash_execution_update":
    case "session_info_changed":
    case "thinking_level_changed":
    case "error":
    case "process_exited":
    case "extension_ui_request":
    case "extension_snackbar":
    case "extension_widget":
    case "extension_ui_effect":
      return false;
    default: {
      const _exhaustive: never = event;
      return _exhaustive;
    }
  }
}

function hasToolIdInMessages(messages: readonly PiChatMessage[], id: string): boolean {
  return messages.some((m) => Boolean(m.toolCalls?.some((t) => t.id === id)));
}

function collectToolIdsExcludingIndex(
  messages: readonly PiChatMessage[],
  excludeIdx: number
): Set<string> {
  const ids = new Set<string>();
  for (let i = 0; i < messages.length; i += 1) {
    if (i === excludeIdx) continue;
    const tools = messages[i].toolCalls;
    if (!tools) continue;
    for (const tool of tools) {
      ids.add(tool.id);
    }
  }
  return ids;
}

function filterUniqueIncomingTools(
  messages: readonly PiChatMessage[],
  targetIdx: number,
  incoming: readonly PiToolCallState[]
): readonly PiToolCallState[] {
  if (incoming.length === 0) return incoming;
  const takenElsewhere = collectToolIdsExcludingIndex(messages, targetIdx);
  if (takenElsewhere.size === 0) return incoming;
  return incoming.filter((t) => !takenElsewhere.has(t.id));
}

function updateToolInMessages(
  messages: readonly PiChatMessage[],
  id: string,
  updater: (tool: PiToolCallState) => PiToolCallState
): readonly PiChatMessage[] {
  return messages.map((m) => {
    if (!m.toolCalls || !m.toolCalls.some((t) => t.id === id)) return m;
    return {
      ...m,
      toolCalls: m.toolCalls.map((t) => (t.id === id ? updater(t) : t)),
    };
  });
}

function mergeToolCalls(
  existing: readonly PiToolCallState[] | undefined,
  incoming: readonly PiToolCallState[]
): readonly PiToolCallState[] | undefined {
  if (incoming.length === 0) return existing;
  if (!existing || existing.length === 0) {
    const byId = new Map<string, PiToolCallState>();
    for (const tool of incoming) {
      byId.set(tool.id, tool);
    }
    return Array.from(byId.values());
  }
  const byId = new Map<string, PiToolCallState>();
  for (const tool of existing) {
    byId.set(tool.id, tool);
  }
  for (const tool of incoming) {
    byId.set(tool.id, tool);
  }
  return Array.from(byId.values());
}

function settleRunningTools(tools: readonly PiToolCallState[]): readonly PiToolCallState[] {
  return tools.map((t) => (t.status === "running" ? { ...t, status: "error" } : t));
}

function findLatestAssistantIndexInCurrentTurn(messages: readonly PiChatMessage[]): number {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const msg = messages[i];
    if (msg.role === "assistant") {
      return i;
    }
    if (msg.role === "user") {
      return -1;
    }
  }
  return -1;
}

function finalizeMessageThinking(m: PiChatMessage, nowMs: number): PiChatMessage {
  const needsStopStreaming = m.streaming === true;
  const needsStopThinking = m.thinkingStreaming === true;
  const needsDuration =
    m.thinkingStartedAtMs !== undefined && m.thinkingDurationMs === undefined;
  if (!needsStopStreaming && !needsStopThinking && !needsDuration) {
    return m;
  }
  return {
    ...m,
    streaming: false,
    ...(m.thinkingStreaming !== undefined ? { thinkingStreaming: false } : {}),
    ...(needsDuration && m.thinkingStartedAtMs !== undefined
      ? { thinkingDurationMs: Math.max(0, nowMs - m.thinkingStartedAtMs) }
      : {}),
  };
}

function attachToolsAndClearStreaming(
  messages: readonly PiChatMessage[],
  toolsToAttach: readonly PiToolCallState[]
): readonly PiChatMessage[] {
  const nowMs = Date.now();
  const latestAssistantIdx =
    toolsToAttach.length > 0 ? findLatestAssistantIndexInCurrentTurn(messages) : -1;
  const uniqueTools =
    latestAssistantIdx >= 0
      ? filterUniqueIncomingTools(messages, latestAssistantIdx, toolsToAttach)
      : [];
  return messages.map((m, idx) => {
    const shouldAttach = idx === latestAssistantIdx && uniqueTools.length > 0;
    const finalized = finalizeMessageThinking(m, nowMs);
    if (finalized === m && !shouldAttach) return m;
    return {
      ...finalized,
      toolCalls: shouldAttach ? mergeToolCalls(finalized.toolCalls, uniqueTools) : finalized.toolCalls,
    };
  });
}

function attachErrorToActiveTurn(
  messages: readonly PiChatMessage[],
  liveTools: readonly PiToolCallState[],
  isStreaming: boolean,
  errorMessage: string
): readonly PiChatMessage[] {
  const nowMs = Date.now();
  const settledTools = settleRunningTools(liveTools);
  const latestAssistantIdx = findLatestAssistantIndexInCurrentTurn(messages);
  if (latestAssistantIdx >= 0) {
    const target = messages[latestAssistantIdx];
    if (target.streaming || isStreaming) {
      const uniqueTools = filterUniqueIncomingTools(messages, latestAssistantIdx, settledTools);
      return messages.map((m, idx) => {
        if (idx === latestAssistantIdx) {
          const finalized = finalizeMessageThinking(m, nowMs);
          return {
            ...finalized,
            streaming: false,
            stopReason: "error",
            errorMessage,
            toolCalls: mergeToolCalls(finalized.toolCalls, uniqueTools),
          };
        }
        return finalizeMessageThinking(m, nowMs);
      });
    }
  }
  if (!isStreaming) {
    return messages.map((m) => finalizeMessageThinking(m, nowMs));
  }
  const uniqueTools = filterUniqueIncomingTools(messages, -1, settledTools);
  const failedTurn: PiChatMessage = {
    id: nextClientId("stream"),
    role: "assistant",
    content: "",
    timestamp: nowTimeLabel(),
    streaming: false,
    stopReason: "error",
    errorMessage,
    toolCalls: uniqueTools.length > 0 ? uniqueTools : undefined,
  };
  return [
    ...messages.map((m) => finalizeMessageThinking(m, nowMs)),
    failedTurn,
  ];
}

function isNonEmptyAssistantMessage(m: PiChatMessage): boolean {
  if (m.role === "system") {
    return m.content.trim().length > 0 || Boolean(m.attachments && m.attachments.length > 0);
  }
  if (m.role !== "assistant") return true;
  const hasText = m.content.trim().length > 0;
  const hasThinking = Boolean(m.thinking && m.thinking.trim().length > 0);
  const hasTools = Boolean(m.toolCalls && m.toolCalls.length > 0);
  const hasError =
    m.stopReason === "error" || Boolean(m.errorMessage && m.errorMessage.trim().length > 0);
  return hasText || hasThinking || hasTools || hasError;
}

type AssistantTarget =
  | { readonly kind: "found"; readonly index: number }
  | { readonly kind: "user_message" }
  | { readonly kind: "none" };

function resolveAssistantTarget(
  messages: readonly PiChatMessage[],
  targetId: string | null
): AssistantTarget {
  if (targetId) {
    const exactIdx = messages.findIndex((m) => m.id === targetId);
    if (exactIdx >= 0) {
      return messages[exactIdx].role === "user"
        ? { kind: "user_message" }
        : { kind: "found", index: exactIdx };
    }
  }
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const msg = messages[i];
    if (msg.role === "assistant" && msg.streaming) {
      return { kind: "found", index: i };
    }
    if (msg.role === "user") {
      break;
    }
  }
  return { kind: "none" };
}

function applyPiEvent(state: PiWorkspaceState, event: PiUiEvent): PiWorkspaceState {
  if (state.ignoreStaleTurnEvents && isTurnScopedEvent(event)) {
    return state;
  }

  switch (event.type) {
    case "agent_start":
    case "turn_start":
      return state.isStreaming ? state : { ...state, isStreaming: true };

    case "message_start": {
      if (isNonAssistantMessageRole(event.role)) {
        return state.isStreaming ? state : { ...state, isStreaming: true };
      }
      const id = event.message_id ?? nextClientId("stream");
      const existingIdx = state.messages.findIndex((m) => m.id === id);
      if (existingIdx >= 0) {
        if (state.messages[existingIdx].role === "user") {
          return state.isStreaming ? state : { ...state, isStreaming: true };
        }
        const messages = state.messages.map((m, idx) =>
          idx === existingIdx ? { ...m, streaming: true } : m
        );
        return { ...state, isStreaming: true, messages };
      }
      const nextMessage: PiChatMessage = {
        id,
        role: "assistant",
        content: "",
        timestamp: nowTimeLabel(),
        streaming: true,
        toolCalls: [],
      };
      return {
        ...state,
        isStreaming: true,
        messages: [...state.messages, nextMessage],
      };
    }

    case "message_update": {
      const delta = event.delta ?? "";
      const thinkingDelta = event.thinking_delta ?? "";
      const textEndContent = event.content ?? "";
      const thinkingEndContent = event.thinking_content ?? "";
      const thinkingPhase = event.thinking_phase ?? null;
      if (!delta && !thinkingDelta && !textEndContent && !thinkingEndContent && !thinkingPhase) {
        return state.isStreaming ? state : { ...state, isStreaming: true };
      }
      const nowMs = Date.now();
      const target = resolveAssistantTarget(state.messages, event.message_id);
      if (target.kind === "user_message") {
        return state;
      }
      if (target.kind === "found") {
        const messages = state.messages.map((m, idx) => {
          if (idx !== target.index) return m;
          const nextContent = delta
            ? m.content + delta
            : m.content.length === 0 && textEndContent
              ? textEndContent
              : m.content;
          const nextThinking = thinkingDelta
            ? (m.thinking ?? "") + thinkingDelta
            : (!m.thinking || m.thinking.length === 0) && thinkingEndContent
              ? thinkingEndContent
              : m.thinking;

          let nextThinkingStreaming = m.thinkingStreaming;
          let nextThinkingStartedAtMs = m.thinkingStartedAtMs;
          let nextThinkingDurationMs = m.thinkingDurationMs;

          if (thinkingPhase === "start" || thinkingDelta.length > 0) {
            nextThinkingStreaming = true;
            nextThinkingStartedAtMs = m.thinkingStartedAtMs ?? nowMs;
          }
          if (
            thinkingPhase === "end" ||
            (delta.length > 0 && (nextThinkingStreaming === true || m.thinkingStreaming === true))
          ) {
            nextThinkingStreaming = false;
            if (nextThinkingStartedAtMs !== undefined) {
              nextThinkingDurationMs = Math.max(0, nowMs - nextThinkingStartedAtMs);
            }
          }

          return {
            ...m,
            content: nextContent,
            thinking: nextThinking,
            thinkingStreaming: nextThinkingStreaming,
            thinkingStartedAtMs: nextThinkingStartedAtMs,
            thinkingDurationMs: nextThinkingDurationMs,
            streaming: true,
          };
        });
        return { ...state, isStreaming: true, messages };
      }

      const startsThinking = thinkingPhase === "start" || thinkingDelta.length > 0;
      const endsThinking = thinkingPhase === "end" || (delta.length > 0 && startsThinking);
      const thinkingStartedAtMs = startsThinking ? nowMs : undefined;
      const thinkingStreaming = endsThinking ? false : startsThinking ? true : undefined;
      const thinkingDurationMs =
        endsThinking && thinkingStartedAtMs !== undefined
          ? Math.max(0, nowMs - thinkingStartedAtMs)
          : undefined;

      const created: PiChatMessage = {
        id: event.message_id ?? nextClientId("stream"),
        role: "assistant",
        content: delta || textEndContent,
        thinking: thinkingDelta || thinkingEndContent || undefined,
        thinkingStartedAtMs,
        thinkingDurationMs,
        thinkingStreaming,
        timestamp: nowTimeLabel(),
        streaming: true,
        toolCalls: [],
      };
      return {
        ...state,
        isStreaming: true,
        messages: [...state.messages, created],
      };
    }

    case "message_end": {
      if (isNonAssistantMessageRole(event.role)) {
        return state;
      }
      const nowMs = Date.now();
      const finalText = event.text ?? null;
      const finalThinking = event.thinking ?? null;
      const stopReason = event.stop_reason ?? null;
      const errorMessage = event.error_message ?? null;
      const isTerminalStop =
        stopReason !== null && TERMINAL_ASSISTANT_STOP_REASONS.has(stopReason);
      const toolsToAttach = isTerminalStop
        ? settleRunningTools(state.liveTools)
        : state.liveTools;
      const target = resolveAssistantTarget(state.messages, event.message_id);
      if (target.kind === "user_message") {
        return state;
      }
      if (target.kind === "found") {
        const uniqueTools = filterUniqueIncomingTools(
          state.messages,
          target.index,
          toolsToAttach
        );
        const messages = state.messages.map((m, idx) => {
          if (idx !== target.index) {
            return isTerminalStop ? finalizeMessageThinking(m, nowMs) : m;
          }
          const hydratedContent = m.content.length === 0 && finalText ? finalText : m.content;
          const hydratedThinking =
            (!m.thinking || m.thinking.length === 0) && finalThinking
              ? finalThinking
              : m.thinking;
          const finalized = finalizeMessageThinking(m, nowMs);
          return {
            ...finalized,
            content: hydratedContent,
            thinking: hydratedThinking,
            streaming: false,
            thinkingStreaming: false,
            stopReason: stopReason ?? m.stopReason,
            errorMessage: errorMessage ?? m.errorMessage,
            toolCalls: mergeToolCalls(m.toolCalls, uniqueTools),
          };
        });
        return {
          ...state,
          messages,
          liveTools: [],
          isStreaming: isTerminalStop ? false : state.isStreaming,
          pendingQueueCount: isTerminalStop ? 0 : state.pendingQueueCount,
          retryBanner: isTerminalStop ? null : state.retryBanner,
        };
      }
      const hasError =
        stopReason === "error" || Boolean(errorMessage && errorMessage.trim().length > 0);
      const uniqueTools = filterUniqueIncomingTools(state.messages, -1, toolsToAttach);
      if (!finalText && !finalThinking && !hasError && uniqueTools.length === 0) {
        if (!isTerminalStop) {
          return state;
        }
        return {
          ...state,
          isStreaming: false,
          pendingQueueCount: 0,
          retryBanner: null,
          liveTools: [],
          messages: state.messages.map((m) => finalizeMessageThinking(m, nowMs)),
        };
      }
      const completed: PiChatMessage = {
        id: event.message_id ?? nextClientId("stream"),
        role: "assistant",
        content: finalText ?? "",
        thinking: finalThinking ?? undefined,
        thinkingStreaming: false,
        timestamp: nowTimeLabel(),
        streaming: false,
        stopReason,
        errorMessage,
        toolCalls: uniqueTools.length > 0 ? uniqueTools : undefined,
      };
      const baseMessages = isTerminalStop
        ? state.messages.map((m) => finalizeMessageThinking(m, nowMs))
        : state.messages;
      return {
        ...state,
        messages: [...baseMessages, completed],
        liveTools: [],
        isStreaming: isTerminalStop ? false : state.isStreaming,
        pendingQueueCount: isTerminalStop ? 0 : state.pendingQueueCount,
        retryBanner: isTerminalStop ? null : state.retryBanner,
      };
    }

    case "tool_execution_start": {
      const id = event.tool_call_id ?? nextClientId("tool");
      const name = event.tool_name ?? "tool";
      const nextTool: PiToolCallState = { id, name, status: "running" };
      if (state.liveTools.some((t) => t.id === id)) {
        const liveTools = state.liveTools.map((t) => (t.id === id ? nextTool : t));
        return { ...state, isStreaming: true, liveTools };
      }
      if (hasToolIdInMessages(state.messages, id)) {
        const messages = updateToolInMessages(state.messages, id, () => nextTool);
        return { ...state, isStreaming: true, messages };
      }
      return { ...state, isStreaming: true, liveTools: [...state.liveTools, nextTool] };
    }

    case "tool_execution_update": {
      const partial = event.partial_output;
      if (partial === null) {
        return state;
      }
      const id = event.tool_call_id;
      const summaryDetail = partial.trim().split(/\r?\n/)[0]?.slice(0, 120) || undefined;
      if (id) {
        if (state.liveTools.some((t) => t.id === id)) {
          const liveTools = state.liveTools.map((t) =>
            t.id === id
              ? { ...t, partialOutput: partial, detail: summaryDetail ?? t.detail }
              : t
          );
          return { ...state, isStreaming: true, liveTools };
        }
        if (hasToolIdInMessages(state.messages, id)) {
          const messages = updateToolInMessages(state.messages, id, (t) => ({
            ...t,
            partialOutput: partial,
            detail: summaryDetail ?? t.detail,
          }));
          return { ...state, isStreaming: true, messages };
        }
        const created: PiToolCallState = {
          id,
          name: event.tool_name ?? "tool",
          status: "running",
          detail: summaryDetail,
          partialOutput: partial,
        };
        return { ...state, isStreaming: true, liveTools: [...state.liveTools, created] };
      }
      const matchIdx = state.liveTools.findIndex(
        (t) =>
          (event.tool_name ? t.name === event.tool_name : true) && t.status === "running"
      );
      if (matchIdx < 0) {
        return state;
      }
      const liveTools = state.liveTools.map((t, idx) =>
        idx === matchIdx
          ? { ...t, partialOutput: partial, detail: summaryDetail ?? t.detail }
          : t
      );
      return { ...state, isStreaming: true, liveTools };
    }

    case "tool_execution_end": {
      const id = event.tool_call_id;
      const nextStatus: PiToolCallState["status"] = event.is_error ? "error" : "done";
      const endResult = event.result ?? null;
      const endDetail =
        endResult !== null
          ? endResult.trim().split(/\r?\n/)[0]?.slice(0, 120) || undefined
          : undefined;
      const finalizeTool = (t: PiToolCallState): PiToolCallState => ({
        ...t,
        status: nextStatus,
        partialOutput:
          (!t.partialOutput || t.partialOutput.trim().length === 0) && endResult !== null
            ? endResult
            : t.partialOutput,
        detail:
          (!t.detail || t.detail.trim().length === 0) && endDetail !== undefined
            ? endDetail
            : t.detail,
      });
      if (id) {
        if (state.liveTools.some((t) => t.id === id)) {
          const liveTools = state.liveTools.map((t) => (t.id === id ? finalizeTool(t) : t));
          return { ...state, liveTools };
        }
        if (hasToolIdInMessages(state.messages, id)) {
          const messages = updateToolInMessages(state.messages, id, finalizeTool);
          return { ...state, messages };
        }
        return state;
      }
      const matchIdx = state.liveTools.findIndex(
        (t) => t.name === event.tool_name && t.status === "running"
      );
      if (matchIdx < 0) {
        return state;
      }
      const liveTools = state.liveTools.map((t, idx) =>
        idx === matchIdx ? finalizeTool(t) : t
      );
      return { ...state, liveTools };
    }

    case "queue_update":
      return {
        ...state,
        pendingQueueCount: state.isStreaming ? Math.max(0, event.pending_count ?? 0) : 0,
      };

    case "auto_retry_start":
      return {
        ...state,
        isStreaming: true,
        retryBanner: {
          attempt: event.attempt ?? 1,
          maxAttempts: event.max_attempts ?? 3,
          delayMs: event.delay_ms ?? 2000,
          errorMessage: event.error_message ?? null,
        },
      };

    case "auto_retry_end": {
      if (event.success === false) {
        const finalError = event.final_error?.trim() || "Auto-retry failed.";
        const snackbar: ExtensionSnackbar = {
          id: nextClientId("sb"),
          message: finalError,
          tone: "error",
        };
        const messages = attachErrorToActiveTurn(
          state.messages,
          state.liveTools,
          state.isStreaming,
          finalError
        );
        return {
          ...state,
          isStreaming: false,
          retryBanner: null,
          pendingQueueCount: 0,
          liveTools: [],
          messages,
          extensionSnackbars: [...state.extensionSnackbars.slice(-3), snackbar],
        };
      }
      return { ...state, retryBanner: null };
    }

    case "error": {
      const errMessage = event.message.trim() || "Unknown runtime error.";
      const snackbar: ExtensionSnackbar = {
        id: nextClientId("sb"),
        message: errMessage,
        tone: "error",
      };
      const messages = attachErrorToActiveTurn(
        state.messages,
        state.liveTools,
        state.isStreaming,
        errMessage
      );
      return {
        ...state,
        isStreaming: false,
        retryBanner: null,
        pendingQueueCount: 0,
        liveTools: [],
        messages,
        extensionSnackbars: [...state.extensionSnackbars.slice(-3), snackbar],
      };
    }

    case "process_exited": {
      const tail = event.stderr_tail?.trim();
      const exitLabel = event.exit_code !== null ? `code ${event.exit_code}` : "unknown code";
      const errMessage = tail
        ? `Pi process exited (${exitLabel}): ${tail}`
        : `Pi process exited unexpectedly (${exitLabel}).`;
      const snackbar: ExtensionSnackbar = {
        id: nextClientId("sb"),
        message: errMessage,
        tone: "error",
      };
      const messages = attachErrorToActiveTurn(
        state.messages,
        state.liveTools,
        state.isStreaming,
        errMessage
      );
      return {
        ...state,
        connectionState: "disconnected",
        isStreaming: false,
        retryBanner: null,
        pendingQueueCount: 0,
        liveTools: [],
        messages,
        extensionDialogs: [],
        extensionSnackbars: [...state.extensionSnackbars.slice(-3), snackbar],
      };
    }

    case "turn_end":
    case "agent_settled": {
      const settledTools = settleRunningTools(state.liveTools);
      const finalized = attachToolsAndClearStreaming(state.messages, settledTools).filter(
        isNonEmptyAssistantMessage
      );
      return {
        ...state,
        isStreaming: false,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
        messages: finalized,
      };
    }

    case "extension_ui_request": {
      if (state.extensionDialogs.some((d) => d.id === event.id)) {
        return state;
      }
      const nextDialog: QueuedExtensionDialog = {
        id: event.id,
        kind: event.kind,
        title: event.title ?? "Extension prompt",
        options: event.options,
        timeoutMs: event.timeout_ms,
      };
      return {
        ...state,
        extensionDialogs: [...state.extensionDialogs, nextDialog],
      };
    }

    case "extension_snackbar":
      return {
        ...state,
        extensionSnackbars: [
          ...state.extensionSnackbars.slice(-3),
          { id: event.id, message: event.message, tone: event.tone },
        ],
      };

    case "extension_widget": {
      if (event.dismissed) {
        const nextWidgetLines = { ...state.extensionWidgetLines };
        delete nextWidgetLines[event.id];
        return {
          ...state,
          extensionWidgets: state.extensionWidgets.filter((w) => w.id !== event.id),
          extensionWidgetLines: nextWidgetLines,
        };
      }
      const lines = event.body.split(/\r?\n/);
      const nextWidget: ExtensionWidget = {
        id: event.id,
        title: event.title,
        body: event.body,
        lines,
      };
      const exists = state.extensionWidgets.some((w) => w.id === event.id);
      return {
        ...state,
        extensionWidgets: exists
          ? state.extensionWidgets.map((w) => (w.id === event.id ? nextWidget : w))
          : [...state.extensionWidgets, nextWidget],
        extensionWidgetLines: {
          ...state.extensionWidgetLines,
          [event.id]: lines,
        },
      };
    }

    case "compaction_start":
      return {
        ...state,
        isCompacting: true,
      };

    case "compaction_end": {
      if (event.error_message && event.error_message.trim().length > 0) {
        const errText = event.error_message.trim();
        const toast: ExtensionToastItem = {
          id: nextClientId("toast"),
          message: errText,
          notifyType: "error",
          timestamp: nowTimeLabel(),
        };
        const snackbar: ExtensionSnackbar = {
          id: toast.id,
          message: errText,
          tone: "error",
        };
        return {
          ...state,
          isCompacting: false,
          extensionToasts: [...state.extensionToasts.slice(-5), toast],
          extensionSnackbars: [...state.extensionSnackbars.slice(-3), snackbar],
        };
      }
      return {
        ...state,
        isCompacting: false,
      };
    }

    case "bash_execution_update": {
      const nextOutput = event.chunk ? state.bashOutput + event.chunk : state.bashOutput;
      return {
        ...state,
        bashRunning: !event.done,
        bashOutput: nextOutput,
      };
    }

    case "session_info_changed": {
      const nextName = event.name?.trim();
      if (!nextName) return state;
      const targetId = event.session_id ?? state.activeSessionId;
      if (!targetId) return state;
      return {
        ...state,
        sessions: state.sessions.map((s) =>
          s.id === targetId ? { ...s, name: nextName } : s
        ),
      };
    }

    case "thinking_level_changed":
      return {
        ...state,
        thinkingLevel: event.level,
        thinkingLevels: normalizeThinkingLevels(state.thinkingLevels, event.level),
      };

    case "extension_ui_effect": {
      const method = event.method;
      if (method === "notify") {
        const message = (event.message ?? event.text ?? "").trim();
        if (!message) return state;
        const rawType = (event.notifyType ?? event.notify_type ?? "info").toLowerCase();
        const notifyType: ExtensionToastItem["notifyType"] =
          rawType === "error" ? "error" : rawType === "warning" || rawType === "warn" ? "warning" : "info";
        const toast: ExtensionToastItem = {
          id: nextClientId("toast"),
          message,
          notifyType,
          timestamp: nowTimeLabel(),
        };
        const snackbar: ExtensionSnackbar = {
          id: toast.id,
          message,
          tone: notifyType === "error" ? "error" : "info",
        };
        return {
          ...state,
          extensionToasts: [...state.extensionToasts.slice(-5), toast],
          extensionSnackbars: [...state.extensionSnackbars.slice(-3), snackbar],
        };
      }

      if (method === "setStatus" || method === "set_status") {
        const key = (event.statusKey ?? event.status_key ?? "").trim();
        if (!key) return state;
        const text = event.statusText ?? event.status_text;
        const nextStatus = { ...state.extensionStatus };
        if (text === null || text === undefined || text.trim().length === 0) {
          delete nextStatus[key];
        } else {
          nextStatus[key] = text;
        }
        return {
          ...state,
          extensionStatus: nextStatus,
        };
      }

      if (method === "setWidget" || method === "set_widget") {
        const key = (event.widgetKey ?? event.widget_key ?? "").trim();
        if (!key) return state;
        const lines = event.widgetLines ?? event.widget_lines;
        const nextWidgetLines = { ...state.extensionWidgetLines };
        if (!lines || lines.length === 0) {
          delete nextWidgetLines[key];
          return {
            ...state,
            extensionWidgets: state.extensionWidgets.filter((w) => w.id !== key),
            extensionWidgetLines: nextWidgetLines,
          };
        }
        const copiedLines = [...lines];
        nextWidgetLines[key] = copiedLines;
        const nextWidget: ExtensionWidget = {
          id: key,
          title: event.title?.trim() || key,
          body: copiedLines.join("\n"),
          lines: copiedLines,
        };
        const exists = state.extensionWidgets.some((w) => w.id === key);
        return {
          ...state,
          extensionWidgets: exists
            ? state.extensionWidgets.map((w) => (w.id === key ? nextWidget : w))
            : [...state.extensionWidgets, nextWidget],
          extensionWidgetLines: nextWidgetLines,
        };
      }

      if (method === "set_editor_text" || method === "setEditorText") {
        const text = event.text ?? event.message ?? "";
        return {
          ...state,
          editorPrefillText: text,
        };
      }

      return state;
    }

    default: {
      const _exhaustive: never = event;
      return _exhaustive;
    }
  }
}

export function piReducer(state: PiWorkspaceState, action: PiAction): PiWorkspaceState {
  switch (action.type) {
    case "bootstrap_start":
      return {
        ...state,
        loading: true,
        error: null,
        connectionState: "connecting",
      };

    case "bootstrap_loaded": {
      const listenerAlreadyFailed =
        state.connectionState === "disconnected" && state.error !== null;
      return {
        ...state,
        loading: false,
        error: listenerAlreadyFailed ? state.error : null,
        connectionState: listenerAlreadyFailed ? "disconnected" : "connected",
        isStreaming: false,
        ignoreStaleTurnEvents: false,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
        projects: action.data.projects,
        sessions: action.data.sessions,
        activeProjectId: action.data.activeProjectId,
        activeSessionId: action.data.activeSessionId,
        messages: action.data.messages,
        models: normalizeModels(
          action.data.models,
          action.data.selectedModel,
          action.data.providers
        ),
        selectedModel: action.data.selectedModel,
        thinkingLevels: normalizeThinkingLevels(
          action.data.thinkingLevels,
          action.data.thinkingLevel
        ),
        thinkingLevel: action.data.thinkingLevel,
        commands: action.data.commands,
        providers: action.data.providers,
        canRedoRevert: false,
        lastRevertedFiles: [],
      };
    }

    case "set_error": {
      if (!action.error) {
        return {
          ...state,
          error: null,
          loading: false,
        };
      }
      const settledTools = settleRunningTools(state.liveTools);
      const messages = attachToolsAndClearStreaming(state.messages, settledTools);
      return {
        ...state,
        error: action.error,
        loading: false,
        connectionState: "disconnected",
        isStreaming: false,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
        messages,
      };
    }

    case "event_stream_failed": {
      const detail = action.error.trim() || "Unknown event stream error";
      const message = detail.startsWith("Event stream unavailable:")
        ? detail
        : `Event stream unavailable: ${detail}`;
      const snackbar: ExtensionSnackbar = {
        id: nextClientId("sb"),
        message,
        tone: "error",
      };
      const settledTools = settleRunningTools(state.liveTools);
      const messages = attachToolsAndClearStreaming(state.messages, settledTools);
      return {
        ...state,
        error: message,
        loading: false,
        connectionState: "disconnected",
        isStreaming: false,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
        messages,
        extensionSnackbars: [...state.extensionSnackbars.slice(-3), snackbar],
      };
    }

    case "set_connection_state":
      return { ...state, connectionState: action.connectionState };

    case "toggle_theme":
      return { ...state, theme: state.theme === "dark" ? "light" : "dark" };

    case "select_project": {
      const activeBelongsToProject = state.sessions.some(
        (s) => s.id === state.activeSessionId && s.projectId === action.projectId
      );
      if (activeBelongsToProject) {
        return { ...state, activeProjectId: action.projectId };
      }
      return {
        ...state,
        activeProjectId: action.projectId,
        activeSessionId: "",
        messages: [],
        isStreaming: false,
        ignoreStaleTurnEvents: true,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
      };
    }

    case "session_switched":
    case "session_created": {
      const session = action.session;
      const updateExisting = action.type === "session_switched"
        && state.sessions.some((s) => s.id === session.id);
      const sessions = updateExisting
        ? state.sessions.map((s) => s.id === session.id ? session : s)
        : [session, ...state.sessions.filter((s) => s.id !== session.id)];
      return {
        ...state,
        sessions,
        activeProjectId: session.projectId,
        activeSessionId: session.id,
        messages: action.messages,
        isStreaming: false,
        ignoreStaleTurnEvents: true,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
        modal: action.type === "session_created" ? { kind: "none" } : state.modal,
        canRedoRevert: false,
        lastRevertedFiles: [],
      };
    }

    case "session_renamed":
      return {
        ...state,
        sessions: state.sessions.map((s) =>
          s.id === action.session.id ? action.session : s
        ),
      };

    case "set_draft":
      return {
        ...state,
        draft: action.draft,
        recall: { ...state.recall, index: null },
      };

    case "clear_composer":
      return {
        ...state,
        draft: "",
        attachments: [],
        recall: { ...state.recall, index: null },
      };

    case "restore_composer":
      return {
        ...state,
        draft: action.draft,
        attachments: action.attachments,
        recall: { ...state.recall, index: null },
      };

    case "recall_up": {
      const res = recallUp(state.recall, state.draft);
      return { ...state, draft: res.nextDraft, recall: res.recall };
    }

    case "recall_down": {
      const res = recallDown(state.recall, state.draft);
      return { ...state, draft: res.nextDraft, recall: res.recall };
    }

    case "add_attachment":
      if (state.attachments.some((a) => a.filename === action.attachment.filename)) {
        return state;
      }
      return {
        ...state,
        attachments: [...state.attachments, action.attachment],
      };

    case "remove_attachment":
      return {
        ...state,
        attachments: state.attachments.filter((a) => a.filename !== action.filename),
      };

    case "prompt_submitted": {
      const trimmed = action.message.content.trim();
      const nextHistory =
        trimmed && state.recall.history[state.recall.history.length - 1] !== trimmed
          ? [...state.recall.history.slice(-49), trimmed]
          : state.recall.history;
      return {
        ...state,
        messages: [...state.messages, action.message],
        isStreaming: true,
        ignoreStaleTurnEvents: false,
        draft: "",
        attachments: [],
        recall: { history: nextHistory, index: null, draftSnapshot: "" },
        canRedoRevert: false,
        lastRevertedFiles: [],
      };
    }

    case "prompt_failed": {
      const nowMs = Date.now();
      return {
        ...state,
        isStreaming: false,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
        messages: state.messages.map((m) => finalizeMessageThinking(m, nowMs)),
      };
    }

    case "turn_aborted": {
      const settledTools = settleRunningTools(state.liveTools);
      const messages = attachToolsAndClearStreaming(state.messages, settledTools);
      return {
        ...state,
        isStreaming: false,
        retryBanner: null,
        pendingQueueCount: 0,
        liveTools: [],
        messages,
      };
    }

    case "model_selected":
      return {
        ...state,
        selectedModel: action.model,
        models: normalizeModels(state.models, action.model),
      };

    case "thinking_selected":
      return {
        ...state,
        thinkingLevel: action.level,
        thinkingLevels: normalizeThinkingLevels(state.thinkingLevels, action.level),
      };

    case "set_modal":
      return { ...state, modal: action.modal };

    case "providers_updated": {
      const baseModels = action.refreshedModels ?? state.models;
      const deletedId = action.deletedProviderId;
      const providerWasDeleted = Boolean(
        deletedId && !action.providers.some((p) => p.id === deletedId)
      );
      const filteredBase = providerWasDeleted
        ? baseModels.filter((m) => m.provider !== deletedId)
        : baseModels;
      const remainingModels = collectModels(filteredBase, action.providers);
      const nextSelectedModel =
        providerWasDeleted && state.selectedModel.provider === deletedId
          ? (remainingModels[0] ?? FALLBACK_MODEL)
          : state.selectedModel;
      return {
        ...state,
        providers: action.providers,
        selectedModel: nextSelectedModel,
        models: normalizeModels(filteredBase, nextSelectedModel, action.providers),
      };
    }

    case "pi_event":
      return applyPiEvent(state, action.event);

    case "shift_extension_dialog":
      return {
        ...state,
        extensionDialogs: state.extensionDialogs.filter((d) => d.id !== action.dialogId),
      };

    case "push_snackbar":
      return {
        ...state,
        extensionSnackbars: [
          ...state.extensionSnackbars.slice(-3),
          action.snackbar,
        ],
      };

    case "dismiss_snackbar":
      return {
        ...state,
        extensionSnackbars: state.extensionSnackbars.filter((s) => s.id !== action.id),
      };

    case "push_toast":
      return {
        ...state,
        extensionToasts: [...state.extensionToasts.slice(-5), action.toast],
      };

    case "dismiss_toast":
      return {
        ...state,
        extensionToasts: state.extensionToasts.filter((t) => t.id !== action.id),
        extensionSnackbars: state.extensionSnackbars.filter((s) => s.id !== action.id),
      };

    case "dismiss_widget": {
      const nextWidgetLines = { ...state.extensionWidgetLines };
      delete nextWidgetLines[action.id];
      return {
        ...state,
        extensionWidgets: state.extensionWidgets.filter((w) => w.id !== action.id),
        extensionWidgetLines: nextWidgetLines,
      };
    }

    case "set_auto_compaction":
      return { ...state, autoCompactionEnabled: action.enabled };

    case "set_auto_retry":
      return { ...state, autoRetryEnabled: action.enabled };

    case "set_steering_mode":
      return { ...state, steeringMode: action.mode };

    case "set_follow_up_mode":
      return { ...state, followUpMode: action.mode };

    case "set_session_stats":
      return { ...state, sessionStats: action.stats };

    case "bash_started":
      return {
        ...state,
        bashRunning: true,
        bashOutput: `$ ${action.command}\n`,
      };

    case "bash_finished": {
      const exitLabel =
        action.result.exitCode !== null ? `exit ${action.result.exitCode}` : "finished";
      const statusSuffix = action.result.cancelled ? " (cancelled)" : "";
      const bodyText = action.result.output.trim().length > 0
        ? action.result.output
        : "(no output)";
      const bashMessage: PiChatMessage = {
        id: nextClientId("bash"),
        role: "system",
        content: `\`$ ${action.command}\` — ${exitLabel}${statusSuffix}\n\n\`\`\`\n${bodyText}\n\`\`\``,
        timestamp: nowTimeLabel(),
      };
      return {
        ...state,
        bashRunning: false,
        bashOutput: action.result.output,
        messages: [...state.messages, bashMessage],
      };
    }

    case "bash_aborted":
      return {
        ...state,
        bashRunning: false,
      };

    case "open_settings_modal":
      return {
        ...state,
        settingsModalOpen: true,
        settingsModalTab: action.tab ?? state.settingsModalTab,
      };

    case "close_settings_modal":
      return {
        ...state,
        settingsModalOpen: false,
      };

    case "consume_editor_prefill":
      return {
        ...state,
        editorPrefillText: null,
      };

    case "turn_reverted": {
      const hasSession = state.sessions.some((s) => s.id === action.session.id);
      const nextSessions = hasSession
        ? state.sessions.map((s) => (s.id === action.session.id ? action.session : s))
        : [action.session, ...state.sessions];
      return {
        ...state,
        sessions: nextSessions,
        activeProjectId: action.session.projectId || state.activeProjectId,
        activeSessionId: action.session.id,
        messages: action.messages,
        isStreaming: false,
        ignoreStaleTurnEvents: true,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
        canRedoRevert: action.canRedo,
        lastRevertedFiles: action.revertedFiles,
        editorPrefillText:
          action.restoredPrompt !== null ? action.restoredPrompt : state.editorPrefillText,
      };
    }

    case "revert_redone":
      return {
        ...state,
        canRedoRevert: false,
        lastRevertedFiles: [],
      };

    default: {
      const _exhaustive: never = action;
      return _exhaustive;
    }
  }
}

// ============================================================
// Unified Workspace Hook
// ============================================================

export function usePiWorkspace() {
  const [state, dispatch] = useReducer(piReducer, undefined, createInitialPiState);
  const [bootstrapNonce, setBootstrapNonce] = useState(0);

  const stateRef = useRef(state);
  stateRef.current = state;

  const modelRequestIdRef = useRef(0);
  const thinkingRequestIdRef = useRef(0);
  const providerRequestIdRef = useRef(0);
  const submittingRef = useRef(false);
  const changingWorkspaceRef = useRef(false);

  const lastConfirmedModelRef = useRef<ModelOption>(state.selectedModel);
  const lastConfirmedThinkingRef = useRef<ThinkingLevel>(state.thinkingLevel);

  // Sync dynamic MD3 tokens and persist theme preference whenever theme changes
  useEffect(() => {
    applyMd3Theme(state.theme);
    persistTheme(state.theme);
  }, [state.theme]);

  // Bootstrap workspace data + subscribe to Pi JSONL stdout events
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;

    dispatch({ type: "bootstrap_start" });

    void (async () => {
      try {
        const fn = await tauriApi.listenPiEvents((event) => {
          if (active) dispatch({ type: "pi_event", event });
        });
        if (active) {
          unlisten = fn;
        } else {
          fn();
          return;
        }
      } catch (err: unknown) {
        if (!active) return;
        dispatch({
          type: "event_stream_failed",
          error: formatErrorMessage(err),
        });
      }

      try {
        const data = await tauriApi.bootstrapPiWorkspace();
        if (!active) return;
        lastConfirmedModelRef.current = data.selectedModel;
        lastConfirmedThinkingRef.current = data.thinkingLevel;
        dispatch({ type: "bootstrap_loaded", data });
      } catch (err: unknown) {
        if (active) {
          dispatch({
            type: "set_error",
            error: formatErrorMessage(err),
          });
        }
      }
    })();

    return () => {
      active = false;
      unlisten?.();
    };
  }, [bootstrapNonce]);

  const notify = useCallback((message: string, tone: "info" | "error" = "info") => {
    dispatch({
      type: "push_snackbar",
      snackbar: { id: nextClientId("sb"), message, tone },
    });
  }, []);

  const retryBootstrap = useCallback(() => {
    setBootstrapNonce((n) => n + 1);
  }, []);

  const hydrateBootstrap = useCallback((data: PiBootstrapData) => {
    lastConfirmedModelRef.current = data.selectedModel;
    lastConfirmedThinkingRef.current = data.thinkingLevel;
    dispatch({ type: "bootstrap_loaded", data });
  }, []);

  const changeWorkspace = useCallback(async (operation: () => Promise<void>) => {
    if (changingWorkspaceRef.current || submittingRef.current || stateRef.current.isStreaming) {
      notify("Wait for the current chat operation to finish.", "error");
      return;
    }
    changingWorkspaceRef.current = true;
    try {
      await operation();
    } finally {
      changingWorkspaceRef.current = false;
    }
  }, [notify]);

  const selectProject = useCallback(
    async (pathOrId: string) => changeWorkspace(async () => {
      const matched = stateRef.current.projects.find(
        (p) => p.path === pathOrId || p.id === pathOrId
      );
      const targetPath = matched?.path ?? pathOrId;
      try {
        const data = await tauriApi.piSwitchProject(targetPath);
        hydrateBootstrap(data);
      } catch (err: unknown) {
        notify(`Failed to switch project: ${formatErrorMessage(err)}`, "error");
      }
    }),
    [changeWorkspace, hydrateBootstrap, notify]
  );

  const addProject = useCallback(
    async (path: string) => changeWorkspace(async () => {
      const trimmed = path.trim();
      if (!trimmed) return;
      try {
        const data = await tauriApi.piAddProject(trimmed);
        hydrateBootstrap(data);
        notify(`Added project "${trimmed}"`);
      } catch (err: unknown) {
        notify(`Failed to add project: ${formatErrorMessage(err)}`, "error");
      }
    }),
    [changeWorkspace, hydrateBootstrap, notify]
  );

  const pickAndAddProject = useCallback(async () => {
    try {
      const picked = await tauriApi.piPickProjectFolder();
      if (picked) {
        await addProject(picked);
      }
    } catch (err: unknown) {
      notify(`Failed to pick project folder: ${formatErrorMessage(err)}`, "error");
    }
  }, [addProject, notify]);

  const removeProject = useCallback(
    async (path: string) => changeWorkspace(async () => {
      const trimmed = path.trim();
      if (!trimmed) return;
      try {
        const data = await tauriApi.piRemoveProject(trimmed);
        hydrateBootstrap(data);
        notify(`Removed project "${trimmed}"`);
      } catch (err: unknown) {
        notify(`Failed to remove project: ${formatErrorMessage(err)}`, "error");
      }
    }),
    [changeWorkspace, hydrateBootstrap, notify]
  );

  const activateSession = useCallback(async (
    type: "session_switched" | "session_created",
    result: SessionWithMessages
  ) => {
    if (result.session.projectId !== stateRef.current.activeProjectId) {
      const data = await tauriApi.bootstrapPiWorkspace().catch(() => null);
      if (
        data?.activeProjectId === result.session.projectId &&
        data.activeSessionId === result.session.id
      ) {
        hydrateBootstrap(data);
      }
    }
    dispatch({ type, ...result });
  }, [hydrateBootstrap]);

  const switchSession = useCallback(
    async (session: PiSessionSummary) => changeWorkspace(async () => {
      try {
        const result = await tauriApi.switchSession(session);
        if (result) await activateSession("session_switched", result);
      } catch (err: unknown) {
        notify(`Failed to switch session: ${formatErrorMessage(err)}`, "error");
      }
    }),
    [activateSession, changeWorkspace, notify]
  );

  const pushToast = useCallback(
    (message: string, notifyType: ExtensionToastItem["notifyType"] = "info") => {
      const id = nextClientId("toast");
      dispatch({
        type: "push_toast",
        toast: { id, message, notifyType, timestamp: nowTimeLabel() },
      });
      dispatch({
        type: "push_snackbar",
        snackbar: { id, message, tone: notifyType === "error" ? "error" : "info" },
      });
    },
    []
  );

  const createSession = useCallback(
    async (projectPathOrId?: string) => changeWorkspace(async () => {
      const current = stateRef.current;
      try {
        const project = current.projects.find(
          (p) => p.path === projectPathOrId || p.id === (projectPathOrId ?? current.activeProjectId)
        );
        if (!project) throw new Error("Select a project first.");
        const result = await tauriApi.newSession(project.id);
        if (result) await activateSession("session_created", result);
      } catch (err: unknown) {
        notify(`Failed to create session: ${formatErrorMessage(err)}`, "error");
      }
    }),
    [activateSession, changeWorkspace, notify]
  );

  const renameSession = useCallback(
    async (sessionId: string, name: string) => changeWorkspace(async () => {
      try {
        const session = stateRef.current.sessions.find((entry) => entry.id === sessionId);
        if (!session) throw new Error("The selected chat is unavailable.");
        const updated = await tauriApi.renameSession(session, name);
        if (updated) {
          dispatch({ type: "session_renamed", session: updated });
        }
      } catch (err: unknown) {
        notify(`Failed to rename session: ${formatErrorMessage(err)}`, "error");
      }
    }),
    [changeWorkspace, notify]
  );

  const archiveSession = useCallback(
    async (sessionId: string, archived: boolean) => changeWorkspace(async () => {
      try {
        const data = await tauriApi.piArchiveSession(sessionId, archived);
        hydrateBootstrap(data);
        pushToast(archived ? "Chat archived." : "Chat restored.");
      } catch (err: unknown) {
        notify(
          `Failed to ${archived ? "archive" : "restore"} chat: ${formatErrorMessage(err)}`,
          "error"
        );
      }
    }),
    [changeWorkspace, hydrateBootstrap, notify, pushToast]
  );

  const deleteSession = useCallback(
    async (sessionId: string) => changeWorkspace(async () => {
      try {
        const data = await tauriApi.piDeleteSession(sessionId);
        hydrateBootstrap(data);
        pushToast("Chat deleted.");
      } catch (err: unknown) {
        notify(`Failed to delete chat: ${formatErrorMessage(err)}`, "error");
      }
    }),
    [changeWorkspace, hydrateBootstrap, notify, pushToast]
  );

  const openHistoryTree = useCallback(async () => {
    try {
      const nodes = await tauriApi.getHistoryTree();
      dispatch({ type: "set_modal", modal: { kind: "history_tree", nodes } });
    } catch (err: unknown) {
      notify(`Failed to load history tree: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify]);

  const openForkModal = useCallback(async () => {
    try {
      const candidates = await tauriApi.getForkCandidates();
      dispatch({ type: "set_modal", modal: { kind: "fork", candidates } });
    } catch (err: unknown) {
      notify(`Failed to load fork points: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify]);

  const openProvidersModal = useCallback(() => {
    dispatch({ type: "set_modal", modal: { kind: "providers" } });
  }, []);

  const closeModal = useCallback(() => {
    dispatch({ type: "set_modal", modal: { kind: "none" } });
  }, []);

  const forkAtEntry = useCallback(
    async (entryId: string) => changeWorkspace(async () => {
      try {
        const result = await tauriApi.forkSession(entryId);
        dispatch({
          type: "session_created",
          session: result.session,
          messages: result.messages,
        });
        notify(`Forked session "${result.session.name}"`);
      } catch (err: unknown) {
        notify(`Failed to fork session: ${formatErrorMessage(err)}`, "error");
      }
    }),
    [changeWorkspace, notify]
  );

  const cloneCurrentSession = useCallback(async () => changeWorkspace(async () => {
    try {
      const result = await tauriApi.cloneSession();
      dispatch({
        type: "session_created",
        session: result.session,
        messages: result.messages,
      });
      notify(`Cloned session "${result.session.name}"`);
    } catch (err: unknown) {
      notify(`Failed to clone session: ${formatErrorMessage(err)}`, "error");
    }
  }), [changeWorkspace, notify]);

  const exportSessionHtml = useCallback(async () => {
    try {
      const exported = await tauriApi.exportSessionHtml();
      if (!tauriApi.isDesktop() && typeof document !== "undefined") {
        const blob = new Blob([exported.html], { type: "text/html;charset=utf-8" });
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = `${stateRef.current.activeSessionId || "pi-session"}.html`;
        link.click();
        URL.revokeObjectURL(url);
      }
      notify(`Exported HTML to ${exported.path}`);
    } catch (err: unknown) {
      notify(`Failed to export HTML: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify]);

  const compactSession = useCallback(
    async (customInstructions?: string) => {
      try {
        await tauriApi.compactSession(customInstructions);
        notify("Compacted session context.");
      } catch (err: unknown) {
        notify(`Failed to compact session: ${formatErrorMessage(err)}`, "error");
      }
    },
    [notify]
  );

  const runBash = useCallback(
    async (command: string) => {
      const trimmed = command.trim();
      if (!trimmed) return;
      dispatch({ type: "bash_started", command: trimmed });
      try {
        const result = await tauriApi.piBash(trimmed);
        dispatch({ type: "bash_finished", command: trimmed, result });
      } catch (err: unknown) {
        dispatch({ type: "bash_aborted" });
        notify(`Bash execution failed: ${formatErrorMessage(err)}`, "error");
      }
    },
    [notify]
  );

  const abortBash = useCallback(async () => {
    try {
      await tauriApi.piAbortBash();
    } catch (err: unknown) {
      notify(`Failed to abort bash: ${formatErrorMessage(err)}`, "error");
    } finally {
      dispatch({ type: "bash_aborted" });
    }
  }, [notify]);

  const refreshSessionStats = useCallback(async () => {
    try {
      const stats = await tauriApi.piGetSessionStats();
      dispatch({ type: "set_session_stats", stats });
    } catch (err: unknown) {
      notify(`Failed to load session stats: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify]);

  const setAutoCompaction = useCallback(
    async (enabled: boolean) => {
      const prev = stateRef.current.autoCompactionEnabled;
      dispatch({ type: "set_auto_compaction", enabled });
      try {
        await tauriApi.piSetAutoCompaction(enabled);
      } catch (err: unknown) {
        dispatch({ type: "set_auto_compaction", enabled: prev });
        notify(`Failed to update auto-compaction: ${formatErrorMessage(err)}`, "error");
      }
    },
    [notify]
  );

  const setAutoRetry = useCallback(
    async (enabled: boolean) => {
      const prev = stateRef.current.autoRetryEnabled;
      dispatch({ type: "set_auto_retry", enabled });
      try {
        await tauriApi.piSetAutoRetry(enabled);
      } catch (err: unknown) {
        dispatch({ type: "set_auto_retry", enabled: prev });
        notify(`Failed to update auto-retry: ${formatErrorMessage(err)}`, "error");
      }
    },
    [notify]
  );

  const setSteeringMode = useCallback(
    async (mode: PiQueueMode) => {
      const prev = stateRef.current.steeringMode;
      dispatch({ type: "set_steering_mode", mode });
      try {
        await tauriApi.piSetSteeringMode(mode);
      } catch (err: unknown) {
        dispatch({ type: "set_steering_mode", mode: prev });
        notify(`Failed to update steering mode: ${formatErrorMessage(err)}`, "error");
      }
    },
    [notify]
  );

  const setFollowUpMode = useCallback(
    async (mode: PiQueueMode) => {
      const prev = stateRef.current.followUpMode;
      dispatch({ type: "set_follow_up_mode", mode });
      try {
        await tauriApi.piSetFollowUpMode(mode);
      } catch (err: unknown) {
        dispatch({ type: "set_follow_up_mode", mode: prev });
        notify(`Failed to update follow-up mode: ${formatErrorMessage(err)}`, "error");
      }
    },
    [notify]
  );

  const selectModel = useCallback(
    async (provider: string, modelId: string) => {
      const optimisticModel =
        stateRef.current.models.find((m) => m.provider === provider && m.modelId === modelId) ?? {
          provider,
          modelId,
          label: modelId,
        };
      const reqId = ++modelRequestIdRef.current;
      dispatch({ type: "model_selected", model: optimisticModel });
      try {
        const model = await tauriApi.setModel(provider, modelId);
        if (reqId === modelRequestIdRef.current) {
          lastConfirmedModelRef.current = model;
          dispatch({ type: "model_selected", model });
        }
      } catch (err: unknown) {
        if (reqId === modelRequestIdRef.current) {
          dispatch({ type: "model_selected", model: lastConfirmedModelRef.current });
          notify(`Failed to switch model: ${formatErrorMessage(err)}`, "error");
        }
      }
    },
    [notify]
  );

  const selectThinkingLevel = useCallback(
    async (level: ThinkingLevel) => {
      const reqId = ++thinkingRequestIdRef.current;
      dispatch({ type: "thinking_selected", level });
      try {
        const next = await tauriApi.setThinkingLevel(level);
        if (reqId === thinkingRequestIdRef.current) {
          lastConfirmedThinkingRef.current = next;
          dispatch({ type: "thinking_selected", level: next });
        }
      } catch (err: unknown) {
        if (reqId === thinkingRequestIdRef.current) {
          dispatch({ type: "thinking_selected", level: lastConfirmedThinkingRef.current });
          notify(`Failed to update thinking level: ${formatErrorMessage(err)}`, "error");
        }
      }
    },
    [notify]
  );

  const cycleModel = useCallback(async () => {
    try {
      const cycled = await tauriApi.piCycleModel();
      const current = stateRef.current;
      const resolution = resolveCycledModel(current.models, current.selectedModel, cycled);
      switch (resolution.kind) {
        case "cycled":
          lastConfirmedModelRef.current = resolution.model;
          dispatch({ type: "model_selected", model: resolution.model });
          if (resolution.thinkingLevel) {
            lastConfirmedThinkingRef.current = resolution.thinkingLevel;
            dispatch({ type: "thinking_selected", level: resolution.thinkingLevel });
          }
          pushToast(`Switched model to ${resolution.model.label}`);
          return;
        case "fallback_set_model":
          await selectModel(resolution.model.provider, resolution.model.modelId);
          if (
            lastConfirmedModelRef.current.provider === resolution.model.provider &&
            lastConfirmedModelRef.current.modelId === resolution.model.modelId
          ) {
            pushToast(`Switched model to ${resolution.model.label}`);
          }
          return;
        case "single_model":
          pushToast(
            "Only one model is configured. Add more models in Provider Settings.",
            "warning"
          );
          return;
        default: {
          const _exhaustive: never = resolution;
          return _exhaustive;
        }
      }
    } catch (err: unknown) {
      notify(`Failed to cycle model: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify, pushToast, selectModel]);

  const cycleThinkingLevel = useCallback(async () => {
    try {
      const cycled = await tauriApi.piCycleThinkingLevel();
      const current = stateRef.current;
      const resolution = resolveCycledThinkingLevel(
        current.thinkingLevels,
        current.thinkingLevel,
        cycled
      );
      switch (resolution.kind) {
        case "cycled":
          lastConfirmedThinkingRef.current = resolution.level;
          dispatch({ type: "thinking_selected", level: resolution.level });
          pushToast(`Thinking level set to ${resolution.level}`);
          return;
        case "fallback_set_thinking":
          await selectThinkingLevel(resolution.level);
          if (lastConfirmedThinkingRef.current === resolution.level) {
            pushToast(`Thinking level set to ${resolution.level}`);
          }
          return;
        case "single_level":
          return;
        default: {
          const _exhaustive: never = resolution;
          return _exhaustive;
        }
      }
    } catch (err: unknown) {
      notify(`Failed to cycle thinking level: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify, pushToast, selectThinkingLevel]);

  const revertTurn = useCallback(
    async (
      entryId: string,
      revertCode: boolean,
      fallbackPromptText?: string,
      userTurnIndex?: number
    ) => changeWorkspace(async () => {
      const resolvedUserTurnIndex =
        userTurnIndex !== undefined
          ? userTurnIndex
          : (() => {
              const userMessages = stateRef.current.messages.filter((m) => m.role === "user");
              const idx = userMessages.findIndex((m) => m.id === entryId);
              return idx >= 0 ? idx : undefined;
            })();
      try {
        const result = await tauriApi.piRevertTurn(
          entryId,
          revertCode,
          resolvedUserTurnIndex,
          fallbackPromptText
        );
        const restoredPrompt = result.restoredPrompt ?? fallbackPromptText ?? null;
        dispatch({
          type: "turn_reverted",
          session: result.session,
          messages: result.messages,
          restoredPrompt,
          canRedo: result.canRedo,
          revertedFiles: result.revertedFiles,
        });
        if (result.codeRevertNote) {
          pushToast(result.codeRevertNote, "warning");
        } else if (result.codeReverted) {
          const count = result.revertedFiles.length;
          pushToast(
            count === 1 ? "Reverted chat and 1 file." : `Reverted chat and ${count} files.`
          );
        } else {
          pushToast("Reverted chat only.");
        }
      } catch (err: unknown) {
        notify(`Failed to revert turn: ${formatErrorMessage(err)}`, "error");
      }
    }),
    [changeWorkspace, notify, pushToast]
  );

  const redoRevert = useCallback(async () => changeWorkspace(async () => {
    try {
      const result = await tauriApi.piRedoRevert();
      dispatch({ type: "revert_redone" });
      const count = result.restoredFiles.length;
      pushToast(
        count === 1 ? "Restored 1 reverted file." : `Restored ${count} reverted files.`
      );
    } catch (err: unknown) {
      notify(`Failed to restore reverted files: ${formatErrorMessage(err)}`, "error");
    }
  }), [changeWorkspace, notify, pushToast]);

  const copyFullChat = useCallback(async () => {
    try {
      const markdown = formatChatTranscriptMarkdown(stateRef.current.messages);
      if (!markdown) {
        pushToast("No messages available to copy.", "warning");
        return;
      }
      if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(markdown);
        pushToast("Copied full chat to clipboard.");
      } else {
        pushToast("Clipboard API unavailable in this environment.", "warning");
      }
    } catch (err: unknown) {
      notify(`Failed to copy chat: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify, pushToast]);

  const copyLastAssistantText = useCallback(async () => {
    try {
      const text = await tauriApi.piGetLastAssistantText();
      if (!text || text.trim().length === 0) {
        pushToast("No assistant response available to copy.", "warning");
        return;
      }
      if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
        pushToast("Copied last assistant response to clipboard.");
      } else {
        pushToast("Clipboard API unavailable in this environment.", "warning");
      }
    } catch (err: unknown) {
      notify(`Failed to copy response: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify, pushToast]);

  const openSettingsModal = useCallback(
    (tab?: SettingsModalTab) => {
      dispatch({ type: "open_settings_modal", tab });
      if (tab === "stats") {
        void refreshSessionStats();
      }
    },
    [refreshSessionStats]
  );

  const closeSettingsModal = useCallback(() => {
    dispatch({ type: "close_settings_modal" });
  }, []);

  const dismissToast = useCallback((id: string) => {
    dispatch({ type: "dismiss_toast", id });
  }, []);

  const consumeEditorPrefill = useCallback((): string | null => {
    const current = stateRef.current.editorPrefillText;
    if (current !== null) {
      dispatch({ type: "consume_editor_prefill" });
    }
    return current;
  }, []);

  const steerPrompt = useCallback(async () => {
    if (submittingRef.current || changingWorkspaceRef.current) return;
    const prevDraft = stateRef.current.draft;
    const prevAttachments = stateRef.current.attachments;
    const text = prevDraft.trim();
    if (!text) return;

    submittingRef.current = true;
    dispatch({ type: "clear_composer" });
    try {
      await tauriApi.steer(text, prevAttachments);
    } catch (err: unknown) {
      dispatch({ type: "restore_composer", draft: prevDraft, attachments: prevAttachments });
      notify(`Failed to send steering instruction: ${formatErrorMessage(err)}`, "error");
    } finally {
      submittingRef.current = false;
    }
  }, [notify]);

  const followUpPrompt = useCallback(async () => {
    if (submittingRef.current || changingWorkspaceRef.current) return;
    const prevDraft = stateRef.current.draft;
    const prevAttachments = stateRef.current.attachments;
    const text = prevDraft.trim();
    if (!text) return;

    submittingRef.current = true;
    dispatch({ type: "clear_composer" });
    try {
      await tauriApi.followUp(text, prevAttachments);
    } catch (err: unknown) {
      dispatch({ type: "restore_composer", draft: prevDraft, attachments: prevAttachments });
      notify(`Failed to queue follow-up: ${formatErrorMessage(err)}`, "error");
    } finally {
      submittingRef.current = false;
    }
  }, [notify]);

  const clearQueue = useCallback(async () => {
    try {
      await tauriApi.clearQueue();
    } catch (err: unknown) {
      notify(`Failed to clear queue: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify]);

  const abortTurn = useCallback(async () => {
    try {
      if (stateRef.current.retryBanner) {
        const results = await Promise.allSettled([tauriApi.abort(), tauriApi.abortRetry()]);
        const rejected = results.find(
          (r): r is PromiseRejectedResult => r.status === "rejected"
        );
        if (rejected) {
          notify(`Failed to abort turn: ${formatErrorMessage(rejected.reason)}`, "error");
        }
      } else {
        await tauriApi.abort();
      }
    } catch (err: unknown) {
      notify(`Failed to abort turn: ${formatErrorMessage(err)}`, "error");
    } finally {
      dispatch({ type: "turn_aborted" });
    }
  }, [notify]);

  const abortRetry = useCallback(async () => {
    try {
      await tauriApi.abortRetry();
    } catch (err: unknown) {
      notify(`Failed to cancel retry: ${formatErrorMessage(err)}`, "error");
    } finally {
      dispatch({ type: "turn_aborted" });
    }
  }, [notify]);

  const sendPrompt = useCallback(async () => {
    if (submittingRef.current || stateRef.current.isStreaming) return;
    if (changingWorkspaceRef.current) {
      notify("Wait for the current chat operation to finish.", "error");
      return;
    }
    const text = stateRef.current.draft.trim();
    const attachments = stateRef.current.attachments;
    if (!text && attachments.length === 0) return;

    const bashInvocation = parseBashPrefix(text);
    if (bashInvocation) {
      dispatch({ type: "set_draft", draft: "" });
      await runBash(bashInvocation.command);
      return;
    }

    const slashInvocation = parseBuiltinSlashInvocation(text);
    if (slashInvocation) {
      dispatch({ type: "set_draft", draft: "" });
      switch (slashInvocation.command) {
        case "new":
          await createSession();
          return;
        case "name": {
          const title = slashInvocation.args.trim();
          const activeId = stateRef.current.activeSessionId;
          if (!title) {
            pushToast("Usage: /name <title>", "warning");
            return;
          }
          if (activeId) {
            await renameSession(activeId, title);
          }
          return;
        }
        case "compact":
          await compactSession(slashInvocation.args || undefined);
          return;
        case "fork":
          await openForkModal();
          return;
        case "tree":
          await openHistoryTree();
          return;
        case "export":
          await exportSessionHtml();
          return;
        case "session":
          openSettingsModal("stats");
          await refreshSessionStats();
          return;
        case "copy":
          await copyLastAssistantText();
          return;
        case "model": {
          const arg = slashInvocation.args.trim().toLowerCase();
          if (!arg) {
            await cycleModel();
            return;
          }
          const matched = stateRef.current.models.find(
            (m) =>
              m.modelId.toLowerCase() === arg ||
              m.label.toLowerCase() === arg ||
              `${m.provider}/${m.modelId}`.toLowerCase() === arg ||
              m.modelId.toLowerCase().includes(arg) ||
              m.label.toLowerCase().includes(arg)
          );
          if (matched) {
            await selectModel(matched.provider, matched.modelId);
          } else {
            pushToast(`No model matching "${slashInvocation.args.trim()}".`, "warning");
          }
          return;
        }
        case "thinking": {
          const arg = slashInvocation.args.trim().toLowerCase();
          if (!arg) {
            await cycleThinkingLevel();
            return;
          }
          if (
            arg === "off" ||
            arg === "minimal" ||
            arg === "low" ||
            arg === "medium" ||
            arg === "high" ||
            arg === "xhigh"
          ) {
            await selectThinkingLevel(arg);
          } else {
            pushToast(
              `Invalid thinking level "${slashInvocation.args.trim()}". Use off, minimal, low, medium, high, or xhigh.`,
              "warning"
            );
          }
          return;
        }
        case "settings":
          openSettingsModal("behavior");
          return;
        default: {
          const _exhaustive: never = slashInvocation.command;
          return _exhaustive;
        }
      }
    }

    submittingRef.current = true;
    const optimisticMsg: PiChatMessage = {
      id: nextClientId("local"),
      role: "user",
      content: text,
      timestamp: nowTimeLabel(),
      attachments: attachments.length > 0 ? attachments : undefined,
    };
    dispatch({ type: "prompt_submitted", message: optimisticMsg });
    try {
      await tauriApi.sendPrompt(text, attachments, {
        projectId: stateRef.current.activeProjectId,
        sessionId: stateRef.current.activeSessionId,
      });
    } catch (err: unknown) {
      dispatch({ type: "prompt_failed" });
      notify(`Failed to send prompt: ${formatErrorMessage(err)}`, "error");
    } finally {
      submittingRef.current = false;
    }
  }, [
    runBash,
    createSession,
    renameSession,
    compactSession,
    openForkModal,
    openHistoryTree,
    exportSessionHtml,
    openSettingsModal,
    refreshSessionStats,
    copyLastAssistantText,
    cycleModel,
    selectModel,
    cycleThinkingLevel,
    selectThinkingLevel,
    pushToast,
    notify,
  ]);

  const upsertProvider = useCallback(
    async (input: ProviderProfileInput) => {
      const reqId = ++providerRequestIdRef.current;
      try {
        await tauriApi.upsertProvider(input);
        const [list, refreshed] = await Promise.all([
          tauriApi.listProviders(),
          tauriApi.bootstrapPiWorkspace().catch(() => null),
        ]);
        if (reqId === providerRequestIdRef.current) {
          dispatch({
            type: "providers_updated",
            providers: list,
            refreshedModels: refreshed?.models,
          });
          notify(`Saved provider "${input.name}"`);
        }
      } catch (err: unknown) {
        if (reqId === providerRequestIdRef.current) {
          notify(`Failed to save provider: ${formatErrorMessage(err)}`, "error");
        }
        throw err;
      }
    },
    [notify]
  );

  const deleteProvider = useCallback(
    async (id: string) => {
      const reqId = ++providerRequestIdRef.current;
      try {
        await tauriApi.deleteProvider(id);
        const [list, refreshed] = await Promise.all([
          tauriApi.listProviders(),
          tauriApi.bootstrapPiWorkspace().catch(() => null),
        ]);
        if (reqId === providerRequestIdRef.current) {
          const baseModels = refreshed?.models ?? stateRef.current.models;
          const filteredBase = baseModels.filter((m) => m.provider !== id);
          const remainingModels = collectModels(filteredBase, list);
          if (
            lastConfirmedModelRef.current.provider === id &&
            !list.some((p) => p.id === id)
          ) {
            lastConfirmedModelRef.current = remainingModels[0] ?? FALLBACK_MODEL;
          }
          dispatch({
            type: "providers_updated",
            providers: list,
            refreshedModels: refreshed?.models,
            deletedProviderId: id,
          });
          notify(`Deleted provider "${id}"`);
        }
      } catch (err: unknown) {
        if (reqId === providerRequestIdRef.current) {
          notify(`Failed to delete provider: ${formatErrorMessage(err)}`, "error");
        }
        throw err;
      }
    },
    [notify]
  );

  const respondExtensionDialog = useCallback(
    async (dialogId: string, answer: ExtensionUiAnswer) => {
      dispatch({ type: "shift_extension_dialog", dialogId });
      try {
        await tauriApi.respondExtensionUi(dialogId, answer);
      } catch (err: unknown) {
        notify(`Failed to send dialog response: ${formatErrorMessage(err)}`, "error");
      }
    },
    [notify]
  );

  const projectSessions = useMemo(
    () =>
      state.activeProjectId
        ? state.sessions.filter((s) => s.projectId === state.activeProjectId)
        : state.sessions,
    [state.sessions, state.activeProjectId]
  );

  const activeSession = useMemo(
    () => state.sessions.find((s) => s.id === state.activeSessionId) ?? null,
    [state.sessions, state.activeSessionId]
  );

  return {
    state,
    projectSessions,
    activeSession,
    actions: {
      toggleTheme: () => dispatch({ type: "toggle_theme" }),
      retryBootstrap,
      selectProject,
      pickAndAddProject,
      addProject,
      removeProject,
      switchSession,
      createSession,
      renameSession,
      archiveSession,
      deleteSession,
      openHistoryTree,
      openForkModal,
      openProvidersModal,
      closeModal,
      forkAtEntry,
      cloneCurrentSession,
      exportSessionHtml,
      compactSession,
      setDraft: (draft: string) => dispatch({ type: "set_draft", draft }),
      recallUp: () => dispatch({ type: "recall_up" }),
      recallDown: () => dispatch({ type: "recall_down" }),
      addAttachment: (attachment: PendingAttachment) =>
        dispatch({ type: "add_attachment", attachment }),
      removeAttachment: (filename: string) =>
        dispatch({ type: "remove_attachment", filename }),
      sendPrompt,
      steerPrompt,
      followUpPrompt,
      clearQueue,
      abortTurn,
      abortRetry,
      selectModel,
      selectThinkingLevel,
      upsertProvider,
      deleteProvider,
      respondExtensionDialog,
      runBash,
      abortBash,
      refreshSessionStats,
      setAutoCompaction,
      setAutoRetry,
      setSteeringMode,
      setFollowUpMode,
      cycleModel,
      cycleThinkingLevel,
      revertTurn,
      redoRevert,
      copyFullChat,
      copyLastAssistantText,
      dismissToast,
      openSettingsModal,
      closeSettingsModal,
      consumeEditorPrefill,
      dismissSnackbar: (id: string) => dispatch({ type: "dismiss_snackbar", id }),
      dismissWidget: (id: string) => dispatch({ type: "dismiss_widget", id }),
    },
  };
}
