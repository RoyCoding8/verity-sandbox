#!/usr/bin/env node
/**
 * Executable property-based & trace verification suite for the real
 * TypeScript `tauri.ts` boundary parser and `piStore.ts` reducer.
 *
 * Run via: `node desktop/ui/src/verify/piStore.laws.mjs`
 */
import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiRoot = path.resolve(__dirname, "../..");

function createPrng(seed) {
  let a = seed >>> 0;
  return function next() {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function pick(rand, arr) {
  return arr[Math.floor(rand() * arr.length)];
}

function hasVisibleError(state) {
  if (state.error !== null && state.error.trim().length > 0) return true;
  if (state.extensionSnackbars.some((s) => s.tone === "error" && s.message.trim().length > 0)) {
    return true;
  }
  if (
    state.messages.some(
      (m) => m.stopReason === "error" || Boolean(m.errorMessage && m.errorMessage.trim().length > 0)
    )
  ) {
    return true;
  }
  return false;
}

function countAssistantMessages(state) {
  return state.messages.filter((m) => m.role === "assistant").length;
}

function assertNoDuplicateToolIds(state, context) {
  const seen = new Set();
  for (const msg of state.messages) {
    if (!msg.toolCalls) continue;
    for (const tool of msg.toolCalls) {
      assert.ok(
        !seen.has(tool.id),
        `${context}: duplicate tool ID "${tool.id}" found in message "${msg.id}"`
      );
      seen.add(tool.id);
    }
  }
  for (const tool of state.liveTools) {
    assert.ok(
      !seen.has(tool.id),
      `${context}: duplicate tool ID "${tool.id}" found in liveTools`
    );
    seen.add(tool.id);
  }
}

function isNonProseToolcallRaw(rawPayload) {
  if (!rawPayload || typeof rawPayload !== "object") return false;
  const checkNested = (obj) => {
    if (!obj || typeof obj !== "object") return false;
    if (obj.type === "toolcall_delta" || obj.type === "input_json_delta") return true;
    if (obj.delta && typeof obj.delta === "object") {
      if (obj.delta.type === "toolcall_delta" || obj.delta.type === "input_json_delta") return true;
    }
    return false;
  };
  if (
    checkNested(rawPayload.assistantMessageEvent) ||
    checkNested(rawPayload.assistant_message_event) ||
    checkNested(rawPayload.delta)
  ) {
    return true;
  }
  if (typeof rawPayload.raw_json === "string") {
    try {
      const parsed = JSON.parse(rawPayload.raw_json);
      if (
        checkNested(parsed) ||
        checkNested(parsed?.assistantMessageEvent) ||
        checkNested(parsed?.assistant_message_event) ||
        checkNested(parsed?.delta)
      ) {
        return true;
      }
    } catch {
      // ignore unparseable raw_json
    }
  }
  return false;
}

function assertStepInvariants(prevState, action, nextState, context, rawPayload = null) {
  // Law 4: No duplicate tool IDs ever appear across messages or liveTools.
  assertNoDuplicateToolIds(nextState, context);

  // Law 5: pendingQueueCount === 0 whenever isStreaming === false.
  if (!nextState.isStreaming) {
    assert.equal(
      nextState.pendingQueueCount,
      0,
      `${context}: Law 5 violated (pendingQueueCount=${nextState.pendingQueueCount} while isStreaming=false)`
    );
  }

  // Law 1: turn_aborted ALWAYS yields isStreaming === false, retryBanner === null,
  // pendingQueueCount === 0, and zero messages with streaming === true.
  if (action.type === "turn_aborted") {
    assert.equal(nextState.isStreaming, false, `${context}: Law 1 violated (isStreaming !== false)`);
    assert.equal(nextState.retryBanner, null, `${context}: Law 1 violated (retryBanner !== null)`);
    assert.equal(
      nextState.pendingQueueCount,
      0,
      `${context}: Law 1 violated (pendingQueueCount !== 0)`
    );
    const stillStreaming = nextState.messages.filter((m) => m.streaming === true);
    assert.equal(
      stillStreaming.length,
      0,
      `${context}: Law 1 violated (${stillStreaming.length} messages still streaming)`
    );
  }

  // Law 2: process_exited ALWAYS yields isStreaming === false, connectionState === "disconnected",
  // and hasVisibleError preserved across a trailing agent_settled.
  if (action.type === "pi_event" && action.event.type === "process_exited") {
    assert.equal(nextState.isStreaming, false, `${context}: Law 2 violated (isStreaming !== false)`);
    assert.equal(
      nextState.connectionState,
      "disconnected",
      `${context}: Law 2 violated (connectionState !== "disconnected")`
    );
    assert.equal(
      hasVisibleError(nextState),
      true,
      `${context}: Law 2 violated (hasVisibleError !== true after process_exited)`
    );
  }

  // Law 3: message_start(user) and message_end(user) NEVER create an assistant bubble.
  if (
    action.type === "pi_event" &&
    (action.event.type === "message_start" || action.event.type === "message_end") &&
    action.event.role === "user"
  ) {
    assert.equal(
      countAssistantMessages(nextState),
      countAssistantMessages(prevState),
      `${context}: Law 3 violated (${action.event.type}(user) changed assistant message count)`
    );
  }

  // Law 6: Non-prose toolcall_delta / input_json_delta inside raw_json NEVER leaks into msg.content or msg.thinking.
  if (action.type === "pi_event" && isNonProseToolcallRaw(rawPayload)) {
    assert.equal(
      action.event.delta ?? null,
      null,
      `${context}: Law 6 violated (toolcall_delta leaked into event.delta)`
    );
    assert.equal(
      action.event.thinking_delta ?? null,
      null,
      `${context}: Law 6 violated (toolcall_delta leaked into event.thinking_delta)`
    );
    const prevById = new Map(prevState.messages.map((m) => [m.id, m]));
    for (const msg of nextState.messages) {
      if (msg.role !== "assistant") continue;
      const prev = prevById.get(msg.id);
      assert.equal(
        msg.content,
        prev ? prev.content : "",
        `${context}: Law 6 violated (msg "${msg.id}" content mutated by toolcall_delta)`
      );
      assert.equal(
        msg.thinking ?? "",
        prev ? (prev.thinking ?? "") : "",
        `${context}: Law 6 violated (msg "${msg.id}" thinking mutated by toolcall_delta)`
      );
    }
  }

  // Law 7: No tool.partialOutput or tool.detail begins with '{"content":[' when unwrapped from a Pi content-block wrapper.
  const allTools = [...nextState.liveTools];
  for (const msg of nextState.messages) {
    if (msg.toolCalls) {
      for (const tool of msg.toolCalls) allTools.push(tool);
    }
  }
  for (const tool of allTools) {
    if (typeof tool.partialOutput === "string") {
      assert.equal(
        tool.partialOutput.trimStart().startsWith('{"content":['),
        false,
        `${context}: Law 7 violated (tool "${tool.id}" partialOutput still JSON-wrapped)`
      );
    }
    if (typeof tool.detail === "string") {
      assert.equal(
        tool.detail.trimStart().startsWith('{"content":['),
        false,
        `${context}: Law 7 violated (tool "${tool.id}" detail still JSON-wrapped)`
      );
    }
  }

  // Law 8: Whenever a message has thinkingStartedAtMs and is no longer thinkingStreaming (or !streaming),
  // thinkingDurationMs is a non-negative finite number.
  for (const msg of nextState.messages) {
    if (
      msg.thinkingStartedAtMs !== undefined &&
      (msg.thinkingStreaming === false || msg.streaming !== true)
    ) {
      assert.ok(
        typeof msg.thinkingDurationMs === "number" &&
          Number.isFinite(msg.thinkingDurationMs) &&
          msg.thinkingDurationMs >= 0,
        `${context}: Law 8 violated (msg "${msg.id}" missing valid thinkingDurationMs=${msg.thinkingDurationMs})`
      );
    }
  }
}

async function main() {
  const server = await createServer({
    root: uiRoot,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "error",
  });

  try {
    const tauriMod = await server.ssrLoadModule("/src/api/tauri.ts");
    const storeMod = await server.ssrLoadModule("/src/state/piStore.ts");
    const slashMod = await server.ssrLoadModule("/src/lib/slashCommands.ts");

    const { parsePiUiEventPayload, tauriApi } = tauriMod;
    const { createInitialPiState, piReducer, resolveCycledModel, resolveCycledThinkingLevel, formatChatTranscriptMarkdown } = storeMod;
    const {
      normalizeSlashName,
      displaySlashName,
      BUILTIN_SLASH_COMMANDS,
      mergeSlashCommands,
      filterSlashCommands,
      parseBashPrefix,
      parseBuiltinSlashInvocation,
    } = slashMod;

    function dispatchEvent(state, rawPayload, label) {
      const event = parsePiUiEventPayload(rawPayload);
      assert.ok(event !== null, `${label}: parsePiUiEventPayload returned null`);
      const action = { type: "pi_event", event };
      const next = piReducer(state, action);
      assertStepInvariants(state, action, next, label, rawPayload);
      return next;
    }

    function dispatchAction(state, action, label) {
      const next = piReducer(state, action);
      assertStepInvariants(state, action, next, label);
      return next;
    }

    const scenarios = [];

    // ------------------------------------------------------------
    // Scenario 1: OpenRouter streaming with surrogate-pair emojis,
    // thinking_delta + reasoning_delta -> text_delta -> message_end(stop) -> turn_end -> agent_end
    // ------------------------------------------------------------
    scenarios.push(["OpenRouter streaming + surrogate-pair emojis + turn_end/agent_end", () => {
      let s = createInitialPiState();
      s = dispatchAction(
        s,
        {
          type: "prompt_submitted",
          message: { id: "u-1", role: "user", content: "Say hi with emojis", timestamp: "10:00" },
        },
        "S1.prompt"
      );
      s = dispatchEvent(
        s,
        { type: "message_start", message: { id: "or-msg-1", role: "assistant" } },
        "S1.message_start"
      );
      assert.equal(s.isStreaming, true);

      s = dispatchEvent(
        s,
        {
          type: "message_update",
          message: { id: "or-msg-1" },
          assistantMessageEvent: { type: "thinking_start" },
        },
        "S1.thinking_start"
      );
      assert.equal(s.messages.find((m) => m.id === "or-msg-1")?.thinkingStreaming, true);
      assert.equal(typeof s.messages.find((m) => m.id === "or-msg-1")?.thinkingStartedAtMs, "number");

      s = dispatchEvent(
        s,
        {
          type: "message_update",
          message: { id: "or-msg-1" },
          assistantMessageEvent: { type: "thinking_delta", delta: "Planning 🚀" },
        },
        "S1.thinking_delta"
      );
      s = dispatchEvent(
        s,
        {
          type: "message_update",
          message: { id: "or-msg-1" },
          assistantMessageEvent: { type: "reasoning_delta", delta: " + math ∑" },
        },
        "S1.reasoning_delta"
      );
      s = dispatchEvent(
        s,
        {
          type: "message_update",
          message: { id: "or-msg-1" },
          assistantMessageEvent: { type: "thinking_end", content: "Planning 🚀 + math ∑" },
        },
        "S1.thinking_end"
      );
      const afterThinkingEnd = s.messages.find((m) => m.id === "or-msg-1");
      assert.equal(afterThinkingEnd?.thinkingStreaming, false);
      assert.ok(
        typeof afterThinkingEnd?.thinkingDurationMs === "number" &&
          afterThinkingEnd.thinkingDurationMs >= 0,
        "S1: thinkingDurationMs must be recorded on thinking_end"
      );

      s = dispatchEvent(
        s,
        {
          type: "message_update",
          message: { id: "or-msg-1" },
          assistantMessageEvent: { type: "text_delta", delta: "Hello 🌍✨" },
        },
        "S1.text_delta_1"
      );
      s = dispatchEvent(
        s,
        {
          type: "message_update",
          message: { id: "or-msg-1" },
          assistantMessageEvent: { type: "text_delta", delta: " — OpenRouter works! 🎉" },
        },
        "S1.text_delta_2"
      );
      s = dispatchEvent(
        s,
        {
          type: "message_end",
          message: {
            id: "or-msg-1",
            role: "assistant",
            stopReason: "stop",
            content: [
              { type: "thinking", thinking: "Planning 🚀 + math ∑" },
              { type: "text", text: "Hello 🌍✨ — OpenRouter works! 🎉" },
            ],
          },
        },
        "S1.message_end"
      );
      const msg = s.messages.find((m) => m.id === "or-msg-1");
      assert.ok(msg, "S1: assistant message must exist");
      assert.equal(msg.streaming, false, "S1: message.streaming must be false after message_end(stop)");
      assert.equal(s.isStreaming, false, "S1: isStreaming must be false after message_end(stop)");
      assert.equal(msg.content, "Hello 🌍✨ — OpenRouter works! 🎉");
      assert.equal(msg.thinking, "Planning 🚀 + math ∑");

      s = dispatchEvent(s, { type: "turn_end" }, "S1.turn_end");
      assert.equal(s.isStreaming, false);
      s = dispatchEvent(s, { type: "agent_end" }, "S1.agent_end");
      assert.equal(s.isStreaming, false);
    }]);

    // ------------------------------------------------------------
    // Scenario 2: Non-streaming message_end-only hydration via raw_json
    // ------------------------------------------------------------
    scenarios.push(["Non-streaming message_end-only hydration", () => {
      let s = createInitialPiState();
      s = dispatchAction(
        s,
        {
          type: "prompt_submitted",
          message: { id: "u-2", role: "user", content: "Hydrate me", timestamp: "10:01" },
        },
        "S2.prompt"
      );
      const rawJson = JSON.stringify({
        type: "message_end",
        message: {
          id: "hydrated-1",
          role: "assistant",
          stopReason: "end_turn",
          content: [
            { type: "thinking", thinking: "Computed offline." },
            { type: "text", text: "Hydrated answer 🧠" },
          ],
        },
      });
      s = dispatchEvent(s, { type: "message_end", raw_json: rawJson }, "S2.message_end");
      const msg = s.messages.find((m) => m.id === "hydrated-1");
      assert.ok(msg, "S2: hydrated message must be created");
      assert.equal(msg.content, "Hydrated answer 🧠");
      assert.equal(msg.thinking, "Computed offline.");
      assert.equal(msg.streaming, false);
      assert.equal(msg.stopReason, "end_turn");
      assert.equal(s.isStreaming, false);
    }]);

    // ------------------------------------------------------------
    // Scenario 3: Terminal vs non-terminal stop_reason handling
    // ------------------------------------------------------------
    scenarios.push(["Terminal stop_reasons settle isStreaming; toolUse keeps streaming", () => {
      for (const stopReason of ["stop", "end_turn", "length", "error", "aborted"]) {
        let s = createInitialPiState();
        s = dispatchAction(
          s,
          {
            type: "prompt_submitted",
            message: { id: `u-${stopReason}`, role: "user", content: "test", timestamp: "10:02" },
          },
          `S3.${stopReason}.prompt`
        );
        s = dispatchEvent(s, { type: "message_start", message_id: `a-${stopReason}`, role: "assistant" }, `S3.${stopReason}.start`);
        s = dispatchEvent(s, { type: "queue_update", pending_count: 2 }, `S3.${stopReason}.queue`);
        s = dispatchEvent(
          s,
          {
            type: "message_end",
            message_id: `a-${stopReason}`,
            role: "assistant",
            stop_reason: stopReason,
            text: stopReason === "error" ? null : "done",
            error_message: stopReason === "error" ? "boom" : null,
          },
          `S3.${stopReason}.end`
        );
        assert.equal(s.isStreaming, false, `S3: ${stopReason} must settle isStreaming`);
        assert.equal(s.pendingQueueCount, 0, `S3: ${stopReason} must clear pendingQueueCount`);
        assert.equal(s.messages.some((m) => m.streaming), false, `S3: ${stopReason} must clear message.streaming`);
      }

      // Non-terminal toolUse keeps isStreaming true until final turn finishes.
      let s = createInitialPiState();
      s = dispatchEvent(s, { type: "message_start", message_id: "a-tool", role: "assistant" }, "S3.toolUse.start");
      s = dispatchEvent(
        s,
        { type: "message_end", message_id: "a-tool", role: "assistant", stop_reason: "toolUse", text: "Calling tool..." },
        "S3.toolUse.end"
      );
      assert.equal(s.isStreaming, true, "S3: toolUse must keep isStreaming true");
      assert.equal(s.messages.find((m) => m.id === "a-tool")?.streaming, false);
    }]);

    // ------------------------------------------------------------
    // Scenario 4: Multi-step tool calls + toolResult messages
    // ------------------------------------------------------------
    scenarios.push(["Multi-step tool calls + toolResult role ignored", () => {
      let s = createInitialPiState();
      s = dispatchAction(
        s,
        {
          type: "prompt_submitted",
          message: { id: "u-4", role: "user", content: "Inspect repo", timestamp: "10:03" },
        },
        "S4.prompt"
      );
      s = dispatchEvent(s, { type: "message_start", message_id: "m-step1", role: "assistant" }, "S4.m1.start");
      s = dispatchEvent(s, { type: "message_update", message_id: "m-step1", delta: "Reading file..." }, "S4.m1.update");
      s = dispatchEvent(s, { type: "message_end", message_id: "m-step1", role: "assistant", stop_reason: "toolUse" }, "S4.m1.end");
      s = dispatchEvent(s, { type: "tool_execution_start", toolCallId: "tc-1", toolName: "read" }, "S4.tc1.start");
      s = dispatchEvent(s, { type: "tool_execution_end", toolCallId: "tc-1", toolName: "read", isError: false }, "S4.tc1.end");
      const beforeToolResult = countAssistantMessages(s);
      s = dispatchEvent(s, { type: "message_start", message: { id: "tr-1", role: "toolResult" } }, "S4.tr1.start");
      s = dispatchEvent(
        s,
        { type: "message_end", message: { id: "tr-1", role: "toolResult", content: [{ type: "text", text: "raw tool output" }] } },
        "S4.tr1.end"
      );
      assert.equal(countAssistantMessages(s), beforeToolResult, "S4: toolResult must not create assistant bubble");

      s = dispatchEvent(s, { type: "message_start", message_id: "m-step2", role: "assistant" }, "S4.m2.start");
      s = dispatchEvent(s, { type: "tool_execution_start", toolCallId: "tc-2", toolName: "bash" }, "S4.tc2.start");
      s = dispatchEvent(s, { type: "tool_execution_end", toolCallId: "tc-2", toolName: "bash", isError: true }, "S4.tc2.end");
      s = dispatchEvent(s, { type: "message_update", message_id: "m-step2", delta: "All done." }, "S4.m2.update");
      s = dispatchEvent(s, { type: "message_end", message_id: "m-step2", role: "assistant", stop_reason: "stop" }, "S4.m2.end");
      s = dispatchEvent(s, { type: "agent_settled", summary: "done" }, "S4.settled");

      assert.equal(s.isStreaming, false);
      assert.equal(s.liveTools.length, 0);
      assertNoDuplicateToolIds(s, "S4.final");
    }]);

    // ------------------------------------------------------------
    // Scenario 5: Provider HTTP 500 message_end(error) + agent_settled
    // ------------------------------------------------------------
    scenarios.push(["Provider HTTP 500 message_end(error) + agent_settled", () => {
      let s = createInitialPiState();
      s = dispatchAction(
        s,
        {
          type: "prompt_submitted",
          message: { id: "u-5", role: "user", content: "Trigger 500", timestamp: "10:04" },
        },
        "S5.prompt"
      );
      s = dispatchEvent(s, { type: "message_start", message_id: "m-500", role: "assistant" }, "S5.start");
      s = dispatchEvent(
        s,
        {
          type: "message_end",
          message: {
            id: "m-500",
            role: "assistant",
            stopReason: "error",
            errorMessage: "OpenRouter HTTP 500: Internal Server Error",
          },
        },
        "S5.end"
      );
      assert.equal(s.isStreaming, false);
      s = dispatchEvent(s, { type: "agent_settled", summary: null }, "S5.settled");
      const errMsg = s.messages.find((m) => m.id === "m-500");
      assert.ok(errMsg, "S5: error assistant message must survive agent_settled");
      assert.equal(errMsg.stopReason, "error");
      assert.equal(errMsg.errorMessage, "OpenRouter HTTP 500: Internal Server Error");
      assert.equal(hasVisibleError(s), true);
    }]);

    // ------------------------------------------------------------
    // Scenario 6: process_exited + trailing agent_settled
    // ------------------------------------------------------------
    scenarios.push(["process_exited + trailing agent_settled preserves error", () => {
      let s = createInitialPiState();
      s = dispatchAction(
        s,
        {
          type: "prompt_submitted",
          message: { id: "u-6", role: "user", content: "Run", timestamp: "10:05" },
        },
        "S6.prompt"
      );
      s = dispatchEvent(s, { type: "tool_execution_start", tool_call_id: "tc-crash", tool_name: "bash" }, "S6.tool");
      s = dispatchEvent(
        s,
        { type: "process_exited", exit_code: 137, stderr_tail: "OOMKilled" },
        "S6.process_exited"
      );
      assert.equal(s.isStreaming, false);
      assert.equal(s.connectionState, "disconnected");
      assert.equal(hasVisibleError(s), true);

      s = dispatchEvent(s, { type: "agent_settled", summary: "exited" }, "S6.trailing_settled");
      assert.equal(s.isStreaming, false);
      assert.equal(s.connectionState, "disconnected");
      assert.equal(hasVisibleError(s), true);
    }]);

    // ------------------------------------------------------------
    // Scenario 7: auto_retry_start -> auto_retry_end(false)
    // ------------------------------------------------------------
    scenarios.push(["auto_retry_start -> auto_retry_end(false)", () => {
      let s = createInitialPiState();
      s = dispatchAction(
        s,
        {
          type: "prompt_submitted",
          message: { id: "u-7", role: "user", content: "Retry me", timestamp: "10:06" },
        },
        "S7.prompt"
      );
      s = dispatchEvent(s, { type: "queue_update", pending_count: 2 }, "S7.queue");
      s = dispatchEvent(
        s,
        {
          type: "auto_retry_start",
          attempt: 1,
          maxAttempts: 3,
          delayMs: 2500,
          errorMessage: "429 Too Many Requests",
        },
        "S7.retry_start"
      );
      assert.ok(s.retryBanner !== null);
      s = dispatchEvent(
        s,
        {
          type: "auto_retry_end",
          attempt: 3,
          success: false,
          finalError: "429 exhausted after 3 attempts",
        },
        "S7.retry_end_false"
      );
      assert.equal(s.isStreaming, false);
      assert.equal(s.retryBanner, null);
      assert.equal(s.pendingQueueCount, 0);
      assert.equal(hasVisibleError(s), true);
      assert.equal(s.messages.some((m) => m.streaming), false);
    }]);

    // ------------------------------------------------------------
    // Scenario 8: turn_aborted (UserAbort)
    // ------------------------------------------------------------
    scenarios.push(["turn_aborted clears streaming, retryBanner, queue, and liveTools", () => {
      let s = createInitialPiState();
      s = dispatchAction(
        s,
        {
          type: "prompt_submitted",
          message: { id: "u-8", role: "user", content: "Abort me", timestamp: "10:07" },
        },
        "S8.prompt"
      );
      s = dispatchEvent(s, { type: "message_start", message_id: "m-8", role: "assistant" }, "S8.start");
      s = dispatchEvent(s, { type: "tool_execution_start", tool_call_id: "tc-8", tool_name: "bash" }, "S8.tool");
      s = dispatchEvent(s, { type: "queue_update", pending_count: 3 }, "S8.queue");
      s = dispatchEvent(s, { type: "auto_retry_start", attempt: 2, max_attempts: 3, delay_ms: 1000 }, "S8.retry");
      s = dispatchAction(s, { type: "turn_aborted" }, "S8.abort");
      assert.equal(s.isStreaming, false);
      assert.equal(s.retryBanner, null);
      assert.equal(s.pendingQueueCount, 0);
      assert.equal(s.liveTools.length, 0);
      assert.equal(s.messages.some((m) => m.streaming), false);
    }]);

    // ------------------------------------------------------------
    // Scenario 9: listenPiEvents ACL rejection + bootstrap_loaded race
    // ------------------------------------------------------------
    scenarios.push(["listenPiEvents ACL rejection sets error, snackbar, disconnected, and settles streaming", async () => {
      const prevWindow = globalThis.window;
      try {
        globalThis.window = {
          __TAURI_INTERNALS__: {
            invoke: async (cmd) => {
              throw new Error(`ACL denied command: ${cmd}`);
            },
            transformCallback: () => 1,
            unregisterCallback: () => {},
          },
        };
        let rejectedErr = null;
        try {
          await tauriApi.listenPiEvents(() => {});
        } catch (err) {
          rejectedErr = err;
        }
        assert.ok(rejectedErr !== null, "S9: listenPiEvents must reject on Tauri ACL denial");
        const errMsg = rejectedErr instanceof Error ? rejectedErr.message : String(rejectedErr);

        // Simulate active streaming state when ACL failure hits.
        let s = createInitialPiState();
        s = dispatchAction(
          s,
          {
            type: "prompt_submitted",
            message: { id: "u-9", role: "user", content: "Hello", timestamp: "10:08" },
          },
          "S9.prompt"
        );
        s = dispatchEvent(s, { type: "queue_update", pending_count: 1 }, "S9.queue");
        s = dispatchAction(s, { type: "event_stream_failed", error: errMsg }, "S9.acl_failed");
        assert.ok(s.error && s.error.includes("Event stream unavailable"), "S9: state.error must be set");
        assert.equal(s.connectionState, "disconnected", "S9: connectionState must be disconnected");
        assert.equal(s.isStreaming, false, "S9: isStreaming must be settled false");
        assert.equal(s.pendingQueueCount, 0, "S9: pendingQueueCount must be 0");
        assert.ok(
          s.extensionSnackbars.some((sb) => sb.tone === "error" && sb.message.includes("Event stream unavailable")),
          "S9: error snackbar must be pushed"
        );

        // Race: bootstrap_loaded resolving AFTER event_stream_failed must NOT clear the ACL error.
        let raceState = createInitialPiState();
        raceState = dispatchAction(raceState, { type: "bootstrap_start" }, "S9.race.start");
        raceState = dispatchAction(raceState, { type: "event_stream_failed", error: errMsg }, "S9.race.acl");
        raceState = dispatchAction(
          raceState,
          {
            type: "bootstrap_loaded",
            data: {
              projects: [],
              sessions: [],
              activeProjectId: "p1",
              activeSessionId: "s1",
              messages: [],
              models: [{ provider: "openrouter", modelId: "openai/gpt-4o", label: "GPT-4o" }],
              selectedModel: { provider: "openrouter", modelId: "openai/gpt-4o", label: "GPT-4o" },
              thinkingLevels: ["off", "medium"],
              thinkingLevel: "medium",
              commands: [],
              providers: [],
            },
          },
          "S9.race.loaded"
        );
        assert.equal(raceState.connectionState, "disconnected", "S9: race must keep disconnected");
        assert.ok(raceState.error !== null, "S9: race must preserve ACL error");
      } finally {
        if (prevWindow === undefined) {
          delete globalThis.window;
        } else {
          globalThis.window = prevWindow;
        }
      }
    }]);

    // ------------------------------------------------------------
    // Scenario 10: session_switched & session_created block stale events from prior turn
    // ------------------------------------------------------------
    scenarios.push(["session_switched / session_created isolate stale events from prior turn", () => {
      let s = createInitialPiState();
      s = dispatchAction(
        s,
        {
          type: "prompt_submitted",
          message: { id: "u-a", role: "user", content: "Session A prompt", timestamp: "10:09" },
        },
        "S10.promptA"
      );
      s = dispatchEvent(s, { type: "message_start", message_id: "m-a", role: "assistant" }, "S10.startA");

      const sessionBMessages = [
        { id: "b-u1", role: "user", content: "Old B question", timestamp: "09:00" },
        { id: "b-a1", role: "assistant", content: "Old B answer", timestamp: "09:01" },
      ];
      s = dispatchAction(
        s,
        {
          type: "session_switched",
          sessionId: "sess-b",
          projectId: "proj-1",
          messages: sessionBMessages,
        },
        "S10.switchB"
      );
      // Stale events from Session A arrive after switching to Session B.
      s = dispatchEvent(s, { type: "message_update", message_id: "m-a", delta: "STALE A" }, "S10.staleUpdate1");
      s = dispatchEvent(s, { type: "message_update", message_id: null, delta: "STALE NULL" }, "S10.staleUpdate2");
      s = dispatchEvent(s, { type: "message_end", message_id: "m-a", stop_reason: "stop", text: "STALE END" }, "S10.staleEnd");
      s = dispatchEvent(s, { type: "tool_execution_start", tool_call_id: "stale-tc", tool_name: "bash" }, "S10.staleTool");
      assert.deepEqual(s.messages, sessionBMessages, "S10: Session B messages must not be corrupted by stale events");
      assert.equal(s.isStreaming, false);
      assert.equal(s.liveTools.length, 0);

      // Creating a new session also isolates stale events.
      s = dispatchAction(
        s,
        {
          type: "session_created",
          session: {
            id: "sess-c",
            projectId: "proj-1",
            name: "New C",
            path: "/tmp/c.jsonl",
            updatedAt: "Just now",
            messageCount: 0,
          },
          messages: [],
        },
        "S10.createC"
      );
      s = dispatchEvent(s, { type: "message_update", message_id: "m-a", delta: "STALE C" }, "S10.staleC");
      assert.equal(s.messages.length, 0, "S10: new session must ignore stale events");

      // Submitting a prompt in Session C re-arms turn events.
      s = dispatchAction(
        s,
        {
          type: "prompt_submitted",
          message: { id: "u-c", role: "user", content: "Fresh C prompt", timestamp: "10:10" },
        },
        "S10.promptC"
      );
      s = dispatchEvent(s, { type: "message_start", message_id: "m-c", role: "assistant" }, "S10.startC");
      s = dispatchEvent(s, { type: "message_update", message_id: "m-c", delta: "Fresh reply!" }, "S10.updateC");
      s = dispatchEvent(s, { type: "message_end", message_id: "m-c", stop_reason: "stop" }, "S10.endC");
      assert.equal(s.messages.find((m) => m.id === "m-c")?.content, "Fresh reply!");
      assert.equal(s.isStreaming, false);
    }]);

    // Run all deterministic scenarios
    // Scenario 11: system/developer message_start never creates an empty System bubble
    scenarios.push([
      "system/developer message_start & message_end never leave empty System bubbles",
      () => {
        let s = createInitialPiState();
        s = dispatchAction(
          s,
          {
            type: "prompt_submitted",
            message: { id: "u-sys-1", role: "user", content: "Hey", timestamp: "11:16 PM" },
          },
          "S11.prompt"
        );
        s = dispatchEvent(
          s,
          { type: "message_start", message: { id: "sys-1", role: "system" } },
          "S11.system_start"
        );
        s = dispatchEvent(
          s,
          { type: "message_start", message: { id: "asst-1", role: "assistant" } },
          "S11.assistant_start"
        );
        s = dispatchEvent(
          s,
          {
            type: "message_update",
            message: { id: "asst-1" },
            assistantMessageEvent: { type: "thinking_delta", delta: "Greeting user." },
          },
          "S11.thinking_delta"
        );
        s = dispatchEvent(
          s,
          {
            type: "message_update",
            message: { id: "asst-1" },
            assistantMessageEvent: { type: "text_delta", delta: "Hello!" },
          },
          "S11.text_delta"
        );
        s = dispatchEvent(
          s,
          {
            type: "message_end",
            message: { id: "asst-1", role: "assistant", stopReason: "stop" },
          },
          "S11.message_end"
        );
        s = dispatchEvent(s, { type: "agent_end" }, "S11.agent_end");
        assert.equal(
          s.messages.some((m) => m.role === "system"),
          false,
          "system message_start must not create an empty system bubble"
        );
        assert.equal(s.messages.length, 2);
        assert.equal(s.messages[1].content, "Hello!");
        assert.equal(s.isStreaming, false);
      },
    ]);

    // ------------------------------------------------------------
    // Scenario 12: Slash command '/' prefix normalization, merging & filtering
    // ------------------------------------------------------------
    scenarios.push([
      "Slash command '/' prefix normalization, merging, and filtering",
      () => {
        assert.equal(normalizeSlashName("/skill:foo"), "skill:foo");
        assert.equal(normalizeSlashName("  ///compact  "), "compact");
        assert.equal(displaySlashName("skill:foo"), "/skill:foo");
        assert.equal(displaySlashName("/compact"), "/compact");
        assert.ok(BUILTIN_SLASH_COMMANDS.length >= 11);

        const merged = mergeSlashCommands([
          {
            trigger: "/",
            name: "skill:foo",
            description: "Run foo skill",
            insertText: "/skill:foo ",
            source: "skill",
            location: ".pi/skills/foo/SKILL.md",
          },
          {
            trigger: "/",
            name: "/prompt:audit",
            description: "Audit codebase",
            insertText: "/prompt:audit ",
            source: "prompt",
            location: ".pi/prompts/audit.md",
          },
          {
            trigger: "/",
            name: "compact",
            description: "Compact conversation context with optional focus instructions",
            insertText: "/compact ",
            source: "builtin",
          },
        ]);

        // Deduplicated by normalized lowercase name
        assert.equal(
          merged.filter((c) => c.normalizedName === "compact").length,
          1,
          "duplicate /compact must be deduplicated"
        );

        // Fix for '/' prefix bug: "/ski" must match "skill:foo" ranked first (prefix match beats description match on "/settings")
        const withSlash = filterSlashCommands(merged, "/ski");
        assert.ok(withSlash.length >= 1);
        assert.equal(withSlash[0].normalizedName, "skill:foo");
        assert.equal(withSlash[0].name, "/skill:foo");
        assert.equal(withSlash[0].source, "skill");
        assert.equal(withSlash[0].location, ".pi/skills/foo/SKILL.md");

        // Works without leading '/' as well
        const withoutSlash = filterSlashCommands(merged, "ski");
        assert.ok(withoutSlash.length >= 1);
        assert.equal(withoutSlash[0].normalizedName, "skill:foo");

        // Builtin slash invocation parser
        assert.deepEqual(parseBuiltinSlashInvocation("/compact focus on scope.rs"), {
          command: "compact",
          args: "focus on scope.rs",
        });
        assert.deepEqual(parseBuiltinSlashInvocation("/name My Session"), {
          command: "name",
          args: "My Session",
        });
        assert.deepEqual(parseBuiltinSlashInvocation("/session"), {
          command: "session",
          args: "",
        });
        // Non-builtin slash commands pass through (return null)
        assert.equal(parseBuiltinSlashInvocation("/skill:foo arg1"), null);
        assert.equal(parseBuiltinSlashInvocation("/prompt:audit"), null);
      },
    ]);

    // ------------------------------------------------------------
    // Scenario 13: !cmd and !!cmd bash prefix parsing
    // ------------------------------------------------------------
    scenarios.push([
      "!cmd and !!cmd bash prefix parsing",
      () => {
        assert.deepEqual(parseBashPrefix("!git status"), {
          kind: "bash",
          command: "git status",
          excludeFromContext: false,
        });
        assert.deepEqual(parseBashPrefix("!!cargo test --workspace"), {
          kind: "bash",
          command: "cargo test --workspace",
          excludeFromContext: true,
        });
        assert.deepEqual(parseBashPrefix("  !  ls -la  "), {
          kind: "bash",
          command: "ls -la",
          excludeFromContext: false,
        });
        assert.equal(parseBashPrefix("!"), null);
        assert.equal(parseBashPrefix("!!   "), null);
        assert.equal(parseBashPrefix("hello !world"), null);
      },
    ]);

    // ------------------------------------------------------------
    // Scenario 14: compaction_start / compaction_end state transitions
    // ------------------------------------------------------------
    scenarios.push([
      "compaction_start and compaction_end state transitions",
      () => {
        let s = createInitialPiState();
        assert.equal(s.isCompacting, false);
        s = dispatchEvent(s, { type: "compaction_start", reason: "threshold" }, "S14.start");
        assert.equal(s.isCompacting, true);
        s = dispatchEvent(
          s,
          { type: "compaction_end", aborted: false, will_retry: false, summary: "Compacted 12 messages" },
          "S14.end_ok"
        );
        assert.equal(s.isCompacting, false);
        assert.equal(s.extensionToasts.length, 0);

        // Compaction error pushes error toast + snackbar and clears isCompacting
        s = dispatchEvent(s, { type: "compaction_start", reason: "manual" }, "S14.start2");
        assert.equal(s.isCompacting, true);
        s = dispatchEvent(
          s,
          { type: "compaction_end", aborted: true, willRetry: false, errorMessage: "Context limit exceeded" },
          "S14.end_err"
        );
        assert.equal(s.isCompacting, false);
        assert.equal(s.extensionToasts.length, 1);
        assert.equal(s.extensionToasts[0].notifyType, "error");
        assert.equal(s.extensionToasts[0].message, "Context limit exceeded");
      },
    ]);

    // ------------------------------------------------------------
    // Scenario 15: tool_execution_update streaming partial output updates
    // ------------------------------------------------------------
    scenarios.push([
      "tool_execution_update streams partial output into liveTools",
      () => {
        let s = createInitialPiState();
        s = dispatchEvent(
          s,
          { type: "tool_execution_start", tool_call_id: "tc-stream-1", tool_name: "bash" },
          "S15.start"
        );
        s = dispatchEvent(
          s,
          {
            type: "tool_execution_update",
            toolCallId: "tc-stream-1",
            toolName: "bash",
            partialResult: { content: [{ type: "text", text: "running test 1/3..." }] },
          },
          "S15.update1"
        );
        assert.equal(s.liveTools.length, 1);
        assert.equal(s.liveTools[0].partialOutput, "running test 1/3...");
        assert.equal(s.liveTools[0].detail, "running test 1/3...");

        // Stringified Pi content-block wrapper WITHOUT "type":"text" must unwrap cleanly
        s = dispatchEvent(
          s,
          {
            type: "tool_execution_update",
            tool_call_id: "tc-stream-1",
            tool_name: "bash",
            partial_output: JSON.stringify({ content: [{ text: "running test 2/3..." }] }),
          },
          "S15.update_wrapped_str"
        );
        assert.equal(s.liveTools[0].partialOutput, "running test 2/3...");
        assert.equal(s.liveTools[0].detail, "running test 2/3...");

        s = dispatchEvent(
          s,
          {
            type: "tool_execution_update",
            tool_call_id: "tc-stream-1",
            tool_name: "bash",
            partial_output: "running test 3/3... ok",
          },
          "S15.update2"
        );
        assert.equal(s.liveTools[0].partialOutput, "running test 3/3... ok");
        s = dispatchEvent(
          s,
          { type: "tool_execution_end", tool_call_id: "tc-stream-1", tool_name: "bash", is_error: false },
          "S15.end"
        );
        assert.equal(s.liveTools[0].status, "done");
        assert.equal(s.liveTools[0].partialOutput, "running test 3/3... ok");

        // tool_execution_end with stringified wrapper result populates partialOutput/detail when absent
        s = dispatchEvent(
          s,
          { type: "tool_execution_start", tool_call_id: "tc-stream-2", tool_name: "read" },
          "S15.start2"
        );
        s = dispatchEvent(
          s,
          {
            type: "tool_execution_end",
            tool_call_id: "tc-stream-2",
            tool_name: "read",
            is_error: false,
            result: JSON.stringify({ content: [{ text: "fn main() {}\nline 2" }] }),
          },
          "S15.end2"
        );
        const t2 = s.liveTools.find((t) => t.id === "tc-stream-2");
        assert.equal(t2?.status, "done");
        assert.equal(t2?.partialOutput, "fn main() {}\nline 2");
        assert.equal(t2?.detail, "fn main() {}");
      },
    ]);

    // ------------------------------------------------------------
    // Scenario 16: extension_ui_effect (notify, setStatus, setWidget, set_editor_text)
    // ------------------------------------------------------------
    scenarios.push([
      "extension_ui_effect handles notify, setStatus, setWidget, and set_editor_text",
      () => {
        let s = createInitialPiState();

        // notify (snake_case)
        s = dispatchEvent(
          s,
          {
            type: "extension_ui_effect",
            method: "notify",
            message: "Build succeeded",
            notify_type: "info",
          },
          "S16.notify_info"
        );
        assert.equal(s.extensionToasts.length, 1);
        assert.equal(s.extensionToasts[0].message, "Build succeeded");
        assert.equal(s.extensionToasts[0].notifyType, "info");

        // setStatus (camelCase) then clear
        s = dispatchEvent(
          s,
          {
            type: "extension_ui_effect",
            method: "setStatus",
            statusKey: "git",
            statusText: "wt/frontend-store-and-api",
          },
          "S16.status_set"
        );
        assert.equal(s.extensionStatus["git"], "wt/frontend-store-and-api");
        s = dispatchEvent(
          s,
          {
            type: "extension_ui_effect",
            method: "setStatus",
            status_key: "git",
            status_text: "",
          },
          "S16.status_clear"
        );
        assert.equal("git" in s.extensionStatus, false);

        // setWidget (snake_case) then clear
        s = dispatchEvent(
          s,
          {
            type: "extension_ui_effect",
            method: "setWidget",
            widget_key: "coverage",
            widget_lines: ["Lines: 94%", "Branches: 88%"],
          },
          "S16.widget_set"
        );
        assert.deepEqual(s.extensionWidgetLines["coverage"], ["Lines: 94%", "Branches: 88%"]);
        assert.equal(s.extensionWidgets.length, 1);
        assert.equal(s.extensionWidgets[0].id, "coverage");
        assert.equal(s.extensionWidgets[0].body, "Lines: 94%\nBranches: 88%");

        s = dispatchEvent(
          s,
          {
            type: "extension_ui_effect",
            method: "setWidget",
            widgetKey: "coverage",
            widgetLines: [],
          },
          "S16.widget_clear"
        );
        assert.equal("coverage" in s.extensionWidgetLines, false);
        assert.equal(s.extensionWidgets.length, 0);

        // set_editor_text + consume_editor_prefill
        s = dispatchEvent(
          s,
          {
            type: "extension_ui_effect",
            method: "set_editor_text",
            text: "/skill:kani-verify scope.rs",
          },
          "S16.prefill"
        );
        assert.equal(s.editorPrefillText, "/skill:kani-verify scope.rs");
        s = dispatchAction(s, { type: "consume_editor_prefill" }, "S16.consume");
        assert.equal(s.editorPrefillText, null);
      },
    ]);

    // ------------------------------------------------------------
    // Scenario 17: Cycle Model nested/flat/fallback wrap-around & single-model handling
    // ------------------------------------------------------------
    scenarios.push([
      "Cycle Model nested/flat/fallback wrap-around & single-model handling",
      () => {
        const models = [
          { provider: "anthropic", modelId: "claude-sonnet-5-5", label: "Claude Sonnet 5.5" },
          { provider: "openai", modelId: "gpt-6.1-sol", label: "GPT-6.1 Sol" },
          { provider: "google", modelId: "gemini-2.5-pro", label: "Gemini 2.5 Pro" },
        ];

        // Nested wirePayload.model.{provider, id, name} + thinkingLevel
        const nested = resolveCycledModel(models, models[0], {
          model: { provider: "openai", id: "gpt-6.1-sol", name: "GPT-6.1 Sol" },
          thinkingLevel: "high",
        });
        assert.equal(nested.kind, "cycled");
        assert.equal(nested.model.provider, "openai");
        assert.equal(nested.model.modelId, "gpt-6.1-sol");
        assert.equal(nested.model.label, "GPT-6.1 Sol");
        assert.equal(nested.thinkingLevel, "high");

        // Flat wirePayload.{provider, modelId, label} for a new model not in list
        const flat = resolveCycledModel(models, models[0], {
          provider: "anthropic",
          modelId: "claude-opus-5",
          label: "Claude Opus 5",
        });
        assert.equal(flat.kind, "cycled");
        assert.deepEqual(flat.model, {
          provider: "anthropic",
          modelId: "claude-opus-5",
          label: "Claude Opus 5",
        });

        // Null / empty payload triggers fallback_set_model to next model
        const fallbackNull = resolveCycledModel(models, models[0], null);
        assert.equal(fallbackNull.kind, "fallback_set_model");
        assert.equal(fallbackNull.model.modelId, "gpt-6.1-sol");

        // Unchanged model wraps around from last model back to index 0
        const fallbackWrap = resolveCycledModel(models, models[2], {
          model: { provider: "google", modelId: "gemini-2.5-pro" },
        });
        assert.equal(fallbackWrap.kind, "fallback_set_model");
        assert.equal(fallbackWrap.model.modelId, "claude-sonnet-5-5");

        // Single-model list returns single_model
        const single = resolveCycledModel([models[0]], models[0], null);
        assert.equal(single.kind, "single_model");
        const singleSame = resolveCycledModel([models[0]], models[0], {
          provider: "anthropic",
          id: "claude-sonnet-5-5",
        });
        assert.equal(singleSame.kind, "single_model");

        // Thinking level cycling: accepts thinkingLevel or level, and falls back with wrap-around
        const thinkingLevels = ["off", "low", "medium", "high"];
        assert.deepEqual(
          resolveCycledThinkingLevel(thinkingLevels, "medium", { thinkingLevel: "high" }),
          { kind: "cycled", level: "high" }
        );
        assert.deepEqual(
          resolveCycledThinkingLevel(thinkingLevels, "high", { level: "high" }),
          { kind: "fallback_set_thinking", level: "off" }
        );
        assert.deepEqual(
          resolveCycledThinkingLevel(thinkingLevels, "low", null),
          { kind: "fallback_set_thinking", level: "medium" }
        );
      },
    ]);

    // ------------------------------------------------------------
    // Scenario 18: Revert turn + redo state transitions & copy markdown formatting
    // ------------------------------------------------------------
    scenarios.push([
      "Revert turn (chat-only & chat+code) + redo state transitions",
      async () => {
        let s = createInitialPiState();
        assert.equal(s.canRedoRevert, false);
        assert.deepEqual(s.lastRevertedFiles, []);

        // Seed 2 turns
        const baseSession = {
          id: "sess-rev",
          projectId: "proj-verity",
          name: "Revert test",
          path: "/tmp/sess-rev.jsonl",
          updatedAt: "Just now",
          messageCount: 4,
        };
        const msgs = [
          { id: "u-1", role: "user", content: "First prompt", timestamp: "10:00" },
          { id: "a-1", role: "assistant", content: "First reply", timestamp: "10:00" },
          { id: "u-2", role: "user", content: "Second prompt", timestamp: "10:01" },
          { id: "a-2", role: "assistant", content: "Second reply", timestamp: "10:01" },
        ];
        s = dispatchAction(
          s,
          { type: "session_created", session: baseSession, messages: msgs },
          "S18.seed"
        );

        // Verify Markdown formatting for Copy Full Chat
        const md = formatChatTranscriptMarkdown(s.messages);
        assert.equal(
          md,
          "## You\nFirst prompt\n\n## Verity\nFirst reply\n\n## You\nSecond prompt\n\n## Verity\nSecond reply"
        );

        // Revert turn with code snapshot -> truncates messages, prefills prompt, enables redo
        s = dispatchAction(
          s,
          {
            type: "turn_reverted",
            session: { ...baseSession, messageCount: 2 },
            messages: msgs.slice(0, 2),
            restoredPrompt: "Second prompt",
            canRedo: true,
            revertedFiles: ["src/a.ts", "src/b.ts"],
          },
          "S18.revert_code"
        );
        assert.equal(s.messages.length, 2);
        assert.equal(s.canRedoRevert, true);
        assert.deepEqual(s.lastRevertedFiles, ["src/a.ts", "src/b.ts"]);
        assert.equal(s.editorPrefillText, "Second prompt");
        assert.equal(s.isStreaming, false);
        assert.equal(s.pendingQueueCount, 0);

        // Redo revert clears canRedoRevert & lastRevertedFiles
        s = dispatchAction(s, { type: "revert_redone" }, "S18.redo");
        assert.equal(s.canRedoRevert, false);
        assert.deepEqual(s.lastRevertedFiles, []);

        // Revert chat only leaves canRedoRevert false
        s = dispatchAction(
          s,
          {
            type: "turn_reverted",
            session: { ...baseSession, messageCount: 0 },
            messages: [],
            restoredPrompt: "First prompt",
            canRedo: false,
            revertedFiles: [],
          },
          "S18.revert_chat_only"
        );
        assert.equal(s.messages.length, 0);
        assert.equal(s.canRedoRevert, false);
        assert.equal(s.editorPrefillText, "First prompt");

        // BrowserPreviewRuntime IPC wrappers work end-to-end, including local-* ID resolution via userTurnIndex/promptText
        const freshSession = await tauriApi.newSession("proj-verity");
        await tauriApi.sendPrompt("Local turn one", []);
        await tauriApi.abort();
        await tauriApi.sendPrompt("Local turn two", []);
        await tauriApi.abort();

        // Revert "local-2" by userTurnIndex=1
        const revLocal2 = await tauriApi.piRevertTurn("local-2", false, 1, "Local turn two");
        assert.equal(revLocal2.restoredPrompt, "Local turn two");
        assert.equal(revLocal2.messages.filter((m) => m.role === "user").length, 1);

        // Revert "local-1" by promptText fallback (no userTurnIndex)
        const revLocal1 = await tauriApi.piRevertTurn("local-1", true, undefined, "Local turn one");
        assert.equal(revLocal1.restoredPrompt, "Local turn one");
        assert.equal(revLocal1.messages.length, 0);
        assert.equal(revLocal1.codeReverted, true);
        assert.equal(revLocal1.canRedo, true);
        const redone = await tauriApi.piRedoRevert();
        assert.ok(redone.restoredFiles.length >= 1);

        // Clean up temporary preview session so Scenario 20 starts on default active session
        await tauriApi.piDeleteSession(freshSession.id);
      },
    ]);

    // ------------------------------------------------------------
    // Scenario 19: Incremental message_update streaming deltas including
    // whitespace chunks (" ", "\n"), snake_case assistant_message_event,
    // Anthropic content_block_delta, and text_end hydration before message_end
    // ------------------------------------------------------------
    scenarios.push([
      "Incremental message_update streaming deltas (whitespace, snake_case, content_block_delta, text_end)",
      () => {
        let s = createInitialPiState();
        s = dispatchAction(
          s,
          {
            type: "prompt_submitted",
            message: { id: "u-19", role: "user", content: "Stream incrementally", timestamp: "10:19" },
          },
          "S19.prompt"
        );
        // message_start with role: null still creates the streaming assistant message
        s = dispatchEvent(
          s,
          { type: "message_start", message_id: "m-19", role: null },
          "S19.start"
        );
        assert.equal(s.isStreaming, true);
        assert.equal(s.messages.find((m) => m.id === "m-19")?.streaming, true);

        // Anthropic-style content_block_delta for thinking
        s = dispatchEvent(
          s,
          {
            type: "content_block_delta",
            message_id: "m-19",
            delta: { type: "thinking_delta", thinking: "Step 1" },
          },
          "S19.thinking_cbd"
        );
        // snake_case assistant_message_event for thinking whitespace + continuation
        s = dispatchEvent(
          s,
          {
            type: "message_update",
            message_id: "m-19",
            assistant_message_event: { type: "thinking_delta", delta: "\nStep 2" },
          },
          "S19.thinking_snake"
        );
        assert.equal(s.messages.find((m) => m.id === "m-19")?.thinking, "Step 1\nStep 2");

        // Incremental text deltas including whitespace-only " " and "\n" BEFORE message_end
        s = dispatchEvent(s, { type: "message_update", message_id: "m-19", delta: "Hello" }, "S19.d1");
        assert.equal(s.messages.find((m) => m.id === "m-19")?.content, "Hello");
        assert.equal(s.messages.find((m) => m.id === "m-19")?.streaming, true);
        assert.equal(s.isStreaming, true);

        s = dispatchEvent(s, { type: "message_update", message_id: "m-19", delta: " " }, "S19.d2_space");
        assert.equal(s.messages.find((m) => m.id === "m-19")?.content, "Hello ");

        s = dispatchEvent(
          s,
          {
            type: "content_block_delta",
            message_id: "m-19",
            delta: { type: "text_delta", text: "world" },
          },
          "S19.d3_cbd"
        );
        assert.equal(s.messages.find((m) => m.id === "m-19")?.content, "Hello world");

        s = dispatchEvent(
          s,
          {
            type: "message_update",
            message_id: "m-19",
            assistant_message_event: { type: "text_delta", delta: "\n" },
          },
          "S19.d4_newline"
        );
        assert.equal(s.messages.find((m) => m.id === "m-19")?.content, "Hello world\n");

        s = dispatchEvent(
          s,
          {
            type: "message_update",
            message_id: "m-19",
            delta: { type: "text_delta", text: "Line 2" },
          },
          "S19.d5_nested_delta"
        );
        assert.equal(s.messages.find((m) => m.id === "m-19")?.content, "Hello world\nLine 2");

        // Rust-emitted toolcall_delta and input_json_delta inside raw_json must NEVER leak into prose or thinking
        s = dispatchEvent(
          s,
          {
            type: "message_update",
            message_id: "m-19",
            delta: null,
            raw_json: JSON.stringify({
              type: "message_update",
              message: { id: "m-19", role: "assistant" },
              assistantMessageEvent: {
                type: "toolcall_delta",
                delta: '{"path":"crates/verity-core/src/scope.rs"}',
              },
            }),
          },
          "S19.toolcall_delta_raw_json"
        );
        s = dispatchEvent(
          s,
          {
            type: "content_block_delta",
            message_id: "m-19",
            delta: {
              type: "input_json_delta",
              partial_json: '{"command":"cargo test"}',
              delta: '{"command":"cargo test"}',
            },
          },
          "S19.input_json_delta"
        );
        assert.equal(s.messages.find((m) => m.id === "m-19")?.content, "Hello world\nLine 2");
        assert.equal(s.messages.find((m) => m.id === "m-19")?.thinking, "Step 1\nStep 2");

        // text_end after deltas must NOT duplicate or overwrite streamed content
        s = dispatchEvent(
          s,
          {
            type: "message_update",
            message_id: "m-19",
            assistantMessageEvent: { type: "text_end", content: "SHOULD_NOT_OVERWRITE" },
          },
          "S19.text_end_noop"
        );
        assert.equal(s.messages.find((m) => m.id === "m-19")?.content, "Hello world\nLine 2");
        assert.equal(s.isStreaming, true);

        s = dispatchEvent(
          s,
          { type: "message_end", message_id: "m-19", role: "assistant", stop_reason: "stop" },
          "S19.end"
        );
        assert.equal(s.isStreaming, false);
        assert.equal(s.messages.find((m) => m.id === "m-19")?.streaming, false);
        assert.equal(s.messages.find((m) => m.id === "m-19")?.content, "Hello world\nLine 2");

        // text_end when NO prior text deltas arrived hydrates cleanly
        s = dispatchAction(
          s,
          {
            type: "prompt_submitted",
            message: { id: "u-19b", role: "user", content: "Text end only", timestamp: "10:20" },
          },
          "S19b.prompt"
        );
        s = dispatchEvent(s, { type: "message_start", message_id: "m-19b", role: "assistant" }, "S19b.start");
        s = dispatchEvent(
          s,
          {
            type: "message_update",
            message_id: "m-19b",
            assistant_message_event: { type: "text_end", content: "Hydrated via text_end" },
          },
          "S19b.text_end"
        );
        assert.equal(s.messages.find((m) => m.id === "m-19b")?.content, "Hydrated via text_end");
        assert.equal(s.messages.find((m) => m.id === "m-19b")?.streaming, true);
      },
    ]);

    // ------------------------------------------------------------
    // Scenario 20: Session archive/unarchive, delete active fallback,
    // and per-project chat grouping
    // ------------------------------------------------------------
    scenarios.push([
      "Session archive/unarchive, delete active fallback, and project grouping",
      async () => {
        const initial = await tauriApi.bootstrapPiWorkspace();
        let s = createInitialPiState();
        s = dispatchAction(s, { type: "bootstrap_loaded", data: initial }, "S20.bootstrap");

        // Verify sessions group cleanly by projectId
        const activeProj = s.projects[0];
        assert.ok(activeProj, "S20: expected at least one project");
        const projSessionsBefore = s.sessions.filter(
          (sess) => sess.projectId === activeProj.id && !sess.archived
        );
        assert.ok(projSessionsBefore.length >= 2, "S20: expected >= 2 sessions in first project");

        const targetId = s.activeSessionId;
        assert.ok(targetId, "S20: expected an activeSessionId");

        // Archive the active session -> moves to archived and falls back activeSessionId
        const afterArchive = await tauriApi.piArchiveSession(targetId, true);
        s = dispatchAction(s, { type: "bootstrap_loaded", data: afterArchive }, "S20.archive");
        const archivedEntry = s.sessions.find((sess) => sess.id === targetId);
        assert.equal(archivedEntry?.archived, true, "S20: target session must be marked archived");
        assert.notEqual(
          s.activeSessionId,
          targetId,
          "S20: archiving active session must fall back to another non-archived session"
        );
        const visibleInProj = s.sessions.filter(
          (sess) => sess.projectId === activeProj.id && !sess.archived
        );
        assert.equal(visibleInProj.some((sess) => sess.id === targetId), false);
        assert.equal(s.sessions.filter((sess) => Boolean(sess.archived)).length, 1);

        // Unarchive (restore) the session -> returns to project group
        const afterRestore = await tauriApi.piArchiveSession(targetId, false);
        s = dispatchAction(s, { type: "bootstrap_loaded", data: afterRestore }, "S20.restore");
        assert.equal(s.sessions.find((sess) => sess.id === targetId)?.archived, false);
        assert.equal(s.sessions.filter((sess) => Boolean(sess.archived)).length, 0);

        // Delete the currently active session -> removed from sessions + active fallback
        const toDeleteId = s.activeSessionId;
        const countBeforeDelete = s.sessions.length;
        const afterDelete = await tauriApi.piDeleteSession(toDeleteId);
        s = dispatchAction(s, { type: "bootstrap_loaded", data: afterDelete }, "S20.delete");
        assert.equal(s.sessions.length, countBeforeDelete - 1, "S20: deleted session must be removed");
        assert.equal(s.sessions.some((sess) => sess.id === toDeleteId), false);
        assert.notEqual(s.activeSessionId, toDeleteId, "S20: deleting active session must fall back");
        assert.ok(s.activeSessionId.length > 0, "S20: fallback activeSessionId must be non-empty");
      },
    ]);

    let scenariosPassed = 0;
    for (const [name, fn] of scenarios) {
      await fn();
      scenariosPassed += 1;
      console.log(`  [PASS] Scenario ${scenariosPassed}: ${name}`);
    }

    // ------------------------------------------------------------
    // Part B: 500-trace property-based fuzzer over piReducer
    // ------------------------------------------------------------
    const TRACE_COUNT = 500;
    const rand = createPrng(0x5eed2026);
    const msgIdPool = ["m-1", "m-2", "m-3", "u-1", null];
    const toolIdPool = ["tc-1", "tc-2", "tc-3", "tc-4", null];
    const toolNamePool = ["read", "write", "bash", "grep"];
    const stopReasons = ["stop", "end_turn", "length", "error", "aborted", "toolUse", null];
    const roles = ["assistant", "user", "toolResult", "system", null];
    const textPool = ["hello ", "🌍✨", "chunk ", "", null];

    let totalSteps = 0;

    function randomAction(stepIdx) {
      const r = rand();
      if (r < 0.08) {
        return {
          type: "prompt_submitted",
          message: {
            id: `u-fuzz-${stepIdx}`,
            role: "user",
            content: `Prompt ${stepIdx}`,
            timestamp: "10:00",
          },
        };
      }
      if (r < 0.12) return { type: "prompt_failed" };
      if (r < 0.18) return { type: "turn_aborted" };
      if (r < 0.21) {
        return {
          type: "session_switched",
          sessionId: `sess-${stepIdx}`,
          projectId: "proj-1",
          messages: [
            { id: `hist-u-${stepIdx}`, role: "user", content: "hist", timestamp: "09:00" },
          ],
        };
      }
      if (r < 0.24) {
        return {
          type: "session_created",
          session: {
            id: `sess-new-${stepIdx}`,
            projectId: "proj-1",
            name: `Session ${stepIdx}`,
            path: `/tmp/${stepIdx}.jsonl`,
            updatedAt: "Just now",
            messageCount: 0,
          },
          messages: [],
        };
      }
      if (r < 0.27) return { type: "event_stream_failed", error: "ACL denied pi:event" };
      if (r < 0.30) return { type: "set_error", error: rand() < 0.5 ? "IPC error" : null };

      // Raw Pi JSONL events parsed through parsePiUiEventPayload
      let raw;
      const er = rand();
      if (er < 0.12) {
        raw = { type: "message_start", message_id: pick(rand, msgIdPool), role: pick(rand, roles) };
      } else if (er < 0.22) {
        raw = {
          type: "message_update",
          message_id: pick(rand, msgIdPool),
          delta: pick(rand, textPool),
          thinking_delta: pick(rand, textPool),
        };
      } else if (er < 0.28) {
        const nestedKind = pick(rand, ["thinking_start", "thinking_delta", "thinking_end", "toolcall_delta", "input_json_delta"]);
        raw = {
          type: "message_update",
          message_id: pick(rand, msgIdPool),
          delta: null,
          raw_json: JSON.stringify({
            type: "message_update",
            assistantMessageEvent:
              nestedKind === "thinking_start"
                ? { type: "thinking_start" }
                : nestedKind === "thinking_end"
                  ? { type: "thinking_end", content: "thought summary" }
                  : nestedKind === "thinking_delta"
                    ? { type: "thinking_delta", delta: "hmm " }
                    : { type: nestedKind, delta: '{"arg":"val"}' },
          }),
        };
      } else if (er < 0.44) {
        const sr = pick(rand, stopReasons);
        raw = {
          type: "message_end",
          message_id: pick(rand, msgIdPool),
          role: pick(rand, roles),
          stop_reason: sr,
          text: pick(rand, textPool),
          thinking: pick(rand, textPool),
          error_message: sr === "error" ? "HTTP 500" : null,
        };
      } else if (er < 0.56) {
        raw = {
          type: "tool_execution_start",
          tool_call_id: pick(rand, toolIdPool),
          tool_name: pick(rand, toolNamePool),
        };
      } else if (er < 0.61) {
        const wrapped = rand() < 0.5;
        raw = {
          type: "tool_execution_update",
          tool_call_id: pick(rand, toolIdPool),
          tool_name: pick(rand, toolNamePool),
          partial_output: wrapped
            ? JSON.stringify({ content: [{ text: "unwrapped chunk" }] })
            : pick(rand, textPool),
        };
      } else if (er < 0.66) {
        const withWrappedResult = rand() < 0.4;
        raw = {
          type: "tool_execution_end",
          tool_call_id: pick(rand, toolIdPool),
          tool_name: pick(rand, toolNamePool),
          is_error: rand() < 0.3,
          ...(withWrappedResult
            ? { result: JSON.stringify({ content: [{ text: "final tool text" }] }) }
            : {}),
        };
      } else if (er < 0.74) {
        raw = { type: "queue_update", pending_count: pick(rand, [0, 1, 2, 5]) };
      } else if (er < 0.80) {
        raw = {
          type: "auto_retry_start",
          attempt: 1,
          max_attempts: 3,
          delay_ms: 1500,
          error_message: "429",
        };
      } else if (er < 0.86) {
        raw = {
          type: "auto_retry_end",
          attempt: 2,
          success: rand() < 0.5,
          final_error: "Retry failed",
        };
      } else if (er < 0.90) {
        raw = { type: "error", message: "Runtime error" };
      } else if (er < 0.94) {
        raw = { type: "process_exited", exit_code: 1, stderr_tail: "crash" };
      } else {
        raw = { type: pick(rand, ["agent_settled", "agent_end", "turn_end"]), summary: "done" };
      }

      const event = parsePiUiEventPayload(raw);
      assert.ok(event !== null, `Fuzzer generated unparseable event: ${JSON.stringify(raw)}`);
      return { type: "pi_event", event, __raw: raw };
    }

    for (let trace = 0; trace < TRACE_COUNT; trace += 1) {
      let state = createInitialPiState();
      const steps = 25 + Math.floor(rand() * 21);
      for (let step = 0; step < steps; step += 1) {
        const actionWithMeta = randomAction(step);
        const rawPayload = actionWithMeta.__raw ?? null;
        const action =
          actionWithMeta.type === "pi_event"
            ? { type: "pi_event", event: actionWithMeta.event }
            : actionWithMeta;
        const ctx = `Trace ${trace + 1} step ${step + 1} (${action.type}${
          action.type === "pi_event" ? `:${action.event.type}` : ""
        })`;
        const next = piReducer(state, action);
        assertStepInvariants(state, action, next, ctx, rawPayload);

        // Law 2 trailing-agent_settled probe from every process_exited state
        if (action.type === "pi_event" && action.event.type === "process_exited") {
          const settledEvent = parsePiUiEventPayload({ type: "agent_settled", summary: "trailing" });
          const afterSettled = piReducer(next, { type: "pi_event", event: settledEvent });
          assert.equal(afterSettled.isStreaming, false, `${ctx}+agent_settled: isStreaming !== false`);
          assert.equal(
            afterSettled.connectionState,
            "disconnected",
            `${ctx}+agent_settled: connectionState !== disconnected`
          );
          assert.equal(
            hasVisibleError(afterSettled),
            true,
            `${ctx}+agent_settled: hasVisibleError lost after trailing agent_settled`
          );
          assertStepInvariants(next, { type: "pi_event", event: settledEvent }, afterSettled, `${ctx}+agent_settled`);
        }

        state = next;
        totalSteps += 1;
      }
    }

    console.log(
      `\nALL PASSED: ${scenariosPassed}/${scenarios.length} realistic scenarios + ${TRACE_COUNT}/${TRACE_COUNT} property-based traces (${totalSteps} steps, 8 laws verified at every step).`
    );
  } finally {
    await server.close();
  }
}

main().catch((err) => {
  console.error("\nVERIFICATION FAILED:", err);
  process.exit(1);
});
