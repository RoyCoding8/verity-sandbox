import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { chmod, copyFile, mkdir, mkdtemp, readFile, rm, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { createInterface } from "node:readline";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const extension = fileURLToPath(
  new URL("../../../src-tauri/resources/verity-extension.ts", import.meta.url),
);
const verityTools = ["verity_list_handoffs", "verity_run_readonly", "verity_spawn_write"];
const host = spawnSync("rustc", ["-vV"], { encoding: "utf8" }).stdout?.match(/^host: (.+)$/m)?.[1];
assert.ok(host, "rustc must report the host target for the staged CLI test");
const stagedCli = process.env.VERITY_TEST_CLI || fileURLToPath(new URL(
  `../../../src-tauri/binaries/verity-${host}${host.includes("windows") ? ".exe" : ""}`, import.meta.url,
));
const piPackageCli = "@earendil-works/pi-coding-agent/dist/cli.js";
const piCli = process.env.VERITY_PI_CLI || (process.env.PATH || "").split(delimiter).flatMap((dir) => [
  join(dir, "..", piPackageCli), join(dir, "node_modules", piPackageCli),
  ...(process.platform !== "win32" && existsSync(join(dir, "pi")) ? [realpathSync(join(dir, "pi"))] : []),
]).find(existsSync);
assert.ok(piCli, "Install Pi or set VERITY_PI_CLI to its dist/cli.js");

async function readPiRegistration({ enabled, cliConfigured, cliPath = stagedCli, resolutionError = "", handoffs, executeCliTool = false, recovery = false }) {
  const dir = await mkdtemp(join(tmpdir(), "verity-pi-test-"));
  const probe = join(dir, "probe.ts");
  const resultPath = join(dir, "tools.json");
  const selectedCli = recovery ? join(dir, `verity${process.platform === "win32" ? ".exe" : ""}`) : cliPath;
  try {
    if (cliPath.startsWith("./")) await copyFile(stagedCli, join(dir, cliPath));
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
  const tools = new Map();
  verityExtension({ ...pi, registerTool(tool) { tools.set(tool.name, tool); pi.registerTool(tool); } });
  pi.on("session_start", async (_, ctx) => {
    const result = { tools: pi.getAllTools().map(tool => tool.name), activeTools: pi.getActiveTools() };
    if (${Boolean(handoffs)}) {
      try {
        result.listing = await tools.get("verity_list_handoffs").execute("test", {}, undefined, undefined, ctx);
      } catch (error) {
        result.error = String(error);
      }
    }
    if (${executeCliTool}) {
      result.execution = await tools.get("verity_spawn_write").execute("test", {
        task_id: "../invalid", verifier: "kani", scope: ["src/lib.rs"], goal: "CLI boundary test",
      }, undefined, undefined, ctx);
    }
    writeFileSync(process.env.VERITY_TEST_TOOL_RESULT, JSON.stringify(result));
  });
}
`);
    const child = spawn(process.execPath, [piCli, "--mode", "rpc", "--offline", "--no-session",
      "--no-extensions", "--no-skills", "--no-prompt-templates", "--no-themes",
      "-e", probe], {
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
        VERITY_MCP_BIN: cliConfigured ? selectedCli : "",
        VERITY_MCP_ERROR: resolutionError,
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
      lines.on("line", (line) => {
        try {
          const event = JSON.parse(line);
          if (event.type === "extension_ui_request") diagnostics.push(event);
        } catch {}
      });
      const request = (type, id) => new Promise((resolve, reject) => {
        const onLine = (line) => {
          try {
            const response = JSON.parse(line);
            if (response.id === id) {
              clearTimeout(timeout);
              lines.off("line", onLine);
              resolve(response);
            }
          } catch (error) {
            reject(error);
          }
        };
        lines.on("line", onLine);
        child.once("error", reject);
        child.once("close", (code) => reject(new Error(`Pi exited with ${code}. ${stderr}`)));
        timeout = setTimeout(() => reject(new Error(`Pi did not return ${id}. ${stderr}`)), 15_000);
        child.stdin.write(JSON.stringify({ type, id }) + "\n");
      });
      child.stdin.on("error", () => {});
      const reply = await request("get_available_models", "models");
      assert.equal(reply.success, true, stderr);
      const model = reply.data.models.find((item) =>
        item.provider === "verity-fixture" && item.id === "fixture-model");
      assert.ok(model, `Pi did not register the configured model. ${stderr}`);
      if (recovery) {
        const snapshot = async () => ({
          ...JSON.parse(await readFile(resultPath, "utf8")),
          status: diagnostics.filter((event) => event.method === "setStatus").at(-1),
        });
        const snapshots = [await snapshot()];
        for (const ready of [true, false, true]) {
          if (ready) await copyFile(stagedCli, selectedCli);
          else await unlink(selectedCli);
          const reply = await request("new_session", `session-${snapshots.length}`);
          assert.equal(reply.success, true, JSON.stringify(reply));
          snapshots.push(await snapshot());
        }
        return { snapshots, diagnostics };
      }
      const result = JSON.parse(await readFile(resultPath, "utf8"));
      result.tools = result.tools.filter((name) => name.startsWith("verity_")).sort();
      result.builtinTools = result.activeTools.filter((name) => !name.startsWith("verity_")).sort();
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
  const ready = await readPiRegistration({ enabled: true, cliConfigured: true });
  assert.deepEqual(ready.tools, verityTools);
  assert.deepEqual(ready.activeTools, verityTools);
  assert.equal(ready.diagnostics.some((event) => event.method === "notify"), false, JSON.stringify(ready.diagnostics));
  assert.deepEqual((await readPiRegistration({ enabled: false, cliConfigured: true })).tools, []);
  assert.deepEqual((await readPiRegistration({ enabled: true, cliConfigured: false })).tools, []);
});

test("an invalid CLI is inactive and produces a persistent startup status and warning", async () => {
  for (const cliPath of [join(tmpdir(), "verity-does-not-exist"), process.execPath, `./verity${process.platform === "win32" ? ".exe" : ""}`]) {
    const result = await readPiRegistration({ enabled: true, cliConfigured: true, cliPath });
    assert.deepEqual(result.activeTools, []);
    assert.deepEqual(result.builtinTools, ["bash", "edit", "read", "write"]);
    assert.ok(result.diagnostics.some((event) => event.method === "setStatus" && /unavailable/i.test(event.statusText)));
    assert.ok(result.diagnostics.some((event) => event.method === "notify" && event.notifyType === "warning" && event.message.includes(cliPath)));
    if (cliPath.startsWith("./")) assert.ok(result.diagnostics.some((event) => /must be an absolute path/.test(event.message)));
  }
});

test("a nonexecutable CLI is inactive without disabling Pi built-ins", async () => {
  const dir = await mkdtemp(join(tmpdir(), "verity-nonexecutable-"));
  try {
    const cliPath = join(dir, "verity.txt");
    await writeFile(cliPath, "not an executable");
    await chmod(cliPath, 0o644);
    const result = await readPiRegistration({ enabled: true, cliConfigured: true, cliPath });
    assert.deepEqual(result.activeTools, []);
    assert.deepEqual(result.builtinTools, ["bash", "edit", "read", "write"]);
    assert.ok(result.diagnostics.some((event) => event.method === "notify" && event.message.includes(cliPath)));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("repeated Pi sessions suspend and recover tools as the CLI disappears and returns", async () => {
  const { snapshots } = await readPiRegistration({ enabled: true, cliConfigured: true, recovery: true });
  for (const [index, snapshot] of snapshots.entries()) {
    assert.deepEqual(snapshot.activeTools.filter((name) => name.startsWith("verity_")).sort(), index % 2 ? verityTools : []);
    assert.deepEqual(snapshot.activeTools.filter((name) => !name.startsWith("verity_")).sort(), ["bash", "edit", "read", "write"]);
    if (index % 2) assert.equal(snapshot.status.statusText, undefined);
    else assert.match(snapshot.status.statusText, /unavailable/i);
  }
});

test("desktop resolution errors show the rejected path and intentional disable remains quiet", async () => {
  const resolutionError = "VERITY_MCP_BIN does not name a CLI file: /missing/explicit/verity";
  const rejected = await readPiRegistration({ enabled: true, cliConfigured: false, resolutionError });
  assert.deepEqual(rejected.activeTools, []);
  assert.deepEqual(rejected.builtinTools, ["bash", "edit", "read", "write"]);
  assert.ok(rejected.diagnostics.some((event) => event.method === "notify" && event.message === resolutionError));
  assert.deepEqual((await readPiRegistration({ enabled: false, cliConfigured: false, resolutionError })).diagnostics, []);
});

test("an active delegation tool executes the staged CLI and reports its argument rejection", async () => {
  const result = await readPiRegistration({ enabled: true, cliConfigured: true, executeCliTool: true });
  assert.deepEqual(result.activeTools, verityTools);
  assert.equal(result.execution.details.exitCode, 2);
  assert.match(result.execution.content[0].text, /invalid task id:/);
  assert.match(result.execution.content[0].text, /\[exit code: 2\]/);
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
