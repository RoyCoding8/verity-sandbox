import { useEffect, type CSSProperties } from "react";
import {
  digestsMatch,
  formatVerifierLabel,
  shortenDigest,
  type MergeQueueEntry,
} from "../types/verity";

export interface MergeQueueModalProps {
  open: boolean;
  entries: readonly MergeQueueEntry[];
  onTriggerMerge: (entry: MergeQueueEntry) => void | Promise<void>;
  onInspectSpec?: (taskId: string) => void;
  onClose: () => void;
}

const styles: Record<string, CSSProperties> = {
  backdrop: {
    position: "fixed",
    inset: 0,
    background: "rgba(0, 0, 0, 0.48)",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
    zIndex: 1000,
  },
  dialog: {
    width: "100%",
    maxWidth: 720,
    maxHeight: "84vh",
    display: "flex",
    flexDirection: "column",
    background: "var(--md-sys-color-surface-container)",
    color: "var(--md-sys-color-on-surface)",
    borderRadius: "var(--md-sys-shape-corner-lg)",
    border: "1px solid var(--md-sys-color-outline-variant)",
    overflow: "hidden",
    boxShadow: "0 12px 32px rgba(0, 0, 0, 0.28)",
  },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    padding: "18px 22px",
    borderBottom: "1px solid var(--md-sys-color-outline-variant)",
  },
  title: {
    margin: 0,
    fontSize: 18,
    fontWeight: 600,
  },
  subtitle: {
    margin: "2px 0 0",
    fontSize: 12,
    color: "var(--md-sys-color-on-surface-variant)",
  },
  closeBtn: {
    border: "1px solid var(--md-sys-color-outline-variant)",
    background: "transparent",
    color: "var(--md-sys-color-on-surface-variant)",
    borderRadius: "var(--md-sys-shape-corner-full)",
    padding: "6px 14px",
    fontSize: 12,
    fontWeight: 600,
    cursor: "pointer",
  },
  body: {
    padding: "16px 22px 22px",
    overflowY: "auto",
    display: "flex",
    flexDirection: "column",
    gap: 12,
  },
  entryCard: {
    background: "var(--md-sys-color-surface-container-low)",
    border: "1px solid var(--md-sys-color-outline-variant)",
    borderRadius: "var(--md-sys-shape-corner-md)",
    padding: 16,
    display: "flex",
    flexDirection: "column",
    gap: 10,
  },
  entryTop: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
    flexWrap: "wrap",
  },
  taskId: {
    fontSize: 14,
    fontWeight: 600,
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
  },
  chipRow: {
    display: "flex",
    flexWrap: "wrap",
    gap: 6,
    alignItems: "center",
  },
  chip: {
    display: "inline-flex",
    alignItems: "center",
    padding: "3px 10px",
    borderRadius: "var(--md-sys-shape-corner-full)",
    fontSize: 11.5,
    fontWeight: 600,
  },
  metaGrid: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
    gap: 8,
    fontSize: 12,
    color: "var(--md-sys-color-on-surface-variant)",
  },
  mono: {
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    color: "var(--md-sys-color-on-surface)",
  },
  conflictBox: {
    background: "var(--md-sys-color-error-container)",
    color: "var(--md-sys-color-on-error-container)",
    borderRadius: 10,
    padding: "9px 12px",
    fontSize: 12,
    display: "flex",
    flexDirection: "column",
    gap: 4,
  },
  actionRow: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
    flexWrap: "wrap",
    paddingTop: 6,
  },
  secondaryBtn: {
    border: "1px solid var(--md-sys-color-outline-variant)",
    background: "transparent",
    color: "var(--md-sys-color-on-surface)",
    borderRadius: "var(--md-sys-shape-corner-full)",
    padding: "7px 14px",
    fontSize: 12,
    fontWeight: 600,
    cursor: "pointer",
  },
  mergeBtn: {
    border: "none",
    background: "var(--md-sys-color-primary)",
    color: "var(--md-sys-color-on-primary)",
    borderRadius: "var(--md-sys-shape-corner-full)",
    padding: "8px 18px",
    fontSize: 12.5,
    fontWeight: 600,
    cursor: "pointer",
    marginLeft: "auto",
  },
  emptyState: {
    padding: 28,
    textAlign: "center",
    color: "var(--md-sys-color-on-surface-variant)",
    fontSize: 13,
  },
};

function mergeTreeBadge(entry: MergeQueueEntry): {
  label: string;
  background: string;
  color: string;
} {
  switch (entry.mergeTree.kind) {
    case "clean":
      return {
        label: "merge-tree: clean",
        background: "var(--md-sys-color-tertiary-container)",
        color: "var(--md-sys-color-on-tertiary-container)",
      };
    case "conflict":
      return {
        label: `merge-tree: ${entry.mergeTree.conflictedPaths.length} conflict${entry.mergeTree.conflictedPaths.length === 1 ? "" : "s"}`,
        background: "var(--md-sys-color-error-container)",
        color: "var(--md-sys-color-on-error-container)",
      };
    case "unchecked":
      return {
        label: "merge-tree: unchecked",
        background: "var(--md-sys-color-surface-container-high)",
        color: "var(--md-sys-color-on-surface-variant)",
      };
    default: {
      const _exhaustive: never = entry.mergeTree;
      return _exhaustive;
    }
  }
}

export function MergeQueueModal({
  open,
  entries,
  onTriggerMerge,
  onInspectSpec,
  onClose,
}: MergeQueueModalProps) {
  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, onClose]);

  if (!open) {
    return null;
  }

  return (
    <div
      style={styles.backdrop}
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="verity-merge-queue-title"
        style={styles.dialog}
      >
        <header style={styles.header}>
          <div>
            <h2 id="verity-merge-queue-title" style={styles.title}>
              Merge Queue
            </h2>
            <p style={styles.subtitle}>
              Approved write worktrees verified for lock-free{" "}
              <code>git merge-tree</code> + CAS ref integration.
            </p>
          </div>
          <button type="button" style={styles.closeBtn} onClick={onClose}>
            Close
          </button>
        </header>

        <div style={styles.body}>
          {entries.length === 0 ? (
            <div style={styles.emptyState}>
              No approved write worktrees are queued for merge. Approve a
              passing handoff in Spec Review first.
            </div>
          ) : (
            entries.map((entry) => {
              const digestOk = digestsMatch(
                entry.handoff.spec_digest,
                entry.approvedDigest
              );
              const treeBadge = mergeTreeBadge(entry);
              const hasConflicts = entry.mergeTree.kind === "conflict";
              const isMerged = entry.mergeState === "merged";
              const isMerging = entry.mergeState === "merging";
              const canMerge =
                digestOk && !hasConflicts && !isMerged && !isMerging;

              return (
                <article key={entry.taskId} style={styles.entryCard}>
                  <div style={styles.entryTop}>
                    <span style={styles.taskId}>{entry.taskId}</span>
                    <div style={styles.chipRow}>
                      <span
                        style={{
                          ...styles.chip,
                          background: "var(--md-sys-color-primary-container)",
                          color: "var(--md-sys-color-on-primary-container)",
                        }}
                      >
                        {formatVerifierLabel(entry.handoff.verifier)}
                      </span>

                      <span
                        style={{
                          ...styles.chip,
                          background: digestOk
                            ? "var(--md-sys-color-tertiary-container)"
                            : "var(--md-sys-color-error-container)",
                          color: digestOk
                            ? "var(--md-sys-color-on-tertiary-container)"
                            : "var(--md-sys-color-on-error-container)",
                        }}
                      >
                        {digestOk ? "Digest Match" : "Digest Mismatch"}
                      </span>

                      <span
                        style={{
                          ...styles.chip,
                          background: treeBadge.background,
                          color: treeBadge.color,
                        }}
                      >
                        {treeBadge.label}
                      </span>
                    </div>
                  </div>

                  <div style={styles.metaGrid}>
                    <div>
                      Target ref:{" "}
                      <span style={styles.mono}>{entry.targetRef}</span>
                    </div>
                    <div>
                      SpecDigest:{" "}
                      <span style={styles.mono}>
                        {shortenDigest(entry.handoff.spec_digest)}
                      </span>
                    </div>
                    <div>
                      Changed paths:{" "}
                      <span style={styles.mono}>
                        {entry.handoff.changed_paths.length}
                      </span>
                    </div>
                    <div>
                      Worktree:{" "}
                      <span style={styles.mono}>{entry.worktreePath}</span>
                    </div>
                  </div>

                  {entry.mergeTree.kind === "conflict" && (
                    <div style={styles.conflictBox} role="alert">
                      <strong>git merge-tree conflicts detected:</strong>
                      <code>{entry.mergeTree.conflictedPaths.join(", ")}</code>
                    </div>
                  )}

                  {!digestOk && (
                    <div style={styles.conflictBox} role="alert">
                      <strong>SpecDigest mismatch:</strong> Approved digest{" "}
                      <code>{shortenDigest(entry.approvedDigest)}</code> does
                      not match persisted handoff{" "}
                      <code>{shortenDigest(entry.handoff.spec_digest)}</code>.
                    </div>
                  )}

                  {entry.errorMessage && (
                    <div style={styles.conflictBox} role="alert">
                      {entry.errorMessage}
                    </div>
                  )}

                  {isMerged && entry.mergedCommit && (
                    <div
                      style={{
                        fontSize: 12,
                        color: "var(--md-sys-color-tertiary)",
                      }}
                    >
                      {entry.alreadyMerged ? "Already merged at " : "Merged at "}
                      <code style={styles.mono}>
                        {shortenDigest(entry.mergedCommit, 12, 0)}
                      </code>
                    </div>
                  )}

                  <div style={styles.actionRow}>
                    {onInspectSpec && (
                      <button
                        type="button"
                        style={styles.secondaryBtn}
                        onClick={() => onInspectSpec(entry.taskId)}
                      >
                        Inspect Spec
                      </button>
                    )}

                    <button
                      type="button"
                      disabled={!canMerge}
                      style={{
                        ...styles.mergeBtn,
                        opacity: canMerge ? 1 : 0.45,
                        cursor: canMerge ? "pointer" : "not-allowed",
                      }}
                      onClick={() => {
                        void onTriggerMerge(entry);
                      }}
                    >
                      {isMerged
                        ? "Merged"
                        : isMerging
                          ? "Merging…"
                          : "CAS Merge"}
                    </button>
                  </div>
                </article>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}
