import { useState, type CSSProperties, type FormEvent } from "react";
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

export interface SubagentRailProps {
  tasks: readonly SubagentTaskItem[];
  selectedTaskId: string | null;
  onSelectTask: (taskId: string) => void;
  onSpawnSubagent?: (request: SpawnSubagentRequest) => void | Promise<void>;
  spawning?: boolean;
}

interface StatusPresentation {
  label: string;
  background: string;
  color: string;
}

function statusPresentation(status: SubagentStatus): StatusPresentation {
  switch (status) {
    case "running":
      return {
        label: "Running",
        background: "var(--md-sys-color-primary-container)",
        color: "var(--md-sys-color-on-primary-container)",
      };
    case "awaiting_spec_review":
      return {
        label: "Spec review",
        background: "var(--md-sys-color-secondary-container)",
        color: "var(--md-sys-color-on-secondary-container)",
      };
    case "approved_for_merge":
      return {
        label: "Approved",
        background: "var(--md-sys-color-tertiary-container)",
        color: "var(--md-sys-color-on-tertiary-container)",
      };
    case "merged":
      return {
        label: "Merged",
        background: "var(--md-sys-color-tertiary-container)",
        color: "var(--md-sys-color-on-tertiary-container)",
      };
    case "completed_readonly":
      return {
        label: "Completed",
        background: "var(--md-sys-color-surface-container-high)",
        color: "var(--md-sys-color-on-surface-variant)",
      };
    case "rejected":
      return {
        label: "Rejected",
        background: "var(--md-sys-color-error-container)",
        color: "var(--md-sys-color-on-error-container)",
      };
    case "failed":
      return {
        label: "Failed",
        background: "var(--md-sys-color-error-container)",
        color: "var(--md-sys-color-on-error-container)",
      };
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

function progressBarFillColor(ratio: number): string {
  if (ratio >= 0.9) {
    return "var(--md-sys-color-error)";
  }
  if (ratio >= 0.7) {
    return "var(--md-sys-color-tertiary)";
  }
  return "var(--md-sys-color-primary)";
}

function parseScopeInput(raw: string): string[] {
  return raw
    .split(/[,\n]/)
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
}

const styles: Record<string, CSSProperties> = {
  rail: {
    display: "flex",
    flexDirection: "column",
    gap: 14,
    background: "var(--md-sys-color-surface-container-low)",
    borderRight: "1px solid var(--md-sys-color-outline-variant)",
    padding: "16px 14px",
    minWidth: 300,
    maxWidth: 360,
    height: "100%",
    overflowY: "auto",
  },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  title: {
    margin: 0,
    fontSize: 15,
    fontWeight: 600,
    color: "var(--md-sys-color-on-surface)",
    letterSpacing: "0.01em",
  },
  spawnToggleBtn: {
    border: "none",
    borderRadius: "var(--md-sys-shape-corner-full)",
    padding: "6px 12px",
    fontSize: 12,
    fontWeight: 600,
    cursor: "pointer",
    background: "var(--md-sys-color-primary)",
    color: "var(--md-sys-color-on-primary)",
  },
  formCard: {
    background: "var(--md-sys-color-surface-container)",
    border: "1px solid var(--md-sys-color-outline-variant)",
    borderRadius: "var(--md-sys-shape-corner-md)",
    padding: 12,
    display: "flex",
    flexDirection: "column",
    gap: 10,
  },
  segmentedRow: {
    display: "grid",
    gridTemplateColumns: "1fr 1fr",
    gap: 6,
  },
  segmentBtn: {
    border: "1px solid var(--md-sys-color-outline-variant)",
    borderRadius: "var(--md-sys-shape-corner-full)",
    padding: "6px 8px",
    fontSize: 12,
    fontWeight: 600,
    cursor: "pointer",
    background: "var(--md-sys-color-surface)",
    color: "var(--md-sys-color-on-surface-variant)",
  },
  segmentBtnActive: {
    background: "var(--md-sys-color-secondary-container)",
    color: "var(--md-sys-color-on-secondary-container)",
    borderColor: "transparent",
  },
  fieldLabel: {
    display: "flex",
    flexDirection: "column",
    gap: 4,
    fontSize: 11,
    fontWeight: 600,
    color: "var(--md-sys-color-on-surface-variant)",
  },
  input: {
    width: "100%",
    padding: "7px 10px",
    borderRadius: 8,
    border: "1px solid var(--md-sys-color-outline-variant)",
    background: "var(--md-sys-color-surface)",
    color: "var(--md-sys-color-on-surface)",
    fontSize: 12,
    fontFamily: "inherit",
  },
  textarea: {
    width: "100%",
    padding: "7px 10px",
    borderRadius: 8,
    border: "1px solid var(--md-sys-color-outline-variant)",
    background: "var(--md-sys-color-surface)",
    color: "var(--md-sys-color-on-surface)",
    fontSize: 12,
    fontFamily: "inherit",
    resize: "vertical",
    minHeight: 54,
  },
  formError: {
    fontSize: 11,
    color: "var(--md-sys-color-error)",
    margin: 0,
  },
  sectionLabel: {
    fontSize: 11,
    fontWeight: 700,
    textTransform: "uppercase",
    letterSpacing: "0.06em",
    color: "var(--md-sys-color-on-surface-variant)",
    margin: "4px 2px 0",
  },
  taskList: {
    display: "flex",
    flexDirection: "column",
    gap: 8,
  },
  taskCard: {
    width: "100%",
    textAlign: "left",
    border: "1px solid var(--md-sys-color-outline-variant)",
    borderRadius: "var(--md-sys-shape-corner-md)",
    background: "var(--md-sys-color-surface-container)",
    color: "var(--md-sys-color-on-surface)",
    padding: "11px 12px",
    cursor: "pointer",
    display: "flex",
    flexDirection: "column",
    gap: 8,
    transition: "background 120ms ease, border-color 120ms ease",
  },
  taskCardSelected: {
    background: "var(--md-sys-color-primary-container)",
    color: "var(--md-sys-color-on-primary-container)",
    borderColor: "var(--md-sys-color-primary)",
  },
  cardTopRow: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 6,
  },
  taskIdText: {
    fontSize: 13,
    fontWeight: 600,
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  chipRow: {
    display: "flex",
    flexWrap: "wrap",
    gap: 6,
    alignItems: "center",
  },
  pill: {
    display: "inline-flex",
    alignItems: "center",
    padding: "2px 8px",
    borderRadius: "var(--md-sys-shape-corner-full)",
    fontSize: 11,
    fontWeight: 600,
  },
  goalPreview: {
    margin: 0,
    fontSize: 12,
    lineHeight: 1.35,
    color: "var(--md-sys-color-on-surface-variant)",
    display: "-webkit-box",
    WebkitLineClamp: 2,
    WebkitBoxOrient: "vertical",
    overflow: "hidden",
  },
  budgetBlock: {
    display: "flex",
    flexDirection: "column",
    gap: 5,
  },
  budgetRow: {
    display: "flex",
    flexDirection: "column",
    gap: 3,
  },
  budgetMeta: {
    display: "flex",
    justifyContent: "space-between",
    fontSize: 11,
    color: "var(--md-sys-color-on-surface-variant)",
  },
  progressTrack: {
    width: "100%",
    height: 5,
    borderRadius: "var(--md-sys-shape-corner-full)",
    background: "var(--md-sys-color-surface-container-high)",
    overflow: "hidden",
  },
  emptyState: {
    fontSize: 12,
    color: "var(--md-sys-color-on-surface-variant)",
    background: "var(--md-sys-color-surface-container)",
    borderRadius: "var(--md-sys-shape-corner-md)",
    padding: 14,
    textAlign: "center",
  },
};

interface BudgetMeterProps {
  label: string;
  valueText: string;
  ratio: number;
}

function BudgetMeter({ label, valueText, ratio }: BudgetMeterProps) {
  const pct = Math.round(ratio * 100);
  return (
    <div style={styles.budgetRow}>
      <div style={styles.budgetMeta}>
        <span>{label}</span>
        <span>{valueText}</span>
      </div>
      <div
        style={styles.progressTrack}
        role="progressbar"
        aria-label={label}
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          style={{
            width: `${pct}%`,
            height: "100%",
            background: progressBarFillColor(ratio),
            borderRadius: "var(--md-sys-shape-corner-full)",
            transition: "width 160ms ease",
          }}
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
    const statusStyle = statusPresentation(task.status);
    const toolRatio = budgetRatio(
      task.budget.toolCallsUsed,
      task.budget.toolCallsLimit
    );
    const clockRatio = budgetRatio(
      task.budget.wallClockSecsUsed,
      task.budget.wallClockSecsLimit
    );

    return (
      <button
        key={task.taskId}
        type="button"
        onClick={() => onSelectTask(task.taskId)}
        aria-pressed={selected}
        style={{
          ...styles.taskCard,
          ...(selected ? styles.taskCardSelected : undefined),
        }}
      >
        <div style={styles.cardTopRow}>
          <span style={styles.taskIdText} title={task.taskId}>
            {task.taskId}
          </span>
          <span
            style={{
              ...styles.pill,
              background: statusStyle.background,
              color: statusStyle.color,
            }}
          >
            {statusStyle.label}
          </span>
        </div>

        <div style={styles.chipRow}>
          <span
            style={{
              ...styles.pill,
              background:
                task.mode === "write"
                  ? "var(--md-sys-color-secondary-container)"
                  : "var(--md-sys-color-surface-container-high)",
              color:
                task.mode === "write"
                  ? "var(--md-sys-color-on-secondary-container)"
                  : "var(--md-sys-color-on-surface-variant)",
            }}
          >
            {task.mode === "write" ? "Write · Worktree" : "Read-Only"}
          </span>

          {task.verifier !== null && (
            <span
              style={{
                ...styles.pill,
                background: "var(--md-sys-color-tertiary-container)",
                color: "var(--md-sys-color-on-tertiary-container)",
              }}
            >
              {formatVerifierLabel(task.verifier)}
            </span>
          )}
        </div>

        {task.goal.trim().length > 0 && (
          <p style={styles.goalPreview}>{task.goal}</p>
        )}

        <div style={styles.budgetBlock}>
          <BudgetMeter
            label="Tool calls"
            valueText={`${task.budget.toolCallsUsed}/${task.budget.toolCallsLimit}`}
            ratio={toolRatio}
          />
          <BudgetMeter
            label="Wall clock"
            valueText={`${formatDurationSecs(task.budget.wallClockSecsUsed)} / ${formatDurationSecs(task.budget.wallClockSecsLimit)}`}
            ratio={clockRatio}
          />
        </div>
      </button>
    );
  }

  return (
    <aside style={styles.rail} aria-label="Verity subagent rail">
      <div style={styles.header}>
        <h2 style={styles.title}>Subagents</h2>
        {onSpawnSubagent && (
          <button
            type="button"
            style={styles.spawnToggleBtn}
            onClick={() => {
              setFormOpen((prev) => !prev);
              setFormError(null);
            }}
          >
            {formOpen ? "Close" : "+ Spawn"}
          </button>
        )}
      </div>

      {formOpen && onSpawnSubagent && (
        <form style={styles.formCard} onSubmit={handleSpawnSubmit}>
          <div style={styles.segmentedRow} role="group" aria-label="Subagent mode">
            <button
              type="button"
              style={{
                ...styles.segmentBtn,
                ...(mode === "readonly" ? styles.segmentBtnActive : undefined),
              }}
              onClick={() => setMode("readonly")}
            >
              Read-Only
            </button>
            <button
              type="button"
              style={{
                ...styles.segmentBtn,
                ...(mode === "write" ? styles.segmentBtnActive : undefined),
              }}
              onClick={() => setMode("write")}
            >
              Write (Worktree)
            </button>
          </div>

          {mode === "write" && (
            <>
              <label style={styles.fieldLabel}>
                Task ID
                <input
                  type="text"
                  value={taskId}
                  onChange={(e) => setTaskId(e.target.value)}
                  placeholder="e.g. task-safe-sub"
                  style={styles.input}
                />
              </label>

              <label style={styles.fieldLabel}>
                Write scope (comma-separated paths)
                <input
                  type="text"
                  value={scopeInput}
                  onChange={(e) => setScopeInput(e.target.value)}
                  placeholder="crates/verity-core/src/scope.rs"
                  style={styles.input}
                />
              </label>

              <label style={styles.fieldLabel}>
                Verifier
                <select
                  value={verifier}
                  onChange={(e) => {
                    const next = e.target.value;
                    if (next === "kani" || next === "lean") {
                      setVerifier(next);
                    }
                  }}
                  style={styles.input}
                >
                  <option value="kani">Rust + Kani</option>
                  <option value="lean">Lean 4</option>
                </select>
              </label>
            </>
          )}

          <label style={styles.fieldLabel}>
            Goal
            <textarea
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              placeholder={
                mode === "readonly"
                  ? "Audit read-only sandbox mount configuration..."
                  : "Implement checked subtraction and add kani::cover! probes..."
              }
              style={styles.textarea}
            />
          </label>

          {formError !== null && <p style={styles.formError}>{formError}</p>}

          <button
            type="submit"
            disabled={spawning}
            style={{
              ...styles.spawnToggleBtn,
              opacity: spawning ? 0.65 : 1,
              cursor: spawning ? "wait" : "pointer",
            }}
          >
            {spawning
              ? "Spawning…"
              : mode === "readonly"
                ? "Run Read-Only Subagent"
                : "Spawn Isolated Write Subagent"}
          </button>
        </form>
      )}

      {tasks.length === 0 ? (
        <div style={styles.emptyState}>
          No subagents yet. Spawn a Read-Only audit or an isolated Write
          worktree task.
        </div>
      ) : (
        <>
          {activeTasks.length > 0 && (
            <>
              <div style={styles.sectionLabel}>
                Active ({activeTasks.length})
              </div>
              <div style={styles.taskList}>
                {activeTasks.map(renderTaskCard)}
              </div>
            </>
          )}

          {completedTasks.length > 0 && (
            <>
              <div style={styles.sectionLabel}>
                Completed ({completedTasks.length})
              </div>
              <div style={styles.taskList}>
                {completedTasks.map(renderTaskCard)}
              </div>
            </>
          )}
        </>
      )}
    </aside>
  );
}
