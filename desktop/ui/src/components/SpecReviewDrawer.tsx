import { useState, type CSSProperties } from "react";
import {
  approvalBlockers,
  canApproveHandoff,
  formatVerdictStatus,
  formatVerifierLabel,
  shortenDigest,
  type HandoffView,
} from "../types/verity";

export interface SpecReviewDrawerProps {
  handoff: HandoffView | null;
  diffText?: string;
  onApprove: (handoff: HandoffView) => void | Promise<void>;
  onReject: (handoff: HandoffView, reason?: string) => void | Promise<void>;
  onClose?: () => void;
  submitting?: boolean;
  approved?: boolean;
}

const styles: Record<string, CSSProperties> = {
  drawer: {
    display: "flex",
    flexDirection: "column",
    gap: 16,
    background: "var(--md-sys-color-surface)",
    color: "var(--md-sys-color-on-surface)",
    borderRadius: "var(--md-sys-shape-corner-lg)",
    border: "1px solid var(--md-sys-color-outline-variant)",
    padding: 20,
  },
  header: {
    display: "flex",
    flexDirection: "column",
    gap: 10,
    borderBottom: "1px solid var(--md-sys-color-outline-variant)",
    paddingBottom: 14,
  },
  headerTop: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  titleGroup: {
    display: "flex",
    flexDirection: "column",
    gap: 2,
  },
  eyebrow: {
    fontSize: 11,
    fontWeight: 700,
    textTransform: "uppercase",
    letterSpacing: "0.06em",
    color: "var(--md-sys-color-on-surface-variant)",
  },
  title: {
    margin: 0,
    fontSize: 18,
    fontWeight: 600,
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
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
  badgeRow: {
    display: "flex",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 8,
  },
  chip: {
    display: "inline-flex",
    alignItems: "center",
    gap: 6,
    padding: "4px 11px",
    borderRadius: "var(--md-sys-shape-corner-full)",
    fontSize: 12,
    fontWeight: 600,
  },
  digestPill: {
    display: "inline-flex",
    alignItems: "center",
    gap: 6,
    padding: "4px 11px",
    borderRadius: "var(--md-sys-shape-corner-full)",
    fontSize: 12,
    fontWeight: 600,
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    background: "var(--md-sys-color-surface-container-high)",
    color: "var(--md-sys-color-on-surface)",
    border: "1px solid var(--md-sys-color-outline-variant)",
    cursor: "pointer",
  },
  verdictReason: {
    margin: 0,
    fontSize: 12,
    color: "var(--md-sys-color-on-surface-variant)",
  },
  violationBox: {
    background: "var(--md-sys-color-error-container)",
    color: "var(--md-sys-color-on-error-container)",
    borderRadius: "var(--md-sys-shape-corner-md)",
    padding: "12px 14px",
    display: "flex",
    flexDirection: "column",
    gap: 8,
  },
  violationTitle: {
    margin: 0,
    fontSize: 13,
    fontWeight: 700,
  },
  violationList: {
    margin: 0,
    paddingLeft: 18,
    display: "flex",
    flexDirection: "column",
    gap: 6,
    fontSize: 12,
  },
  splitGrid: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(280px, 1fr))",
    gap: 16,
  },
  paneCard: {
    background: "var(--md-sys-color-surface-container)",
    borderRadius: "var(--md-sys-shape-corner-md)",
    padding: 14,
    display: "flex",
    flexDirection: "column",
    gap: 8,
  },
  paneHeading: {
    margin: 0,
    fontSize: 13,
    fontWeight: 600,
    color: "var(--md-sys-color-on-surface)",
  },
  codeSurface: {
    margin: 0,
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 12.5,
    lineHeight: 1.5,
    background: "var(--md-sys-color-surface-container-low)",
    color: "var(--md-sys-color-on-surface)",
    borderRadius: 10,
    padding: 12,
    overflowX: "auto",
    whiteSpace: "pre-wrap",
  },
  proseSurface: {
    fontSize: 13.5,
    lineHeight: 1.55,
    background: "var(--md-sys-color-surface-container-low)",
    color: "var(--md-sys-color-on-surface)",
    borderRadius: 10,
    padding: 12,
    whiteSpace: "pre-wrap",
  },
  collapsibleCard: {
    background: "var(--md-sys-color-surface-container)",
    borderRadius: "var(--md-sys-shape-corner-md)",
    border: "1px solid var(--md-sys-color-outline-variant)",
    overflow: "hidden",
  },
  collapsibleToggle: {
    width: "100%",
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    padding: "11px 14px",
    border: "none",
    background: "transparent",
    color: "var(--md-sys-color-on-surface)",
    fontSize: 13,
    fontWeight: 600,
    cursor: "pointer",
    textAlign: "left",
  },
  collapsibleBody: {
    padding: "0 14px 14px",
    display: "flex",
    flexDirection: "column",
    gap: 10,
  },
  pathList: {
    display: "flex",
    flexWrap: "wrap",
    gap: 6,
  },
  pathPill: {
    fontFamily: '"JetBrains Mono", ui-monospace, monospace',
    fontSize: 12,
    padding: "3px 9px",
    borderRadius: 8,
    background: "var(--md-sys-color-surface-container-low)",
    color: "var(--md-sys-color-on-surface)",
    border: "1px solid var(--md-sys-color-outline-variant)",
  },
  footer: {
    display: "flex",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    paddingTop: 12,
    borderTop: "1px solid var(--md-sys-color-outline-variant)",
  },
  blockerHint: {
    fontSize: 12,
    color: "var(--md-sys-color-error)",
    margin: 0,
  },
  actions: {
    display: "flex",
    alignItems: "center",
    gap: 10,
    marginLeft: "auto",
  },
  approveBtn: {
    border: "none",
    borderRadius: "var(--md-sys-shape-corner-full)",
    padding: "10px 20px",
    fontSize: 13,
    fontWeight: 600,
    background: "var(--md-sys-color-primary)",
    color: "var(--md-sys-color-on-primary)",
    cursor: "pointer",
  },
  rejectBtn: {
    border: "1px solid var(--md-sys-color-error)",
    borderRadius: "var(--md-sys-shape-corner-full)",
    padding: "9px 18px",
    fontSize: 13,
    fontWeight: 600,
    background: "transparent",
    color: "var(--md-sys-color-error)",
    cursor: "pointer",
  },
  emptyState: {
    padding: 28,
    textAlign: "center",
    color: "var(--md-sys-color-on-surface-variant)",
    fontSize: 13,
  },
};

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

  if (handoff === null) {
    return (
      <section style={styles.drawer} aria-label="Spec review drawer">
        <div style={styles.emptyState}>
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
  const verdictPassed = handoff.verdict.status === "pass";
  const specDigest = handoff.spec_digest;

  function handleCopyDigest() {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      void navigator.clipboard.writeText(specDigest);
      setCopiedDigest(true);
      window.setTimeout(() => setCopiedDigest(false), 1600);
    }
  }

  return (
    <section style={styles.drawer} aria-label="Spec review drawer">
      <header style={styles.header}>
        <div style={styles.headerTop}>
          <div style={styles.titleGroup}>
            <span style={styles.eyebrow}>Spec Trust Boundary</span>
            <h2 style={styles.title}>{handoff.task_id}</h2>
          </div>
          {onClose && (
            <button type="button" style={styles.closeBtn} onClick={onClose}>
              Close
            </button>
          )}
        </div>

        <div style={styles.badgeRow}>
          <span
            style={{
              ...styles.chip,
              background: "var(--md-sys-color-primary-container)",
              color: "var(--md-sys-color-on-primary-container)",
            }}
          >
            {formatVerifierLabel(handoff.verifier)}
          </span>

          <span
            style={{
              ...styles.chip,
              background: hygienePassed
                ? "var(--md-sys-color-tertiary-container)"
                : "var(--md-sys-color-error-container)",
              color: hygienePassed
                ? "var(--md-sys-color-on-tertiary-container)"
                : "var(--md-sys-color-on-error-container)",
            }}
          >
            {hygienePassed
              ? "Layer 1 Hygiene: 0 violations"
              : `Layer 1 Hygiene: ${handoff.hygiene_violations.length} violation${handoff.hygiene_violations.length === 1 ? "" : "s"}`}
          </span>

          <span
            style={{
              ...styles.chip,
              background: verdictPassed
                ? "var(--md-sys-color-tertiary-container)"
                : "var(--md-sys-color-error-container)",
              color: verdictPassed
                ? "var(--md-sys-color-on-tertiary-container)"
                : "var(--md-sys-color-on-error-container)",
            }}
          >
            Layer 2 Verdict: {formatVerdictStatus(handoff.verdict.status)} ·{" "}
            {handoff.verdict.checks_passed} checks ·{" "}
            {handoff.verdict.probes_satisfied} kani::cover! probe
            {handoff.verdict.probes_satisfied === 1 ? "" : "s"}
          </span>

          <button
            type="button"
            onClick={handleCopyDigest}
            title={`Copy canonical SpecDigest: ${handoff.spec_digest}`}
            style={styles.digestPill}
          >
            <span>SpecDigest</span>
            <code>{shortenDigest(handoff.spec_digest)}</code>
            <span>{copiedDigest ? "Copied" : "Copy"}</span>
          </button>
        </div>

        {handoff.verdict.reason.trim().length > 0 && (
          <p style={styles.verdictReason}>{handoff.verdict.reason}</p>
        )}
      </header>

      {!hygienePassed && (
        <div style={styles.violationBox} role="alert">
          <p style={styles.violationTitle}>
            Layer 1 AST Hygiene Violations ({handoff.hygiene_violations.length})
          </p>
          {handoff.hygiene_violations.length > 0 ? (
            <ul style={styles.violationList}>
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
            <p style={{ margin: 0, fontSize: 12 }}>
              Hygiene gate marked unclean without itemized diagnostics.
            </p>
          )}
        </div>
      )}

      <div style={styles.splitGrid}>
        <div style={styles.paneCard}>
          <h3 style={styles.paneHeading}>Extracted Formal Spec &amp; Bounds</h3>
          <pre style={styles.codeSurface}>
            {handoff.extracted_spec.trim().length > 0
              ? handoff.extracted_spec
              : "No extracted spec lines recorded for this handoff."}
          </pre>
        </div>

        <div style={styles.paneCard}>
          <h3 style={styles.paneHeading}>
            Blind Back-Translation (Plain English)
          </h3>
          <div style={styles.proseSurface}>
            {handoff.back_translation &&
            handoff.back_translation.trim().length > 0
              ? handoff.back_translation
              : "No blind back-translation was attached to this handoff."}
          </div>
        </div>
      </div>

      <div style={styles.collapsibleCard}>
        <button
          type="button"
          style={styles.collapsibleToggle}
          aria-expanded={diffExpanded}
          onClick={() => setDiffExpanded((prev) => !prev)}
        >
          <span>
            Changed Paths &amp; Full Implementation Diff (
            {handoff.changed_paths.length} file
            {handoff.changed_paths.length === 1 ? "" : "s"})
          </span>
          <span
            style={{
              fontSize: 12,
              color: "var(--md-sys-color-on-surface-variant)",
            }}
          >
            {diffExpanded ? "Hide" : "Show"}
          </span>
        </button>

        {diffExpanded && (
          <div style={styles.collapsibleBody}>
            <div style={styles.pathList}>
              {handoff.changed_paths.map((path) => (
                <span key={path} style={styles.pathPill}>
                  {path}
                </span>
              ))}
            </div>
            <pre style={styles.codeSurface}>
              {diffText && diffText.trim().length > 0
                ? diffText
                : `Candidate commit ${shortenDigest(handoff.candidate_commit, 10, 0)} on base ${shortenDigest(handoff.base_commit, 10, 0)}.\nFull diff text not loaded for this handoff.`}
            </pre>
          </div>
        )}
      </div>

      <footer style={styles.footer}>
        {blockers.length > 0 && (
          <p style={styles.blockerHint}>
            Approval blocked: {blockers.join("; ")}.
          </p>
        )}

        <div style={styles.actions}>
          <button
            type="button"
            style={{
              ...styles.rejectBtn,
              opacity: submitting ? 0.6 : 1,
              cursor: submitting ? "wait" : "pointer",
            }}
            disabled={submitting}
            onClick={() => {
              void onReject(handoff);
            }}
          >
            Reject
          </button>

          <button
            type="button"
            style={{
              ...styles.approveBtn,
              opacity: !approvable || submitting || approved ? 0.45 : 1,
              cursor:
                !approvable || submitting || approved
                  ? "not-allowed"
                  : "pointer",
            }}
            disabled={!approvable || submitting || approved}
            onClick={() => {
              void onApprove(handoff);
            }}
          >
            {approved
              ? "Approved for Merge"
              : submitting
                ? "Approving…"
                : "Approve Spec for Merge"}
          </button>
        </div>
      </footer>
    </section>
  );
}
