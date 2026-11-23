import { useEffect, useMemo, useState } from "react";
import {
  classifyRunLogKind,
  parseRunLogLines,
  shortenDigest,
  type RunLogCategory,
  type RunLogEntry,
} from "../types/verity";
import {
  AlertIcon,
  CheckIcon,
  ClockIcon,
  CloseIcon,
  SearchIcon,
  TerminalIcon,
} from "./Icons";
import "./VerificationRail.css";

export interface RunLogDrawerProps {
  entries?: readonly RunLogEntry[];
  rawLines?: readonly string[];
  open?: boolean;
  onSelectTask?: (taskId: string) => void;
  onClose?: () => void;
}

type LedgerFilter = "All" | RunLogCategory;

interface FilterOption {
  id: LedgerFilter;
  label: string;
}

const FILTER_OPTIONS: readonly FilterOption[] = [
  { id: "All", label: "All" },
  { id: "Merged", label: "Approved/Merged" },
  { id: "Handoff", label: "Review" },
  { id: "ReadOnly", label: "Read-only" },
  { id: "Other", label: "Other/Unparsed" },
];

function classifyLedgerCategory(kind: string): RunLogCategory {
  const normalized = kind.trim().toLowerCase();
  if (normalized === "approved_for_merge") {
    return "Merged";
  }
  return classifyRunLogKind(kind);
}

function isUnparsedEntry(entry: RunLogEntry): boolean {
  return entry.kind.trim().toLowerCase() === "unparsed";
}

function badgeClassForEntry(entry: RunLogEntry): string {
  const normalized = entry.kind.trim().toLowerCase();
  if (
    normalized === "unparsed" ||
    normalized === "rejected_scope" ||
    normalized === "rejected_spec" ||
    normalized === "merge_conflict"
  ) {
    return "verity-badge verity-badge--danger";
  }
  if (normalized === "merged" || normalized === "approved_for_merge") {
    return "verity-badge verity-badge--success";
  }
  if (
    normalized === "awaiting_spec_review" ||
    normalized === "handoff_ready"
  ) {
    return "verity-badge verity-badge--warning";
  }
  return "verity-badge verity-badge--neutral";
}

function hashString(input: string): string {
  let hash = 2166136261;
  for (let i = 0; i < input.length; i += 1) {
    hash ^= input.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(16);
}

function extractDigest(detail: string): string | null {
  const match = detail.match(/\b(?:sha256:)?[a-f0-9]{16,64}\b/i);
  return match ? match[0] : null;
}

export function RunLogDrawer({
  entries,
  rawLines,
  open = true,
  onSelectTask,
  onClose,
}: RunLogDrawerProps) {
  const [query, setQuery] = useState("");
  const [activeFilter, setActiveFilter] = useState<LedgerFilter>("All");

  useEffect(() => {
    if (!open || !onClose) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose?.();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, onClose]);

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
    const counts: Record<LedgerFilter, number> = {
      All: resolvedEntries.length,
      Merged: 0,
      Handoff: 0,
      ReadOnly: 0,
      Other: 0,
    };
    for (const entry of resolvedEntries) {
      const cat = classifyLedgerCategory(entry.kind);
      counts[cat] += 1;
    }
    return counts;
  }, [resolvedEntries]);

  const keyedFilteredEntries = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const seen = new Map<string, number>();
    const results: Array<{ entry: RunLogEntry; stableKey: string }> = [];

    for (const entry of resolvedEntries) {
      if (
        activeFilter !== "All" &&
        classifyLedgerCategory(entry.kind) !== activeFilter
      ) {
        continue;
      }
      if (
        needle.length > 0 &&
        !entry.task_id.toLowerCase().includes(needle) &&
        !entry.kind.toLowerCase().includes(needle) &&
        !entry.detail.toLowerCase().includes(needle)
      ) {
        continue;
      }

      const baseKey = [
        entry.lineNumber !== undefined ? `L${entry.lineNumber}` : "E",
        entry.timestamp ?? "nt",
        entry.task_id,
        entry.kind,
        hashString(entry.detail),
      ].join(":");
      const dupCount = seen.get(baseKey) ?? 0;
      seen.set(baseKey, dupCount + 1);
      const stableKey = dupCount === 0 ? baseKey : `${baseKey}#${dupCount}`;
      results.push({ entry, stableKey });
    }

    return results;
  }, [resolvedEntries, activeFilter, query]);

  if (!open) {
    return null;
  }

  return (
    <section
      className="vr-panel vr-panel--surface"
      aria-label="Run log drawer"
    >
      <header className="vr-header vr-header--bordered">
        <div className="vr-title-group">
          <div className="vr-title-row">
            <TerminalIcon size={15} />
            <h2 className="vr-title">Run Log Receipts</h2>
            <span className="vr-count-pill">{resolvedEntries.length}</span>
          </div>
          <p className="vr-subtitle">.verity/run-log.jsonl</p>
        </div>
        {onClose && (
          <button
            type="button"
            className="vr-icon-close"
            onClick={onClose}
            aria-label="Close run log drawer"
            title="Close (Esc)"
          >
            <CloseIcon size={15} />
          </button>
        )}
      </header>

      <div className="vr-search-wrap">
        <span className="vr-search-icon" aria-hidden="true">
          <SearchIcon size={14} />
        </span>
        <input
          type="search"
          className="vr-search-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search by task ID, event kind, digest, or detail…"
          aria-label="Search run log receipts"
        />
      </div>

      <div className="vr-filter-row" role="group" aria-label="Run log filters">
        {FILTER_OPTIONS.map(({ id, label }) => {
          const active = id === activeFilter;
          const isOtherFilter = id === "Other";
          return (
            <button
              key={id}
              type="button"
              aria-pressed={active}
              onClick={() => setActiveFilter(id)}
              className={`vr-filter-chip${active ? " is-active" : ""}${
                isOtherFilter ? " vr-filter-chip--danger" : ""
              }`}
            >
              <span>{label}</span>
              <span>({filterCounts[id]})</span>
            </button>
          );
        })}
      </div>

      {keyedFilteredEntries.length === 0 ? (
        <div className="vr-empty-state">
          {resolvedEntries.length === 0
            ? "No receipts recorded in .verity/run-log.jsonl yet."
            : "No receipts match the current filter and search query."}
        </div>
      ) : (
        <div className="vr-ledger">
          {keyedFilteredEntries.map(({ entry, stableKey }) => {
            const unparsed = isUnparsedEntry(entry);
            const digest = unparsed ? null : extractDigest(entry.detail);

            return (
              <article
                key={stableKey}
                className={`vr-ledger-row${
                  unparsed ? " vr-ledger-row--unparsed" : ""
                }`}
              >
                <div className="vr-ledger-top">
                  <div className="vr-ledger-meta">
                    {onSelectTask && !unparsed ? (
                      <button
                        type="button"
                        className="vr-ledger-task-btn"
                        onClick={() => onSelectTask(entry.task_id)}
                      >
                        {entry.task_id}
                      </button>
                    ) : (
                      <span className="vr-ledger-task-plain">
                        {entry.task_id}
                      </span>
                    )}

                    <span className={badgeClassForEntry(entry)}>
                      {unparsed ? (
                        <AlertIcon size={11} />
                      ) : entry.kind.trim().toLowerCase() === "merged" ||
                        entry.kind.trim().toLowerCase() ===
                          "approved_for_merge" ? (
                        <CheckIcon size={11} />
                      ) : null}
                      {entry.kind}
                    </span>

                    {digest !== null && (
                      <span className="vr-ledger-digest" title={digest}>
                        {shortenDigest(digest, 8, 6)}
                      </span>
                    )}
                  </div>

                  {entry.timestamp ? (
                    <span className="vr-ledger-time">
                      <ClockIcon size={11} />
                      {entry.timestamp}
                    </span>
                  ) : entry.lineNumber !== undefined ? (
                    <span className="vr-ledger-time">#{entry.lineNumber}</span>
                  ) : null}
                </div>

                <pre className="vr-ledger-detail">{entry.detail}</pre>
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}
