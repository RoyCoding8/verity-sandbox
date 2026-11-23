import { useEffect, useMemo, useState } from "react";
import type { ForkCandidate } from "../api/tauri";
import { ClockIcon, CloseIcon, GitForkIcon, SearchIcon } from "./Icons";
import "./ModalsOverlay.css";

export interface ForkModalProps {
  readonly open: boolean;
  readonly candidates: readonly ForkCandidate[];
  readonly onFork: (entryId: string) => void;
  readonly onClose: () => void;
}

export function ForkModal({
  open,
  candidates,
  onFork,
  onClose,
}: ForkModalProps) {
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (!open) {
      setQuery("");
      return;
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  const trimmedQuery = query.trim();

  const filtered = useMemo(() => {
    const q = trimmedQuery.toLowerCase();
    if (!q) return candidates;
    return candidates.filter(
      (c) =>
        c.preview.toLowerCase().includes(q) ||
        c.entryId.toLowerCase().includes(q) ||
        c.role.toLowerCase().includes(q)
    );
  }, [candidates, trimmedQuery]);

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
        <header className="md3-dialog-header">
          <div className="vo-dialog-heading">
            <span className="vo-dialog-icon" aria-hidden="true">
              <GitForkIcon size={17} />
            </span>
            <div className="vo-dialog-header-text">
              <h2 id="fork-modal-title" className="md3-dialog-title">
                Fork Session
              </h2>
              <p className="md3-dialog-subtitle">
                Choose a message checkpoint to branch into a new independent
                session.
              </p>
            </div>
          </div>
          <button
            type="button"
            className="md3-icon-btn"
            aria-label="Close fork dialog"
            onClick={onClose}
          >
            <CloseIcon size={16} />
          </button>
        </header>

        <div className="md3-dialog-body">
          <div className="vo-search-row">
            <div className="vo-search-wrap">
              <span className="vo-search-icon" aria-hidden="true">
                <SearchIcon size={15} />
              </span>
              <input
                type="search"
                className="md3-input vo-search-input"
                aria-label="Filter fork checkpoints"
                placeholder="Search messages to fork from..."
                value={query}
                autoFocus
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            {candidates.length > 0 && (
              <span className="vo-search-count">
                {trimmedQuery
                  ? `${filtered.length} of ${candidates.length}`
                  : `${candidates.length} checkpoint${candidates.length === 1 ? "" : "s"}`}
              </span>
            )}
          </div>

          {candidates.length === 0 ? (
            <div className="vo-empty-state">
              <span className="vo-empty-state-icon" aria-hidden="true">
                <GitForkIcon size={18} />
              </span>
              <p className="vo-empty-state-title">No messages yet</p>
              <p className="vo-empty-state-desc">
                Send a message in this session first to create a checkpoint you
                can fork from.
              </p>
            </div>
          ) : filtered.length === 0 ? (
            <div className="vo-empty-state">
              <span className="vo-empty-state-icon" aria-hidden="true">
                <SearchIcon size={18} />
              </span>
              <p className="vo-empty-state-title">
                No messages match your search
              </p>
              <p className="vo-empty-state-desc">
                No checkpoints matched "{trimmedQuery}". Try a different keyword
                or clear the filter.
              </p>
              <button
                type="button"
                className="md3-btn-outlined"
                onClick={() => setQuery("")}
              >
                Clear search
              </button>
            </div>
          ) : (
            <div className="pi-fork-list">
              {filtered.map((candidate) => (
                <div key={candidate.entryId} className="pi-fork-card">
                  <div className="vo-fork-card-body">
                    <div className="vo-meta-row">
                      <span
                        className={`verity-badge ${
                          candidate.role === "user"
                            ? "verity-badge--info"
                            : "verity-badge--neutral"
                        }`}
                      >
                        {candidate.role}
                      </span>
                      <code className="vo-entry-id">{candidate.entryId}</code>
                      <span className="vo-timestamp">
                        <ClockIcon size={12} />
                        {candidate.timestamp}
                      </span>
                    </div>
                    <div className="vo-preview-text">{candidate.preview}</div>
                  </div>
                  <button
                    type="button"
                    className="md3-btn-filled"
                    onClick={() => onFork(candidate.entryId)}
                  >
                    <GitForkIcon size={14} />
                    Fork here
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        <footer className="md3-dialog-footer">
          <button type="button" className="md3-btn-text" onClick={onClose}>
            Cancel
          </button>
        </footer>
      </div>
    </div>
  );
}
