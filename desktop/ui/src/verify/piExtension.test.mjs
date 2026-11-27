import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const extension = fileURLToPath(
  new URL("../../../src-tauri/resources/verity-extension.ts", import.meta.url),
);
const verityTools = ["verity_list_handoffs", "verity_run_readonly", "verity_spawn_write"];

async function readPiRegistration({ enabled, cliConfigured }) {
  const dir = await mkdtemp(join(tmpdir(), "verity-pi-test-"));
  const probe = join(dir, "probe.ts");
  const resultPath = join(dir, "tools.json");
  try {
    await writeFile(probe, `
import { writeFileSync } from "node:fs";
export default function probe(pi) {
  pi.on("session_start", () => {
    writeFileSync(process.env.VERITY_TEST_TOOL_RESULT, JSON.stringify(pi.getAllTools().map(tool => tool.name)));
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
    let timeout;
    try {
      const response = new Promise((resolve, reject) => {
        lines.on("line", (line) => {
          try {
            const response = JSON.parse(line);
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
      const tools = JSON.parse(await readFile(resultPath, "utf8"));
      return tools.filter((name) => name.startsWith("verity_")).sort();
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
  assert.deepEqual(await readPiRegistration({ enabled: true, cliConfigured: true }), verityTools);
  assert.deepEqual(await readPiRegistration({ enabled: false, cliConfigured: true }), []);
  assert.deepEqual(await readPiRegistration({ enabled: true, cliConfigured: false }), []);
});
