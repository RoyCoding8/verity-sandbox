import { useEffect, useMemo, useState } from "react";
import type { HistoryTreeNode } from "../api/tauri";

export interface HistoryTreeModalProps {
  readonly open: boolean;
  readonly nodes: readonly HistoryTreeNode[];
  readonly onForkFromNode: (entryId: string) => void;
  readonly onClose: () => void;
}

export function HistoryTreeModal({
  open,
  nodes,
  onForkFromNode,
  onClose,
}: HistoryTreeModalProps) {
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
    if (!q) return nodes;
    return nodes.filter(
      (n) =>
        n.preview.toLowerCase().includes(q) ||
        n.id.toLowerCase().includes(q) ||
        n.role.toLowerCase().includes(q)
    );
  }, [nodes, query]);

  if (!open) return null;

  return (
    <div className="md3-scrim" onClick={onClose}>
      <div
        className="md3-dialog md3-dialog-wide"
        role="dialog"
        aria-modal="true"
        aria-labelledby="history-tree-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="md3-dialog-header">
          <div>
            <h2 id="history-tree-title" className="md3-dialog-title">
              Session History Tree
            </h2>
            <p className="md3-dialog-subtitle">
              Search turns across branches and fork cleanly from any checkpoint.
            </p>
          </div>
          <button
            type="button"
            className="md3-icon-btn"
            aria-label="Close history tree"
            onClick={onClose}
          >
            ×
          </button>
        </div>

        <div className="md3-dialog-body">
          <input
            type="search"
            className="md3-input"
            aria-label="Search history tree"
            placeholder="Filter by text, entry id, or role..."
            value={query}
            autoFocus
            onChange={(e) => setQuery(e.target.value)}
          />

          {filtered.length === 0 ? (
            <p style={{ color: "var(--md-sys-color-on-surface-variant)", margin: "12px 0" }}>
              No matching history entries.
            </p>
          ) : (
            <div className="pi-tree-list">
              {filtered.map((node) => (
                <div
                  key={node.id}
                  className={`pi-tree-node ${node.isActiveBranch ? "active" : ""}`}
                  style={{ marginLeft: Math.min(node.depth, 6) * 14 }}
                >
                  <div className="pi-tree-node-content">
                    <div>
                      <span
                        className={`chip ${
                          node.role === "user" ? "chip-info" : "chip-neutral"
                        }`}
                      >
                        {node.role}
                      </span>
                      <code style={{ fontSize: 11, opacity: 0.75 }}>{node.id}</code>
                      <span style={{ fontSize: 11, marginLeft: 8, opacity: 0.7 }}>
                        {node.timestamp}
                      </span>
                    </div>
                    <div className="pi-tree-node-preview">{node.preview}</div>
                  </div>
                  <button
                    type="button"
                    className="md3-btn-tonal"
                    onClick={() => onForkFromNode(node.id)}
                  >
                    Fork
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="md3-dialog-footer">
          <button type="button" className="md3-btn-text" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
