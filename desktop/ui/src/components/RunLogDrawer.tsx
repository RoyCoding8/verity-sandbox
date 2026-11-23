import { useMemo, useState, type CSSProperties } from "react";
import {
  classifyRunLogKind,
  parseRunLogLines,
  type RunLogCategory,
  type RunLogEntry,
  type RunLogFilter,
} from "../types/verity";

export interface RunLogDrawerProps {
  entries?: readonly RunLogEntry[];
  rawLines?: readonly string[];
  open?: boolean;
  onSelectTask?: (taskId: string) => void;
  onClose?: () => void;
}

const FILTERS: readonly RunLogFilter[] = ["All", "Merged", "Handoff", "ReadOnly"];

function categoryBadgeStyle(category: RunLogCategory): {
  background: string;
  color: string;
} {
  switch (category) {
    case "Merged":
      return {
        background: "var(--md-sys-color-tertiary-container)",
        color: "var(--md-sys-color-on-tertiary-container)",
      };
    case "Handoff":
      return {
        background: "var(--md-sys-color-primary-container)",
        color: "var(--md-sys-color-on-primary-container)",
      };
    case "ReadOnly":
      return {
        background: "var(--md-sys-color-secondary-container)",
        color: "var(--md-sys-color-on-secondary-container)",
      };
    case "Other":
      return {
        background: "var(--md-sys-color-surface-container-high)",
        color: "var(--md-sys-color-on-surface-variant)",
      };
    default: {
      const _exhaustive: never = category;
      return _exhaustive;
    }
  }
}

const styles: Record<string, CSSProperties> = {
  drawer: {
    display: "flex",
    flexDirection: "column",
    gap: 14,
    background: "var(--md-sys-color-surface)",
    color: "var(--md-sys-color-on-surface)",
    border: "1px solid var(--md-sys-color-outline-variant)",
    borderRadius: "var(--md-sys-shape-corner-lg)",
    padding: 18,
  },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
  },
  titleGroup: {
    display: "flex",
    flexDirection: "column",
    gap: 2,
  },
  title: {
    margin: 0,
    fontSize: 17,
    fontWeight: 600,
  },
  subtitle: {
    margin: 0,
    fontSize: 12,
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    color: "var(--md-sys-color-on-surface-variant)",
  },
  closeBtn: {
    border: "1px solid var(--md-sys-color-outline-variant)",
    background: "transparent",
    color: "var(--md-sys-color-on-surface-variant)",
    borderRadius: "var(--md-sys-shape-corner-full)",
    padding: "6px 12px",
    fontSize: 12,
    fontWeight: 600,
    cursor: "pointer",
  },
  controls: {
    display: "flex",
    flexDirection: "column",
    gap: 10,
  },
  searchInput: {
    width: "100%",
    padding: "9px 14px",
    borderRadius: "var(--md-sys-shape-corner-full)",
    border: "1px solid var(--md-sys-color-outline-variant)",
    background: "var(--md-sys-color-surface-container-low)",
    color: "var(--md-sys-color-on-surface)",
    fontSize: 13,
    fontFamily: "inherit",
  },
  chipRow: {
    display: "flex",
    flexWrap: "wrap",
    gap: 8,
  },
  filterChip: {
    border: "1px solid var(--md-sys-color-outline-variant)",
    borderRadius: "var(--md-sys-shape-corner-full)",
    padding: "5px 12px",
    fontSize: 12,
    fontWeight: 600,
    cursor: "pointer",
    background: "var(--md-sys-color-surface-container-low)",
    color: "var(--md-sys-color-on-surface-variant)",
  },
  filterChipActive: {
    background: "var(--md-sys-color-secondary-container)",
    color: "var(--md-sys-color-on-secondary-container)",
    borderColor: "transparent",
  },
  list: {
    display: "flex",
    flexDirection: "column",
    gap: 8,
    maxHeight: 460,
    overflowY: "auto",
  },
  receiptCard: {
    background: "var(--md-sys-color-surface-container)",
    borderRadius: "var(--md-sys-shape-corner-md)",
    padding: "11px 13px",
    display: "flex",
    flexDirection: "column",
    gap: 6,
  },
  receiptTop: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    flexWrap: "wrap",
  },
  receiptMeta: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    flexWrap: "wrap",
  },
  taskBtn: {
    border: "none",
    background: "transparent",
    padding: 0,
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 13,
    fontWeight: 600,
    color: "var(--md-sys-color-primary)",
    cursor: "pointer",
    textDecoration: "underline",
  },
  taskPlain: {
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 13,
    fontWeight: 600,
    color: "var(--md-sys-color-on-surface)",
  },
  kindBadge: {
    display: "inline-flex",
    alignItems: "center",
    padding: "2px 9px",
    borderRadius: "var(--md-sys-shape-corner-full)",
    fontSize: 11,
    fontWeight: 600,
  },
  timestamp: {
    fontSize: 11,
    color: "var(--md-sys-color-on-surface-variant)",
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
  },
  detail: {
    margin: 0,
    fontSize: 12.5,
    lineHeight: 1.45,
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    background: "var(--md-sys-color-surface-container-low)",
    color: "var(--md-sys-color-on-surface)",
    borderRadius: 8,
    padding: "8px 10px",
    overflowX: "auto",
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  },
  emptyState: {
    padding: 24,
    textAlign: "center",
    fontSize: 13,
    color: "var(--md-sys-color-on-surface-variant)",
    background: "var(--md-sys-color-surface-container)",
    borderRadius: "var(--md-sys-shape-corner-md)",
  },
};

export function RunLogDrawer({
  entries,
  rawLines,
  open = true,
  onSelectTask,
  onClose,
}: RunLogDrawerProps) {
  const [query, setQuery] = useState("");
  const [activeFilter, setActiveFilter] = useState<RunLogFilter>("All");

  const resolvedEntries = useMemo<readonly RunLogEntry[]>(() => {
    if (entries !== undefined) {
      return entries;
    }
    if (rawLines !== undefined) {
      return parseRunLogLines(rawLines);
    }
    return [];
  }, [entries, rawLines]);

  const filterCounts = useMemo(() => {
    const counts: Record<RunLogFilter, number> = {
      All: resolvedEntries.length,
      Merged: 0,
      Handoff: 0,
      ReadOnly: 0,
    };
    for (const entry of resolvedEntries) {
      const cat = classifyRunLogKind(entry.kind);
      if (cat === "Merged" || cat === "Handoff" || cat === "ReadOnly") {
        counts[cat] += 1;
      }
    }
    return counts;
  }, [resolvedEntries]);

  const filteredEntries = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return resolvedEntries.filter((entry) => {
      if (
        activeFilter !== "All" &&
        classifyRunLogKind(entry.kind) !== activeFilter
      ) {
        return false;
      }
      if (needle.length === 0) {
        return true;
      }
      return (
        entry.task_id.toLowerCase().includes(needle) ||
        entry.kind.toLowerCase().includes(needle) ||
        entry.detail.toLowerCase().includes(needle)
      );
    });
  }, [resolvedEntries, activeFilter, query]);

  if (!open) {
    return null;
  }

  return (
    <section style={styles.drawer} aria-label="Run log drawer">
      <header style={styles.header}>
        <div style={styles.titleGroup}>
          <h2 style={styles.title}>Run Log Receipts</h2>
          <p style={styles.subtitle}>.verity/run-log.jsonl</p>
        </div>
        {onClose && (
          <button type="button" style={styles.closeBtn} onClick={onClose}>
            Close
          </button>
        )}
      </header>

      <div style={styles.controls}>
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search by task ID, event kind, digest, or detail…"
          aria-label="Search run log receipts"
          style={styles.searchInput}
        />

        <div style={styles.chipRow} role="group" aria-label="Run log filters">
          {FILTERS.map((filter) => {
            const active = filter === activeFilter;
            return (
              <button
                key={filter}
                type="button"
                aria-pressed={active}
                onClick={() => setActiveFilter(filter)}
                style={{
                  ...styles.filterChip,
                  ...(active ? styles.filterChipActive : undefined),
                }}
              >
                {filter} ({filterCounts[filter]})
              </button>
            );
          })}
        </div>
      </div>

      {filteredEntries.length === 0 ? (
        <div style={styles.emptyState}>
          {resolvedEntries.length === 0
            ? "No receipts recorded in .verity/run-log.jsonl yet."
            : "No receipts match the current filter and search query."}
        </div>
      ) : (
        <div style={styles.list}>
          {filteredEntries.map((entry, idx) => {
            const category = classifyRunLogKind(entry.kind);
            const badge = categoryBadgeStyle(category);
            const key = `${entry.lineNumber ?? idx}-${entry.task_id}-${entry.kind}`;

            return (
              <article key={key} style={styles.receiptCard}>
                <div style={styles.receiptTop}>
                  <div style={styles.receiptMeta}>
                    {onSelectTask ? (
                      <button
                        type="button"
                        style={styles.taskBtn}
                        onClick={() => onSelectTask(entry.task_id)}
                      >
                        {entry.task_id}
                      </button>
                    ) : (
                      <span style={styles.taskPlain}>{entry.task_id}</span>
                    )}

                    <span
                      style={{
                        ...styles.kindBadge,
                        background: badge.background,
                        color: badge.color,
                      }}
                    >
                      {entry.kind}
                    </span>
                  </div>

                  {entry.timestamp ? (
                    <span style={styles.timestamp}>{entry.timestamp}</span>
                  ) : entry.lineNumber !== undefined ? (
                    <span style={styles.timestamp}>#{entry.lineNumber}</span>
                  ) : null}
                </div>

                <pre style={styles.detail}>{entry.detail}</pre>
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}
