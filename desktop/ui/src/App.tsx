import React, { useState } from "react";
import "./md3-tokens.css";

type Screen = "orchestrator" | "spec-review" | "merge-queue" | "run-log";

interface SubagentCard {
  id: string;
  mode: "ReadOnly" | "Write";
  verifier: "RustKani" | "Lean4Comparator" | "None";
  state: "Running" | "AwaitingSpecReview" | "ApprovedForMerge" | "CompletedReadOnly";
  toolCallsUsed: number;
  toolCallsLimit: number;
  changedPaths: string[];
  extractedSpec: string;
  backTranslation: string;
  probesSatisfied: number;
  hygieneViolations: number;
  specDigest: string;
}

const INITIAL_TASKS: SubagentCard[] = [
  {
    id: "task-safe-sub",
    mode: "Write",
    verifier: "RustKani",
    state: "AwaitingSpecReview",
    toolCallsUsed: 9,
    toolCallsLimit: 64,
    changedPaths: ["crates/verity-core/src/scope.rs"],
    extractedSpec: [
      "// crates/verity-core/src/scope.rs",
      "24: pub fn within(path: &RelPath, prefix: &RelPath) -> bool",
      "35: pub fn check_scope(changed_paths: &[RelPath], write_scope: &[RelPath], forbidden: &[RelPath]) -> Result<(), ScopeError>",
      "52: assert!(res.is_ok() == expected_ok);",
      "54: kani::cover!(res.is_ok(), \"scope_accept_reachable\");",
    ].join("\n"),
    backTranslation:
      "check_scope accepts a diff only when changed_paths is non-empty, every path falls inside write_scope on a segment boundary, and no path matches forbidden. Both accept and reject branches are reachable.",
    probesSatisfied: 3,
    hygieneViolations: 0,
    specDigest: "9c8f4b21e0a3...d714",
  },
  {
    id: "audit-git-hooks",
    mode: "ReadOnly",
    verifier: "None",
    state: "CompletedReadOnly",
    toolCallsUsed: 4,
    toolCallsLimit: 32,
    changedPaths: [],
    extractedSpec: "",
    backTranslation: "Confirmed $GIT_COMMON_DIR is mounted read-only in bubblewrap.",
    probesSatisfied: 0,
    hygieneViolations: 0,
    specDigest: "-",
  },
];

export function App() {
  const [screen, setScreen] = useState<Screen>("spec-review");
  const [tasks, setTasks] = useState<SubagentCard[]>(INITIAL_TASKS);
  const [selectedId, setSelectedId] = useState<string>("task-safe-sub");

  const active = tasks.find((t) => t.id === selectedId) ?? tasks[0];

  function approveSpec(id: string) {
    setTasks((prev) =>
      prev.map((t) =>
        t.id === id && t.state === "AwaitingSpecReview"
          ? { ...t, state: "ApprovedForMerge" }
          : t
      )
    );
  }

  return (
    <div className="app-shell">
      <nav className="nav-rail" aria-label="Verity sections">
        {(
          [
            ["orchestrator", "Agents"],
            ["spec-review", "Spec"],
            ["merge-queue", "Merge"],
            ["run-log", "Log"],
          ] as [Screen, string][]
        ).map(([id, label]) => (
          <button
            key={id}
            className={`nav-btn ${screen === id ? "active" : ""}`}
            onClick={() => setScreen(id)}
          >
            {label}
          </button>
        ))}
      </nav>

      <main className="main-pane">
        {screen === "orchestrator" && (
          <section>
            <h1>Orchestrator workspace</h1>
            {tasks.map((t) => (
              <div className="card" key={t.id}>
                <h3>{t.id}</h3>
                <div>
                  <span className="chip chip-info">{t.mode}</span>
                  <span className="chip chip-pass">{t.state}</span>
                  <span>
                    Tools: {t.toolCallsUsed}/{t.toolCallsLimit}
                  </span>
                </div>
              </div>
            ))}
          </section>
        )}

        {screen === "spec-review" && active && (
          <section>
            <h1>Spec review drawer</h1>
            <div className="card">
              <div>
                <span className="chip chip-info">{active.verifier}</span>
                <span className="chip chip-pass">
                  Hygiene violations: {active.hygieneViolations}
                </span>
                <span className="chip chip-pass">
                  cover! probes satisfied: {active.probesSatisfied}
                </span>
              </div>
              <p>
                Spec digest: <code>{active.specDigest}</code>
              </p>
              <div className="split-grid">
                <div>
                  <h3>Extracted spec and bounds</h3>
                  <pre className="code-block">{active.extractedSpec || "Read-only task."}</pre>
                </div>
                <div>
                  <h3>Blind back-translation</h3>
                  <div className="code-block">{active.backTranslation}</div>
                </div>
              </div>
              {active.state === "AwaitingSpecReview" && (
                <div style={{ marginTop: 16 }}>
                  <button
                    className="btn-filled"
                    onClick={() => approveSpec(active.id)}
                  >
                    Approve spec for merge
                  </button>
                </div>
              )}
            </div>
          </section>
        )}

        {screen === "merge-queue" && (
          <section>
            <h1>Merge queue</h1>
            {tasks
              .filter((t) => t.mode === "Write")
              .map((t) => (
                <div className="card" key={t.id}>
                  <h3>{t.id}</h3>
                  <p>
                    State: <strong>{t.state}</strong>. Changed paths:{" "}
                    <code>{t.changedPaths.join(", ")}</code>
                  </p>
                  <button
                    className="btn-filled"
                    disabled={t.state !== "ApprovedForMerge"}
                    onClick={() => setSelectedId(t.id)}
                  >
                    {t.state === "ApprovedForMerge"
                      ? "Run merge-tree and CAS merge"
                      : "Awaiting spec approval"}
                  </button>
                </div>
              ))}
          </section>
        )}

        {screen === "run-log" && (
          <section>
            <h1>Run log</h1>
            <div className="card">
              <pre className="code-block">
                {`{"task_id":"task-safe-sub","kind":"awaiting_spec_review","detail":"9c8f4b21e0a3...d714"}\n{"task_id":"audit-git-hooks","kind":"completed_readonly","detail":"ok"}`}
              </pre>
            </div>
          </section>
        )}
      </main>
    </div>
  );
}
