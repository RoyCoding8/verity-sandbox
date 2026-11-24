import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ============================================================
// Domain Types (Boundary-Validated)
// ============================================================

export type ProviderApiKind =
  | "openai-completions"
  | "openai-responses"
  | "anthropic-messages";

export interface PublicProviderProfile {
  readonly id: string;
  readonly name: string;
  readonly api: ProviderApiKind;
  readonly base_url: string;
  readonly sign_in_url: string | null;
  readonly models: readonly string[];
  readonly has_api_key: boolean;
}

export interface ProviderProfileInput {
  readonly id: string;
  readonly name: string;
  readonly api: ProviderApiKind;
  readonly base_url: string;
  readonly sign_in_url?: string | null;
  readonly models: readonly string[];
  /** `undefined` preserves existing key, `""` clears it, non-empty replaces it. */
  readonly api_key?: string;
}

export interface PendingAttachment {
  readonly filename: string;
  readonly mime_type: "image/png" | "image/jpeg" | "image/webp" | "image/gif";
  readonly base64_data: string;
  readonly size_bytes: number;
}

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";

export interface ModelOption {
  readonly provider: string;
  readonly modelId: string;
  readonly label: string;
}

export type PiCommandSource = "skill" | "prompt" | "extension" | "builtin" | string;

export interface SlashOrSkillCommand {
  readonly trigger: "/" | "$";
  readonly name: string;
  readonly description: string;
  readonly insertText: string;
  readonly source?: PiCommandSource;
  readonly location?: string;
}

export type PiCommandInfo = SlashOrSkillCommand;

export type PiQueueMode = "all" | "one-at-a-time";

export interface PiSessionStats {
  readonly sessionFile?: string | null;
  readonly sessionId?: string | null;
  readonly userMessages?: number;
  readonly assistantMessages?: number;
  readonly toolCalls?: number;
  readonly toolResults?: number;
  readonly totalMessages?: number;
  readonly tokens?: {
    readonly input?: number;
    readonly output?: number;
    readonly cacheRead?: number;
    readonly cacheWrite?: number;
    readonly total?: number;
  };
  readonly cost?: number;
  readonly contextTokens?: number | null;
  readonly contextWindow?: number | null;
  readonly contextUsagePercent?: number | null;
}

export interface PiBashResult {
  readonly output: string;
  readonly exitCode: number | null;
  readonly cancelled: boolean;
  readonly truncated: boolean;
  readonly fullOutputPath?: string | null;
}

export interface ExtensionToastItem {
  readonly id: string;
  readonly message: string;
  readonly notifyType: "info" | "warning" | "error";
  readonly timestamp: string;
}

export interface ExtensionWidgetItem {
  readonly key: string;
  readonly lines: readonly string[];
}

export interface PiProject {
  readonly id: string;
  readonly name: string;
  readonly path: string;
}

export interface PiSessionSummary {
  readonly id: string;
  readonly projectId: string;
  readonly name: string;
  readonly path: string;
  readonly updatedAt: string;
  readonly messageCount: number;
  readonly parentSessionId?: string;
}

export interface PiToolCallState {
  readonly id: string;
  readonly name: string;
  readonly status: "running" | "done" | "error";
  readonly detail?: string;
  readonly partialOutput?: string;
}

export interface PiChatMessage {
  readonly id: string;
  readonly role: "user" | "assistant" | "system";
  readonly content: string;
  readonly thinking?: string;
  readonly timestamp: string;
  readonly streaming?: boolean;
  readonly attachments?: readonly PendingAttachment[];
  readonly toolCalls?: readonly PiToolCallState[];
  readonly stopReason?: string | null;
  readonly errorMessage?: string | null;
}

export interface PiRetryBanner {
  readonly attempt: number;
  readonly maxAttempts: number;
  readonly delayMs: number;
  readonly errorMessage?: string | null;
}

export interface HistoryTreeNode {
  readonly id: string;
  readonly parentId: string | null;
  readonly role: "user" | "assistant" | "system";
  readonly preview: string;
  readonly timestamp: string;
  readonly depth: number;
  readonly isActiveBranch: boolean;
}

export interface ForkCandidate {
  readonly entryId: string;
  readonly role: "user" | "assistant";
  readonly preview: string;
  readonly timestamp: string;
}

export type DialogKind = "select" | "confirm" | "input" | "editor";

export type ExtensionUiAnswer =
  | { readonly kind: "cancelled" }
  | { readonly kind: "confirmed"; readonly data: boolean }
  | { readonly kind: "value"; readonly data: string };

export type PiUiEvent =
  | { readonly type: "agent_settled"; readonly summary: string | null }
  | { readonly type: "message_start"; readonly message_id: string | null; readonly role: string | null }
  | {
      readonly type: "message_update";
      readonly message_id: string | null;
      readonly delta: string | null;
      readonly thinking_delta?: string | null;
    }
  | {
      readonly type: "message_end";
      readonly message_id: string | null;
      readonly stop_reason: string | null;
      readonly role?: string | null;
      readonly text?: string | null;
      readonly thinking?: string | null;
      readonly error_message?: string | null;
    }
  | { readonly type: "agent_start" }
  | { readonly type: "turn_start" }
  | { readonly type: "turn_end"; readonly summary: string | null }
  | {
      readonly type: "tool_execution_start";
      readonly tool_call_id: string | null;
      readonly tool_name: string | null;
    }
  | {
      readonly type: "tool_execution_update";
      readonly tool_call_id: string | null;
      readonly tool_name: string | null;
      readonly partial_output: string | null;
    }
  | {
      readonly type: "tool_execution_end";
      readonly tool_call_id: string | null;
      readonly tool_name: string | null;
      readonly is_error: boolean;
    }
  | {
      readonly type: "compaction_start";
      readonly reason: string | null;
    }
  | {
      readonly type: "compaction_end";
      readonly aborted: boolean;
      readonly will_retry: boolean;
      readonly error_message: string | null;
      readonly summary: string | null;
    }
  | {
      readonly type: "bash_execution_update";
      readonly chunk: string | null;
      readonly done: boolean;
      readonly exit_code: number | null;
    }
  | {
      readonly type: "session_info_changed";
      readonly session_id: string | null;
      readonly name: string | null;
    }
  | {
      readonly type: "thinking_level_changed";
      readonly level: ThinkingLevel;
    }
  | { readonly type: "queue_update"; readonly pending_count: number | null }
  | {
      readonly type: "auto_retry_start";
      readonly attempt: number | null;
      readonly max_attempts: number | null;
      readonly delay_ms: number | null;
      readonly error_message?: string | null;
    }
  | {
      readonly type: "auto_retry_end";
      readonly attempt: number | null;
      readonly success: boolean | null;
      readonly final_error?: string | null;
    }
  | { readonly type: "error"; readonly message: string }
  | {
      readonly type: "process_exited";
      readonly exit_code: number | null;
      readonly stderr_tail: string | null;
    }
  | {
      readonly type: "extension_ui_request";
      readonly id: string;
      readonly kind: DialogKind;
      readonly title: string | null;
      readonly options: readonly string[];
      readonly timeout_ms: number | null;
    }
  | {
      readonly type: "extension_snackbar";
      readonly id: string;
      readonly message: string;
      readonly tone: "info" | "error";
    }
  | {
      readonly type: "extension_widget";
      readonly id: string;
      readonly title: string;
      readonly body: string;
      readonly dismissed?: boolean;
    }
  | {
      readonly type: "extension_ui_effect";
      readonly method: string;
      readonly message?: string | null;
      readonly notifyType?: string | null;
      readonly notify_type?: string | null;
      readonly statusKey?: string | null;
      readonly status_key?: string | null;
      readonly statusText?: string | null;
      readonly status_text?: string | null;
      readonly widgetKey?: string | null;
      readonly widget_key?: string | null;
      readonly widgetLines?: readonly string[] | null;
      readonly widget_lines?: readonly string[] | null;
      readonly title?: string | null;
      readonly text?: string | null;
    };

// Verity Orchestration Types (matching desktop/src-tauri/src/verity_bridge.rs)
export interface HygieneViolationView {
  readonly rule: string;
  readonly path: string;
  readonly line: number;
  readonly detail: string;
}

export interface VerdictView {
  readonly status: string;
  readonly reason: string;
  readonly checks_passed: number;
  readonly probes_satisfied: number;
}

export interface HandoffView {
  readonly task_id: string;
  readonly base_commit: string;
  readonly verifier: string;
  readonly candidate_commit: string;
  readonly changed_paths: readonly string[];
  readonly extracted_spec: string;
  readonly back_translation: string | null;
  readonly hygiene_clean: boolean;
  readonly hygiene_violations: readonly HygieneViolationView[];
  readonly verdict: VerdictView;
  readonly spec_digest: string;
}

export interface ReadOnlyResponse {
  readonly summary: string;
  readonly tool_calls: number;
  readonly settled: boolean;
}

export interface MergeResponse {
  readonly status: string;
  readonly commit: string | null;
  readonly already_merged: boolean;
  readonly conflicts: readonly string[];
}

export interface SpawnWriteTaskInput {
  readonly repo_path: string;
  readonly task_id: string;
  readonly base_ref: string;
  readonly write_scope: readonly string[];
  readonly forbidden?: readonly string[];
  readonly verifier: string;
  readonly goal: string;
}

export interface PiBootstrapData {
  readonly projects: readonly PiProject[];
  readonly sessions: readonly PiSessionSummary[];
  readonly activeProjectId: string;
  readonly activeProjectPath?: string;
  readonly activeSessionId: string;
  readonly messages: readonly PiChatMessage[];
  readonly models: readonly ModelOption[];
  readonly selectedModel: ModelOption;
  readonly thinkingLevels: readonly ThinkingLevel[];
  readonly thinkingLevel: ThinkingLevel;
  readonly commands: readonly SlashOrSkillCommand[];
  readonly providers: readonly PublicProviderProfile[];
}

export interface ExportHtmlResult {
  readonly path: string;
  readonly html: string;
}

export interface PiCycleModelPayload {
  readonly provider?: string;
  readonly id?: string;
  readonly modelId?: string;
  readonly label?: string;
  readonly name?: string;
  readonly model?: unknown;
  readonly thinkingLevel?: string;
  readonly thinking_level?: string;
}

export interface PiCycleThinkingPayload {
  readonly level?: string;
  readonly thinkingLevel?: string;
  readonly thinking_level?: string;
}

export interface TurnCheckpointView {
  readonly entryId: string;
  readonly promptPreview: string;
  readonly hasCodeSnapshot: boolean;
  readonly createdAt: string;
}

export interface RevertTurnResult {
  readonly session: PiSessionSummary;
  readonly messages: readonly PiChatMessage[];
  readonly restoredPrompt: string | null;
  readonly codeReverted: boolean;
  readonly revertedFiles: readonly string[];
  readonly canRedo: boolean;
  readonly codeRevertNote: string | null;
}

export interface RedoRevertResult {
  readonly restoredFiles: readonly string[];
}

// ============================================================
// Boundary Detection & Validation Helpers
// ============================================================

export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in window &&
    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== undefined
  );
}

function isRecord(val: unknown): val is Record<string, unknown> {
  return typeof val === "object" && val !== null && !Array.isArray(val);
}

function readString(obj: Record<string, unknown>, key: string): string | null {
  const v = obj[key];
  return typeof v === "string" ? v : null;
}

function readNumber(obj: Record<string, unknown>, key: string): number | null {
  const v = obj[key];
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function readBool(obj: Record<string, unknown>, key: string): boolean | null {
  const v = obj[key];
  return typeof v === "boolean" ? v : null;
}

function isDialogKind(raw: string | null): raw is DialogKind {
  return raw === "select" || raw === "confirm" || raw === "input" || raw === "editor";
}

function isThinkingLevel(raw: string | null): raw is ThinkingLevel {
  return (
    raw === "off" ||
    raw === "minimal" ||
    raw === "low" ||
    raw === "medium" ||
    raw === "high" ||
    raw === "xhigh"
  );
}

function readStringArray(obj: Record<string, unknown>, key: string): readonly string[] | null {
  const v = obj[key];
  if (!Array.isArray(v)) return null;
  return v.filter((item): item is string => typeof item === "string");
}

function extractPartialToolOutput(raw: Record<string, unknown>): string | null {
  const direct =
    readString(raw, "partial_output") ??
    readString(raw, "partialOutput") ??
    readString(raw, "output") ??
    readString(raw, "delta") ??
    readString(raw, "text");
  if (direct !== null) return direct;

  const partialResult = raw["partialResult"] ?? raw["partial_result"];
  if (typeof partialResult === "string") return partialResult;
  if (isRecord(partialResult)) {
    const extracted = extractMessageContentParts(partialResult);
    if (extracted.text !== null) return extracted.text;
    const resultText =
      readString(partialResult, "output") ?? readString(partialResult, "text");
    if (resultText !== null) return resultText;
  }

  const rawJson = parseRawJsonRecord(raw);
  if (rawJson) {
    const nested =
      readString(rawJson, "partial_output") ??
      readString(rawJson, "partialOutput") ??
      readString(rawJson, "output") ??
      readString(rawJson, "delta");
    if (nested !== null) return nested;
    const nestedPartial = rawJson["partialResult"] ?? rawJson["partial_result"];
    if (typeof nestedPartial === "string") return nestedPartial;
    if (isRecord(nestedPartial)) {
      const extracted = extractMessageContentParts(nestedPartial);
      if (extracted.text !== null) return extracted.text;
    }
  }
  return null;
}

function parseRawJsonRecord(obj: Record<string, unknown>): Record<string, unknown> | null {
  const rawJson = readString(obj, "raw_json");
  if (!rawJson) return null;
  try {
    const parsed: unknown = JSON.parse(rawJson);
    return isRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function extractMessageContentParts(
  messageObj: Record<string, unknown> | null
): { readonly text: string | null; readonly thinking: string | null } {
  if (!messageObj) return { text: null, thinking: null };
  const rawContent = messageObj["content"];
  if (typeof rawContent === "string") {
    return { text: rawContent || null, thinking: null };
  }
  if (!Array.isArray(rawContent)) {
    return { text: null, thinking: null };
  }
  const textParts: string[] = [];
  const thinkingParts: string[] = [];
  for (const item of rawContent) {
    if (!isRecord(item)) continue;
    const blockType = readString(item, "type");
    if (blockType === "text") {
      const t = readString(item, "text");
      if (t) textParts.push(t);
    } else if (blockType === "thinking") {
      const th = readString(item, "thinking") ?? readString(item, "text");
      if (th) thinkingParts.push(th);
    }
  }
  return {
    text: textParts.length > 0 ? textParts.join("") : null,
    thinking: thinkingParts.length > 0 ? thinkingParts.join("") : null,
  };
}

/**
 * Boundary parser for incoming `pi:event` payloads from Tauri.
 */
export function parsePiUiEventPayload(raw: unknown): PiUiEvent | null {
  if (!isRecord(raw)) return null;
  const type = readString(raw, "type");
  switch (type) {
    case "agent_start":
      return { type: "agent_start" };
    case "turn_start":
      return { type: "turn_start" };
    case "turn_end":
      return { type: "turn_end", summary: readString(raw, "summary") };
    case "agent_settled":
    case "agent_end":
      return { type: "agent_settled", summary: readString(raw, "summary") };
    case "message_start": {
      const directMsg = isRecord(raw["message"]) ? raw["message"] : null;
      let messageId =
        readString(raw, "message_id") ??
        readString(raw, "messageId") ??
        (directMsg
          ? readString(directMsg, "id") ??
            readString(directMsg, "messageId") ??
            readString(directMsg, "message_id")
          : null);
      let role =
        readString(raw, "role") ??
        (directMsg ? readString(directMsg, "role") : null);
      if (!messageId || !role) {
        const rawJson = parseRawJsonRecord(raw);
        const nestedMsg = rawJson && isRecord(rawJson["message"]) ? rawJson["message"] : null;
        if (!messageId) {
          messageId =
            (nestedMsg
              ? readString(nestedMsg, "id") ??
                readString(nestedMsg, "messageId") ??
                readString(nestedMsg, "message_id")
              : null) ??
            (rawJson
              ? readString(rawJson, "messageId") ?? readString(rawJson, "message_id")
              : null);
        }
        if (!role) {
          role =
            (nestedMsg ? readString(nestedMsg, "role") : null) ??
            (rawJson ? readString(rawJson, "role") : null);
        }
      }
      return { type, message_id: messageId, role };
    }
    case "message_update": {
      const directMsg = isRecord(raw["message"]) ? raw["message"] : null;
      let messageId =
        readString(raw, "message_id") ??
        readString(raw, "messageId") ??
        (directMsg
          ? readString(directMsg, "id") ??
            readString(directMsg, "messageId") ??
            readString(directMsg, "message_id")
          : null);
      let delta = readString(raw, "delta");
      let thinkingDelta =
        readString(raw, "thinking_delta") ?? readString(raw, "thinkingDelta");
      if (!delta && !thinkingDelta) {
        const directAssistantEvent = isRecord(raw["assistantMessageEvent"])
          ? raw["assistantMessageEvent"]
          : null;
        const rawJson = directAssistantEvent ? null : parseRawJsonRecord(raw);
        const assistantEvent =
          directAssistantEvent ??
          (rawJson && isRecord(rawJson["assistantMessageEvent"])
            ? rawJson["assistantMessageEvent"]
            : null);
        if (!messageId && rawJson) {
          const nestedMsg = isRecord(rawJson["message"]) ? rawJson["message"] : null;
          messageId =
            (nestedMsg ? readString(nestedMsg, "id") : null) ??
            readString(rawJson, "messageId") ??
            readString(rawJson, "message_id");
        }
        if (assistantEvent) {
          const evType = readString(assistantEvent, "type");
          if (evType === "thinking_delta" || evType === "reasoning_delta") {
            thinkingDelta =
              readString(assistantEvent, "delta") ??
              readString(assistantEvent, "thinking") ??
              readString(assistantEvent, "text");
          } else if (evType === "text_delta") {
            delta =
              readString(assistantEvent, "delta") ??
              readString(assistantEvent, "text");
          }
        }
      }
      return {
        type,
        message_id: messageId,
        delta,
        thinking_delta: thinkingDelta,
      };
    }
    case "message_end": {
      const directMsg = isRecord(raw["message"]) ? raw["message"] : null;
      const directExtracted = extractMessageContentParts(directMsg);
      let messageId =
        readString(raw, "message_id") ??
        readString(raw, "messageId") ??
        (directMsg
          ? readString(directMsg, "id") ??
            readString(directMsg, "messageId") ??
            readString(directMsg, "message_id")
          : null);
      let stopReason =
        readString(raw, "stop_reason") ??
        readString(raw, "stopReason") ??
        (directMsg
          ? readString(directMsg, "stopReason") ?? readString(directMsg, "stop_reason")
          : null);
      let role =
        readString(raw, "role") ??
        (directMsg ? readString(directMsg, "role") : null);
      let text = readString(raw, "text") ?? directExtracted.text;
      let thinking = readString(raw, "thinking") ?? directExtracted.thinking;
      let errorMessage =
        readString(raw, "error_message") ??
        readString(raw, "errorMessage") ??
        (directMsg
          ? readString(directMsg, "errorMessage") ??
            readString(directMsg, "error_message") ??
            readString(directMsg, "error")
          : null);
      if (
        messageId === null ||
        text === null ||
        thinking === null ||
        role === null ||
        errorMessage === null ||
        stopReason === null
      ) {
        const rawJson = parseRawJsonRecord(raw);
        const nestedMsg = rawJson && isRecord(rawJson["message"]) ? rawJson["message"] : null;
        const extracted = extractMessageContentParts(nestedMsg);
        if (messageId === null) {
          messageId =
            (nestedMsg
              ? readString(nestedMsg, "id") ??
                readString(nestedMsg, "messageId") ??
                readString(nestedMsg, "message_id")
              : null) ??
            (rawJson
              ? readString(rawJson, "messageId") ?? readString(rawJson, "message_id")
              : null);
        }
        if (text === null) text = extracted.text;
        if (thinking === null) thinking = extracted.thinking;
        if (role === null) {
          role =
            (nestedMsg ? readString(nestedMsg, "role") : null) ??
            (rawJson ? readString(rawJson, "role") : null);
        }
        if (errorMessage === null) {
          errorMessage =
            (nestedMsg
              ? readString(nestedMsg, "errorMessage") ??
                readString(nestedMsg, "error_message") ??
                readString(nestedMsg, "error")
              : null) ??
            (rawJson
              ? readString(rawJson, "errorMessage") ??
                readString(rawJson, "error_message") ??
                readString(rawJson, "error")
              : null);
        }
        if (stopReason === null) {
          stopReason =
            (nestedMsg
              ? readString(nestedMsg, "stopReason") ?? readString(nestedMsg, "stop_reason")
              : null) ??
            (rawJson
              ? readString(rawJson, "stopReason") ?? readString(rawJson, "stop_reason")
              : null);
        }
      }
      return {
        type,
        message_id: messageId,
        stop_reason: stopReason,
        role,
        text,
        thinking,
        error_message: errorMessage,
      };
    }
    case "tool_execution_start":
      return {
        type,
        tool_call_id:
          readString(raw, "tool_call_id") ??
          readString(raw, "toolCallId") ??
          readString(raw, "id"),
        tool_name:
          readString(raw, "tool_name") ??
          readString(raw, "toolName") ??
          readString(raw, "name"),
      };
    case "tool_execution_update":
      return {
        type,
        tool_call_id:
          readString(raw, "tool_call_id") ??
          readString(raw, "toolCallId") ??
          readString(raw, "id"),
        tool_name:
          readString(raw, "tool_name") ??
          readString(raw, "toolName") ??
          readString(raw, "name"),
        partial_output: extractPartialToolOutput(raw),
      };
    case "tool_execution_end":
      return {
        type,
        tool_call_id:
          readString(raw, "tool_call_id") ??
          readString(raw, "toolCallId") ??
          readString(raw, "id"),
        tool_name:
          readString(raw, "tool_name") ??
          readString(raw, "toolName") ??
          readString(raw, "name"),
        is_error: readBool(raw, "is_error") ?? readBool(raw, "isError") ?? false,
      };
    case "queue_update":
      return {
        type,
        pending_count:
          readNumber(raw, "pending_count") ??
          readNumber(raw, "pendingCount") ??
          readNumber(raw, "count"),
      };
    case "auto_retry_start": {
      let errorMessage =
        readString(raw, "error_message") ??
        readString(raw, "errorMessage") ??
        readString(raw, "error") ??
        readString(raw, "reason");
      if (errorMessage === null) {
        const rawJson = parseRawJsonRecord(raw);
        errorMessage = rawJson
          ? readString(rawJson, "errorMessage") ??
            readString(rawJson, "error_message") ??
            readString(rawJson, "error") ??
            readString(rawJson, "reason")
          : null;
      }
      return {
        type,
        attempt: readNumber(raw, "attempt"),
        max_attempts: readNumber(raw, "max_attempts") ?? readNumber(raw, "maxAttempts"),
        delay_ms: readNumber(raw, "delay_ms") ?? readNumber(raw, "delayMs"),
        error_message: errorMessage,
      };
    }
    case "auto_retry_end": {
      let finalError =
        readString(raw, "final_error") ??
        readString(raw, "finalError") ??
        readString(raw, "errorMessage") ??
        readString(raw, "error_message") ??
        readString(raw, "error");
      if (finalError === null) {
        const rawJson = parseRawJsonRecord(raw);
        finalError = rawJson
          ? readString(rawJson, "finalError") ??
            readString(rawJson, "final_error") ??
            readString(rawJson, "errorMessage") ??
            readString(rawJson, "error_message") ??
            readString(rawJson, "error")
          : null;
      }
      return {
        type,
        attempt: readNumber(raw, "attempt"),
        success: readBool(raw, "success"),
        final_error: finalError,
      };
    }
    case "error":
    case "extension_error": {
      const message =
        readString(raw, "message") ??
        readString(raw, "error") ??
        readString(raw, "errorMessage") ??
        readString(raw, "error_message") ??
        readString(raw, "detail") ??
        readString(raw, "reason");
      return {
        type: "error",
        message: message && message.trim().length > 0 ? message : "Unknown error",
      };
    }
    case "process_exited":
      return {
        type,
        exit_code: readNumber(raw, "exit_code") ?? readNumber(raw, "exitCode"),
        stderr_tail: readString(raw, "stderr_tail") ?? readString(raw, "stderrTail"),
      };
    case "extension_ui_request": {
      const id = readString(raw, "id");
      const kind = readString(raw, "kind");
      if (!id || !isDialogKind(kind)) return null;
      const rawOpts = raw["options"];
      const options = Array.isArray(rawOpts)
        ? rawOpts.filter((item): item is string => typeof item === "string")
        : [];
      return {
        type,
        id,
        kind,
        title: readString(raw, "title"),
        options,
        timeout_ms: readNumber(raw, "timeout_ms"),
      };
    }
    case "extension_snackbar": {
      const id = readString(raw, "id");
      const message = readString(raw, "message");
      if (!id || !message) return null;
      const tone = readString(raw, "tone") === "error" ? "error" : "info";
      return { type, id, message, tone };
    }
    case "extension_widget": {
      const id = readString(raw, "id");
      const title = readString(raw, "title");
      const body = readString(raw, "body");
      if (!id || !title || !body) return null;
      return {
        type,
        id,
        title,
        body,
        dismissed: readBool(raw, "dismissed") ?? false,
      };
    }
    case "compaction_start":
      return {
        type,
        reason: readString(raw, "reason"),
      };
    case "compaction_end":
      return {
        type,
        aborted: readBool(raw, "aborted") ?? false,
        will_retry: readBool(raw, "will_retry") ?? readBool(raw, "willRetry") ?? false,
        error_message:
          readString(raw, "error_message") ?? readString(raw, "errorMessage"),
        summary: readString(raw, "summary"),
      };
    case "bash_execution_update":
      return {
        type,
        chunk:
          readString(raw, "chunk") ??
          readString(raw, "output") ??
          readString(raw, "delta"),
        done: readBool(raw, "done") ?? false,
        exit_code: readNumber(raw, "exit_code") ?? readNumber(raw, "exitCode"),
      };
    case "session_info_changed":
      return {
        type,
        session_id: readString(raw, "session_id") ?? readString(raw, "sessionId"),
        name:
          readString(raw, "name") ??
          readString(raw, "sessionName") ??
          readString(raw, "session_name"),
      };
    case "thinking_level_changed": {
      const rawLevel =
        readString(raw, "level") ??
        readString(raw, "thinkingLevel") ??
        readString(raw, "thinking_level");
      if (!isThinkingLevel(rawLevel)) return null;
      return {
        type,
        level: rawLevel,
      };
    }
    case "extension_ui_effect": {
      const method = readString(raw, "method");
      if (!method) return null;
      const notifyType =
        readString(raw, "notifyType") ?? readString(raw, "notify_type");
      const statusKey =
        readString(raw, "statusKey") ?? readString(raw, "status_key");
      const statusText =
        readString(raw, "statusText") ?? readString(raw, "status_text");
      const widgetKey =
        readString(raw, "widgetKey") ?? readString(raw, "widget_key");
      const widgetLines =
        readStringArray(raw, "widgetLines") ?? readStringArray(raw, "widget_lines");
      return {
        type,
        method,
        message: readString(raw, "message"),
        notifyType,
        notify_type: notifyType,
        statusKey,
        status_key: statusKey,
        statusText,
        status_text: statusText,
        widgetKey,
        widget_key: widgetKey,
        widgetLines,
        widget_lines: widgetLines,
        title: readString(raw, "title"),
        text: readString(raw, "text"),
      };
    }
    default:
      return null;
  }
}

export function validateProviderBaseUrl(url: string): string | null {
  const trimmed = url.trim();
  if (!trimmed) return "Base URL is required.";
  if (/\s/.test(trimmed)) return "Base URL must not contain whitespace.";
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
      return "Base URL must start with https:// or http://.";
    }
    if (parsed.username || parsed.password) {
      return "Base URL must not embed credentials.";
    }
    if (parsed.search || parsed.hash) {
      return "Base URL must not contain query parameters or fragments.";
    }
    return null;
  } catch {
    return "Base URL is not a valid URL.";
  }
}

export function validateExternalSignInUrl(url: string): string | null {
  const trimmed = url.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "https:") {
      return "External sign-in URL must use https://.";
    }
    return null;
  } catch {
    return "External sign-in URL is not a valid URL.";
  }
}

// ============================================================
// Interactive In-Memory Browser Preview Engine
// ============================================================

type EventListener = (event: PiUiEvent) => void;

class BrowserPreviewRuntime {
  private listeners = new Set<EventListener>();
  private timers: ReturnType<typeof setTimeout>[] = [];
  private seq = 100;
  private streamGeneration = 0;
  private streaming = false;
  private retrying = false;
  private pendingQueue: string[] = [];

  private projects: PiProject[] = [
    { id: "proj-verity", name: "verity", path: "/workspace/verity" },
    { id: "proj-pi-ext", name: "pi-extensions", path: "/workspace/pi-extensions" },
  ];

  private sessions: PiSessionSummary[] = [
    {
      id: "sess-1",
      projectId: "proj-verity",
      name: "Kani scope verification",
      path: "/workspace/verity/.pi/sessions/sess-1.jsonl",
      updatedAt: "2m ago",
      messageCount: 2,
    },
    {
      id: "sess-2",
      projectId: "proj-verity",
      name: "MD3 desktop workspace",
      path: "/workspace/verity/.pi/sessions/sess-2.jsonl",
      updatedAt: "18m ago",
      messageCount: 1,
    },
    {
      id: "sess-3",
      projectId: "proj-pi-ext",
      name: "Extension UI bridge",
      path: "/workspace/pi-extensions/.pi/sessions/sess-3.jsonl",
      updatedAt: "1h ago",
      messageCount: 1,
    },
  ];

  private activeProjectId = "proj-verity";
  private activeSessionId = "sess-1";

  private messagesBySession = new Map<string, PiChatMessage[]>([
    [
      "sess-1",
      [
        {
          id: "msg-1",
          role: "user",
          content: "Verify that `check_scope` rejects path traversal outside the write scope.",
          timestamp: "10:12 AM",
        },
        {
          id: "msg-2",
          role: "assistant",
          content: [
            "Checked `crates/verity-core/src/scope.rs` and ran the Kani harness.",
            "",
            "- **Segment boundary check**: `within(path, prefix)` compares path segments rather than raw prefixes.",
            "- **Reachability**: `kani::cover!` probes for both accept and reject paths passed with 0 hygiene violations.",
            "",
            "```rust",
            "pub fn check_scope(changed: &[RelPath], write_scope: &[RelPath], forbidden: &[RelPath]) -> Result<(), ScopeError>",
            "```",
          ].join("\n"),
          timestamp: "10:12 AM",
          toolCalls: [
            { id: "tc-init-1", name: "read", status: "done", detail: "scope.rs" },
            { id: "tc-init-2", name: "bash", status: "done", detail: "cargo kani" },
          ],
        },
      ],
    ],
    [
      "sess-2",
      [
        {
          id: "msg-20",
          role: "user",
          content: "Wire up dynamic Material Design 3 tokens from `#1A5FB4`.",
          timestamp: "9:55 AM",
        },
      ],
    ],
    [
      "sess-3",
      [
        {
          id: "msg-30",
          role: "assistant",
          content: "Ready to test FIFO `select`, `confirm`, `input`, and `editor` extension dialogs.",
          timestamp: "9:10 AM",
        },
      ],
    ],
  ]);

  private models: ModelOption[] = [
    { provider: "anthropic", modelId: "claude-sonnet-5-5", label: "Claude Sonnet 5.5" },
    { provider: "openai", modelId: "gpt-6.1-sol", label: "GPT-6.1 Sol" },
    { provider: "google", modelId: "gemini-2.5-pro", label: "Gemini 2.5 Pro" },
    { provider: "local-vllm", modelId: "qwen-2.5-coder-32b", label: "Qwen 2.5 Coder 32B" },
  ];

  private selectedModel: ModelOption = this.models[0];
  private thinkingLevels: readonly ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh"];
  private thinkingLevel: ThinkingLevel = "medium";
  private autoCompactionEnabled = true;
  private autoRetryEnabled = true;
  private steeringMode: PiQueueMode = "one-at-a-time";
  private followUpMode: PiQueueMode = "one-at-a-time";
  private bashRunning = false;
  private lastRevertedFiles: string[] = [];

  private commands: readonly SlashOrSkillCommand[] = [
    { trigger: "/", name: "/compact", description: "Compact conversation history into a summary", insertText: "/compact ", source: "builtin" },
    { trigger: "/", name: "/fork", description: "Fork the current session from a message", insertText: "/fork", source: "builtin" },
    { trigger: "/", name: "/tree", description: "Open searchable session History Tree", insertText: "/tree", source: "builtin" },
    { trigger: "/", name: "/export", description: "Export current session as standalone HTML", insertText: "/export", source: "builtin" },
    { trigger: "/", name: "/retry-demo", description: "Preview auto-retry countdown banner", insertText: "/retry-demo", source: "extension" },
    { trigger: "/", name: "/ext-demo", description: "Trigger FIFO Extension UI dialog + widget", insertText: "/ext-demo", source: "extension" },
    { trigger: "/", name: "skill:kani-verify", description: "Run Kani formal verification skill", insertText: "/skill:kani-verify ", source: "skill", location: ".pi/skills/kani-verify/SKILL.md" },
    { trigger: "/", name: "prompt:scope-audit", description: "Audit git write-scope and hygiene rules", insertText: "/prompt:scope-audit ", source: "prompt", location: ".pi/prompts/scope-audit.md" },
    { trigger: "$", name: "$kani-verify", description: "Run Kani formal verification skill", insertText: "$kani-verify ", source: "skill" },
    { trigger: "$", name: "$scope-audit", description: "Audit git write-scope and hygiene rules", insertText: "$scope-audit ", source: "skill" },
    { trigger: "$", name: "$spec-review", description: "Generate blind back-translation spec summary", insertText: "$spec-review ", source: "skill" },
  ];

  private providers: PublicProviderProfile[] = [
    {
      id: "anthropic",
      name: "Anthropic",
      api: "anthropic-messages",
      base_url: "https://api.anthropic.com/v1",
      sign_in_url: "https://console.anthropic.com",
      models: ["claude-sonnet-5-5", "claude-opus-5"],
      has_api_key: true,
    },
    {
      id: "local-vllm",
      name: "Local vLLM",
      api: "openai-completions",
      base_url: "http://127.0.0.1:8000/v1",
      sign_in_url: null,
      models: ["qwen-2.5-coder-32b"],
      has_api_key: false,
    },
  ];

  private handoffs: HandoffView[] = [
    {
      task_id: "task-safe-sub",
      base_commit: "464af031234567890abcdef1234567890abcdef1",
      verifier: "kani",
      candidate_commit: "9c8f4b21e0a31234567890abcdef12345678d714",
      changed_paths: ["crates/verity-core/src/scope.rs"],
      extracted_spec: "pub fn check_scope(...) -> Result<(), ScopeError>",
      back_translation: "check_scope accepts only non-empty diffs inside write_scope with zero forbidden matches.",
      hygiene_clean: true,
      hygiene_violations: [],
      verdict: {
        status: "pass",
        reason: "All Kani assertions and cover! probes satisfied",
        checks_passed: 4,
        probes_satisfied: 3,
      },
      spec_digest: "9c8f4b21e0a3d714",
    },
  ];

  subscribe(listener: EventListener): UnlistenFn {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private emit(event: PiUiEvent): void {
    for (const listener of this.listeners) {
      listener(event);
    }
  }

  private schedule(fn: () => void, ms: number): void {
    const id = globalThis.setTimeout(() => {
      this.timers = this.timers.filter((t) => t !== id);
      fn();
    }, ms);
    this.timers.push(id);
  }

  private clearTimers(): void {
    for (const id of this.timers) {
      globalThis.clearTimeout(id);
    }
    this.timers = [];
  }

  private cancelInFlightWork(): void {
    this.streamGeneration += 1;
    this.clearTimers();
    this.streaming = false;
    this.retrying = false;
    this.pendingQueue = [];
  }

  private nowLabel(): string {
    return new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  }

  bootstrap(): PiBootstrapData {
    const activeProject = this.projects.find((p) => p.id === this.activeProjectId);
    return {
      projects: [...this.projects],
      sessions: [...this.sessions],
      activeProjectId: this.activeProjectId,
      activeProjectPath: activeProject?.path,
      activeSessionId: this.activeSessionId,
      messages: [...(this.messagesBySession.get(this.activeSessionId) ?? [])],
      models: [...this.models],
      selectedModel: this.selectedModel,
      thinkingLevels: this.thinkingLevels,
      thinkingLevel: this.thinkingLevel,
      commands: this.commands,
      providers: [...this.providers],
    };
  }

  switchSession(sessionId: string): readonly PiChatMessage[] {
    this.cancelInFlightWork();
    const found = this.sessions.find((s) => s.id === sessionId);
    if (found) {
      this.activeSessionId = found.id;
      this.activeProjectId = found.projectId;
    }
    return [...(this.messagesBySession.get(this.activeSessionId) ?? [])];
  }

  newSession(projectId: string, parentSessionId?: string): PiSessionSummary {
    this.cancelInFlightWork();
    const id = `sess-${++this.seq}`;
    const created: PiSessionSummary = {
      id,
      projectId,
      name: parentSessionId ? `Fork ${this.seq}` : `New session ${this.seq}`,
      path: `/workspace/${projectId}/.pi/sessions/${id}.jsonl`,
      updatedAt: "Just now",
      messageCount: 0,
      parentSessionId,
    };
    this.sessions = [created, ...this.sessions];
    this.messagesBySession.set(id, []);
    this.activeProjectId = projectId;
    this.activeSessionId = id;
    return created;
  }

  renameSession(sessionId: string, name: string): PiSessionSummary | null {
    const trimmed = name.trim();
    if (!trimmed) return null;
    let updated: PiSessionSummary | null = null;
    this.sessions = this.sessions.map((s) => {
      if (s.id === sessionId) {
        updated = { ...s, name: trimmed };
        return updated;
      }
      return s;
    });
    return updated;
  }

  getHistoryTree(): readonly HistoryTreeNode[] {
    const msgs = this.messagesBySession.get(this.activeSessionId) ?? [];
    return msgs.map((m, idx) => ({
      id: m.id,
      parentId: idx === 0 ? null : msgs[idx - 1].id,
      role: m.role,
      preview: m.content.replace(/\s+/g, " ").slice(0, 120),
      timestamp: m.timestamp,
      depth: idx,
      isActiveBranch: true,
    }));
  }

  getForkCandidates(): readonly ForkCandidate[] {
    const msgs = this.messagesBySession.get(this.activeSessionId) ?? [];
    return msgs
      .filter((m): m is PiChatMessage & { role: "user" | "assistant" } => m.role !== "system")
      .map((m) => ({
        entryId: m.id,
        role: m.role,
        preview: m.content.replace(/\s+/g, " ").slice(0, 140),
        timestamp: m.timestamp,
      }));
  }

  forkAt(entryId: string): { session: PiSessionSummary; messages: readonly PiChatMessage[] } {
    const source = this.messagesBySession.get(this.activeSessionId) ?? [];
    const idx = source.findIndex((m) => m.id === entryId);
    const sliced = idx >= 0 ? source.slice(0, idx + 1) : [...source];
    const forked = this.newSession(this.activeProjectId, this.activeSessionId);
    const renamed = this.renameSession(forked.id, `Fork @ ${entryId}`) ?? forked;
    this.messagesBySession.set(renamed.id, sliced);
    this.sessions = this.sessions.map((s) =>
      s.id === renamed.id ? { ...s, messageCount: sliced.length } : s
    );
    return { session: { ...renamed, messageCount: sliced.length }, messages: sliced };
  }

  cloneSession(): { session: PiSessionSummary; messages: readonly PiChatMessage[] } {
    const source = this.messagesBySession.get(this.activeSessionId) ?? [];
    const current = this.sessions.find((s) => s.id === this.activeSessionId);
    const cloned = this.newSession(this.activeProjectId, this.activeSessionId);
    const renamed =
      this.renameSession(cloned.id, `${current?.name ?? "Session"} (clone)`) ?? cloned;
    this.messagesBySession.set(renamed.id, [...source]);
    this.sessions = this.sessions.map((s) =>
      s.id === renamed.id ? { ...s, messageCount: source.length } : s
    );
    return { session: { ...renamed, messageCount: source.length }, messages: [...source] };
  }

  exportHtml(): ExportHtmlResult {
    const current = this.sessions.find((s) => s.id === this.activeSessionId);
    const msgs = this.messagesBySession.get(this.activeSessionId) ?? [];
    const escaped = (s: string) =>
      s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    const body = msgs
      .map(
        (m) =>
          `<article style="margin:16px 0;padding:14px 18px;border-radius:16px;background:${
            m.role === "user" ? "#d6e3ff" : "#f3f3fa"
          }"><strong>${escaped(m.role)}</strong> <small>${escaped(
            m.timestamp
          )}</small><pre style="white-space:pre-wrap;font-family:inherit">${escaped(
            m.content
          )}</pre></article>`
      )
      .join("\n");
    const html = `<!doctype html><html><head><meta charset="utf-8"><title>${escaped(
      current?.name ?? "Pi Session"
    )}</title></head><body style="font-family:system-ui,sans-serif;max-width:820px;margin:32px auto;padding:0 16px"><h1>${escaped(
      current?.name ?? "Pi Session"
    )}</h1>${body}</body></html>`;
    return {
      path: `/tmp/${this.activeSessionId}.html`,
      html,
    };
  }

  setModel(provider: string, modelId: string): ModelOption {
    const found = this.models.find((m) => m.provider === provider && m.modelId === modelId);
    if (found) {
      this.selectedModel = found;
    } else {
      this.selectedModel = { provider, modelId, label: modelId };
    }
    return this.selectedModel;
  }

  setThinkingLevel(level: ThinkingLevel): ThinkingLevel {
    this.thinkingLevel = level;
    return level;
  }

  sendPrompt(message: string, images: readonly PendingAttachment[]): void {
    const trimmed = message.trim();
    const list = this.messagesBySession.get(this.activeSessionId) ?? [];
    const userMsg: PiChatMessage = {
      id: `msg-${++this.seq}`,
      role: "user",
      content: trimmed,
      timestamp: this.nowLabel(),
      attachments: images.length > 0 ? [...images] : undefined,
    };
    this.messagesBySession.set(this.activeSessionId, [...list, userMsg]);
    this.updateSessionCount(this.activeSessionId);

    if (trimmed.startsWith("/ext-demo")) {
      this.emit({
        type: "extension_widget",
        id: "w-verity-status",
        title: "Verity Sandbox",
        body: "Read-only bubblewrap active · 0 scope violations",
      });
      this.emit({
        type: "extension_ui_request",
        id: `dlg-${++this.seq}`,
        kind: "confirm",
        title: "Run Kani verification harness on scope.rs?",
        options: [],
        timeout_ms: 30000,
      });
      this.emit({
        type: "extension_ui_request",
        id: `dlg-${++this.seq}`,
        kind: "select",
        title: "Choose target branch for candidate merge",
        options: ["master", "release/v0.2", "staging"],
        timeout_ms: null,
      });
      this.emit({ type: "agent_settled", summary: null });
      return;
    }

    if (trimmed.startsWith("/retry-demo")) {
      const generation = ++this.streamGeneration;
      this.retrying = true;
      this.emit({
        type: "auto_retry_start",
        attempt: 1,
        max_attempts: 3,
        delay_ms: 4000,
        error_message: "Transient rate limit (preview)",
      });
      this.schedule(() => {
        if (!this.retrying || generation !== this.streamGeneration) return;
        this.retrying = false;
        this.emit({ type: "auto_retry_end", attempt: 1, success: true });
        this.streamAssistantReply(trimmed, images.length);
      }, 3500);
      return;
    }

    this.streamAssistantReply(trimmed, images.length);
  }

  private streamAssistantReply(promptText: string, imageCount: number): void {
    const generation = ++this.streamGeneration;
    const targetSessionId = this.activeSessionId;
    this.streaming = true;
    const msgId = `msg-${++this.seq}`;
    const toolId = `tc-${++this.seq}`;

    const attachmentNote =
      imageCount > 0 ? ` Received **${imageCount}** image attachment${imageCount > 1 ? "s" : ""}.` : "";
    const thinkingChunks =
      this.thinkingLevel === "off"
        ? []
        : [
            `Inspecting request with ${this.selectedModel.label} (thinking: ${this.thinkingLevel}).\n`,
            `Checking active session context and workspace files before drafting a response.`,
          ];
    const chunks = [
      `Working with **${this.selectedModel.label}** (thinking: \`${this.thinkingLevel}\`).${attachmentNote}\n\n`,
      `I inspected the active workspace for:\n> ${promptText.slice(0, 140)}\n\n`,
      `- Verified session state and active branch\n- Prepared concise changes with strict TypeScript boundaries\n\n`,
      "```ts\nexport const status = \"verified\" as const;\n```",
    ];

    this.emit({ type: "message_start", message_id: msgId, role: "assistant" });
    this.emit({ type: "tool_execution_start", tool_call_id: toolId, tool_name: "read" });

    let fullThinking = "";
    thinkingChunks.forEach((chunk, index) => {
      this.schedule(() => {
        if (!this.streaming || generation !== this.streamGeneration) return;
        fullThinking += chunk;
        this.emit({
          type: "message_update",
          message_id: msgId,
          delta: null,
          thinking_delta: chunk,
        });
      }, 140 + index * 180);
    });

    this.schedule(() => {
      if (!this.streaming || generation !== this.streamGeneration) return;
      this.emit({
        type: "tool_execution_end",
        tool_call_id: toolId,
        tool_name: "read",
        is_error: false,
      });
    }, 280);

    const textStartMs = 140 + thinkingChunks.length * 180 + 180;
    let fullText = "";
    chunks.forEach((chunk, index) => {
      this.schedule(() => {
        if (!this.streaming || generation !== this.streamGeneration) return;
        fullText += chunk;
        this.emit({ type: "message_update", message_id: msgId, delta: chunk });
      }, textStartMs + index * 220);
    });

    this.schedule(() => {
      if (!this.streaming || generation !== this.streamGeneration) return;
      this.streaming = false;
      const list = this.messagesBySession.get(targetSessionId) ?? [];
      this.messagesBySession.set(targetSessionId, [
        ...list,
        {
          id: msgId,
          role: "assistant",
          content: fullText,
          thinking: fullThinking || undefined,
          timestamp: this.nowLabel(),
          toolCalls: [{ id: toolId, name: "read", status: "done" }],
        },
      ]);
      this.updateSessionCount(targetSessionId);
      this.emit({
        type: "message_end",
        message_id: msgId,
        stop_reason: "end_turn",
        text: fullText,
        thinking: fullThinking || null,
      });
      this.emit({ type: "agent_settled", summary: "Turn complete" });

      if (this.pendingQueue.length > 0) {
        const next = this.pendingQueue.shift();
        this.emit({ type: "queue_update", pending_count: this.pendingQueue.length });
        if (next) {
          this.streamAssistantReply(next, 0);
        }
      }
    }, textStartMs + chunks.length * 220 + 180);
  }

  steer(message: string): void {
    this.pendingQueue.unshift(message.trim());
    this.emit({ type: "queue_update", pending_count: this.pendingQueue.length });
    this.emit({
      type: "extension_snackbar",
      id: `sb-${++this.seq}`,
      message: "Steering instruction prioritized in queue.",
      tone: "info",
    });
  }

  followUp(message: string): void {
    this.pendingQueue.push(message.trim());
    this.emit({ type: "queue_update", pending_count: this.pendingQueue.length });
  }

  clearQueue(): void {
    this.pendingQueue = [];
    this.emit({ type: "queue_update", pending_count: 0 });
  }

  abort(): void {
    this.cancelInFlightWork();
    this.emit({ type: "queue_update", pending_count: 0 });
    this.emit({ type: "agent_settled", summary: "Aborted by user" });
  }

  abortRetry(): void {
    this.cancelInFlightWork();
    this.emit({
      type: "auto_retry_end",
      attempt: 1,
      success: false,
      final_error: "Retry aborted by user",
    });
    this.emit({ type: "agent_settled", summary: "Retry aborted by user" });
  }

  compactSession(customInstructions?: string): void {
    this.emit({ type: "compaction_start", reason: customInstructions ?? "manual" });
    const suffix = customInstructions?.trim()
      ? ` (${customInstructions.trim()})`
      : "";
    this.emit({
      type: "compaction_end",
      aborted: false,
      will_retry: false,
      error_message: null,
      summary: `Compacted session history${suffix}.`,
    });
    this.emit({
      type: "extension_snackbar",
      id: `sb-${++this.seq}`,
      message: `Compacted session history${suffix}.`,
      tone: "info",
    });
  }

  pickProjectFolder(): string | null {
    return `/workspace/project-${++this.seq}`;
  }

  addProject(path: string): PiBootstrapData {
    const trimmed = path.trim();
    if (trimmed) {
      const existing = this.projects.find((p) => p.path === trimmed);
      if (existing) {
        this.activeProjectId = existing.id;
      } else {
        const name = trimmed.split("/").filter(Boolean).pop() ?? trimmed;
        const id = `proj-${++this.seq}`;
        this.projects = [...this.projects, { id, name, path: trimmed }];
        this.activeProjectId = id;
        const created = this.newSession(id);
        this.activeSessionId = created.id;
      }
    }
    return this.bootstrap();
  }

  removeProject(path: string): PiBootstrapData {
    const trimmed = path.trim();
    const target = this.projects.find((p) => p.path === trimmed || p.id === trimmed);
    if (target) {
      this.projects = this.projects.filter((p) => p.id !== target.id);
      this.sessions = this.sessions.filter((s) => s.projectId !== target.id);
      if (this.activeProjectId === target.id) {
        this.activeProjectId = this.projects[0]?.id ?? "";
        const nextSess = this.sessions.find((s) => s.projectId === this.activeProjectId);
        this.activeSessionId = nextSess?.id ?? "";
      }
    }
    return this.bootstrap();
  }

  switchProject(path: string): PiBootstrapData {
    this.cancelInFlightWork();
    const trimmed = path.trim();
    const target = this.projects.find((p) => p.path === trimmed || p.id === trimmed);
    if (target) {
      this.activeProjectId = target.id;
      const firstSess = this.sessions.find((s) => s.projectId === target.id);
      this.activeSessionId = firstSess?.id ?? "";
    }
    return this.bootstrap();
  }

  runBash(command: string): PiBashResult {
    this.bashRunning = true;
    const output = `$ ${command}\n[preview] executed cleanly`;
    this.emit({ type: "bash_execution_update", chunk: `${output}\n`, done: false, exit_code: null });
    this.bashRunning = false;
    this.emit({ type: "bash_execution_update", chunk: null, done: true, exit_code: 0 });
    return {
      output,
      exitCode: 0,
      cancelled: false,
      truncated: false,
      fullOutputPath: null,
    };
  }

  abortBash(): void {
    if (this.bashRunning) {
      this.bashRunning = false;
      this.emit({ type: "bash_execution_update", chunk: "\n[aborted]\n", done: true, exit_code: 130 });
    }
  }

  getSessionStats(): PiSessionStats {
    const msgs = this.messagesBySession.get(this.activeSessionId) ?? [];
    const userMessages = msgs.filter((m) => m.role === "user").length;
    const assistantMessages = msgs.filter((m) => m.role === "assistant").length;
    const toolCalls = msgs.reduce((sum, m) => sum + (m.toolCalls?.length ?? 0), 0);
    const totalChars = msgs.reduce((sum, m) => sum + m.content.length, 0);
    const approxTokens = Math.max(128, Math.round(totalChars / 4));
    return {
      sessionFile: this.sessions.find((s) => s.id === this.activeSessionId)?.path ?? null,
      sessionId: this.activeSessionId || null,
      userMessages,
      assistantMessages,
      toolCalls,
      toolResults: toolCalls,
      totalMessages: msgs.length,
      tokens: {
        input: approxTokens,
        output: Math.round(approxTokens * 0.6),
        cacheRead: Math.round(approxTokens * 0.25),
        cacheWrite: 0,
        total: Math.round(approxTokens * 1.85),
      },
      cost: Number((approxTokens * 0.000015).toFixed(4)),
      contextTokens: approxTokens,
      contextWindow: 200000,
      contextUsagePercent: Number(((approxTokens / 200000) * 100).toFixed(2)),
    };
  }

  getLastAssistantText(): string | null {
    const msgs = this.messagesBySession.get(this.activeSessionId) ?? [];
    for (let i = msgs.length - 1; i >= 0; i -= 1) {
      if (msgs[i].role === "assistant" && msgs[i].content.trim().length > 0) {
        return msgs[i].content;
      }
    }
    return null;
  }

  setAutoCompaction(enabled: boolean): void {
    this.autoCompactionEnabled = enabled;
  }

  setAutoRetry(enabled: boolean): void {
    this.autoRetryEnabled = enabled;
  }

  setSteeringMode(mode: PiQueueMode): void {
    this.steeringMode = mode;
  }

  setFollowUpMode(mode: PiQueueMode): void {
    this.followUpMode = mode;
  }

  cycleModel(): PiCycleModelPayload | null {
    if (this.models.length === 0) return null;
    const idx = this.models.findIndex(
      (m) => m.provider === this.selectedModel.provider && m.modelId === this.selectedModel.modelId
    );
    const next = this.models[(idx + 1) % this.models.length];
    this.selectedModel = next;
    return { provider: next.provider, id: next.modelId, model: next };
  }

  cycleThinkingLevel(): PiCycleThinkingPayload | null {
    if (this.thinkingLevels.length === 0) return null;
    const idx = this.thinkingLevels.indexOf(this.thinkingLevel);
    const next = this.thinkingLevels[(idx + 1) % this.thinkingLevels.length];
    this.thinkingLevel = next;
    this.emit({ type: "thinking_level_changed", level: next });
    return { level: next };
  }

  listCheckpoints(): TurnCheckpointView[] {
    const msgs = this.messagesBySession.get(this.activeSessionId) ?? [];
    return msgs
      .filter((m) => m.role === "user")
      .map((m) => ({
        entryId: m.id,
        promptPreview: m.content.replace(/\s+/g, " ").slice(0, 120),
        hasCodeSnapshot: true,
        createdAt: m.timestamp,
      }));
  }

  revertTurn(entryId: string, revertCode: boolean): RevertTurnResult {
    this.cancelInFlightWork();
    const source = this.messagesBySession.get(this.activeSessionId) ?? [];
    const idx = source.findIndex((m) => m.id === entryId);
    const restoredPrompt = idx >= 0 ? source[idx].content : null;
    const sliced = idx >= 0 ? source.slice(0, idx) : [...source];
    this.messagesBySession.set(this.activeSessionId, sliced);
    this.updateSessionCount(this.activeSessionId);
    const session: PiSessionSummary =
      this.sessions.find((s) => s.id === this.activeSessionId) ?? {
        id: this.activeSessionId,
        projectId: this.activeProjectId,
        name: "Session",
        path: `/workspace/${this.activeProjectId}/.pi/sessions/${this.activeSessionId}.jsonl`,
        updatedAt: "Just now",
        messageCount: sliced.length,
      };
    if (revertCode) {
      const files = ["crates/verity-core/src/scope.rs"];
      this.lastRevertedFiles = files;
      return {
        session,
        messages: sliced,
        restoredPrompt,
        codeReverted: true,
        revertedFiles: files,
        canRedo: true,
        codeRevertNote: null,
      };
    }
    this.lastRevertedFiles = [];
    return {
      session,
      messages: sliced,
      restoredPrompt,
      codeReverted: false,
      revertedFiles: [],
      canRedo: false,
      codeRevertNote: null,
    };
  }

  redoRevert(): RedoRevertResult {
    const restoredFiles = [...this.lastRevertedFiles];
    this.lastRevertedFiles = [];
    return { restoredFiles };
  }

  getCommands(): PiCommandInfo[] {
    return [...this.commands];
  }

  getSettingsSnapshot(): {
    autoCompactionEnabled: boolean;
    autoRetryEnabled: boolean;
    steeringMode: PiQueueMode;
    followUpMode: PiQueueMode;
  } {
    return {
      autoCompactionEnabled: this.autoCompactionEnabled,
      autoRetryEnabled: this.autoRetryEnabled,
      steeringMode: this.steeringMode,
      followUpMode: this.followUpMode,
    };
  }

  respondExtensionUi(dialogId: string, answer: ExtensionUiAnswer): void {
    const label =
      answer.kind === "cancelled"
        ? "cancelled"
        : answer.kind === "confirmed"
          ? answer.data
            ? "confirmed"
            : "declined"
          : `selected "${answer.data}"`;
    this.emit({
      type: "extension_snackbar",
      id: `sb-${++this.seq}`,
      message: `Extension dialog ${dialogId}: ${label}`,
      tone: "info",
    });
  }

  listProviders(): readonly PublicProviderProfile[] {
    return [...this.providers];
  }

  upsertProvider(input: ProviderProfileInput): PublicProviderProfile {
    const baseErr = validateProviderBaseUrl(input.base_url);
    if (baseErr) throw new Error(baseErr);
    if (input.sign_in_url) {
      const signErr = validateExternalSignInUrl(input.sign_in_url);
      if (signErr) throw new Error(signErr);
    }
    const existing = this.providers.find((p) => p.id === input.id);
    const hasKey =
      input.api_key === undefined
        ? existing?.has_api_key ?? false
        : input.api_key.length > 0;
    const saved: PublicProviderProfile = {
      id: input.id.trim(),
      name: input.name.trim(),
      api: input.api,
      base_url: input.base_url.trim(),
      sign_in_url: input.sign_in_url?.trim() || null,
      models: input.models.map((m) => m.trim()).filter(Boolean),
      has_api_key: hasKey,
    };
    const exists = this.providers.some((p) => p.id === saved.id);
    this.providers = exists
      ? this.providers.map((p) => (p.id === saved.id ? saved : p))
      : [...this.providers, saved].sort((a, b) => a.id.localeCompare(b.id));
    return saved;
  }

  deleteProvider(id: string): boolean {
    const before = this.providers.length;
    this.providers = this.providers.filter((p) => p.id !== id);
    return this.providers.length < before;
  }

  listHandoffs(): readonly HandoffView[] {
    return [...this.handoffs];
  }

  getHandoff(taskId: string): HandoffView {
    const found = this.handoffs.find((h) => h.task_id === taskId);
    if (!found) throw new Error(`Handoff '${taskId}' not found`);
    return found;
  }

  listRunLog(): readonly string[] {
    return [
      `{"task_id":"task-safe-sub","kind":"awaiting_spec_review","detail":"9c8f4b21e0a3d714"}`,
      `{"task_id":"audit-git-hooks","kind":"completed_readonly","detail":"ok"}`,
    ];
  }

  private updateSessionCount(sessionId: string): void {
    const count = (this.messagesBySession.get(sessionId) ?? []).length;
    this.sessions = this.sessions.map((s) =>
      s.id === sessionId ? { ...s, messageCount: count, updatedAt: "Just now" } : s
    );
  }
}

const previewRuntime = new BrowserPreviewRuntime();

// ============================================================
// Typed Tauri IPC Client (with Seamless Browser Preview Mode)
// ============================================================

export const tauriApi = {
  isDesktop: isTauriRuntime,

  async listenPiEvents(handler: (event: PiUiEvent) => void): Promise<UnlistenFn> {
    if (!isTauriRuntime()) {
      return previewRuntime.subscribe(handler);
    }
    return listen<unknown>("pi:event", (ev) => {
      const parsed = parsePiUiEventPayload(ev.payload);
      if (parsed) handler(parsed);
    });
  },

  async bootstrapPiWorkspace(): Promise<PiBootstrapData> {
    if (!isTauriRuntime()) {
      return previewRuntime.bootstrap();
    }
    const [providers, state] = await Promise.all([
      invoke<PublicProviderProfile[]>("list_providers"),
      invoke<PiBootstrapData>("pi_bootstrap"),
    ]);
    return { ...state, providers };
  },

  async switchSession(sessionPath: string, sessionId: string): Promise<readonly PiChatMessage[]> {
    if (!isTauriRuntime()) {
      return previewRuntime.switchSession(sessionId);
    }
    return invoke<PiChatMessage[]>("pi_switch_session", { sessionPath });
  },

  async newSession(projectId: string, parentSession?: string): Promise<PiSessionSummary> {
    if (!isTauriRuntime()) {
      return previewRuntime.newSession(projectId, parentSession);
    }
    return invoke<PiSessionSummary>("pi_new_session", {
      projectId,
      parentSession: parentSession ?? null,
    });
  },

  async renameSession(sessionId: string, name: string): Promise<PiSessionSummary | null> {
    if (!isTauriRuntime()) {
      return previewRuntime.renameSession(sessionId, name);
    }
    return invoke<PiSessionSummary>("pi_set_session_name", { sessionId, name });
  },

  async getHistoryTree(): Promise<readonly HistoryTreeNode[]> {
    if (!isTauriRuntime()) {
      return previewRuntime.getHistoryTree();
    }
    return invoke<HistoryTreeNode[]>("pi_get_tree");
  },

  async getForkCandidates(): Promise<readonly ForkCandidate[]> {
    if (!isTauriRuntime()) {
      return previewRuntime.getForkCandidates();
    }
    return invoke<ForkCandidate[]>("pi_get_fork_messages");
  },

  async forkSession(
    entryId: string
  ): Promise<{ session: PiSessionSummary; messages: readonly PiChatMessage[] }> {
    if (!isTauriRuntime()) {
      return previewRuntime.forkAt(entryId);
    }
    return invoke<{ session: PiSessionSummary; messages: PiChatMessage[] }>("pi_fork", {
      entryId,
    });
  },

  async cloneSession(
    entryId?: string
  ): Promise<{ session: PiSessionSummary; messages: readonly PiChatMessage[] }> {
    if (!isTauriRuntime()) {
      return previewRuntime.cloneSession();
    }
    return invoke<{ session: PiSessionSummary; messages: PiChatMessage[] }>("pi_clone", {
      entryId: entryId ?? null,
    });
  },

  async exportSessionHtml(outputPath?: string): Promise<ExportHtmlResult> {
    if (!isTauriRuntime()) {
      return previewRuntime.exportHtml();
    }
    return invoke<ExportHtmlResult>("pi_export_html", {
      outputPath: outputPath ?? null,
    });
  },

  async setModel(provider: string, modelId: string): Promise<ModelOption> {
    if (!isTauriRuntime()) {
      return previewRuntime.setModel(provider, modelId);
    }
    return invoke<ModelOption>("pi_set_model", { provider, modelId });
  },

  async setThinkingLevel(level: ThinkingLevel): Promise<ThinkingLevel> {
    if (!isTauriRuntime()) {
      return previewRuntime.setThinkingLevel(level);
    }
    return invoke<ThinkingLevel>("pi_set_thinking_level", { level });
  },

  async sendPrompt(message: string, images: readonly PendingAttachment[]): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.sendPrompt(message, images);
      return;
    }
    await invoke<void>("pi_prompt", { message, images });
  },

  async steer(message: string, images: readonly PendingAttachment[]): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.steer(message);
      return;
    }
    await invoke<void>("pi_steer", { message, images });
  },

  async followUp(message: string, images: readonly PendingAttachment[]): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.followUp(message);
      return;
    }
    await invoke<void>("pi_follow_up", { message, images });
  },

  async clearQueue(): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.clearQueue();
      return;
    }
    await invoke<void>("pi_clear_queue");
  },

  async abort(): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.abort();
      return;
    }
    await invoke<void>("pi_abort");
  },

  async abortRetry(): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.abortRetry();
      return;
    }
    await invoke<void>("pi_abort_retry");
  },

  async compactSession(customInstructions?: string): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.compactSession(customInstructions);
      return;
    }
    await invoke<void>("pi_compact", {
      customInstructions: customInstructions ?? null,
    });
  },

  async piCompact(customInstructions?: string): Promise<void> {
    return this.compactSession(customInstructions);
  },

  async piPickProjectFolder(): Promise<string | null> {
    if (!isTauriRuntime()) {
      return previewRuntime.pickProjectFolder();
    }
    return invoke<string | null>("pi_pick_project_folder");
  },

  async piAddProject(path: string): Promise<PiBootstrapData> {
    if (!isTauriRuntime()) {
      return previewRuntime.addProject(path);
    }
    return invoke<PiBootstrapData>("pi_add_project", { path });
  },

  async piRemoveProject(path: string): Promise<PiBootstrapData> {
    if (!isTauriRuntime()) {
      return previewRuntime.removeProject(path);
    }
    return invoke<PiBootstrapData>("pi_remove_project", { path });
  },

  async piSwitchProject(path: string): Promise<PiBootstrapData> {
    if (!isTauriRuntime()) {
      return previewRuntime.switchProject(path);
    }
    return invoke<PiBootstrapData>("pi_switch_project", { path });
  },

  async piBash(command: string): Promise<PiBashResult> {
    if (!isTauriRuntime()) {
      return previewRuntime.runBash(command);
    }
    return invoke<PiBashResult>("pi_bash", { command });
  },

  async piAbortBash(): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.abortBash();
      return;
    }
    await invoke<void>("pi_abort_bash");
  },

  async piGetSessionStats(): Promise<PiSessionStats> {
    if (!isTauriRuntime()) {
      return previewRuntime.getSessionStats();
    }
    return invoke<PiSessionStats>("pi_get_session_stats");
  },

  async piGetLastAssistantText(): Promise<string | null> {
    if (!isTauriRuntime()) {
      return previewRuntime.getLastAssistantText();
    }
    return invoke<string | null>("pi_get_last_assistant_text");
  },

  async piSetAutoCompaction(enabled: boolean): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.setAutoCompaction(enabled);
      return;
    }
    await invoke<void>("pi_set_auto_compaction", { enabled });
  },

  async piSetAutoRetry(enabled: boolean): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.setAutoRetry(enabled);
      return;
    }
    await invoke<void>("pi_set_auto_retry", { enabled });
  },

  async piSetSteeringMode(mode: PiQueueMode): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.setSteeringMode(mode);
      return;
    }
    await invoke<void>("pi_set_steering_mode", { mode });
  },

  async piSetFollowUpMode(mode: PiQueueMode): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.setFollowUpMode(mode);
      return;
    }
    await invoke<void>("pi_set_follow_up_mode", { mode });
  },

  async piCycleModel(): Promise<PiCycleModelPayload | null> {
    if (!isTauriRuntime()) {
      return previewRuntime.cycleModel();
    }
    return invoke<PiCycleModelPayload | null>("pi_cycle_model");
  },

  async piCycleThinkingLevel(): Promise<PiCycleThinkingPayload | null> {
    if (!isTauriRuntime()) {
      return previewRuntime.cycleThinkingLevel();
    }
    return invoke<PiCycleThinkingPayload | null>("pi_cycle_thinking_level");
  },

  async piListCheckpoints(): Promise<TurnCheckpointView[]> {
    if (!isTauriRuntime()) {
      return previewRuntime.listCheckpoints();
    }
    return invoke<TurnCheckpointView[]>("pi_list_checkpoints");
  },

  async piRevertTurn(entryId: string, revertCode: boolean): Promise<RevertTurnResult> {
    if (!isTauriRuntime()) {
      return previewRuntime.revertTurn(entryId, revertCode);
    }
    return invoke<RevertTurnResult>("pi_revert_turn", { entryId, revertCode });
  },

  async piRedoRevert(): Promise<RedoRevertResult> {
    if (!isTauriRuntime()) {
      return previewRuntime.redoRevert();
    }
    return invoke<RedoRevertResult>("pi_redo_revert");
  },

  async piGetCommands(): Promise<PiCommandInfo[]> {
    if (!isTauriRuntime()) {
      return previewRuntime.getCommands();
    }
    const rawList = await invoke<unknown[]>("pi_get_commands");
    if (!Array.isArray(rawList)) return [];
    const normalized: PiCommandInfo[] = [];
    for (const item of rawList) {
      if (!isRecord(item)) continue;
      const name = readString(item, "name");
      if (!name) continue;
      const triggerRaw = readString(item, "trigger");
      const trigger: "/" | "$" = triggerRaw === "$" || name.startsWith("$") ? "$" : "/";
      const description = readString(item, "description") ?? "";
      const insertText =
        readString(item, "insertText") ??
        readString(item, "insert_text") ??
        (name.startsWith("/") || name.startsWith("$") ? name : `${trigger}${name}`);
      const source = readString(item, "source") ?? undefined;
      const location = readString(item, "location") ?? undefined;
      normalized.push({
        trigger,
        name,
        description,
        insertText,
        source,
        location,
      });
    }
    return normalized;
  },

  async openExternalUrl(url: string): Promise<void> {
    if (!isTauriRuntime()) {
      if (typeof window !== "undefined") {
        window.open(url, "_blank", "noopener,noreferrer");
      }
      return;
    }
    await invoke<void>("pi_open_external_url", { url });
  },

  async respondExtensionUi(dialogId: string, answer: ExtensionUiAnswer): Promise<void> {
    if (!isTauriRuntime()) {
      previewRuntime.respondExtensionUi(dialogId, answer);
      return;
    }
    await invoke<void>("pi_extension_ui_response", { dialogId, answer });
  },

  // --- Provider CRUD (exact Rust commands in desktop/src-tauri/src/providers.rs) ---
  async listProviders(): Promise<readonly PublicProviderProfile[]> {
    if (!isTauriRuntime()) {
      return previewRuntime.listProviders();
    }
    return invoke<PublicProviderProfile[]>("list_providers");
  },

  async upsertProvider(input: ProviderProfileInput): Promise<PublicProviderProfile> {
    if (!isTauriRuntime()) {
      return previewRuntime.upsertProvider(input);
    }
    return invoke<PublicProviderProfile>("upsert_provider", { input });
  },

  async deleteProvider(id: string): Promise<boolean> {
    if (!isTauriRuntime()) {
      return previewRuntime.deleteProvider(id);
    }
    return invoke<boolean>("delete_provider", { id });
  },

  async validateProviderSignInUrl(url: string): Promise<string> {
    if (!isTauriRuntime()) {
      const err = validateExternalSignInUrl(url);
      if (err) throw new Error(err);
      return url;
    }
    return invoke<string>("validate_provider_sign_in_url", { url });
  },

  // --- Verity Orchestration Commands (exact Rust commands in verity_bridge.rs) ---
  async listRunLog(repoPath: string): Promise<readonly string[]> {
    if (!isTauriRuntime()) {
      return previewRuntime.listRunLog();
    }
    return invoke<string[]>("list_run_log", { repoPath });
  },

  async runReadonlySubagent(repoPath: string, goal: string): Promise<ReadOnlyResponse> {
    if (!isTauriRuntime()) {
      return {
        summary: `Preview read-only subagent completed goal: ${goal}`,
        tool_calls: 3,
        settled: true,
      };
    }
    return invoke<ReadOnlyResponse>("run_readonly_subagent", { repoPath, goal });
  },

  async spawnWriteSubagent(input: SpawnWriteTaskInput): Promise<HandoffView> {
    if (!isTauriRuntime()) {
      return {
        task_id: input.task_id,
        base_commit: "464af031234567890abcdef1234567890abcdef1",
        verifier: input.verifier,
        candidate_commit: "7777777777777777777777777777777777777777",
        changed_paths: [...input.write_scope],
        extracted_spec: `// Preview spec for ${input.task_id}`,
        back_translation: input.goal,
        hygiene_clean: true,
        hygiene_violations: [],
        verdict: {
          status: "pass",
          reason: "Preview verifier passed",
          checks_passed: 2,
          probes_satisfied: 1,
        },
        spec_digest: "preview-digest-01",
      };
    }
    return invoke<HandoffView>("spawn_write_subagent", { input });
  },

  async listHandoffs(repoPath: string): Promise<readonly HandoffView[]> {
    if (!isTauriRuntime()) {
      return previewRuntime.listHandoffs();
    }
    return invoke<HandoffView[]>("list_handoffs", { repoPath });
  },

  async getHandoff(repoPath: string, taskId: string): Promise<HandoffView> {
    if (!isTauriRuntime()) {
      return previewRuntime.getHandoff(taskId);
    }
    return invoke<HandoffView>("get_handoff", { repoPath, taskId });
  },

  async approveAndMergeHandoff(args: {
    repoPath: string;
    taskId: string;
    targetRef: string;
    approvedDigest: string;
  }): Promise<MergeResponse> {
    if (!isTauriRuntime()) {
      return {
        status: "merged",
        commit: "9c8f4b21e0a3d714",
        already_merged: false,
        conflicts: [],
      };
    }
    return invoke<MergeResponse>("approve_and_merge_handoff", {
      repoPath: args.repoPath,
      taskId: args.taskId,
      targetRef: args.targetRef,
      approvedDigest: args.approvedDigest,
    });
  },
};

export const piCompact = (customInstructions?: string): Promise<void> =>
  tauriApi.compactSession(customInstructions);
export const piPickProjectFolder = (): Promise<string | null> =>
  tauriApi.piPickProjectFolder();
export const piAddProject = (path: string): Promise<PiBootstrapData> =>
  tauriApi.piAddProject(path);
export const piRemoveProject = (path: string): Promise<PiBootstrapData> =>
  tauriApi.piRemoveProject(path);
export const piSwitchProject = (path: string): Promise<PiBootstrapData> =>
  tauriApi.piSwitchProject(path);
export const piBash = (command: string): Promise<PiBashResult> =>
  tauriApi.piBash(command);
export const piAbortBash = (): Promise<void> =>
  tauriApi.piAbortBash();
export const piGetSessionStats = (): Promise<PiSessionStats> =>
  tauriApi.piGetSessionStats();
export const piGetLastAssistantText = (): Promise<string | null> =>
  tauriApi.piGetLastAssistantText();
export const piSetAutoCompaction = (enabled: boolean): Promise<void> =>
  tauriApi.piSetAutoCompaction(enabled);
export const piSetAutoRetry = (enabled: boolean): Promise<void> =>
  tauriApi.piSetAutoRetry(enabled);
export const piSetSteeringMode = (mode: PiQueueMode): Promise<void> =>
  tauriApi.piSetSteeringMode(mode);
export const piSetFollowUpMode = (mode: PiQueueMode): Promise<void> =>
  tauriApi.piSetFollowUpMode(mode);
export const piCycleModel = (): Promise<PiCycleModelPayload | null> =>
  tauriApi.piCycleModel();
export const piCycleThinkingLevel = (): Promise<PiCycleThinkingPayload | null> =>
  tauriApi.piCycleThinkingLevel();
export const piListCheckpoints = (): Promise<TurnCheckpointView[]> =>
  tauriApi.piListCheckpoints();
export const piRevertTurn = (entryId: string, revertCode: boolean): Promise<RevertTurnResult> =>
  tauriApi.piRevertTurn(entryId, revertCode);
export const piRedoRevert = (): Promise<RedoRevertResult> =>
  tauriApi.piRedoRevert();
export const piGetCommands = (): Promise<PiCommandInfo[]> =>
  tauriApi.piGetCommands();
