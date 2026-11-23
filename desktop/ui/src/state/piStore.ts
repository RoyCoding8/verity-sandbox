import { useCallback, useEffect, useMemo, useReducer } from "react";
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

export interface RetryBannerState {
  readonly attempt: number;
  readonly maxAttempts: number;
  readonly delayMs: number;
}

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

const FALLBACK_MODEL: ModelOption = {
  provider: "anthropic",
  modelId: "claude-sonnet-4-5",
  label: "Claude Sonnet 4.5",
};

export function createInitialPiState(): PiWorkspaceState {
  return {
    theme: detectPreferredTheme(),
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
    thinkingLevels: ["off", "minimal", "low", "medium", "high", "xhigh"],
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
  | { readonly type: "bootstrap_loaded"; readonly data: PiBootstrapData }
  | { readonly type: "set_error"; readonly error: string | null }
  | { readonly type: "toggle_theme" }
  | { readonly type: "select_project"; readonly projectId: string }
  | {
      readonly type: "session_switched";
      readonly sessionId: string;
      readonly projectId: string;
      readonly messages: readonly PiChatMessage[];
    }
  | { readonly type: "session_created"; readonly session: PiSessionSummary; readonly messages: readonly PiChatMessage[] }
  | { readonly type: "session_renamed"; readonly session: PiSessionSummary }
  | { readonly type: "set_draft"; readonly draft: string }
  | { readonly type: "recall_up" }
  | { readonly type: "recall_down" }
  | { readonly type: "add_attachment"; readonly attachment: PendingAttachment }
  | { readonly type: "remove_attachment"; readonly filename: string }
  | { readonly type: "prompt_submitted"; readonly message: PiChatMessage }
  | { readonly type: "model_selected"; readonly model: ModelOption }
  | { readonly type: "thinking_selected"; readonly level: ThinkingLevel }
  | { readonly type: "set_modal"; readonly modal: ActiveModal }
  | { readonly type: "providers_updated"; readonly providers: readonly PublicProviderProfile[] }
  | { readonly type: "pi_event"; readonly event: PiUiEvent }
  | { readonly type: "shift_extension_dialog"; readonly dialogId: string }
  | { readonly type: "push_snackbar"; readonly snackbar: ExtensionSnackbar }
  | { readonly type: "dismiss_snackbar"; readonly id: string }
  | { readonly type: "dismiss_widget"; readonly id: string };

function applyPiEvent(state: PiWorkspaceState, event: PiUiEvent): PiWorkspaceState {
  switch (event.type) {
    case "message_start": {
      const id = event.message_id ?? `stream-${Date.now()}`;
      const exists = state.messages.some((m) => m.id === id);
      const role =
        event.role === "user" || event.role === "system" ? event.role : "assistant";
      const nextMessages: readonly PiChatMessage[] = exists
        ? state.messages
        : [
            ...state.messages,
            {
              id,
              role,
              content: "",
              timestamp: new Date().toLocaleTimeString([], {
                hour: "numeric",
                minute: "2-digit",
              }),
              streaming: true,
              toolCalls: [],
            },
          ];
      return {
        ...state,
        isStreaming: true,
        messages: nextMessages,
      };
    }

    case "message_update": {
      const delta = event.delta ?? "";
      if (!delta) return state;
      const targetId = event.message_id;
      let updated = false;
      const messages = state.messages.map((m, idx) => {
        const isTarget = targetId ? m.id === targetId : idx === state.messages.length - 1;
        if (!isTarget) return m;
        updated = true;
        return { ...m, content: m.content + delta, streaming: true };
      });
      if (!updated) {
        return {
          ...state,
          isStreaming: true,
          messages: [
            ...state.messages,
            {
              id: targetId ?? `stream-${Date.now()}`,
              role: "assistant",
              content: delta,
              timestamp: new Date().toLocaleTimeString([], {
                hour: "numeric",
                minute: "2-digit",
              }),
              streaming: true,
            },
          ],
        };
      }
      return { ...state, isStreaming: true, messages };
    }

    case "message_end": {
      const messages = state.messages.map((m) =>
        !event.message_id || m.id === event.message_id
          ? {
              ...m,
              streaming: false,
              toolCalls: state.liveTools.length > 0 ? state.liveTools : m.toolCalls,
            }
          : m
      );
      return { ...state, messages };
    }

    case "tool_execution_start": {
      const id = event.tool_call_id ?? `tool-${Date.now()}`;
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
      const liveTools = state.liveTools.map((t) =>
        (id && t.id === id) || (!id && t.name === event.tool_name)
          ? { ...t, status: nextStatus }
          : t
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
        retryBanner: {
          attempt: event.attempt ?? 1,
          maxAttempts: event.max_attempts ?? 3,
          delayMs: event.delay_ms ?? 2000,
        },
      };

    case "auto_retry_end":
      return { ...state, retryBanner: null };

    case "agent_settled":
      return {
        ...state,
        isStreaming: false,
        liveTools: [],
        retryBanner: null,
        messages: state.messages.map((m) =>
          m.streaming ? { ...m, streaming: false } : m
        ),
      };

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
    case "bootstrap_loaded":
      return {
        ...state,
        loading: false,
        error: null,
        projects: action.data.projects,
        sessions: action.data.sessions,
        activeProjectId: action.data.activeProjectId,
        activeSessionId: action.data.activeSessionId,
        messages: action.data.messages,
        models: action.data.models,
        selectedModel: action.data.selectedModel,
        thinkingLevels: action.data.thinkingLevels,
        thinkingLevel: action.data.thinkingLevel,
        commands: action.data.commands,
        providers: action.data.providers,
      };

    case "set_error":
      return { ...state, error: action.error, loading: false };

    case "toggle_theme":
      return { ...state, theme: state.theme === "dark" ? "light" : "dark" };

    case "select_project":
      return { ...state, activeProjectId: action.projectId };

    case "session_switched":
      return {
        ...state,
        activeProjectId: action.projectId,
        activeSessionId: action.sessionId,
        messages: action.messages,
        isStreaming: false,
        liveTools: [],
        retryBanner: null,
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
        draft: "",
        attachments: [],
        recall: { history: nextHistory, index: null, draftSnapshot: "" },
      };
    }

    case "model_selected":
      return { ...state, selectedModel: action.model };

    case "thinking_selected":
      return { ...state, thinkingLevel: action.level };

    case "set_modal":
      return { ...state, modal: action.modal };

    case "providers_updated":
      return { ...state, providers: action.providers };

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

  // Sync dynamic MD3 tokens whenever theme changes
  useEffect(() => {
    applyMd3Theme(state.theme);
  }, [state.theme]);

  // Bootstrap workspace data + subscribe to Pi JSONL stdout events
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;

    void tauriApi
      .bootstrapPiWorkspace()
      .then((data) => {
        if (active) dispatch({ type: "bootstrap_loaded", data });
      })
      .catch((err: unknown) => {
        if (active) {
          dispatch({
            type: "set_error",
            error: err instanceof Error ? err.message : String(err),
          });
        }
      });

    void tauriApi.listenPiEvents((event) => {
      if (active) dispatch({ type: "pi_event", event });
    }).then((fn) => {
      if (active) {
        unlisten = fn;
      } else {
        fn();
      }
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  const notify = useCallback((message: string, tone: "info" | "error" = "info") => {
    dispatch({
      type: "push_snackbar",
      snackbar: { id: `sb-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`, message, tone },
    });
  }, []);

  const selectProject = useCallback(
    async (projectId: string) => {
      dispatch({ type: "select_project", projectId });
      const firstSession = state.sessions.find((s) => s.projectId === projectId);
      if (firstSession) {
        const messages = await tauriApi.switchSession(firstSession.path, firstSession.id);
        dispatch({
          type: "session_switched",
          sessionId: firstSession.id,
          projectId,
          messages,
        });
      }
    },
    [state.sessions]
  );

  const switchSession = useCallback(async (session: PiSessionSummary) => {
    const messages = await tauriApi.switchSession(session.path, session.id);
    dispatch({
      type: "session_switched",
      sessionId: session.id,
      projectId: session.projectId,
      messages,
    });
  }, []);

  const createSession = useCallback(async () => {
    const projectId = state.activeProjectId || state.projects[0]?.id || "proj-verity";
    const session = await tauriApi.newSession(projectId);
    dispatch({ type: "session_created", session, messages: [] });
  }, [state.activeProjectId, state.projects]);

  const renameSession = useCallback(
    async (sessionId: string, name: string) => {
      const updated = await tauriApi.renameSession(sessionId, name);
      if (updated) {
        dispatch({ type: "session_renamed", session: updated });
      }
    },
    []
  );

  const openHistoryTree = useCallback(async () => {
    const nodes = await tauriApi.getHistoryTree();
    dispatch({ type: "set_modal", modal: { kind: "history_tree", nodes } });
  }, []);

  const openForkModal = useCallback(async () => {
    const candidates = await tauriApi.getForkCandidates();
    dispatch({ type: "set_modal", modal: { kind: "fork", candidates } });
  }, []);

  const openProvidersModal = useCallback(() => {
    dispatch({ type: "set_modal", modal: { kind: "providers" } });
  }, []);

  const closeModal = useCallback(() => {
    dispatch({ type: "set_modal", modal: { kind: "none" } });
  }, []);

  const forkAtEntry = useCallback(
    async (entryId: string) => {
      const result = await tauriApi.forkSession(entryId);
      dispatch({
        type: "session_created",
        session: result.session,
        messages: result.messages,
      });
      notify(`Forked session "${result.session.name}"`);
    },
    [notify]
  );

  const cloneCurrentSession = useCallback(async () => {
    const result = await tauriApi.cloneSession();
    dispatch({
      type: "session_created",
      session: result.session,
      messages: result.messages,
    });
    notify(`Cloned session "${result.session.name}"`);
  }, [notify]);

  const exportSessionHtml = useCallback(async () => {
    const exported = await tauriApi.exportSessionHtml();
    if (!tauriApi.isDesktop() && typeof document !== "undefined") {
      const blob = new Blob([exported.html], { type: "text/html;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = `${state.activeSessionId || "pi-session"}.html`;
      link.click();
      URL.revokeObjectURL(url);
    }
    notify(`Exported HTML to ${exported.path}`);
  }, [notify, state.activeSessionId]);

  const sendPrompt = useCallback(async () => {
    const text = state.draft.trim();
    if (!text && state.attachments.length === 0) return;

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

    const attachments = state.attachments;
    const optimisticMsg: PiChatMessage = {
      id: `local-${Date.now()}`,
      role: "user",
      content: text,
      timestamp: new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }),
      attachments: attachments.length > 0 ? attachments : undefined,
    };
    dispatch({ type: "prompt_submitted", message: optimisticMsg });
    await tauriApi.sendPrompt(text, attachments);
  }, [state.draft, state.attachments, openHistoryTree, openForkModal, exportSessionHtml]);

  const steerPrompt = useCallback(async () => {
    const text = state.draft.trim();
    if (!text) return;
    const attachments = state.attachments;
    dispatch({ type: "set_draft", draft: "" });
    await tauriApi.steer(text, attachments);
  }, [state.draft, state.attachments]);

  const followUpPrompt = useCallback(async () => {
    const text = state.draft.trim();
    if (!text) return;
    const attachments = state.attachments;
    dispatch({ type: "set_draft", draft: "" });
    await tauriApi.followUp(text, attachments);
  }, [state.draft, state.attachments]);

  const clearQueue = useCallback(async () => {
    await tauriApi.clearQueue();
  }, []);

  const abortTurn = useCallback(async () => {
    await tauriApi.abort();
  }, []);

  const abortRetry = useCallback(async () => {
    await tauriApi.abortRetry();
  }, []);

  const selectModel = useCallback(async (provider: string, modelId: string) => {
    const model = await tauriApi.setModel(provider, modelId);
    dispatch({ type: "model_selected", model });
  }, []);

  const selectThinkingLevel = useCallback(async (level: ThinkingLevel) => {
    const next = await tauriApi.setThinkingLevel(level);
    dispatch({ type: "thinking_selected", level: next });
  }, []);

  const upsertProvider = useCallback(
    async (input: ProviderProfileInput) => {
      await tauriApi.upsertProvider(input);
      const list = await tauriApi.listProviders();
      dispatch({ type: "providers_updated", providers: list });
      notify(`Saved provider "${input.name}"`);
    },
    [notify]
  );

  const deleteProvider = useCallback(
    async (id: string) => {
      await tauriApi.deleteProvider(id);
      const list = await tauriApi.listProviders();
      dispatch({ type: "providers_updated", providers: list });
      notify(`Deleted provider "${id}"`);
    },
    [notify]
  );

  const respondExtensionDialog = useCallback(
    async (dialogId: string, answer: ExtensionUiAnswer) => {
      dispatch({ type: "shift_extension_dialog", dialogId });
      await tauriApi.respondExtensionUi(dialogId, answer);
    },
    []
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
