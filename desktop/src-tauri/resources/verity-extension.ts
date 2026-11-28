import * as fs from "node:fs/promises";
import * as path from "node:path";
import { StringEnum } from "@earendil-works/pi-ai";
import {
  truncateTail,
  type ExtensionAPI,
} from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const DEFAULT_CONTEXT_WINDOW = 128000;
const DEFAULT_MAX_TOKENS = 16384;
const DEFAULT_COST = Object.freeze({
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
});
const KEYLESS_FALLBACK_API_KEY = "verity-no-key";
const SUBAGENT_EXEC_TIMEOUT_MS = 310_000;
const VERITY_TOOLS = new Set(["verity_run_readonly", "verity_spawn_write", "verity_list_handoffs"]);

const API_MAP: Readonly<Record<string, string>> = Object.freeze({
  "openai-completions": "openai-completions",
  openai_compatible: "openai-completions",
  custom: "openai-completions",
  "openai-responses": "openai-responses",
  "anthropic-messages": "anthropic-messages",
  anthropic: "anthropic-messages",
  "google-generative-ai": "google-generative-ai",
  google: "google-generative-ai",
});

interface RawProviderEntry {
  id?: unknown;
  name?: unknown;
  api?: unknown;
  base_url?: unknown;
  models?: unknown;
  api_key_env?: unknown;
}

interface HandoffSummary {
  task_id: string;
  verifier: "kani" | "lean";
  verdict: "pass" | "counterexample" | "inconclusive" | "invalid_report";
  hygiene_clean: boolean;
  hygiene_violations: HygieneViolation[];
  changed_paths: string[];
  spec_digest: string;
}

interface HygieneViolation {
  rule: string;
  path: string;
  line: number;
  detail: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isHygieneViolation(value: unknown): value is HygieneViolation {
  return isRecord(value) && typeof value.rule === "string" &&
    typeof value.path === "string" && typeof value.detail === "string" &&
    typeof value.line === "number" && Number.isSafeInteger(value.line) && value.line >= 0;
}

function parseHandoffSummary(value: unknown, file: string): HandoffSummary {
  if (!isRecord(value) || typeof value.task_id !== "string" || value.task_id !== file.slice(0, -5) ||
    (value.verifier !== "kani" && value.verifier !== "lean") ||
    !isRecord(value.verdict) || !isRecord(value.hygiene)) {
    throw new Error("invalid handoff identity, verifier, verdict, or hygiene");
  }
  const status = value.verdict.status;
  if (status !== "pass" && status !== "counterexample" &&
    status !== "inconclusive" && status !== "invalid_report") {
    throw new Error("invalid verdict status");
  }
  const { clean, violations } = value.hygiene;
  if (typeof clean !== "boolean" || !Array.isArray(violations) ||
    !violations.every(isHygieneViolation) || clean !== (violations.length === 0)) {
    throw new Error("invalid hygiene report");
  }
  if (!Array.isArray(value.changed_paths) ||
    !value.changed_paths.every((p): p is string => typeof p === "string") ||
    typeof value.spec_digest !== "string" || !/^[0-9a-f]{64}$/.test(value.spec_digest)) {
    throw new Error("invalid changed paths or spec digest");
  }
  return {
    task_id: value.task_id,
    verifier: value.verifier,
    verdict: status,
    hygiene_clean: clean,
    hygiene_violations: violations,
    changed_paths: value.changed_paths,
    spec_digest: value.spec_digest,
  };
}

function resolvePiApi(rawApi: unknown): string {
  if (typeof rawApi !== "string") {
    return "openai-completions";
  }
  const normalized = rawApi.trim();
  if (!normalized) {
    return "openai-completions";
  }
  return API_MAP[normalized] || normalized;
}

function buildModels(rawModels: unknown) {
  if (!Array.isArray(rawModels)) {
    return [];
  }
  const seen = new Set<string>();
  const models: Array<{
    id: string;
    name: string;
    reasoning: boolean;
    input: ("text" | "image")[];
    cost: {
      input: number;
      output: number;
      cacheRead: number;
      cacheWrite: number;
    };
    contextWindow: number;
    maxTokens: number;
  }> = [];
  for (const item of rawModels) {
    if (typeof item !== "string") {
      continue;
    }
    const id = item.trim();
    if (!id || seen.has(id)) {
      continue;
    }
    seen.add(id);
    models.push({
      id,
      name: id,
      reasoning: true,
      input: ["text", "image"],
      cost: { ...DEFAULT_COST },
      contextWindow: DEFAULT_CONTEXT_WINDOW,
      maxTokens: DEFAULT_MAX_TOKENS,
    });
  }
  return models;
}

function isLocalhostBaseUrl(baseUrl: string): boolean {
  try {
    const parsed = new URL(baseUrl);
    const host = parsed.hostname.toLowerCase();
    return (
      host === "localhost" ||
      host === "127.0.0.1" ||
      host === "::1" ||
      host === "[::1]"
    );
  } catch {
    return false;
  }
}

function resolveApiKeyConfig(
  entry: RawProviderEntry,
  baseUrl: string,
): string | undefined {
  if (typeof entry.api_key_env === "string") {
    const envName = entry.api_key_env.trim();
    if (envName) {
      return `$${envName}`;
    }
  }
  if (isLocalhostBaseUrl(baseUrl)) {
    return KEYLESS_FALLBACK_API_KEY;
  }
  return undefined;
}

function registerConfiguredProviders(pi: ExtensionAPI): void {
  if (typeof pi.registerProvider !== "function") {
    return;
  }

  const raw = process.env.VERITY_DESKTOP_PROVIDERS;
  if (typeof raw !== "string" || !raw.trim()) {
    return;
  }

  let providers: unknown;
  try {
    providers = JSON.parse(raw);
  } catch (err) {
    console.error(
      "[verity-providers] failed to parse VERITY_DESKTOP_PROVIDERS:",
      err,
    );
    return;
  }

  if (!Array.isArray(providers)) {
    return;
  }

  for (const rawEntry of providers) {
    if (!rawEntry || typeof rawEntry !== "object") {
      continue;
    }
    const entry = rawEntry as RawProviderEntry;

    const id = typeof entry.id === "string" ? entry.id.trim() : "";
    const baseUrl =
      typeof entry.base_url === "string" ? entry.base_url.trim() : "";
    if (!id || !baseUrl) {
      continue;
    }

    const name =
      typeof entry.name === "string" && entry.name.trim()
        ? entry.name.trim()
        : id;
    const models = buildModels(entry.models);

    const config: Record<string, unknown> = {
      name,
      baseUrl,
      api: resolvePiApi(entry.api),
    };

    const apiKey = resolveApiKeyConfig(entry, baseUrl);
    if (apiKey !== undefined) {
      config.apiKey = apiKey;
    }

    if (models.length > 0) {
      config.models = models;
    }

    try {
      pi.registerProvider(id, config as Parameters<ExtensionAPI["registerProvider"]>[1]);
    } catch (err) {
      console.error(
        `[verity-providers] failed to register provider "${id}":`,
        err,
      );
    }
  }
}

function formatSubagentOutput(
  stdout: string,
  stderr: string,
  code: number | null | undefined,
): string {
  const parts: string[] = [];
  if (stdout.trim()) {
    parts.push(stdout.trimEnd());
  }
  if (stderr.trim()) {
    parts.push(`[stderr]\n${stderr.trimEnd()}`);
  }
  const combined =
    parts.length > 0 ? parts.join("\n\n") : "(subagent produced no output)";
  const truncated = truncateTail(combined, {
    maxLines: 200,
    maxBytes: 32 * 1024,
  });
  const body =
    typeof truncated === "string" ? truncated : truncated.content;
  if (code !== 0 && code !== null && code !== undefined) {
    return `${body}\n\n[exit code: ${code}]`;
  }
  return body;
}

function registerVeritySubagentTools(pi: ExtensionAPI): void {
  if (process.env.VERITY_DISABLE_SUBAGENT_TOOLS === "1") {
    return;
  }

  const verityBin = process.env.VERITY_MCP_BIN?.trim();
  let suspendedTools: string[] = [];
  pi.on("session_start", async (_event, ctx) => {
    const activeTools = pi.getActiveTools();
    suspendedTools = [...new Set([...suspendedTools, ...activeTools.filter((name) => VERITY_TOOLS.has(name))])];
    pi.setActiveTools(activeTools.filter((name) => !VERITY_TOOLS.has(name)));
    let problem = process.env.VERITY_MCP_ERROR?.trim();
    if (!problem && !verityBin) {
      problem = "Verity CLI was not found. Rebuild the desktop app or set VERITY_MCP_BIN to the CLI's absolute path.";
    } else if (!problem && verityBin && !path.isAbsolute(verityBin)) {
      problem = `VERITY_MCP_BIN must be an absolute path: ${verityBin}`;
    } else if (!problem && verityBin) {
      try {
        const result = await pi.exec(verityBin, [], { timeout: 5_000 });
        const output = `${result.stdout}\n${result.stderr}`;
        if (result.code !== 2 || !output.includes("Verity v") || !output.includes("run-readonly") || !output.includes("spawn")) {
          problem = `Verity CLI at ${verityBin} failed its startup check (exit ${result.code}). Rebuild the desktop app or check VERITY_MCP_BIN.`;
        }
      } catch (error) {
        problem = `Verity CLI at ${verityBin} could not start: ${String(error)}`;
      }
    }
    if (problem) {
      ctx.ui.setStatus("verity-delegation", "Verity delegation unavailable");
      ctx.ui.notify(problem, "warning");
    } else {
      pi.setActiveTools([...new Set([...pi.getActiveTools(), ...suspendedTools])]);
      suspendedTools = [];
      ctx.ui.setStatus("verity-delegation", undefined);
    }
  });
  if (!verityBin || typeof pi.registerTool !== "function") {
    return;
  }

  pi.registerTool({
    name: "verity_run_readonly",
    label: "Verity Read-Only Subagent",
    description:
      "Run an isolated read-only Verity subagent in a disposable worktree to investigate code, trace behavior, or validate assumptions without mutating the working tree.",
    promptSnippet:
      "Run a read-only Verity subagent in an isolated worktree to inspect code or verify hypotheses safely.",
    promptGuidelines: [
      "Use verity_run_readonly for deep multi-file investigation or parallel exploration that should not edit files.",
      "Provide a concrete, self-contained goal describing what files, invariants, or questions to investigate.",
    ],
    parameters: Type.Object({
      goal: Type.String({
        description: "Self-contained read-only investigation goal.",
      }),
      repo: Type.Optional(
        Type.String({
          description:
            "Optional repository root path (defaults to current working directory).",
        }),
      ),
    }),
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      const repo = params.repo?.trim() || ctx.cwd;
      onUpdate?.({
        content: [
          {
            type: "text",
            text: `Running Verity read-only subagent in ${repo}...`,
          },
        ],
        details: { repo, goal: params.goal },
      });

      const result = await pi.exec(
        verityBin,
        ["run-readonly", repo, params.goal],
        {
          signal,
          cwd: repo,
          timeout: SUBAGENT_EXEC_TIMEOUT_MS,
        },
      );

      const text = formatSubagentOutput(
        result.stdout ?? "",
        result.stderr ?? "",
        result.code,
      );
      return {
        content: [{ type: "text", text }],
        details: {
          repo,
          exitCode: result.code,
          killed: result.killed ?? false,
        },
      };
    },
  });

  pi.registerTool({
    name: "verity_spawn_write",
    label: "Verity Verified Write Subagent",
    description:
      "Spawn an isolated Verity write subagent in a git worktree with a declared file scope and formal verifier (Kani or Lean) gate.",
    promptSnippet:
      "Spawn a Verity write subagent in an isolated git worktree gated by a Kani or Lean formal verifier.",
    promptGuidelines: [
      "Use verity_spawn_write when delegating a scoped code change that must pass Kani or Lean verification before handoff.",
      "Always specify a tight relative-path scope and a unique kebab-case task_id.",
    ],
    parameters: Type.Object({
      task_id: Type.String({
        description:
          "Unique kebab-case identifier for the subagent task (e.g. 'fix-bounds-check').",
      }),
      verifier: StringEnum(["kani", "lean"] as const, {
        description: "Formal verifier backend to run ('kani' or 'lean').",
      }),
      scope: Type.Array(
        Type.String({
          description: "Allowed relative file or directory path within repo.",
        }),
        {
          minItems: 1,
          description: "Declared write scope (relative paths).",
        },
      ),
      goal: Type.String({
        description:
          "Detailed implementation and verification goal for the write subagent.",
      }),
      base_ref: Type.Optional(
        Type.String({
          description:
            "Optional git base ref to create the worktree from (defaults to HEAD).",
        }),
      ),
      repo: Type.Optional(
        Type.String({
          description:
            "Optional repository root path (defaults to current working directory).",
        }),
      ),
    }),
    async execute(_toolCallId, params, signal, onUpdate, ctx) {
      const repo = params.repo?.trim() || ctx.cwd;
      const baseRef = params.base_ref?.trim() || "HEAD";
      const scopeCsv = params.scope
        .map((entry: string) => entry.trim())
        .filter(Boolean)
        .join(",");

      onUpdate?.({
        content: [
          {
            type: "text",
            text: `Spawning Verity write subagent "${params.task_id}" (${params.verifier}) in ${repo}...`,
          },
        ],
        details: {
          repo,
          task_id: params.task_id,
          verifier: params.verifier,
          base_ref: baseRef,
          scope: params.scope,
        },
      });

      const result = await pi.exec(
        verityBin,
        [
          "spawn",
          repo,
          params.task_id,
          baseRef,
          params.verifier,
          scopeCsv,
          params.goal,
        ],
        {
          signal,
          cwd: repo,
          timeout: SUBAGENT_EXEC_TIMEOUT_MS,
        },
      );

      const text = formatSubagentOutput(
        result.stdout ?? "",
        result.stderr ?? "",
        result.code,
      );
      return {
        content: [{ type: "text", text }],
        details: {
          repo,
          task_id: params.task_id,
          verifier: params.verifier,
          base_ref: baseRef,
          exitCode: result.code,
          killed: result.killed ?? false,
        },
      };
    },
  });

  pi.registerTool({
    name: "verity_list_handoffs",
    label: "Verity List Handoffs",
    description:
      "Read completed Verity subagent handoff summaries from <repo>/.verity/handoffs/*.json.",
    promptSnippet:
      "List Verity subagent handoff verdicts, changed paths, hygiene violations, and spec digests.",
    promptGuidelines: [
      "Use verity_list_handoffs to inspect verified results produced by prior verity_spawn_write runs.",
    ],
    parameters: Type.Object({
      repo: Type.Optional(
        Type.String({
          description:
            "Optional repository root path (defaults to current working directory).",
        }),
      ),
    }),
    async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
      const repo = params.repo?.trim() || ctx.cwd;
      const handoffsDir = path.join(repo, ".verity", "handoffs");

      let entries: string[];
      try {
        entries = await fs.readdir(handoffsDir);
      } catch (err) {
        const code = (err as NodeJS.ErrnoException)?.code;
        if (code === "ENOENT") {
          return {
            content: [
              {
                type: "text",
                text: `No Verity handoffs found in ${handoffsDir}.`,
              },
            ],
            details: { repo, handoffs: [] as HandoffSummary[] },
          };
        }
        throw err;
      }

      const jsonFiles = entries
        .filter((name) => name.endsWith(".json") && !name.includes(".tmp."))
        .sort();

      const handoffs: HandoffSummary[] = [];
      for (const file of jsonFiles) {
        const fullPath = path.join(handoffsDir, file);
        try {
          const raw = await fs.readFile(fullPath, "utf8");
          handoffs.push(parseHandoffSummary(JSON.parse(raw), file));
        } catch (err) {
          throw new Error(`Invalid Verity handoff "${file}": ${String(err)}`);
        }
      }

      const text =
        handoffs.length === 0
          ? `No Verity handoffs found in ${handoffsDir}.`
          : JSON.stringify(handoffs, null, 2);

      return {
        content: [{ type: "text", text }],
        details: { repo, handoffs },
      };
    },
  });
}

/**
 * Verity desktop Pi extension:
 * 1. Reads `VERITY_DESKTOP_PROVIDERS` from the Pi child-process environment and
 *    registers every configured custom provider and model with Pi on startup.
 * 2. Registers Verity subagent tools (`verity_run_readonly`, `verity_spawn_write`,
 *    `verity_list_handoffs`) when `VERITY_MCP_BIN` is set and
 *    `VERITY_DISABLE_SUBAGENT_TOOLS !== "1"`.
 */
export default function verityExtension(pi: ExtensionAPI): void {
  if (!pi) {
    return;
  }
  registerConfiguredProviders(pi);
  registerVeritySubagentTools(pi);
}
