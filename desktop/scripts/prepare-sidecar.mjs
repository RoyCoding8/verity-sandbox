import { spawnSync } from "node:child_process";
import { chmod, copyFile, mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const { values } = parseArgs({
  options: {
    release: { type: "boolean", default: false },
    target: { type: "string" },
  },
});
const target = values.target || process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET;
const release = process.env.TAURI_ENV_DEBUG === undefined
  ? values.release
  : !["true", "1"].includes(process.env.TAURI_ENV_DEBUG);

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with ${result.status}`);
  return result.stdout;
}

const triple = target || run("rustc", ["-vV"]).match(/^host: (.+)$/m)?.[1];
if (!triple) throw new Error("Could not determine the Rust target triple");
const output = run("cargo", [
  "build", "--locked", "--manifest-path", join(root, "Cargo.toml"),
  "-p", "verity-cli", "--bin", "verity", "--message-format=json-render-diagnostics",
  ...(release ? ["--release"] : []),
  ...(target ? ["--target", target] : []),
]);
const artifact = output.trim().split(/\r?\n/).map((line) => JSON.parse(line)).find((entry) =>
  entry.reason === "compiler-artifact" && entry.target.name === "verity" && entry.executable);
if (!artifact) throw new Error("Cargo did not produce the Verity CLI executable");
const staged = join(root, "desktop", "src-tauri", "binaries",
  `verity-${triple}${triple.includes("windows") ? ".exe" : ""}`);
await mkdir(dirname(staged), { recursive: true });
await copyFile(artifact.executable, staged);
if (!triple.includes("windows")) await chmod(staged, 0o755);
console.log(`Staged ${release ? "release" : "debug"} Verity CLI at ${staged}`);
