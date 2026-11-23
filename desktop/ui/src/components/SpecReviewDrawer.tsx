import { useEffect, useRef, useState } from "react";
import {
  approvalBlockers,
  canApproveHandoff,
  formatVerdictStatus,
  formatVerifierLabel,
  shortenDigest,
  type HandoffView,
  type VerdictStatus,
  type VerifierKind,
} from "../types/verity";
import {
  AlertIcon,
  CheckIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  CloseIcon,
  CopyIcon,
  ShieldCheckIcon,
  XCircleIcon,
} from "./Icons";
import "./VerificationRail.css";

export interface SpecReviewDrawerProps {
  handoff: HandoffView | null;
  diffText?: string;
  onApprove: (handoff: HandoffView) => void | Promise<void>;
  onReject: (handoff: HandoffView, reason?: string) => void | Promise<void>;
  onClose?: () => void;
  submitting?: boolean;
  approved?: boolean;
}

function formatProbeLabel(verifier: VerifierKind, count: number): string {
  switch (verifier) {
    case "lean":
      return count === 1
        ? "Lean 4 proof obligation"
        : "Lean 4 proof obligations";
    case "kani":
      return count === 1 ? "kani::cover! probe" : "kani::cover! probes";
    default: {
      const _exhaustive: never = verifier;
      void _exhaustive;
      return count === 1 ? "verification probe" : "verification probes";
    }
  }
}

function verdictTone(status: VerdictStatus): "success" | "warning" | "danger" {
  switch (status) {
    case "pass":
      return "success";
    case "inconclusive":
      return "warning";
    case "counterexample":
    case "invalid_report":
      return "danger";
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
}

function diffLineClass(line: string): string {
  if (
    line.startsWith("diff ") ||
    line.startsWith("index ") ||
    line.startsWith("--- ") ||
    line.startsWith("+++ ")
  ) {
    return "vr-diff-line vr-diff-line--meta";
  }
  if (line.startsWith("@@")) {
    return "vr-diff-line vr-diff-line--hunk";
  }
  if (line.startsWith("+")) {
    return "vr-diff-line vr-diff-line--add";
  }
  if (line.startsWith("-")) {
    return "vr-diff-line vr-diff-line--del";
  }
  return "vr-diff-line vr-diff-line--ctx";
}

export function SpecReviewDrawer({
  handoff,
  diffText,
  onApprove,
  onReject,
  onClose,
  submitting = false,
  approved = false,
}: SpecReviewDrawerProps) {
  const [diffExpanded, setDiffExpanded] = useState(false);
  const [copiedDigest, setCopiedDigest] = useState(false);
  const [rejectNoteOpen, setRejectNoteOpen] = useState(false);
  const [rejectReason, setRejectReason] = useState("");
  const copyTimerRef = useRef<number | null>(null);

  useEffect(() => {
    setRejectNoteOpen(false);
    setRejectReason("");
  }, [handoff?.task_id]);

  useEffect(() => {
    return () => {
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (!onClose || handoff === null) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose?.();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose, handoff]);

  if (handoff === null) {
    return (
      <section
        className="vr-panel vr-panel--surface"
        aria-label="Spec review drawer"
      >
        <div className="vr-empty-state">
          Select a Write subagent handoff to review its extracted formal spec
          and blind back-translation.
        </div>
      </section>
    );
  }

  const approvable = canApproveHandoff(handoff);
  const blockers = approvalBlockers(handoff);
  const hygienePassed =
    handoff.hygiene_clean && handoff.hygiene_violations.length === 0;
  const layer2Tone = verdictTone(handoff.verdict.status);
  const specDigest = handoff.spec_digest;
  const diffLines =
    diffText !== undefined && diffText.trim().length > 0
      ? diffText.split("\n")
      : [];
  const hasDiffText = diffLines.length > 0;

  function handleCopyDigest() {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      void navigator.clipboard.writeText(specDigest);
      setCopiedDigest(true);
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
      copyTimerRef.current = window.setTimeout(() => {
        setCopiedDigest(false);
        copyTimerRef.current = null;
      }, 1600);
    }
  }

  function handleRejectClick() {
    if (!handoff) {
      return;
    }
    const trimmed = rejectReason.trim();
    if (trimmed.length > 0) {
      void onReject(handoff, trimmed);
    } else {
      void onReject(handoff);
    }
    setRejectReason("");
    setRejectNoteOpen(false);
  }

  return (
    <section
      className="vr-panel vr-panel--surface"
      aria-label="Spec review drawer"
    >
      <header className="vr-header vr-header--bordered">
        <div className="vr-title-group">
          <span className="vr-eyebrow">Spec Trust Boundary</span>
          <h2 className="vr-title vr-title--mono" title={handoff.task_id}>
            {handoff.task_id}
          </h2>
        </div>
        {onClose && (
          <button
            type="button"
            className="vr-icon-close"
            onClick={onClose}
            aria-label="Close spec review drawer"
            title="Close (Esc)"
          >
            <CloseIcon size={15} />
          </button>
        )}
      </header>

      <div className="vr-drawer-meta-bar">
        <span className="verity-badge verity-badge--neutral">
          <ShieldCheckIcon size={12} />
          {formatVerifierLabel(handoff.verifier)}
        </span>

        <button
          type="button"
          onClick={handleCopyDigest}
          title={`Copy canonical SpecDigest: ${handoff.spec_digest}`}
          className="vr-digest-btn"
        >
          <span>SpecDigest</span>
          <code className="vr-digest-code">
            {shortenDigest(handoff.spec_digest)}
          </code>
          {copiedDigest ? (
            <span className="vr-digest-status vr-digest-status--copied">
              <CheckIcon size={12} />
            </span>
          ) : (
            <CopyIcon size={12} />
          )}
        </button>
      </div>

      <div className="vr-layer-grid">
        <div className="vr-layer-card">
          <div className="vr-layer-head">
            <span className="vr-layer-kicker">
              Layer 1 · Scope &amp; Hygiene
            </span>
            <span
              className={`vr-layer-status vr-layer-status--${
                hygienePassed ? "success" : "danger"
              }`}
            >
              {hygienePassed ? (
                <ShieldCheckIcon size={12} />
              ) : (
                <XCircleIcon size={12} />
              )}
              {hygienePassed ? "Clean" : "Violations"}
            </span>
          </div>
          <div className="vr-layer-headline">
            {hygienePassed
              ? "0 AST violations"
              : `${handoff.hygiene_violations.length} AST violation${
                  handoff.hygiene_violations.length === 1 ? "" : "s"
                }`}
          </div>
          <p className="vr-layer-detail">
            {handoff.changed_paths.length} changed file
            {handoff.changed_paths.length === 1 ? "" : "s"} inspected against
            write scope.
          </p>
        </div>

        <div className="vr-layer-card">
          <div className="vr-layer-head">
            <span className="vr-layer-kicker">Layer 2 · Formal Verdict</span>
            <span className={`vr-layer-status vr-layer-status--${layer2Tone}`}>
              {layer2Tone === "success" ? (
                <CheckIcon size={12} />
              ) : layer2Tone === "warning" ? (
                <AlertIcon size={12} />
              ) : (
                <XCircleIcon size={12} />
              )}
              {formatVerdictStatus(handoff.verdict.status)}
            </span>
          </div>
          <div className="vr-layer-headline">
            {handoff.verdict.checks_passed} check
            {handoff.verdict.checks_passed === 1 ? "" : "s"} passed
          </div>
          <p className="vr-layer-detail">
            {handoff.verdict.probes_satisfied}{" "}
            {formatProbeLabel(
              handoff.verifier,
              handoff.verdict.probes_satisfied
            )}
          </p>
          {handoff.verdict.reason.trim().length > 0 && (
            <p className="vr-layer-detail">{handoff.verdict.reason.trim()}</p>
          )}
        </div>
      </div>

      {!hygienePassed && (
        <div className="vr-violation-banner" role="alert">
          <p className="vr-violation-title">
            <AlertIcon size={15} />
            Layer 1 AST Hygiene Violations ({handoff.hygiene_violations.length})
          </p>
          {handoff.hygiene_violations.length > 0 ? (
            <ul className="vr-violation-list">
              {handoff.hygiene_violations.map((v, idx) => (
                <li key={`${v.path}:${v.line}:${v.rule}:${idx}`}>
                  <code>
                    {v.path}:{v.line}
                  </code>{" "}
                  <strong>[{v.rule}]</strong> {v.detail}
                </li>
              ))}
            </ul>
          ) : (
            <p className="vr-layer-detail">
              Hygiene gate marked unclean without itemized diagnostics.
            </p>
          )}
        </div>
      )}

      <div className="vr-split-grid">
        <div className="vr-pane-card">
          <h3 className="vr-pane-heading">Extracted Formal Spec &amp; Bounds</h3>
          <pre className="vr-code-surface">
            {handoff.extracted_spec.trim().length > 0
              ? handoff.extracted_spec
              : "No extracted spec lines recorded for this handoff."}
          </pre>
        </div>

        <div className="vr-pane-card">
          <h3 className="vr-pane-heading">
            Blind Back-Translation (Plain English)
          </h3>
          <div className="vr-prose-surface">
            {handoff.back_translation &&
            handoff.back_translation.trim().length > 0
              ? handoff.back_translation
              : "No blind back-translation was attached to this handoff."}
          </div>
        </div>
      </div>

      <div className="vr-collapsible">
        <button
          type="button"
          className="vr-collapsible-toggle"
          aria-expanded={diffExpanded}
          onClick={() => setDiffExpanded((prev) => !prev)}
        >
          <span className="vr-collapsible-lead">
            {diffExpanded ? (
              <ChevronDownIcon size={15} />
            ) : (
              <ChevronRightIcon size={15} />
            )}
            Changed Paths &amp; Implementation Diff (
            {handoff.changed_paths.length} file
            {handoff.changed_paths.length === 1 ? "" : "s"})
          </span>
          <span className="vr-collapsible-hint">
            {diffExpanded ? "Hide" : "Show"}
          </span>
        </button>

        {diffExpanded && (
          <div className="vr-collapsible-body">
            <div className="vr-path-list">
              {handoff.changed_paths.map((path) => (
                <span key={path} className="vr-path-pill">
                  {path}
                </span>
              ))}
            </div>
            {hasDiffText ? (
              <pre className="vr-diff-viewer" aria-label="Unified diff">
                {diffLines.map((line, index) => (
                  <span
                    key={`${index}:${line.slice(0, 24)}`}
                    className={diffLineClass(line)}
                  >
                    {line.length > 0 ? line : " "}
                  </span>
                ))}
              </pre>
            ) : (
              <pre className="vr-code-surface">
                {`Candidate commit ${shortenDigest(handoff.candidate_commit, 10, 0)} on base ${shortenDigest(handoff.base_commit, 10, 0)}.\nFull diff text not loaded for this handoff.`}
              </pre>
            )}
          </div>
        )}
      </div>

      <footer className="vr-drawer-footer">
        {blockers.length > 0 && (
          <p className="vr-blocker-hint" role="alert">
            <AlertIcon size={14} />
            <span>Approval blocked: {blockers.join("; ")}.</span>
          </p>
        )}

        {rejectNoteOpen && (
          <div className="vr-reject-note-row">
            <label className="vr-reject-note-label" htmlFor="vr-reject-reason">
              Rejection reason (optional)
            </label>
            <input
              id="vr-reject-reason"
              type="text"
              className="vr-input"
              value={rejectReason}
              disabled={submitting}
              onChange={(e) => setRejectReason(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !submitting) {
                  e.preventDefault();
                  handleRejectClick();
                }
              }}
              placeholder="Explain what the formal spec or implementation missed…"
            />
          </div>
        )}

        <div className="vr-actions-row">
          <button
            type="button"
            className="vr-btn vr-btn--ghost vr-btn--sm"
            aria-expanded={rejectNoteOpen}
            disabled={submitting}
            onClick={() => setRejectNoteOpen((prev) => !prev)}
          >
            {rejectNoteOpen ? "Hide note" : "Add rejection note"}
          </button>

          <div className="vr-actions-right">
            <button
              type="button"
              className="vr-btn vr-btn--danger-outline"
              disabled={submitting}
              onClick={handleRejectClick}
            >
              <XCircleIcon size={14} />
              Reject
            </button>

            <button
              type="button"
              className="vr-btn vr-btn--success"
              disabled={!approvable || submitting || approved}
              onClick={() => {
                void onApprove(handoff);
              }}
            >
              <CheckIcon size={14} />
              {approved
                ? "Approved for Merge"
                : submitting
                  ? "Approving…"
                  : "Approve Spec for Merge"}
            </button>
          </div>
        </div>
      </footer>
    </section>
  );
}
