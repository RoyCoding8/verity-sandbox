import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import {
  tauriApi,
  type DialogKind,
  type ExtensionUiAnswer,
  type ForkCandidate,
  type HistoryTreeNode,
  type ModelOption,
  type PendingAttachment,
  type PiBootstrapData,
  type PiChatMessage,
  type PiProject,
  type PiRetryBanner,
  type PiSessionSummary,
  type PiToolCallState,
  type PiUiEvent,
  type ProviderProfileInput,
  type PublicProviderProfile,
  type SlashOrSkillCommand,
  type ThinkingLevel,
} from "../api/tauri";
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
}

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
}

const THEME_STORAGE_KEY = "verity.desktop.theme";

const FALLBACK_MODEL: ModelOption = {
  provider: "anthropic",
  modelId: "claude-sonnet-4-5",
  label: "Claude Sonnet 4.5",
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
  | { readonly type: "set_connection_state"; readonly connectionState: ConnectionState }
  | { readonly type: "toggle_theme" }
  | { readonly type: "select_project"; readonly projectId: string }
  | {
      readonly type: "session_switched";
      readonly sessionId: string;
      readonly projectId: string;
      readonly messages: readonly PiChatMessage[];
    }
  | {
      readonly type: "session_created";
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
  | { readonly type: "dismiss_widget"; readonly id: string };

function nowTimeLabel(): string {
  return new Date().toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
  });
}

function mergeToolCalls(
  existing: readonly PiToolCallState[] | undefined,
  incoming: readonly PiToolCallState[]
): readonly PiToolCallState[] | undefined {
  if (incoming.length === 0) return existing;
  if (!existing || existing.length === 0) return incoming;
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

function attachToolsAndClearStreaming(
  messages: readonly PiChatMessage[],
  toolsToAttach: readonly PiToolCallState[]
): readonly PiChatMessage[] {
  const latestAssistantIdx =
    toolsToAttach.length > 0 ? findLatestAssistantIndexInCurrentTurn(messages) : -1;
  return messages.map((m, idx) => {
    const shouldAttach = idx === latestAssistantIdx;
    if (!m.streaming && !shouldAttach) return m;
    return {
      ...m,
      streaming: false,
      toolCalls: shouldAttach ? mergeToolCalls(m.toolCalls, toolsToAttach) : m.toolCalls,
    };
  });
}

function attachErrorToActiveTurn(
  messages: readonly PiChatMessage[],
  liveTools: readonly PiToolCallState[],
  isStreaming: boolean,
  errorMessage: string
): readonly PiChatMessage[] {
  const settledTools = settleRunningTools(liveTools);
  const latestAssistantIdx = findLatestAssistantIndexInCurrentTurn(messages);
  if (latestAssistantIdx >= 0) {
    const target = messages[latestAssistantIdx];
    if (target.streaming || isStreaming) {
      return messages.map((m, idx) => {
        if (idx === latestAssistantIdx) {
          return {
            ...m,
            streaming: false,
            stopReason: "error",
            errorMessage,
            toolCalls: mergeToolCalls(m.toolCalls, settledTools),
          };
        }
        return m.streaming ? { ...m, streaming: false } : m;
      });
    }
  }
  if (!isStreaming) {
    return messages.map((m) => (m.streaming ? { ...m, streaming: false } : m));
  }
  const failedTurn: PiChatMessage = {
    id: nextClientId("stream"),
    role: "assistant",
    content: "",
    timestamp: nowTimeLabel(),
    streaming: false,
    stopReason: "error",
    errorMessage,
    toolCalls: settledTools.length > 0 ? settledTools : undefined,
  };
  return [
    ...messages.map((m) => (m.streaming ? { ...m, streaming: false } : m)),
    failedTurn,
  ];
}

function isNonEmptyAssistantMessage(m: PiChatMessage): boolean {
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
  const lastIdx = messages.length - 1;
  if (lastIdx >= 0 && messages[lastIdx].role === "assistant") {
    return { kind: "found", index: lastIdx };
  }
  return { kind: "none" };
}

function applyPiEvent(state: PiWorkspaceState, event: PiUiEvent): PiWorkspaceState {
  switch (event.type) {
    case "message_start": {
      if (event.role === "user") {
        return state.isStreaming ? state : { ...state, isStreaming: true };
      }
      const id = event.message_id ?? nextClientId("stream");
      const existingIdx = state.messages.findIndex((m) => m.id === id);
      if (existingIdx >= 0) {
        const messages = state.messages.map((m, idx) =>
          idx === existingIdx ? { ...m, streaming: true } : m
        );
        return { ...state, isStreaming: true, messages };
      }
      const role: PiChatMessage["role"] = event.role === "system" ? "system" : "assistant";
      const nextMessage: PiChatMessage = {
        id,
        role,
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
      if (!delta && !thinkingDelta) {
        return state.isStreaming ? state : { ...state, isStreaming: true };
      }
      const target = resolveAssistantTarget(state.messages, event.message_id);
      if (target.kind === "user_message") {
        return state;
      }
      if (target.kind === "found") {
        const messages = state.messages.map((m, idx) => {
          if (idx !== target.index) return m;
          return {
            ...m,
            content: delta ? m.content + delta : m.content,
            thinking: thinkingDelta ? (m.thinking ?? "") + thinkingDelta : m.thinking,
            streaming: true,
          };
        });
        return { ...state, isStreaming: true, messages };
      }
      const created: PiChatMessage = {
        id: event.message_id ?? nextClientId("stream"),
        role: "assistant",
        content: delta,
        thinking: thinkingDelta || undefined,
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
      if (event.role === "user") {
        return state;
      }
      const finalText = event.text ?? null;
      const finalThinking = event.thinking ?? null;
      const stopReason = event.stop_reason ?? null;
      const errorMessage = event.error_message ?? null;
      const target = resolveAssistantTarget(state.messages, event.message_id);
      if (target.kind === "user_message") {
        return state;
      }
      if (target.kind === "found") {
        const messages = state.messages.map((m, idx) => {
          if (idx !== target.index) return m;
          const hydratedContent = m.content.length === 0 && finalText ? finalText : m.content;
          const hydratedThinking =
            (!m.thinking || m.thinking.length === 0) && finalThinking
              ? finalThinking
              : m.thinking;
          return {
            ...m,
            content: hydratedContent,
            thinking: hydratedThinking,
            streaming: false,
            stopReason: stopReason ?? m.stopReason,
            errorMessage: errorMessage ?? m.errorMessage,
            toolCalls: mergeToolCalls(m.toolCalls, state.liveTools),
          };
        });
        return {
          ...state,
          messages,
          liveTools: [],
        };
      }
      const hasError = stopReason === "error" || Boolean(errorMessage && errorMessage.trim().length > 0);
      if (!finalText && !finalThinking && !hasError && state.liveTools.length === 0) {
        return state;
      }
      const completed: PiChatMessage = {
        id: event.message_id ?? nextClientId("stream"),
        role: "assistant",
        content: finalText ?? "",
        thinking: finalThinking ?? undefined,
        timestamp: nowTimeLabel(),
        streaming: false,
        stopReason,
        errorMessage,
        toolCalls: state.liveTools.length > 0 ? state.liveTools : undefined,
      };
      return {
        ...state,
        messages: [...state.messages, completed],
        liveTools: [],
      };
    }

    case "tool_execution_start": {
      const id = event.tool_call_id ?? nextClientId("tool");
      const name = event.tool_name ?? "tool";
      const nextTool: PiToolCallState = { id, name, status: "running" };
      const exists = state.liveTools.some((t) => t.id === id);
      const liveTools = exists
        ? state.liveTools.map((t) => (t.id === id ? nextTool : t))
        : [...state.liveTools, nextTool];
      return { ...state, isStreaming: true, liveTools };
    }

    case "tool_execution_end": {
      const id = event.tool_call_id;
      const nextStatus: PiToolCallState["status"] = event.is_error ? "error" : "done";
      if (id) {
        const liveTools = state.liveTools.map((t) =>
          t.id === id ? { ...t, status: nextStatus } : t
        );
        return { ...state, liveTools };
      }
      const matchIdx = state.liveTools.findIndex(
        (t) => t.name === event.tool_name && t.status === "running"
      );
      if (matchIdx < 0) {
        return state;
      }
      const liveTools = state.liveTools.map((t, idx) =>
        idx === matchIdx ? { ...t, status: nextStatus } : t
      );
      return { ...state, liveTools };
    }

    case "queue_update":
      return {
        ...state,
        pendingQueueCount: Math.max(0, event.pending_count ?? 0),
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
        return {
          ...state,
          isStreaming: false,
          retryBanner: null,
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
        extensionSnackbars: [...state.extensionSnackbars.slice(-3), snackbar],
      };
    }

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
        return {
          ...state,
          extensionWidgets: state.extensionWidgets.filter((w) => w.id !== event.id),
        };
      }
      const nextWidget: ExtensionWidget = {
        id: event.id,
        title: event.title,
        body: event.body,
      };
      const exists = state.extensionWidgets.some((w) => w.id === event.id);
      return {
        ...state,
        extensionWidgets: exists
          ? state.extensionWidgets.map((w) => (w.id === event.id ? nextWidget : w))
          : [...state.extensionWidgets, nextWidget],
      };
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

    case "bootstrap_loaded":
      return {
        ...state,
        loading: false,
        error: null,
        connectionState: "connected",
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
      };

    case "set_error":
      return {
        ...state,
        error: action.error,
        loading: false,
        connectionState: action.error ? "disconnected" : state.connectionState,
      };

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
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
      };
    }

    case "session_switched":
      return {
        ...state,
        activeProjectId: action.projectId,
        activeSessionId: action.sessionId,
        messages: action.messages,
        isStreaming: false,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
      };

    case "session_created": {
      const filtered = state.sessions.filter((s) => s.id !== action.session.id);
      return {
        ...state,
        sessions: [action.session, ...filtered],
        activeProjectId: action.session.projectId,
        activeSessionId: action.session.id,
        messages: action.messages,
        isStreaming: false,
        liveTools: [],
        retryBanner: null,
        pendingQueueCount: 0,
        modal: { kind: "none" },
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
        draft: "",
        attachments: [],
        recall: { history: nextHistory, index: null, draftSnapshot: "" },
      };
    }

    case "prompt_failed":
      return {
        ...state,
        isStreaming: false,
        liveTools: [],
        retryBanner: null,
        messages: state.messages.map((m) =>
          m.streaming ? { ...m, streaming: false } : m
        ),
      };

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

    case "dismiss_widget":
      return {
        ...state,
        extensionWidgets: state.extensionWidgets.filter((w) => w.id !== action.id),
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
  const sessionRequestIdRef = useRef(0);
  const providerRequestIdRef = useRef(0);
  const submittingRef = useRef(false);

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

    void tauriApi
      .bootstrapPiWorkspace()
      .then((data) => {
        if (!active) return;
        lastConfirmedModelRef.current = data.selectedModel;
        lastConfirmedThinkingRef.current = data.thinkingLevel;
        dispatch({ type: "bootstrap_loaded", data });
      })
      .catch((err: unknown) => {
        if (active) {
          dispatch({
            type: "set_error",
            error: formatErrorMessage(err),
          });
        }
      });

    void tauriApi
      .listenPiEvents((event) => {
        if (active) dispatch({ type: "pi_event", event });
      })
      .then((fn) => {
        if (active) {
          unlisten = fn;
        } else {
          fn();
        }
      })
      .catch((err: unknown) => {
        if (!active) return;
        dispatch({ type: "set_connection_state", connectionState: "degraded" });
        dispatch({
          type: "push_snackbar",
          snackbar: {
            id: nextClientId("sb"),
            message: `Event stream unavailable: ${formatErrorMessage(err)}`,
            tone: "error",
          },
        });
      });

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

  const selectProject = useCallback(
    async (projectId: string) => {
      const reqId = ++sessionRequestIdRef.current;
      dispatch({ type: "select_project", projectId });
      const firstSession = stateRef.current.sessions.find((s) => s.projectId === projectId);
      if (!firstSession) return;
      try {
        const messages = await tauriApi.switchSession(firstSession.path, firstSession.id);
        if (reqId === sessionRequestIdRef.current) {
          dispatch({
            type: "session_switched",
            sessionId: firstSession.id,
            projectId,
            messages,
          });
        }
      } catch (err: unknown) {
        if (reqId === sessionRequestIdRef.current) {
          notify(`Failed to switch project session: ${formatErrorMessage(err)}`, "error");
        }
      }
    },
    [notify]
  );

  const switchSession = useCallback(
    async (session: PiSessionSummary) => {
      const reqId = ++sessionRequestIdRef.current;
      try {
        const messages = await tauriApi.switchSession(session.path, session.id);
        if (reqId === sessionRequestIdRef.current) {
          dispatch({
            type: "session_switched",
            sessionId: session.id,
            projectId: session.projectId,
            messages,
          });
        }
      } catch (err: unknown) {
        if (reqId === sessionRequestIdRef.current) {
          notify(`Failed to switch session: ${formatErrorMessage(err)}`, "error");
        }
      }
    },
    [notify]
  );

  const createSession = useCallback(async () => {
    const reqId = ++sessionRequestIdRef.current;
    const current = stateRef.current;
    const projectId = current.activeProjectId || current.projects[0]?.id || "proj-verity";
    try {
      const session = await tauriApi.newSession(projectId);
      if (reqId === sessionRequestIdRef.current) {
        dispatch({ type: "session_created", session, messages: [] });
      }
    } catch (err: unknown) {
      if (reqId === sessionRequestIdRef.current) {
        notify(`Failed to create session: ${formatErrorMessage(err)}`, "error");
      }
    }
  }, [notify]);

  const renameSession = useCallback(
    async (sessionId: string, name: string) => {
      try {
        const updated = await tauriApi.renameSession(sessionId, name);
        if (updated) {
          dispatch({ type: "session_renamed", session: updated });
        }
      } catch (err: unknown) {
        notify(`Failed to rename session: ${formatErrorMessage(err)}`, "error");
      }
    },
    [notify]
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
    async (entryId: string) => {
      const reqId = ++sessionRequestIdRef.current;
      try {
        const result = await tauriApi.forkSession(entryId);
        if (reqId === sessionRequestIdRef.current) {
          dispatch({
            type: "session_created",
            session: result.session,
            messages: result.messages,
          });
          notify(`Forked session "${result.session.name}"`);
        }
      } catch (err: unknown) {
        if (reqId === sessionRequestIdRef.current) {
          notify(`Failed to fork session: ${formatErrorMessage(err)}`, "error");
        }
      }
    },
    [notify]
  );

  const cloneCurrentSession = useCallback(async () => {
    const reqId = ++sessionRequestIdRef.current;
    try {
      const result = await tauriApi.cloneSession();
      if (reqId === sessionRequestIdRef.current) {
        dispatch({
          type: "session_created",
          session: result.session,
          messages: result.messages,
        });
        notify(`Cloned session "${result.session.name}"`);
      }
    } catch (err: unknown) {
      if (reqId === sessionRequestIdRef.current) {
        notify(`Failed to clone session: ${formatErrorMessage(err)}`, "error");
      }
    }
  }, [notify]);

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

  const compactSession = useCallback(async () => {
    try {
      await tauriApi.compactSession();
      notify("Compacted session context.");
    } catch (err: unknown) {
      notify(`Failed to compact session: ${formatErrorMessage(err)}`, "error");
    }
  }, [notify]);

  const sendPrompt = useCallback(async () => {
    if (submittingRef.current || stateRef.current.isStreaming) return;
    const text = stateRef.current.draft.trim();
    const attachments = stateRef.current.attachments;
    if (!text && attachments.length === 0) return;

    if (text === "/tree") {
      dispatch({ type: "set_draft", draft: "" });
      await openHistoryTree();
      return;
    }
    if (text === "/fork") {
      dispatch({ type: "set_draft", draft: "" });
      await openForkModal();
      return;
    }
    if (text === "/export") {
      dispatch({ type: "set_draft", draft: "" });
      await exportSessionHtml();
      return;
    }
    if (text === "/compact" || text.startsWith("/compact ")) {
      dispatch({ type: "set_draft", draft: "" });
      await compactSession();
      return;
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
      await tauriApi.sendPrompt(text, attachments);
    } catch (err: unknown) {
      dispatch({ type: "prompt_failed" });
      notify(`Failed to send prompt: ${formatErrorMessage(err)}`, "error");
    } finally {
      submittingRef.current = false;
    }
  }, [openHistoryTree, openForkModal, exportSessionHtml, compactSession, notify]);

  const steerPrompt = useCallback(async () => {
    if (submittingRef.current) return;
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
    if (submittingRef.current) return;
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
      switchSession,
      createSession,
      renameSession,
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
      dismissSnackbar: (id: string) => dispatch({ type: "dismiss_snackbar", id }),
      dismissWidget: (id: string) => dispatch({ type: "dismiss_widget", id }),
    },
  };
}
