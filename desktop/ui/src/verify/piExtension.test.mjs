import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const extension = fileURLToPath(
  new URL("../../../src-tauri/resources/verity-extension.ts", import.meta.url),
);
const verityTools = ["verity_list_handoffs", "verity_run_readonly", "verity_spawn_write"];

async function readPiRegistration({ enabled, cliConfigured, handoffs }) {
  const dir = await mkdtemp(join(tmpdir(), "verity-pi-test-"));
  const probe = join(dir, "probe.ts");
  const resultPath = join(dir, "tools.json");
  try {
    if (handoffs) {
      const handoffsDir = join(dir, ".verity", "handoffs");
      await mkdir(handoffsDir, { recursive: true });
      for (const [name, contents] of Object.entries(handoffs)) {
        await writeFile(join(handoffsDir, name), contents);
      }
    }
    await writeFile(probe, `
import { writeFileSync } from "node:fs";
import verityExtension from ${JSON.stringify(extension.replaceAll("\\", "/"))};
export default function probe(pi) {
  pi.on("session_start", async (_, ctx) => {
    const result = { tools: pi.getAllTools().map(tool => tool.name), activeTools: pi.getActiveTools() };
    if (${Boolean(handoffs)}) {
      const tools = new Map();
      verityExtension({ registerProvider() {}, registerTool(tool) { tools.set(tool.name, tool); } });
      try {
        result.listing = await tools.get("verity_list_handoffs").execute("test", {}, undefined, undefined, ctx);
      } catch (error) {
        result.error = String(error);
      }
    }
    writeFileSync(process.env.VERITY_TEST_TOOL_RESULT, JSON.stringify(result));
  });
}
`);
    const piCli = process.env.VERITY_PI_CLI;
    const command = piCli ? process.execPath : "pi";
    const prefix = piCli ? [piCli] : [];
    const child = spawn(command, [...prefix, "--mode", "rpc", "--offline", "--no-session",
      "--no-extensions", "--no-skills", "--no-prompt-templates", "--no-themes",
      "-e", extension, "-e", probe], {
      cwd: dir,
      env: {
        PATH: process.env.PATH,
        SystemRoot: process.env.SystemRoot,
        HOME: dir,
        USERPROFILE: dir,
        TEMP: dir,
        TMP: dir,
        PI_CODING_AGENT_DIR: join(dir, "agent"),
        PI_OFFLINE: "1",
        VERITY_TEST_TOOL_RESULT: resultPath,
        VERITY_MCP_BIN: cliConfigured ? join(dir, "verity") : "",
        VERITY_DISABLE_SUBAGENT_TOOLS: enabled ? "0" : "1",
        VERITY_DESKTOP_PROVIDERS: JSON.stringify([{
          id: "verity-fixture",
          name: "Fixture Provider",
          api: "openai-completions",
          base_url: "http://127.0.0.1:11434/v1",
          models: ["fixture-model"],
        }]),
      },
    });
    let stderr = "";
    child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
    const closed = new Promise((resolve) => child.once("close", resolve));
    const lines = createInterface({ input: child.stdout });
    const diagnostics = [];
    let timeout;
    try {
      const response = new Promise((resolve, reject) => {
        lines.on("line", (line) => {
          try {
            const response = JSON.parse(line);
            if (response.type === "extension_ui_request") diagnostics.push(response);
            if (response.id === "models") resolve(response);
          } catch (error) {
            reject(error);
          }
        });
        child.once("error", reject);
        child.once("close", (code) => reject(new Error(`Pi exited with ${code}. ${stderr}`)));
        timeout = setTimeout(() => reject(new Error(`Pi did not return its models. ${stderr}`)), 15_000);
      });
      child.stdin.on("error", () => {});
      child.stdin.write('{"type":"get_available_models","id":"models"}\n');
      const reply = await response;
      assert.equal(reply.success, true, stderr);
      const model = reply.data.models.find((item) =>
        item.provider === "verity-fixture" && item.id === "fixture-model");
      assert.ok(model, `Pi did not register the configured model. ${stderr}`);
      const result = JSON.parse(await readFile(resultPath, "utf8"));
      result.tools = result.tools.filter((name) => name.startsWith("verity_")).sort();
      result.activeTools = result.activeTools.filter((name) => name.startsWith("verity_")).sort();
      result.diagnostics = diagnostics;
      return result;
    } finally {
      clearTimeout(timeout);
      lines.close();
      child.kill();
      await closed;
    }
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

test("offline Pi loads the embedded extension, custom model, and configured tool set", async () => {
  assert.deepEqual((await readPiRegistration({ enabled: true, cliConfigured: true })).tools, verityTools);
  assert.deepEqual((await readPiRegistration({ enabled: false, cliConfigured: true })).tools, []);
  assert.deepEqual((await readPiRegistration({ enabled: true, cliConfigured: false })).tools, []);
});

test("missing CLI produces a persistent startup status and warning; intentional disable stays silent", async () => {
  const missing = await readPiRegistration({ enabled: true, cliConfigured: false });
  assert.deepEqual(missing.activeTools, []);
  assert.ok(missing.diagnostics.some((event) => event.method === "setStatus" && /unavailable/i.test(event.statusText)));
  assert.ok(missing.diagnostics.some((event) => event.method === "notify" && event.notifyType === "warning" && /CLI/i.test(event.message)));
  const disabled = await readPiRegistration({ enabled: false, cliConfigured: false });
  assert.deepEqual(disabled.diagnostics, []);
});

const violation = { rule: "no-unsafe", path: "src/lib.rs", line: 2, detail: "unsafe block" };
function persistedHandoff(status, violations = []) {
  return {
    task_id: status,
    base_commit: "b".repeat(40),
    candidate_commit: "c".repeat(40),
    verifier: "kani",
    changed_paths: ["src/lib.rs"],
    extracted_spec: "fn verify()",
    back_translation: null,
    hygiene: { clean: violations.length === 0, violations },
    verdict: { status, reason: "checker result", checks_passed: 1, probes_satisfied: 1 },
    spec_digest: "a1".repeat(32),
  };
}

test("handoff tool executes against persisted runtime JSON and preserves every verdict and dirty violation", async () => {
  const records = ["pass", "counterexample", "inconclusive", "invalid_report"].map((status) =>
    persistedHandoff(status, status === "counterexample" ? [violation] : []));
  const result = await readPiRegistration({
    enabled: true,
    cliConfigured: true,
    handoffs: {
      ...Object.fromEntries(records.map((record) => [`${record.task_id}.json`, JSON.stringify(record)])),
      "incomplete.json.tmp.123": "{unfinished",
      "notes.txt": "not a handoff",
    },
  });
  assert.equal(result.error, undefined);
  const summaries = result.listing.details.handoffs;
  assert.deepEqual(JSON.parse(result.listing.content[0].text), summaries);
  for (const record of records) {
    const summary = summaries.find((item) => item.task_id === record.task_id);
    assert.equal(summary.verdict, record.verdict.status);
    assert.equal(summary.hygiene_clean, record.hygiene.clean);
    assert.deepEqual(summary.hygiene_violations, record.hygiene.violations);
    assert.deepEqual(summary.changed_paths, record.changed_paths);
    assert.equal(summary.spec_digest, record.spec_digest);
  }
});

test("handoff tool fails closed for malformed persisted records", async () => {
  const valid = persistedHandoff("pass");
  for (const malformed of [
    "{unfinished", "null", "[]",
    ...[
      { verdict: "pass" }, { verdict: { status: "unknown" } },
      { hygiene: undefined }, { hygiene: { clean: true, violations: [violation] } },
      { hygiene: { clean: false, violations: [] } },
      { hygiene: { clean: false, violations: ["dirty"] } },
      { changed_paths: [42] }, { spec_digest: null }, { task_id: "different" },
    ].map((patch) => JSON.stringify({ ...valid, ...patch })),
  ]) {
    const result = await readPiRegistration({
      enabled: true, cliConfigured: true, handoffs: { "pass.json": malformed },
    });
    assert.match(result.error ?? "", /handoff.*pass\.json/i, malformed);
    assert.equal(result.listing, undefined, malformed);
  }
});
