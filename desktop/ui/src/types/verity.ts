/**
 * Domain types and boundary parsers for Verity's formal-verification UI.
 * Wire shapes mirror `desktop/src-tauri/src/verity_bridge.rs` and
 * `crates/verity-runtime/src/handoff.rs` (`RunLog`).
 */

export type VerifierKind = "kani" | "lean";

export type VerdictStatus =
  | "pass"
  | "counterexample"
  | "inconclusive"
  | "invalid_report";

export interface HygieneViolationView {
  rule: string;
  path: string;
  line: number;
  detail: string;
}

export interface VerdictView {
  status: VerdictStatus;
  reason: string;
  checks_passed: number;
  probes_satisfied: number;
}

export interface HandoffView {
  task_id: string;
  base_commit: string;
  verifier: VerifierKind;
  candidate_commit: string;
  changed_paths: string[];
  extracted_spec: string;
  back_translation: string | null;
  hygiene_clean: boolean;
  hygiene_violations: HygieneViolationView[];
  verdict: VerdictView;
  spec_digest: string;
}

export interface ReadOnlyResponse {
  summary: string;
  tool_calls: number;
  settled: boolean;
}

export interface MergeResponse {
  status: "merged" | "conflict";
  commit: string | null;
  already_merged: boolean;
  conflicts: string[];
}

export interface SpawnWriteTaskInput {
  repo_path: string;
  task_id: string;
  base_ref: string;
  write_scope: string[];
  forbidden?: string[];
  verifier: VerifierKind;
  goal: string;
}

export type SubagentMode = "readonly" | "write";

export type SubagentStatus =
  | "running"
  | "awaiting_spec_review"
  | "approved_for_merge"
  | "merged"
  | "rejected"
  | "completed_readonly"
  | "failed";

export interface SubagentBudget {
  toolCallsUsed: number;
  toolCallsLimit: number;
  wallClockSecsUsed: number;
  wallClockSecsLimit: number;
}

export interface SubagentTaskItem {
  taskId: string;
  goal: string;
  mode: SubagentMode;
  status: SubagentStatus;
  verifier: VerifierKind | null;
  worktreePath: string | null;
  writeScope: string[];
  budget: SubagentBudget;
  handoff?: HandoffView;
  diffText?: string;
  readonlySummary?: string;
}

export type SpawnSubagentRequest =
  | {
      mode: "readonly";
      goal: string;
    }
  | {
      mode: "write";
      taskId: string;
      goal: string;
      writeScope: string[];
      verifier: VerifierKind;
    };

export type MergeTreeStatus =
  | { kind: "clean" }
  | { kind: "conflict"; conflictedPaths: string[] }
  | { kind: "unchecked" };

export type MergeQueueState =
  | "ready"
  | "merging"
  | "merged"
  | "conflict"
  | "error";

export interface MergeQueueEntry {
  taskId: string;
  targetRef: string;
  worktreePath: string;
  handoff: HandoffView;
  approvedDigest: string;
  mergeTree: MergeTreeStatus;
  mergeState: MergeQueueState;
  mergedCommit?: string;
  alreadyMerged?: boolean;
  errorMessage?: string;
}

export type RunLogFilter = "All" | "Merged" | "Handoff" | "ReadOnly";
export type RunLogCategory = "Merged" | "Handoff" | "ReadOnly" | "Other";

export interface RunLogEntry {
  task_id: string;
  kind: string;
  detail: string;
  lineNumber?: number;
  timestamp?: string;
}

// ---------------------------------------------------------------------------
// Pure domain helpers & boundary parsers (zero React dependencies)
// ---------------------------------------------------------------------------

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function formatVerifierLabel(verifier: VerifierKind | null): string {
  if (verifier === null) {
    return "No verifier";
  }
  switch (verifier) {
    case "kani":
      return "Rust + Kani";
    case "lean":
      return "Lean 4";
    default: {
      const _exhaustive: never = verifier;
      return _exhaustive;
    }
  }
}

export function formatVerdictStatus(status: VerdictStatus): string {
  switch (status) {
    case "pass":
      return "Pass";
    case "counterexample":
      return "Counterexample";
    case "inconclusive":
      return "Inconclusive";
    case "invalid_report":
      return "Invalid report";
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
}

export function canApproveHandoff(handoff: HandoffView): boolean {
  return (
    handoff.hygiene_clean &&
    handoff.hygiene_violations.length === 0 &&
    handoff.verdict.status === "pass"
  );
}

export function approvalBlockers(handoff: HandoffView): string[] {
  const blockers: string[] = [];
  if (!handoff.hygiene_clean || handoff.hygiene_violations.length > 0) {
    const count = handoff.hygiene_violations.length;
    blockers.push(
      count > 0
        ? `Layer 1 AST hygiene reported ${count} violation${count === 1 ? "" : "s"}`
        : "Layer 1 AST hygiene check did not pass"
    );
  }
  if (handoff.verdict.status !== "pass") {
    blockers.push(
      `Layer 2 verifier verdict is ${formatVerdictStatus(handoff.verdict.status)}`
    );
  }
  return blockers;
}

export function digestsMatch(
  handoffDigest: string,
  approvedDigest: string
): boolean {
  const left = handoffDigest.trim().toLowerCase();
  const right = approvedDigest.trim().toLowerCase();
  return left.length > 0 && left === right;
}

export function shortenDigest(digest: string, head = 12, tail = 6): string {
  const trimmed = digest.trim();
  if (trimmed.length <= head + tail + 3) {
    return trimmed;
  }
  return `${trimmed.slice(0, head)}…${trimmed.slice(-tail)}`;
}

export function budgetRatio(used: number, limit: number): number {
  if (limit <= 0) {
    return 0;
  }
  return Math.max(0, Math.min(1, used / limit));
}

export function formatDurationSecs(totalSeconds: number): string {
  const clamped = Math.max(0, Math.floor(totalSeconds));
  if (clamped < 60) {
    return `${clamped}s`;
  }
  const mins = Math.floor(clamped / 60);
  const secs = clamped % 60;
  return secs === 0 ? `${mins}m` : `${mins}m ${secs}s`;
}

export function classifyRunLogKind(kind: string): RunLogCategory {
  const normalized = kind.trim().toLowerCase();
  if (normalized === "merged" || normalized === "merge_conflict") {
    return "Merged";
  }
  if (
    normalized === "handoff_ready" ||
    normalized === "awaiting_spec_review" ||
    normalized === "approved_for_merge" ||
    normalized === "rejected_scope" ||
    normalized === "rejected_spec"
  ) {
    return "Handoff";
  }
  if (normalized === "completed_readonly" || normalized === "readonly") {
    return "ReadOnly";
  }
  return "Other";
}

/**
 * Parses a single raw `.verity/run-log.jsonl` line at the UI boundary.
 * Returns null for blank lines and preserves malformed non-empty lines as an
 * explicit `unparsed` receipt rather than throwing or dropping audit history.
 */
export function parseRunLogLine(
  rawLine: string,
  lineNumber: number
): RunLogEntry | null {
  const trimmed = rawLine.trim();
  if (trimmed.length === 0) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(trimmed);
    if (
      isRecord(parsed) &&
      typeof parsed.task_id === "string" &&
      typeof parsed.kind === "string" &&
      typeof parsed.detail === "string"
    ) {
      return {
        task_id: parsed.task_id,
        kind: parsed.kind,
        detail: parsed.detail,
        lineNumber,
        timestamp:
          typeof parsed.timestamp === "string" ? parsed.timestamp : undefined,
      };
    }
  } catch {
    // Fall through to unparsed receipt so audit lines are never silently dropped.
  }
  return {
    task_id: `line-${lineNumber}`,
    kind: "unparsed",
    detail: trimmed,
    lineNumber,
  };
}

export function parseRunLogLines(rawLines: readonly string[]): RunLogEntry[] {
  const entries: RunLogEntry[] = [];
  for (let i = 0; i < rawLines.length; i += 1) {
    const entry = parseRunLogLine(rawLines[i], i + 1);
    if (entry !== null) {
      entries.push(entry);
    }
  }
  return entries;
}
