use super::{commit_tree, commit_worktree_changes, create_worktree, read_ref, remove_worktree};
use super::{run_git, update_ref_cas};
use crate::handoff::{build_spec_handoff, RunLog};
use crate::orchestrator::{
    approve_and_merge, handoff_path_for, handoff_to_json, load_handoff, MergeResult,
    OrchestratorError, PersistedHandoff,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use verity_core::{CheckerVerdict, CommitOid, RelPath, TaskId, VerdictStatus, VerifierKind};

type Observer = Box<dyn FnMut(&Path, &[&str], &Output)>;

thread_local! {
    static OBSERVER: RefCell<Option<Observer>> = const { RefCell::new(None) };
}

pub(super) fn after_command(repo: &Path, args: &[&str], output: &Output) {
    if let Some(mut observer) = OBSERVER.with(|slot| slot.take()) {
        observer(repo, args, output);
        OBSERVER.with(|slot| assert!(slot.replace(Some(observer)).is_none()));
    }
}

struct ObservingGit;

impl ObservingGit {
    fn install(observer: Observer) -> Self {
        OBSERVER.with(|slot| assert!(slot.replace(Some(observer)).is_none()));
        Self
    }
}

impl Drop for ObservingGit {
    fn drop(&mut self) {
        OBSERVER.with(|slot| {
            slot.take();
        });
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "verity-approval-{}-{unique}-{label}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create {}: {error}", path.display()),
            }
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct Event {
    actor: &'static str,
    kind: &'static str,
    data: Vec<&'static str>,
}

#[derive(Clone)]
struct Trace {
    name: &'static str,
    mode: &'static str,
    place: &'static str,
    attempts: Vec<&'static str>,
    events: Vec<Event>,
}

struct Fixture {
    temp: TempDir,
    repo: PathBuf,
    base: CommitOid,
    oids: RefCell<HashMap<String, &'static str>>,
    digests: RefCell<HashMap<&'static str, String>>,
    trace: RefCell<Trace>,
}

#[derive(Clone, Copy)]
enum Intervention {
    AdvanceTarget,
    SwitchCheckout,
    EditBeforeStatus,
    EditAfterStatus { affected: bool },
    CompetingApproval,
}

const TARGET: &str = "refs/heads/target";
const OTHER: &str = "refs/heads/other";
const AFFECTED: &str = "affected.txt";
const UNRELATED: &str = "unrelated.txt";

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap().trim_end()
}

fn git(repo: &Path, args: &[&str]) -> String {
    git_text(repo, args).trim_end().to_owned()
}

fn git_text(repo: &Path, args: &[&str]) -> String {
    let output = run_git(repo, args).unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn evidence(status: VerdictStatus) -> &'static str {
    match status {
        VerdictStatus::Pass => "accepted",
        VerdictStatus::Inconclusive => "inconclusive",
        VerdictStatus::Counterexample | VerdictStatus::InvalidReport => "rejected",
    }
}

fn branch_label(full: &str) -> &'static str {
    match full {
        TARGET => "target",
        OTHER => "other",
        _ => panic!("unexpected branch {full:?}"),
    }
}

impl Fixture {
    fn new(name: &'static str, mode: &'static str, place: &'static str) -> Rc<Self> {
        let temp = TempDir::new(name);
        let repo = temp.0.join("repo");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "target"]);
        git(&repo, &["config", "user.name", "Approval Trace"]);
        git(&repo, &["config", "user.email", "trace@example.invalid"]);
        git(&repo, &["config", "core.autocrlf", "false"]);
        fs::write(repo.join(".gitignore"), ".verity/\n").unwrap();
        fs::write(repo.join(AFFECTED), "base\n").unwrap();
        fs::write(repo.join(UNRELATED), "unrelated\n").unwrap();
        let base = commit_worktree_changes(&repo, "base").unwrap();
        git(&repo, &["branch", "other"]);

        if place != "target" {
            git(&repo, &["switch", "other"]);
        }
        if place == "elsewhere" {
            let linked = temp.0.join("linked");
            git(
                &repo,
                &["worktree", "add", linked.to_str().unwrap(), "target"],
            );
        }
        assert_eq!(
            git(&repo, &["symbolic-ref", "HEAD"]),
            if place == "target" { TARGET } else { OTHER }
        );
        let owner = git(&repo, &["for-each-ref", "--format=%(worktreepath)", TARGET]);
        assert_eq!(
            checkout_label(&repo, &owner),
            match place {
                "target" => "local",
                "other" => "unchecked",
                "elsewhere" => "elsewhere",
                _ => panic!("unknown fixture place {place}"),
            }
        );
        let oids = RefCell::new(HashMap::from([(base.as_str().to_string(), "base")]));
        Rc::new(Self {
            temp,
            repo,
            base,
            oids,
            digests: RefCell::new(HashMap::new()),
            trace: RefCell::new(Trace {
                name,
                mode,
                place,
                attempts: Vec::new(),
                events: Vec::new(),
            }),
        })
    }

    fn event(&self, actor: &'static str, kind: &'static str, data: &[&'static str]) {
        self.trace.borrow_mut().events.push(Event {
            actor,
            kind,
            data: data.to_vec(),
        });
    }

    fn remember(&self, oid: &str, label: &'static str) {
        if let Some(previous) = self.oids.borrow_mut().insert(oid.to_owned(), label) {
            assert_eq!(previous, label);
        }
    }

    fn oid(&self, oid: &str) -> &'static str {
        self.oids.borrow()[oid]
    }

    fn task(&self, attempt: &str) -> TaskId {
        let id = if attempt == "A" || self.trace.borrow().mode == "repeat" {
            "task-a"
        } else {
            "task-b"
        };
        TaskId::parse(id).unwrap()
    }

    fn publish(&self, attempt: &'static str, status: VerdictStatus) {
        let task = self.task(attempt);
        let worktree = self.temp.0.join(format!("candidate-{attempt}"));
        create_worktree(&self.repo, &worktree, &task, &self.base).unwrap();
        let source = format!("candidate-{attempt}\n");
        fs::write(worktree.join(AFFECTED), &source).unwrap();
        let candidate =
            commit_worktree_changes(&worktree, &format!("candidate {attempt}")).unwrap();
        remove_worktree(&self.repo, &worktree).unwrap();
        self.remember(candidate.as_str(), attempt);

        let verifier = VerifierKind::RustKani;
        let record = PersistedHandoff {
            task_id: task.clone(),
            base_commit: self.base.clone(),
            verifier,
            handoff: build_spec_handoff(
                candidate,
                &[(RelPath::parse(AFFECTED).unwrap(), source)],
                verifier,
                CheckerVerdict {
                    status,
                    reason: "fixture evidence".to_string(),
                    checks_passed: usize::from(status == VerdictStatus::Pass),
                    probes_satisfied: 0,
                },
                None,
            ),
        };
        let path = handoff_path_for(&self.repo, &task);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, handoff_to_json(&record)).unwrap();

        let actual = load_handoff(&self.repo, &task).unwrap();
        assert_eq!(actual, record);
        assert!(self
            .digests
            .borrow_mut()
            .insert(attempt, actual.handoff.spec_digest.to_hex())
            .is_none());
        self.record_event(attempt, "publish", &actual);
    }

    fn record_event(&self, attempt: &'static str, kind: &'static str, record: &PersistedHandoff) {
        let candidate = self.oid(record.handoff.candidate_commit.as_str());
        let recorded_digest = record.handoff.spec_digest.to_hex();
        assert_eq!(recorded_digest, self.digests.borrow()[candidate]);
        self.event(
            attempt,
            kind,
            &[
                candidate,
                self.oid(record.base_commit.as_str()),
                evidence(record.handoff.verdict.status),
                candidate,
            ],
        );
    }

    fn approval(
        self: &Rc<Self>,
        attempt: &'static str,
        digest_id: &'static str,
        intervention: Option<Intervention>,
    ) {
        self.trace.borrow_mut().attempts.push(attempt);
        let task = self.task(attempt);
        let loaded = load_handoff(&self.repo, &task);
        let candidate = match loaded {
            Ok(record) => {
                self.record_event(attempt, "load", &record);
                Some(record.handoff.candidate_commit)
            }
            Err(OrchestratorError::HandoffNotFound(_)) => {
                self.event(attempt, "load", &["none", "none", "none", "none"]);
                None
            }
            Err(error) => panic!("{error:?}"),
        };
        let digest = self
            .digests
            .borrow()
            .get(digest_id)
            .cloned()
            .unwrap_or_else(|| "0".repeat(64));
        let log = RunLog::open(&self.repo).unwrap();
        let previous_logs = log.read_lines().unwrap();
        let fired = Rc::new(Cell::new(false));

        let outcome = {
            let fixture = Rc::clone(self);
            let fired = Rc::clone(&fired);
            let _observe = ObservingGit::install(Box::new(move |repo, args, output| {
                assert_eq!(repo, fixture.repo);
                let kind = fixture.git_event(attempt, digest_id, candidate.as_ref(), args, output);
                let should_intervene = match intervention {
                    Some(
                        Intervention::AdvanceTarget
                        | Intervention::SwitchCheckout
                        | Intervention::CompetingApproval,
                    ) => kind == "lookup",
                    Some(Intervention::EditBeforeStatus) => kind == "head",
                    Some(Intervention::EditAfterStatus { .. }) => kind == "status",
                    None => false,
                };
                if should_intervene {
                    assert!(!fired.replace(true));
                    fixture.intervene(intervention.unwrap());
                }
            }));
            approve_and_merge(&self.repo, &task, TARGET, &digest)
        };
        assert_eq!(fired.get(), intervention.is_some());
        let (result, oid) = match outcome {
            Ok(MergeResult::Merged {
                commit,
                already_merged,
            }) => (
                if already_merged { "already" } else { "new" },
                self.oid(commit.as_str()),
            ),
            Ok(MergeResult::Conflict { .. }) => ("conflict", "none"),
            Err(OrchestratorError::TargetMoved) => ("moved", "none"),
            Err(OrchestratorError::Git(_)) => ("gitError", "none"),
            Err(OrchestratorError::HandoffNotFound(_)) => ("missing", "none"),
            Err(OrchestratorError::DigestMismatch { .. }) => ("digest", "none"),
            Err(OrchestratorError::GateFailed(_)) => ("gate", "none"),
            other => panic!("unexpected outcome: {other:?}"),
        };
        if ["missing", "digest", "gate"].contains(&result) {
            self.event(attempt, "gate", &[digest_id, result]);
        }
        let logs = log.read_lines().unwrap();
        assert!(logs.starts_with(&previous_logs));
        let task_prefix = format!("\"task_id\":\"{}\"", task.as_str());
        let new_logs: Vec<_> = logs[previous_logs.len()..]
            .iter()
            .filter(|line| line.contains(&task_prefix))
            .collect();
        let logged = match new_logs.as_slice() {
            [] => "unlogged",
            [record] if result == "new" => {
                let oids = self.oids.borrow();
                let actual_oid = oids.iter().find(|(_, label)| **label == oid).unwrap().0;
                assert!(
                    record.contains(&format!("\"kind\":\"merged\",\"detail\":\"{actual_oid}\""))
                );
                "logged"
            }
            [record] if result == "conflict" => {
                assert!(record.contains("\"kind\":\"merge_conflict\""));
                "logged"
            }
            _ => panic!("unexpected new run log rows {new_logs:?}"),
        };
        self.event(attempt, "return", &[result, oid, logged]);
        self.event(attempt, "observe", &self.snapshot());
    }

    fn git_event(
        &self,
        attempt: &'static str,
        digest: &'static str,
        candidate: Option<&CommitOid>,
        args: &[&str],
        output: &Output,
    ) -> &'static str {
        let ok = output.status.success();
        let value = stdout(output);
        let (kind, data) = match args {
            ["check-ref-format", TARGET] if ok => ("gate", vec![digest, "none"]),
            ["rev-parse", "--verify", "--quiet", "--end-of-options", TARGET] if ok => {
                ("target", vec![self.oid(value)])
            }
            ["merge-base", "--is-ancestor", actual_candidate, target] => {
                assert_eq!(*actual_candidate, candidate.unwrap().as_str());
                let result = match output.status.code() {
                    Some(0) => "included",
                    Some(1) => "absent",
                    _ => panic!("unexpected ancestry result {output:?}"),
                };
                (
                    "ancestry",
                    vec![self.oid(target), self.oid(actual_candidate), result],
                )
            }
            ["merge-tree", "--write-tree", "--name-only", "--no-messages", "-z", target, actual_candidate] =>
            {
                assert_eq!(self.oid(target), "base");
                assert_eq!(*actual_candidate, candidate.unwrap().as_str());
                let result = match output.status.code() {
                    Some(0) => "clean",
                    Some(1) if !output.stdout.is_empty() => "conflict",
                    _ => "gitError",
                };
                ("tree", vec![result])
            }
            ["commit-tree", _, "-p", base, "-p", actual_candidate, "-m", _] if ok => {
                assert_eq!(*base, self.base.as_str());
                let candidate = candidate.unwrap().as_str();
                assert_eq!(*actual_candidate, candidate);
                assert_eq!(
                    git(&self.repo, &["show", "-s", "--format=%P", value]),
                    format!("{base} {candidate}")
                );
                assert_eq!(
                    git(&self.repo, &["show", "-s", "--format=%T", value]),
                    git(&self.repo, &["show", "-s", "--format=%T", candidate]),
                );
                let label = match (attempt, self.oid(candidate)) {
                    ("A", "A") => "mergeA",
                    ("B", "B") => "mergeB",
                    pair => panic!("unexpected fixture merge identity {pair:?}"),
                };
                self.remember(value, label);
                ("commit", vec![label])
            }
            ["for-each-ref", "--format=%(worktreepath)", TARGET] if ok => {
                ("lookup", vec![checkout_label(&self.repo, value)])
            }
            ["symbolic-ref", "HEAD"] if ok => ("symbolic", vec![branch_label(value)]),
            ["rev-parse", "--verify", "--quiet", "--end-of-options", "HEAD"] if ok => {
                ("head", vec![self.oid(value)])
            }
            ["status", "--porcelain", "--untracked-files=no"] if ok => (
                "status",
                vec![if value.is_empty() { "clean" } else { "dirty" }],
            ),
            ["update-ref", "--no-deref", "--", TARGET, merged, expected] => {
                assert_eq!(self.oid(expected), "base");
                let actual = read_ref(&self.repo, TARGET).unwrap().unwrap();
                let movement = if ok {
                    assert_eq!(actual.as_str(), *merged);
                    "applied"
                } else {
                    assert_ne!(actual.as_str(), *merged);
                    "refused"
                };
                ("cas", vec![movement])
            }
            ["merge", "--ff-only", "--no-autostash", "--no-overwrite-ignore", merged] => {
                let actual = read_ref(&self.repo, "HEAD").unwrap().unwrap();
                let movement = if ok {
                    assert_eq!(actual.as_str(), *merged);
                    if value == "Already up to date." {
                        "unchanged"
                    } else {
                        "applied"
                    }
                } else {
                    "refused"
                };
                ("merge", vec![movement])
            }
            _ => panic!("unrecognized production Git operation {args:?}: {output:?}"),
        };
        self.event(attempt, kind, &data);
        kind
    }

    fn intervene(self: &Rc<Self>, intervention: Intervention) {
        match intervention {
            Intervention::AdvanceTarget => {
                let tree = read_ref(&self.repo, "HEAD^{tree}").unwrap().unwrap();
                let external =
                    commit_tree(&self.repo, &tree, &[&self.base], "independent commit").unwrap();
                update_ref_cas(&self.repo, TARGET, &external, Some(&self.base)).unwrap();
                assert_eq!(read_ref(&self.repo, TARGET).unwrap().unwrap(), external);
                self.remember(external.as_str(), "external");
                self.event("env", "advance", &["target", "external"]);
            }
            Intervention::SwitchCheckout => {
                git(&self.repo, &["switch", "other"]);
                self.event(
                    "env",
                    "switch",
                    &[branch_label(&git(&self.repo, &["symbolic-ref", "HEAD"]))],
                );
            }
            Intervention::EditBeforeStatus => {
                fs::write(self.repo.join(AFFECTED), "local affected edit\n").unwrap();
                git(&self.repo, &["add", AFFECTED]);
                self.event("env", "edit", &["affected", "staged"]);
            }
            Intervention::EditAfterStatus { affected } => {
                let (path, kind) = if affected {
                    (AFFECTED, "affected")
                } else {
                    (UNRELATED, "unrelated")
                };
                fs::write(self.repo.join(path), format!("local {kind} edit\n")).unwrap();
                self.event("env", "edit", &[kind, "unstaged"]);
            }
            Intervention::CompetingApproval => self.approval("B", "B", None),
        }
    }

    fn snapshot(&self) -> Vec<&'static str> {
        let target = read_ref(&self.repo, TARGET).unwrap().unwrap();
        let other = read_ref(&self.repo, OTHER).unwrap().unwrap();
        let branch = branch_label(&git(&self.repo, &["symbolic-ref", "HEAD"]));
        let index = contents(
            &git_text(&self.repo, &["show", ":affected.txt"]),
            &git_text(&self.repo, &["show", ":unrelated.txt"]),
        );
        let disk = contents(
            &fs::read_to_string(self.repo.join(AFFECTED)).unwrap(),
            &fs::read_to_string(self.repo.join(UNRELATED)).unwrap(),
        );
        vec![
            self.oid(target.as_str()),
            self.oid(other.as_str()),
            branch,
            index.0,
            index.1,
            disk.0,
            disk.1,
        ]
    }

    fn check(&self, attempt: &str, result: &[&str], snapshot: &[&str]) {
        let trace = self.trace.borrow();
        let event = |kind| {
            trace
                .events
                .iter()
                .find(|event| event.actor == attempt && event.kind == kind)
                .unwrap()
        };
        assert_eq!(event("return").data, result, "{}", trace.name);
        assert_eq!(event("observe").data, snapshot, "{}", trace.name);
    }
}

fn checkout_label(repo: &Path, reported: &str) -> &'static str {
    if reported.is_empty() {
        "unchecked"
    } else if Path::new(reported).canonicalize().unwrap() == repo.canonicalize().unwrap() {
        "local"
    } else {
        "elsewhere"
    }
}

fn contents(affected: &str, unrelated: &str) -> (&'static str, &'static str) {
    let unrelated = match unrelated {
        "unrelated\n" => "clean",
        "local unrelated edit\n" => "unrelated",
        unexpected => panic!("unknown unrelated file contents {unexpected:?}"),
    };
    match (affected, unrelated) {
        ("base\n", _) => ("base", unrelated),
        ("candidate-A\n", _) => ("mergeA", unrelated),
        ("candidate-B\n", _) => ("mergeB", unrelated),
        ("local affected edit\n", "clean") => ("base", "affected"),
        unexpected => panic!("unknown affected file contents {unexpected:?}"),
    }
}

fn production_traces() -> Vec<Trace> {
    const CLEAN_TARGET: [&str; 7] = ["base", "base", "target", "base", "clean", "base", "clean"];
    const CLEAN_OTHER: [&str; 7] = ["base", "base", "other", "base", "clean", "base", "clean"];

    let local = Fixture::new("local-repeat", "repeat", "target");
    local.publish("A", VerdictStatus::Pass);
    local.approval("A", "A", None);
    local.approval("B", "A", None);
    let merged = [
        "mergeA", "base", "target", "mergeA", "clean", "mergeA", "clean",
    ];
    local.check("A", &["new", "mergeA", "logged"], &merged);
    local.check("B", &["already", "mergeA", "unlogged"], &merged);

    let competing = Fixture::new("competing", "compete", "other");
    competing.publish("A", VerdictStatus::Pass);
    competing.publish("B", VerdictStatus::Pass);
    competing.approval("A", "A", Some(Intervention::CompetingApproval));
    let merged = ["mergeB", "base", "other", "base", "clean", "base", "clean"];
    competing.check("B", &["new", "mergeB", "logged"], &merged);
    competing.check("A", &["gitError", "none", "unlogged"], &merged);

    let external = Fixture::new("external-update", "compete", "other");
    external.publish("A", VerdictStatus::Pass);
    external.approval("A", "A", Some(Intervention::AdvanceTarget));
    external.check(
        "A",
        &["gitError", "none", "unlogged"],
        &[
            "external", "base", "other", "base", "clean", "base", "clean",
        ],
    );
    external.approval("B", "B", None);
    external.check(
        "B",
        &["missing", "none", "unlogged"],
        &[
            "external", "base", "other", "base", "clean", "base", "clean",
        ],
    );

    let stale = Fixture::new("stale-target", "compete", "other");
    stale.publish("A", VerdictStatus::Pass);
    stale.intervene(Intervention::AdvanceTarget);
    stale.approval("A", "A", None);
    stale.check(
        "A",
        &["moved", "none", "unlogged"],
        &[
            "external", "base", "other", "base", "clean", "base", "clean",
        ],
    );

    let repeat = Fixture::new("new-handoff", "repeat", "target");
    repeat.publish("A", VerdictStatus::Pass);
    repeat.publish("B", VerdictStatus::Pass);
    repeat.approval("A", "A", None);
    repeat.check("A", &["digest", "none", "unlogged"], &CLEAN_TARGET);

    let gate = Fixture::new("refused-evidence", "compete", "target");
    gate.publish("A", VerdictStatus::Inconclusive);
    gate.publish("B", VerdictStatus::Counterexample);
    gate.approval("A", "A", None);
    gate.approval("B", "B", None);
    gate.check("A", &["gate", "none", "unlogged"], &CLEAN_TARGET);
    gate.check("B", &["gate", "none", "unlogged"], &CLEAN_TARGET);

    let switched = Fixture::new("switched-checkout", "compete", "target");
    switched.publish("A", VerdictStatus::Pass);
    switched.approval("A", "A", Some(Intervention::SwitchCheckout));
    switched.check("A", &["gitError", "none", "unlogged"], &CLEAN_OTHER);

    let elsewhere = Fixture::new("other-worktree", "compete", "elsewhere");
    elsewhere.publish("A", VerdictStatus::Pass);
    elsewhere.approval("A", "A", None);
    elsewhere.check("A", &["gitError", "none", "unlogged"], &CLEAN_OTHER);

    let dirty = Fixture::new("dirty-checkout", "compete", "target");
    dirty.publish("A", VerdictStatus::Pass);
    dirty.approval("A", "A", Some(Intervention::EditBeforeStatus));
    dirty.check(
        "A",
        &["gitError", "none", "unlogged"],
        &[
            "base", "base", "target", "base", "affected", "base", "affected",
        ],
    );

    let untouched = Fixture::new("unrelated-edit", "compete", "target");
    untouched.publish("A", VerdictStatus::Pass);
    untouched.approval(
        "A",
        "A",
        Some(Intervention::EditAfterStatus { affected: false }),
    );
    untouched.check(
        "A",
        &["new", "mergeA", "logged"],
        &[
            "mergeA",
            "base",
            "target",
            "mergeA",
            "clean",
            "mergeA",
            "unrelated",
        ],
    );

    let overlapping = Fixture::new("overlapping-edit", "compete", "target");
    overlapping.publish("A", VerdictStatus::Pass);
    overlapping.approval(
        "A",
        "A",
        Some(Intervention::EditAfterStatus { affected: true }),
    );
    overlapping.check(
        "A",
        &["gitError", "none", "unlogged"],
        &[
            "base", "base", "target", "base", "clean", "base", "affected",
        ],
    );

    [
        local,
        competing,
        external,
        stale,
        repeat,
        gate,
        switched,
        elsewhere,
        dirty,
        untouched,
        overlapping,
    ]
    .iter()
    .map(|fixture| fixture.trace.borrow().clone())
    .collect()
}

fn tla_tuple(values: &[&str]) -> String {
    let values: Vec<_> = values.iter().map(|value| format!("{value:?}")).collect();
    format!("<<{}>>", values.join(", "))
}

fn trace_module(traces: &[Trace]) -> String {
    let mut module = String::from("---- MODULE ApprovalTrace ----\nTraces == {\n");
    for (i, trace) in traces.iter().enumerate() {
        if i != 0 {
            writeln!(module, ",").unwrap();
        }
        writeln!(
            module,
            " [name |-> {:?}, mode |-> {:?}, place |-> {:?}, attempts |-> {{{}}}, events |-> <<",
            trace.name,
            trace.mode,
            trace.place,
            trace
                .attempts
                .iter()
                .map(|a| format!("{a:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        for (j, event) in trace.events.iter().enumerate() {
            if j != 0 {
                writeln!(module, ",").unwrap();
            }
            write!(
                module,
                "  <<{:?}, {:?}, {}>>",
                event.actor,
                event.kind,
                tla_tuple(&event.data)
            )
            .unwrap();
        }
        write!(module, "\n >>]").unwrap();
    }
    module.push_str("\n}\n====\n");
    module
}

fn replay(jar: &Path, traces: &[Trace]) -> Output {
    let temp = TempDir::new("replay");
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../formal/tla/approval");
    for name in ["Approval.tla", "ApprovalReplay.tla", "Replay.cfg"] {
        fs::copy(model.join(name), temp.0.join(name)).unwrap();
    }
    fs::write(temp.0.join("ApprovalTrace.tla"), trace_module(traces)).unwrap();
    let java = std::env::var_os("VERITY_TLC_JAVA").unwrap_or_else(|| "java".into());
    Command::new(java)
        .current_dir(&temp.0)
        .args(["-Xmx512m", "-XX:+UseParallelGC", "-cp"])
        .arg(jar)
        .args([
            "tlc2.TLC",
            "-workers",
            "1",
            "-config",
            "Replay.cfg",
            "ApprovalReplay.tla",
        ])
        .output()
        .expect("run Java and TLC")
}

fn replay_output(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
#[ignore = "requires Java and VERITY_TLC_JAR pointing to the pinned tla2tools.jar"]
fn approval_model_and_production_traces() {
    let jar = PathBuf::from(std::env::var_os("VERITY_TLC_JAR").expect("set VERITY_TLC_JAR"));
    let jar = std::path::absolute(jar).unwrap();
    assert!(jar.is_file(), "TLC jar does not exist at {}", jar.display());
    let traces = production_traces();
    let output = replay(&jar, &traces);
    assert!(output.status.success(), "{}", replay_output(&output));
    println!("Replayed {} real-Git approval schedules", traces.len());

    let mut wrong_effect = traces[0].clone();
    let observation = wrong_effect
        .events
        .iter_mut()
        .find(|event| event.actor == "A" && event.kind == "observe")
        .unwrap();
    observation.data[5] = "base";
    let output = replay(&jar, &[wrong_effect]);
    assert!(
        !output.status.success() && replay_output(&output).contains("Deadlock reached"),
        "wrong disk effect was not rejected by replay:\n{}",
        replay_output(&output)
    );

    let mut unknown = traces[0].clone();
    unknown
        .events
        .iter_mut()
        .find(|event| event.kind == "status")
        .unwrap()
        .kind = "unknown";
    let output = replay(&jar, &[unknown]);
    assert!(
        !output.status.success() && replay_output(&output).contains("Deadlock reached"),
        "unknown event was not rejected by replay:\n{}",
        replay_output(&output)
    );

    let mut incomplete = traces[0].clone();
    assert_eq!(incomplete.events.pop().unwrap().kind, "observe");
    let output = replay(&jar, &[incomplete]);
    assert!(
        !output.status.success()
            && replay_output(&output).contains("Invariant Complete is violated"),
        "missing final observation was not rejected by replay:\n{}",
        replay_output(&output)
    );

    let mut empty = traces[0].clone();
    empty.events.clear();
    empty.attempts.clear();
    let output = replay(&jar, &[empty]);
    assert!(
        !output.status.success()
            && replay_output(&output).contains("Invariant TraceHasAttempts is violated"),
        "empty trace was not rejected by replay:\n{}",
        replay_output(&output)
    );
}
