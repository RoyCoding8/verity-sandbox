import assert from "node:assert/strict";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

const noop = () => {};
const digest = "a1".repeat(32);
const cleanHandoff = {
  task_id: "example",
  base_commit: "b".repeat(40),
  candidate_commit: "c".repeat(40),
  verifier: "kani",
  changed_paths: ["src/lib.rs"],
  extracted_spec: "fn verify()",
  back_translation: null,
  hygiene_clean: true,
  hygiene_violations: [],
  verdict: { status: "pass", reason: "Verified", checks_passed: 1, probes_satisfied: 1 },
  spec_digest: digest,
};

function enabledButton(markup, label) {
  const buttons = [...markup.matchAll(/<button\b([^>]*)>(.*?)<\/button>/gs)];
  const match = buttons.find(([, , content]) => content.endsWith(label));
  assert.ok(match, `Missing button ${label}`);
  return !/\bdisabled(?:=|\s|$)/.test(match[1]);
}

test("verification views display decisions made from persisted handoff and merge state", async (t) => {
  const server = await createServer({
    root: fileURLToPath(new URL("../..", import.meta.url)),
    server: { middlewareMode: true, ws: false },
    optimizeDeps: { noDiscovery: true, include: [] },
    appType: "custom",
    logLevel: "error",
  });
  t.after(() => server.close());
  const { SpecReviewDrawer } = await server.ssrLoadModule("/src/components/SpecReviewDrawer.tsx");
  const { MergeQueueModal } = await server.ssrLoadModule("/src/components/MergeQueueModal.tsx");
  const { RunLogDrawer } = await server.ssrLoadModule("/src/components/RunLogDrawer.tsx");

  const reviewCases = [
    { label: "passing handoff", expected: true },
    { label: "failed hygiene flag", handoff: { hygiene_clean: false } },
    {
      label: "reported hygiene violation",
      handoff: {
        hygiene_violations: [{ rule: "example", path: "src/lib.rs", line: 2, detail: "Review required" }],
      },
    },
    ...["counterexample", "inconclusive", "invalid_report"].map((status) => ({
      label: status,
      handoff: { verdict: { ...cleanHandoff.verdict, status } },
    })),
    { label: "approval in progress", props: { submitting: true }, button: "Approving…" },
    { label: "already approved", props: { approved: true }, button: "Approved for Merge" },
  ];
  for (const { label, handoff, props, expected = false, button = "Approve Spec for Merge" } of reviewCases) {
    const html = renderToStaticMarkup(createElement(SpecReviewDrawer, {
      handoff: { ...cleanHandoff, ...handoff },
      onApprove: noop,
      onReject: noop,
      ...props,
    }));
    assert.equal(enabledButton(html, button), expected, label);
  }

  const mergeEntry = {
    taskId: cleanHandoff.task_id,
    targetRef: "refs/heads/main",
    worktreePath: ".verity/worktrees/example",
    handoff: cleanHandoff,
    approvedDigest: digest,
    mergeTree: { kind: "unchecked" },
    mergeState: "ready",
  };
  const mergeCases = [
    { label: "ready", expected: true },
    { label: "current branch", entry: { targetRef: "HEAD" }, expected: true },
    { label: "normalized digest", entry: { approvedDigest: ` ${digest.toUpperCase()} ` }, expected: true },
    { label: "digest mismatch", entry: { approvedDigest: "f".repeat(64) } },
    { label: "missing digest", entry: { approvedDigest: "" } },
    { label: "conflicting tree", entry: { mergeTree: { kind: "conflict", conflictedPaths: ["src/lib.rs"] } } },
    { label: "in-flight merge", entry: { mergeState: "merging" }, button: "Merging..." },
    { label: "completed merge", entry: { mergeState: "merged" }, button: "Merged" },
    { label: "retryable error", entry: { mergeState: "error", errorMessage: "Retry" }, expected: true },
  ];
  for (const { label, entry, expected = false, button = "CAS Merge" } of mergeCases) {
    const html = renderToStaticMarkup(createElement(MergeQueueModal, {
      open: true,
      entries: [{ ...mergeEntry, ...entry }],
      onTriggerMerge: noop,
      onClose: noop,
    }));
    assert.equal(enabledButton(html, button), expected, label);
    if (entry?.targetRef === "HEAD") assert.match(html, /Current checked-out branch \(HEAD\)/);
  }

  const log = renderToStaticMarkup(createElement(RunLogDrawer, {
    rawLines: [
      JSON.stringify({ task_id: "example", kind: "merged", detail: "Merge completed" }),
      "Unparseable line from a prior run",
    ],
  }));
  assert.match(log, /Merge completed/);
  assert.match(log, /Unparseable line from a prior run/);
});
