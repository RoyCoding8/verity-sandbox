import { useState, type FormEvent } from "react";
import {
  budgetRatio,
  formatDurationSecs,
  formatVerifierLabel,
  type SpawnSubagentRequest,
  type SubagentMode,
  type SubagentStatus,
  type SubagentTaskItem,
  type VerifierKind,
} from "../types/verity";
import {
  AlertIcon,
  ClockIcon,
  CloseIcon,
  CpuIcon,
  PlusIcon,
  RefreshIcon,
  ShieldCheckIcon,
  TerminalIcon,
} from "./Icons";
import { Md3Select, type Md3SelectOption } from "./Md3Select";
import "./VerificationRail.css";

export interface SubagentRailProps {
  tasks: readonly SubagentTaskItem[];
  selectedTaskId: string | null;
  onSelectTask: (taskId: string) => void;
  onSpawnSubagent?: (request: SpawnSubagentRequest) => void | Promise<void>;
  spawning?: boolean;
}

type StatusTone = "info" | "warning" | "success" | "danger" | "neutral";

interface StatusPresentation {
  label: string;
  tone: StatusTone;
}

const VERIFIER_OPTIONS: readonly Md3SelectOption[] = [
  { value: "kani", label: "Rust + Kani" },
  { value: "lean", label: "Lean 4" },
];

function statusPresentation(status: SubagentStatus): StatusPresentation {
  switch (status) {
    case "running":
      return { label: "Running", tone: "info" };
    case "awaiting_spec_review":
      return { label: "Review", tone: "warning" };
    case "approved_for_merge":
      return { label: "Approved", tone: "success" };
    case "merged":
      return { label: "Merged", tone: "success" };
    case "completed_readonly":
      return { label: "Done", tone: "neutral" };
    case "rejected":
      return { label: "Rejected", tone: "danger" };
    case "failed":
      return { label: "Failed", tone: "danger" };
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
}

function isActiveStatus(status: SubagentStatus): boolean {
  return (
    status === "running" ||
    status === "awaiting_spec_review" ||
    status === "approved_for_merge"
  );
}

function ratioSeverity(ratio: number): "normal" | "warn" | "danger" {
  if (ratio >= 0.95) {
    return "danger";
  }
  if (ratio >= 0.85) {
    return "warn";
  }
  return "normal";
}

function parseScopeInput(raw: string): string[] {
  return raw
    .split(/[,\n]/)
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
}

interface BudgetMeterProps {
  label: string;
  valueText: string;
  ratio: number;
  icon: "tools" | "clock";
}

function BudgetMeter({ label, valueText, ratio, icon }: BudgetMeterProps) {
  const pct = Math.round(ratio * 100);
  const severity = ratioSeverity(ratio);
  return (
    <div className="vr-budget-row">
      <div className="vr-budget-meta">
        <span className="vr-budget-label">
          {icon === "tools" ? <CpuIcon size={12} /> : <ClockIcon size={12} />}
          {label}
        </span>
        <span className="vr-budget-value">{valueText}</span>
      </div>
      <div
        className="vr-progress-track"
        role="progressbar"
        aria-label={label}
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className={`vr-progress-fill vr-progress-fill--${severity}`}
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

export function SubagentRail({
  tasks,
  selectedTaskId,
  onSelectTask,
  onSpawnSubagent,
  spawning = false,
}: SubagentRailProps) {
  const [formOpen, setFormOpen] = useState(false);
  const [mode, setMode] = useState<SubagentMode>("write");
  const [taskId, setTaskId] = useState("");
  const [goal, setGoal] = useState("");
  const [scopeInput, setScopeInput] = useState("");
  const [verifier, setVerifier] = useState<VerifierKind>("kani");
  const [formError, setFormError] = useState<string | null>(null);

  const activeTasks = tasks.filter((t) => isActiveStatus(t.status));
  const completedTasks = tasks.filter((t) => !isActiveStatus(t.status));

  function handleSpawnSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!onSpawnSubagent) {
      return;
    }

    const trimmedGoal = goal.trim();
    if (trimmedGoal.length === 0) {
      setFormError("Describe the task goal before spawning.");
      return;
    }

    if (mode === "readonly") {
      setFormError(null);
      void onSpawnSubagent({ mode: "readonly", goal: trimmedGoal });
      setGoal("");
      setFormOpen(false);
      return;
    }

    const trimmedId = taskId.trim();
    const writeScope = parseScopeInput(scopeInput);
    if (trimmedId.length === 0) {
      setFormError("Write subagents require a task ID for their worktree.");
      return;
    }
    if (writeScope.length === 0) {
      setFormError("Specify at least one relative path in write scope.");
      return;
    }

    setFormError(null);
    void onSpawnSubagent({
      mode: "write",
      taskId: trimmedId,
      goal: trimmedGoal,
      writeScope,
      verifier,
    });
    setTaskId("");
    setGoal("");
    setScopeInput("");
    setFormOpen(false);
  }

  function renderTaskCard(task: SubagentTaskItem) {
    const selected = task.taskId === selectedTaskId;
    const statusInfo = statusPresentation(task.status);
    const toolRatio = budgetRatio(
      task.budget.toolCallsUsed,
      task.budget.toolCallsLimit
    );
    const clockRatio = budgetRatio(
      task.budget.wallClockSecsUsed,
      task.budget.wallClockSecsLimit
    );
    const maxSeverity = ratioSeverity(Math.max(toolRatio, clockRatio));
    const showMeters = selected || task.status === "running";
    const headline =
      task.goal.trim().length > 0 ? task.goal.trim() : task.taskId;
    const verifierText =
      task.verifier !== null
        ? formatVerifierLabel(task.verifier)
        : task.mode === "readonly"
          ? "Read-Only"
          : "Worktree";

    return (
      <button
        key={task.taskId}
        type="button"
        onClick={() => onSelectTask(task.taskId)}
        aria-pressed={selected}
        className={`vr-task-card${selected ? " is-selected" : ""}`}
      >
        <div className="vr-task-top">
          <div className="vr-task-lead">
            <span
              className={`vr-status-dot vr-status-dot--${statusInfo.tone}`}
              aria-hidden="true"
            />
            <span className="vr-task-goal" title={headline}>
              {headline}
            </span>
          </div>
          <span
            className={`vr-status-label vr-status-label--${statusInfo.tone}`}
          >
            {statusInfo.label}
          </span>
        </div>

        <div className="vr-task-meta">
          <span className="vr-task-id" title={task.taskId}>
            {task.taskId}
          </span>
          <span className="vr-meta-sep" aria-hidden="true">
            ·
          </span>
          <span className="vr-verifier-tag">
            {task.verifier !== null ? (
              <ShieldCheckIcon size={12} />
            ) : (
              <TerminalIcon size={12} />
            )}
            {verifierText}
          </span>
          <span
            className={`vr-budget-compact${
              maxSeverity === "danger"
                ? " vr-budget-compact--danger"
                : maxSeverity === "warn"
                  ? " vr-budget-compact--warn"
                  : ""
            }`}
          >
            {task.budget.toolCallsUsed}/{task.budget.toolCallsLimit} tools ·{" "}
            {formatDurationSecs(task.budget.wallClockSecsUsed)}
          </span>
        </div>

        {showMeters && (
          <div className="vr-budget-block">
            <BudgetMeter
              label="Tool calls"
              valueText={`${task.budget.toolCallsUsed}/${task.budget.toolCallsLimit}`}
              ratio={toolRatio}
              icon="tools"
            />
            <BudgetMeter
              label="Wall clock"
              valueText={`${formatDurationSecs(task.budget.wallClockSecsUsed)} / ${formatDurationSecs(task.budget.wallClockSecsLimit)}`}
              ratio={clockRatio}
              icon="clock"
            />
          </div>
        )}
      </button>
    );
  }

  return (
    <aside className="vr-panel" aria-label="Verity subagent rail">
      <div className="vr-header">
        <div className="vr-title-row">
          <CpuIcon size={16} />
          <h2 className="vr-title">Subagents</h2>
          <span className="vr-count-pill">{tasks.length}</span>
        </div>
        {onSpawnSubagent && (
          <button
            type="button"
            className={`vr-btn vr-btn--sm ${formOpen ? "vr-btn--ghost" : "vr-btn--primary"}`}
            aria-expanded={formOpen}
            onClick={() => {
              setFormOpen((prev) => !prev);
              setFormError(null);
            }}
          >
            {formOpen ? (
              <>
                <CloseIcon size={13} />
                Close
              </>
            ) : (
              <>
                <PlusIcon size={13} />
                Spawn
              </>
            )}
          </button>
        )}
      </div>

      {formOpen && onSpawnSubagent && (
        <form className="vr-spawn-form" onSubmit={handleSpawnSubmit}>
          <div className="vr-segmented" role="group" aria-label="Subagent mode">
            <button
              type="button"
              className={`vr-segmented-btn${mode === "readonly" ? " is-active" : ""}`}
              aria-pressed={mode === "readonly"}
              onClick={() => setMode("readonly")}
            >
              <TerminalIcon size={13} />
              Read-Only
            </button>
            <button
              type="button"
              className={`vr-segmented-btn${mode === "write" ? " is-active" : ""}`}
              aria-pressed={mode === "write"}
              onClick={() => setMode("write")}
            >
              <ShieldCheckIcon size={13} />
              Write · Worktree
            </button>
          </div>

          {mode === "write" && (
            <>
              <div className="vr-field-grid">
                <label className="vr-field">
                  Task ID
                  <input
                    type="text"
                    className="vr-input vr-input--mono"
                    value={taskId}
                    onChange={(e) => setTaskId(e.target.value)}
                    placeholder="task-safe-sub"
                  />
                </label>

                <label className="vr-field" htmlFor="vr-verifier-select">
                  Verifier
                  <Md3Select
                    id="vr-verifier-select"
                    variant="field"
                    placement="bottom"
                    ariaLabel="Verifier"
                    value={verifier}
                    options={VERIFIER_OPTIONS}
                    onChange={(next) => {
                      if (next === "kani" || next === "lean") {
                        setVerifier(next);
                      }
                    }}
                  />
                </label>
              </div>

              <label className="vr-field">
                Write scope (comma-separated paths)
                <input
                  type="text"
                  className="vr-input vr-input--mono"
                  value={scopeInput}
                  onChange={(e) => setScopeInput(e.target.value)}
                  placeholder="crates/verity-core/src/scope.rs"
                />
              </label>
            </>
          )}

          <label className="vr-field">
            Goal
            <textarea
              className="vr-textarea"
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              placeholder={
                mode === "readonly"
                  ? "Audit read-only sandbox mount configuration..."
                  : "Implement checked subtraction and add kani::cover! probes..."
              }
            />
          </label>

          {formError !== null && (
            <p className="vr-form-error" role="alert">
              <AlertIcon size={14} />
              <span>{formError}</span>
            </p>
          )}

          <button
            type="submit"
            disabled={spawning}
            className="vr-btn vr-btn--primary vr-btn--full"
          >
            {spawning ? (
              <>
                <RefreshIcon size={14} />
                Spawning…
              </>
            ) : mode === "readonly" ? (
              "Run Read-Only Subagent"
            ) : (
              "Spawn Isolated Write Subagent"
            )}
          </button>
        </form>
      )}

      {tasks.length === 0 ? (
        <div className="vr-empty-state">
          No subagents yet. Spawn a Read-Only audit or an isolated Write
          worktree task.
        </div>
      ) : (
        <>
          {activeTasks.length > 0 && (
            <>
              <div className="vr-section-label">
                <span>Active</span>
                <span className="tabular-nums">{activeTasks.length}</span>
              </div>
              <div className="vr-task-list">
                {activeTasks.map(renderTaskCard)}
              </div>
            </>
          )}

          {completedTasks.length > 0 && (
            <>
              <div className="vr-section-label">
                <span>Completed</span>
                <span className="tabular-nums">{completedTasks.length}</span>
              </div>
              <div className="vr-task-list">
                {completedTasks.map(renderTaskCard)}
              </div>
            </>
          )}
        </>
      )}
    </aside>
  );
}
