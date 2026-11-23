import { useEffect } from "react";
import {
  digestsMatch,
  formatVerifierLabel,
  shortenDigest,
  type MergeQueueEntry,
} from "../types/verity";
import {
  AlertIcon,
  CheckIcon,
  CloseIcon,
  GitMergeIcon,
  InfoIcon,
  ShieldCheckIcon,
  XCircleIcon,
} from "./Icons";
import "./ModalsOverlay.css";

export interface MergeQueueModalProps {
  open: boolean;
  entries: readonly MergeQueueEntry[];
  onTriggerMerge: (entry: MergeQueueEntry) => void | Promise<void>;
  onInspectSpec?: (taskId: string) => void;
  onClose: () => void;
}

function mergeTreeBadge(entry: MergeQueueEntry): {
  readonly label: string;
  readonly tone: "success" | "danger" | "neutral";
} {
  switch (entry.mergeTree.kind) {
    case "clean":
      return {
        label: "merge-tree: clean",
        tone: "success",
      };
    case "conflict": {
      const count = entry.mergeTree.conflictedPaths.length;
      return {
        label: `merge-tree: ${count} conflict${count === 1 ? "" : "s"}`,
        tone: "danger",
      };
    }
    case "unchecked":
      return {
        label: "merge-tree: unchecked",
        tone: "neutral",
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
      className="md3-scrim"
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        className="md3-dialog md3-dialog-wide"
        role="dialog"
        aria-modal="true"
        aria-labelledby="verity-merge-queue-title"
      >
        <header className="md3-dialog-header">
          <div className="vo-dialog-heading">
            <span className="vo-dialog-icon" aria-hidden="true">
              <GitMergeIcon size={17} />
            </span>
            <div className="vo-dialog-header-text">
              <h2 id="verity-merge-queue-title" className="md3-dialog-title">
                Merge Queue
              </h2>
              <p className="md3-dialog-subtitle">
                Approved write worktrees verified for lock-free{" "}
                <code className="vo-dialog-subtitle-code">git merge-tree</code>{" "}
                + CAS ref integration.
              </p>
            </div>
          </div>
          <button
            type="button"
            className="md3-icon-btn"
            aria-label="Close merge queue"
            onClick={onClose}
          >
            <CloseIcon size={16} />
          </button>
        </header>

        <div className="md3-dialog-body">
          {entries.length === 0 ? (
            <div className="vo-empty-state">
              <span className="vo-empty-state-icon" aria-hidden="true">
                <GitMergeIcon size={18} />
              </span>
              <p className="vo-empty-state-title">Merge queue is empty</p>
              <p className="vo-empty-state-desc">
                No approved write worktrees are queued for merge. Approve a
                passing handoff in Spec Review first.
              </p>
            </div>
          ) : (
            <div className="vo-merge-list">
              {entries.map((entry) => {
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

                const cardStateClass = isMerged
                  ? " vo-merge-card--merged"
                  : !digestOk || hasConflicts
                    ? " vo-merge-card--blocked"
                    : "";

                return (
                  <article
                    key={entry.taskId}
                    className={`vo-merge-card${cardStateClass}`}
                  >
                    <div className="vo-merge-card-top">
                      <span className="vo-merge-task-id">{entry.taskId}</span>
                      <div className="vo-badge-row">
                        <span className="verity-badge verity-badge--info">
                          {formatVerifierLabel(entry.handoff.verifier)}
                        </span>

                        <span
                          className={`verity-badge ${
                            digestOk
                              ? "verity-badge--success"
                              : "verity-badge--danger"
                          }`}
                        >
                          {digestOk ? (
                            <ShieldCheckIcon size={12} />
                          ) : (
                            <XCircleIcon size={12} />
                          )}
                          {digestOk ? "Digest Match" : "Digest Mismatch"}
                        </span>

                        <span
                          className={`verity-badge verity-badge--${treeBadge.tone}`}
                        >
                          {treeBadge.tone === "success" ? (
                            <CheckIcon size={12} />
                          ) : treeBadge.tone === "danger" ? (
                            <XCircleIcon size={12} />
                          ) : (
                            <InfoIcon size={12} />
                          )}
                          {treeBadge.label}
                        </span>
                      </div>
                    </div>

                    <div className="vo-merge-meta">
                      <div className="vo-merge-meta-item">
                        <span className="vo-merge-meta-label">Target ref:</span>
                        <span
                          className="vo-merge-meta-value"
                          title={entry.targetRef}
                        >
                          {entry.targetRef}
                        </span>
                      </div>
                      <div className="vo-merge-meta-item">
                        <span className="vo-merge-meta-label">SpecDigest:</span>
                        <span
                          className="vo-merge-meta-value"
                          title={entry.handoff.spec_digest}
                        >
                          {shortenDigest(entry.handoff.spec_digest)}
                        </span>
                      </div>
                      <div className="vo-merge-meta-item">
                        <span className="vo-merge-meta-label">
                          Changed paths:
                        </span>
                        <span className="vo-merge-meta-value">
                          {entry.handoff.changed_paths.length}
                        </span>
                      </div>
                      <div className="vo-merge-meta-item">
                        <span className="vo-merge-meta-label">Worktree:</span>
                        <span
                          className="vo-merge-meta-value"
                          title={entry.worktreePath}
                        >
                          {entry.worktreePath}
                        </span>
                      </div>
                    </div>

                    {entry.mergeTree.kind === "conflict" && (
                      <div className="vo-alert-box" role="alert">
                        <AlertIcon size={15} className="vo-alert-icon" />
                        <div className="vo-alert-content">
                          <strong>git merge-tree conflicts detected:</strong>
                          <code>
                            {entry.mergeTree.conflictedPaths.join(", ")}
                          </code>
                        </div>
                      </div>
                    )}

                    {!digestOk && (
                      <div className="vo-alert-box" role="alert">
                        <AlertIcon size={15} className="vo-alert-icon" />
                        <div className="vo-alert-content">
                          <span>
                            <strong>SpecDigest mismatch:</strong> Approved
                            digest{" "}
                            <code>{shortenDigest(entry.approvedDigest)}</code>{" "}
                            does not match persisted handoff{" "}
                            <code>
                              {shortenDigest(entry.handoff.spec_digest)}
                            </code>
                            .
                          </span>
                        </div>
                      </div>
                    )}

                    {entry.errorMessage && (
                      <div className="vo-alert-box" role="alert">
                        <AlertIcon size={15} className="vo-alert-icon" />
                        <div className="vo-alert-content">
                          <span>{entry.errorMessage}</span>
                        </div>
                      </div>
                    )}

                    {isMerged && entry.mergedCommit && (
                      <div className="vo-merged-notice">
                        <CheckIcon size={13} />
                        <span>
                          {entry.alreadyMerged
                            ? "Already merged at "
                            : "Merged at "}
                          <code>{shortenDigest(entry.mergedCommit, 12, 0)}</code>
                        </span>
                      </div>
                    )}

                    <div className="vo-merge-actions">
                      {onInspectSpec && (
                        <button
                          type="button"
                          className="md3-btn-outlined"
                          onClick={() => onInspectSpec(entry.taskId)}
                        >
                          Inspect Spec
                        </button>
                      )}

                      <div className="vo-merge-actions-right">
                        <button
                          type="button"
                          className="md3-btn-filled"
                          disabled={!canMerge}
                          onClick={() => {
                            void onTriggerMerge(entry);
                          }}
                        >
                          {isMerged ? (
                            <CheckIcon size={14} />
                          ) : (
                            <GitMergeIcon size={14} />
                          )}
                          {isMerged
                            ? "Merged"
                            : isMerging
                              ? "Merging..."
                              : "CAS Merge"}
                        </button>
                      </div>
                    </div>
                  </article>
                );
              })}
            </div>
          )}
        </div>

        <footer className="md3-dialog-footer">
          <button type="button" className="md3-btn-text" onClick={onClose}>
            Close
          </button>
        </footer>
      </div>
    </div>
  );
}
