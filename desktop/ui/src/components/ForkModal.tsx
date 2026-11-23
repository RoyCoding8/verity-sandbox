import { useEffect, useMemo, useState } from "react";
import type { ForkCandidate } from "../api/tauri";

export interface ForkModalProps {
  readonly open: boolean;
  readonly candidates: readonly ForkCandidate[];
  readonly onFork: (entryId: string) => void;
  readonly onClose: () => void;
}

export function ForkModal({ open, candidates, onFork, onClose }: ForkModalProps) {
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return candidates;
    return candidates.filter(
      (c) =>
        c.preview.toLowerCase().includes(q) ||
        c.entryId.toLowerCase().includes(q) ||
        c.role.toLowerCase().includes(q)
    );
  }, [candidates, query]);

  if (!open) return null;

  return (
    <div className="md3-scrim" onClick={onClose}>
      <div
        className="md3-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="fork-modal-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="md3-dialog-header">
          <div>
            <h2 id="fork-modal-title" className="md3-dialog-title">
              Fork Session
            </h2>
            <p className="md3-dialog-subtitle">
              Choose a message checkpoint to branch into a new independent session.
            </p>
          </div>
          <button
            type="button"
            className="md3-icon-btn"
            aria-label="Close fork dialog"
            onClick={onClose}
          >
            ×
          </button>
        </div>

        <div className="md3-dialog-body">
          <input
            type="search"
            className="md3-input"
            aria-label="Filter fork checkpoints"
            placeholder="Search messages to fork from..."
            value={query}
            autoFocus
            onChange={(e) => setQuery(e.target.value)}
          />

          {filtered.length === 0 ? (
            <p style={{ color: "var(--md-sys-color-on-surface-variant)", margin: "12px 0" }}>
              No forkable messages in this session yet.
            </p>
          ) : (
            <div className="pi-fork-list">
              {filtered.map((candidate) => (
                <div key={candidate.entryId} className="pi-fork-card">
                  <div className="pi-tree-node-content">
                    <div>
                      <span
                        className={`chip ${
                          candidate.role === "user" ? "chip-info" : "chip-neutral"
                        }`}
                      >
                        {candidate.role}
                      </span>
                      <code style={{ fontSize: 11, opacity: 0.75 }}>{candidate.entryId}</code>
                      <span style={{ fontSize: 11, marginLeft: 8, opacity: 0.7 }}>
                        {candidate.timestamp}
                      </span>
                    </div>
                    <div className="pi-tree-node-preview">{candidate.preview}</div>
                  </div>
                  <button
                    type="button"
                    className="md3-btn-filled"
                    onClick={() => onFork(candidate.entryId)}
                  >
                    Fork here
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="md3-dialog-footer">
          <button type="button" className="md3-btn-text" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
