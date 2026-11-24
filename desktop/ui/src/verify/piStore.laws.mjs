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

function assertStepInvariants(prevState, action, nextState, context) {
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

    const { parsePiUiEventPayload, tauriApi } = tauriMod;
    const { createInitialPiState, piReducer } = storeMod;

    function dispatchEvent(state, rawPayload, label) {
      const event = parsePiUiEventPayload(rawPayload);
      assert.ok(event !== null, `${label}: parsePiUiEventPayload returned null`);
      const action = { type: "pi_event", event };
      const next = piReducer(state, action);
      assertStepInvariants(state, action, next, label);
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
      if (er < 0.14) {
        raw = { type: "message_start", message_id: pick(rand, msgIdPool), role: pick(rand, roles) };
      } else if (er < 0.30) {
        raw = {
          type: "message_update",
          message_id: pick(rand, msgIdPool),
          delta: pick(rand, textPool),
          thinking_delta: pick(rand, textPool),
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
      } else if (er < 0.66) {
        raw = {
          type: "tool_execution_end",
          tool_call_id: pick(rand, toolIdPool),
          tool_name: pick(rand, toolNamePool),
          is_error: rand() < 0.3,
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
      return { type: "pi_event", event };
    }

    for (let trace = 0; trace < TRACE_COUNT; trace += 1) {
      let state = createInitialPiState();
      const steps = 25 + Math.floor(rand() * 21);
      for (let step = 0; step < steps; step += 1) {
        const action = randomAction(step);
        const ctx = `Trace ${trace + 1} step ${step + 1} (${action.type}${
          action.type === "pi_event" ? `:${action.event.type}` : ""
        })`;
        const next = piReducer(state, action);
        assertStepInvariants(state, action, next, ctx);

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
      `\nALL PASSED: ${scenariosPassed}/${scenarios.length} realistic scenarios + ${TRACE_COUNT}/${TRACE_COUNT} property-based traces (${totalSteps} steps, 5 laws verified at every step).`
    );
  } finally {
    await server.close();
  }
}

main().catch((err) => {
  console.error("\nVERIFICATION FAILED:", err);
  process.exit(1);
});
