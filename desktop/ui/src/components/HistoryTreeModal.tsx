import { useEffect, useMemo, useState } from "react";
import type { HistoryTreeNode } from "../api/tauri";
import {
  AlertIcon,
  ClockIcon,
  CloseIcon,
  GitBranchIcon,
  GitForkIcon,
  SearchIcon,
} from "./Icons";
import "./ModalsOverlay.css";

export interface HistoryTreeModalProps {
  readonly open: boolean;
  readonly nodes: readonly HistoryTreeNode[];
  readonly onForkFromNode: (entryId: string) => void | Promise<void>;
  readonly onClose: () => void;
}

interface TreeDisplayRow {
  readonly node: HistoryTreeNode;
  readonly isContextOnly: boolean;
  readonly parentPreview: string | null;
  readonly guides: readonly ("line" | "elbow" | "elbow-continues" | "empty")[];
}

const MAX_GUIDE_DEPTH = 6;

function buildTreeRows(
  nodes: readonly HistoryTreeNode[],
  trimmedQuery: string
): { readonly rows: readonly TreeDisplayRow[]; readonly matchCount: number } {
  const nodeById = new Map<string, HistoryTreeNode>();
  for (const node of nodes) {
    nodeById.set(node.id, node);
  }

  const q = trimmedQuery.toLowerCase();
  let visibleNodes: readonly HistoryTreeNode[];
  let matchedIds: ReadonlySet<string> | null = null;

  if (!q) {
    visibleNodes = nodes;
  } else {
    const matches = new Set<string>();
    for (const node of nodes) {
      if (
        node.preview.toLowerCase().includes(q) ||
        node.id.toLowerCase().includes(q) ||
        node.role.toLowerCase().includes(q)
      ) {
        matches.add(node.id);
      }
    }
    if (matches.size === 0) {
      return { rows: [], matchCount: 0 };
    }

    // Keep ancestor chain visible so branch hierarchy remains readable while filtering
    const included = new Set<string>(matches);
    for (const id of matches) {
      let current = nodeById.get(id);
      const visited = new Set<string>([id]);
      while (current?.parentId && !visited.has(current.parentId)) {
        visited.add(current.parentId);
        if (nodeById.has(current.parentId)) {
          included.add(current.parentId);
          current = nodeById.get(current.parentId);
        } else {
          break;
        }
      }
    }

    matchedIds = matches;
    visibleNodes = nodes.filter((n) => included.has(n.id));
  }

  const rows: TreeDisplayRow[] = visibleNodes.map((node, index) => {
    const rawDepth = Math.max(node.depth, 0);
    const clampedDepth = Math.min(rawDepth, MAX_GUIDE_DEPTH);
    const guides: ("line" | "elbow" | "elbow-continues" | "empty")[] = [];

    for (let level = 1; level <= clampedDepth; level++) {
      const compareLevel = level === clampedDepth ? rawDepth : level;
      let hasFollowingAtLevel = false;
      for (let j = index + 1; j < visibleNodes.length; j++) {
        const nextRawDepth = Math.max(visibleNodes[j].depth, 0);
        if (nextRawDepth < compareLevel) {
          break;
        }
        if (nextRawDepth === compareLevel) {
          hasFollowingAtLevel = true;
          break;
        }
      }

      if (level === clampedDepth) {
        guides.push(hasFollowingAtLevel ? "elbow-continues" : "elbow");
      } else {
        guides.push(hasFollowingAtLevel ? "line" : "empty");
      }
    }

    const parentNode = node.parentId ? nodeById.get(node.parentId) : undefined;
    const parentPreview = parentNode
      ? `${parentNode.id} — ${parentNode.preview}`
      : node.parentId
        ? node.parentId
        : null;

    return {
      node,
      isContextOnly: matchedIds !== null && !matchedIds.has(node.id),
      parentPreview,
      guides,
    };
  });

  return {
    rows,
    matchCount: matchedIds ? matchedIds.size : nodes.length,
  };
}

export function HistoryTreeModal({
  open,
  nodes,
  onForkFromNode,
  onClose,
}: HistoryTreeModalProps) {
  const [query, setQuery] = useState("");
  const [forkingId, setForkingId] = useState<string | null>(null);
  const [forkError, setForkError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      setQuery("");
      setForkingId(null);
      setForkError(null);
      return;
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.defaultPrevented) return;
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  const trimmedQuery = query.trim();

  const { rows, matchCount } = useMemo(
    () => buildTreeRows(nodes, trimmedQuery),
    [nodes, trimmedQuery]
  );

  if (!open) return null;

  async function handleFork(entryId: string) {
    if (forkingId !== null) return;
    setForkingId(entryId);
    setForkError(null);
    try {
      await onForkFromNode(entryId);
    } catch (err: unknown) {
      setForkError(err instanceof Error ? err.message : String(err));
    } finally {
      setForkingId(null);
    }
  }

  return (
    <div className="md3-scrim" onClick={onClose}>
      <div
        className="md3-dialog md3-dialog-wide"
        role="dialog"
        aria-modal="true"
        aria-labelledby="history-tree-title"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="md3-dialog-header">
          <div className="vo-dialog-heading">
            <span className="vo-dialog-icon" aria-hidden="true">
              <GitBranchIcon size={17} />
            </span>
            <div className="vo-dialog-header-text">
              <h2 id="history-tree-title" className="md3-dialog-title">
                Session History Tree
              </h2>
              <p className="md3-dialog-subtitle">
                Search turns across branches and fork cleanly from any
                checkpoint.
              </p>
            </div>
          </div>
          <button
            type="button"
            className="md3-icon-btn"
            aria-label="Close history tree"
            onClick={onClose}
          >
            <CloseIcon size={16} />
          </button>
        </header>

        <div className="md3-dialog-body">
          {forkError && (
            <div className="md3-error-banner vo-error-banner" role="alert">
              <AlertIcon size={15} className="vo-error-banner-icon" />
              <span>{forkError}</span>
            </div>
          )}

          <div className="vo-search-row">
            <div className="vo-search-wrap">
              <span className="vo-search-icon" aria-hidden="true">
                <SearchIcon size={15} />
              </span>
              <input
                type="search"
                className="md3-input vo-search-input"
                aria-label="Search history tree"
                placeholder="Filter by text, entry id, or role..."
                value={query}
                autoFocus
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            {nodes.length > 0 && (
              <span className="vo-search-count">
                {trimmedQuery
                  ? `${matchCount} match${matchCount === 1 ? "" : "es"}`
                  : `${nodes.length} turn${nodes.length === 1 ? "" : "s"}`}
              </span>
            )}
          </div>

          {nodes.length === 0 ? (
            <div className="vo-empty-state">
              <span className="vo-empty-state-icon" aria-hidden="true">
                <GitBranchIcon size={18} />
              </span>
              <p className="vo-empty-state-title">No messages yet</p>
              <p className="vo-empty-state-desc">
                Start a conversation in this session to populate the branch
                history tree.
              </p>
            </div>
          ) : rows.length === 0 ? (
            <div className="vo-empty-state">
              <span className="vo-empty-state-icon" aria-hidden="true">
                <SearchIcon size={18} />
              </span>
              <p className="vo-empty-state-title">
                No messages match your search
              </p>
              <p className="vo-empty-state-desc">
                No history entries matched "{trimmedQuery}". Try a different
                query or clear the filter.
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
            <div className="vo-tree-list">
              {rows.map(({ node, isContextOnly, parentPreview, guides }) => {
                const isForkingNode = forkingId === node.id;
                const nodeCardClass = [
                  "vo-tree-node-card",
                  node.isActiveBranch ? "vo-tree-node-card--active" : "",
                  isContextOnly ? "vo-tree-node-card--context" : "",
                ]
                  .filter(Boolean)
                  .join(" ");

                return (
                  <div key={node.id} className="vo-tree-row">
                    {guides.length > 0 && (
                      <div className="vo-tree-guides" aria-hidden="true">
                        {guides.map((guideKind, idx) => {
                          const guideClasses = [
                            "vo-tree-guide",
                            guideKind === "line" ? "vo-tree-guide--line" : "",
                            guideKind === "elbow" ? "vo-tree-guide--elbow" : "",
                            guideKind === "elbow-continues"
                              ? "vo-tree-guide--elbow vo-tree-guide--continues"
                              : "",
                            node.isActiveBranch && idx === guides.length - 1
                              ? "vo-tree-guide--active"
                              : "",
                          ]
                            .filter(Boolean)
                            .join(" ");
                          return <span key={idx} className={guideClasses} />;
                        })}
                      </div>
                    )}

                    <div className="vo-tree-node-wrap">
                      <div className={nodeCardClass}>
                        <div className="vo-tree-node-content">
                          <div className="vo-meta-row">
                            <span
                              className={`verity-badge ${
                                node.role === "user"
                                  ? "verity-badge--info"
                                  : "verity-badge--neutral"
                              }`}
                            >
                              {node.role}
                            </span>
                            {node.isActiveBranch && (
                              <span className="verity-badge verity-badge--success">
                                <GitBranchIcon size={11} />
                                Active
                              </span>
                            )}
                            {isContextOnly && (
                              <span className="verity-badge verity-badge--neutral">
                                Parent context
                              </span>
                            )}
                            <code className="vo-entry-id">{node.id}</code>
                            <span className="vo-timestamp">
                              <ClockIcon size={12} />
                              {node.timestamp}
                            </span>
                          </div>
                          <div className="vo-preview-text">{node.preview}</div>
                          {trimmedQuery && !isContextOnly && parentPreview && (
                            <div
                              className="vo-tree-parent-label"
                              title={parentPreview}
                            >
                              Parent: {parentPreview}
                            </div>
                          )}
                        </div>
                        <button
                          type="button"
                          className="md3-btn-tonal"
                          disabled={forkingId !== null}
                          onClick={() => void handleFork(node.id)}
                        >
                          <GitForkIcon size={14} />
                          {isForkingNode ? "Forking..." : "Fork"}
                        </button>
                      </div>
                    </div>
                  </div>
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
