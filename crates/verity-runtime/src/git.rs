use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use verity_core::{CommitOid, RelPath, TaskId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    CommandFailed {
        args: Vec<String>,
        code: Option<i32>,
        stderr: String,
    },
    InvalidOutput(String),
    Io(String),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::CommandFailed { args, code, stderr } => {
                write!(
                    f,
                    "git {:?} failed (exit {:?}): {}",
                    args,
                    code,
                    stderr.trim()
                )
            }
            GitError::InvalidOutput(msg) => write!(f, "invalid git output: {}", msg),
            GitError::Io(msg) => write!(f, "git io error: {}", msg),
        }
    }
}

impl std::error::Error for GitError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Clean { tree_oid: CommitOid },
    Conflict { conflicted_paths: Vec<RelPath> },
}

/// Runs git with scrubbed environment variables and disabled hooks/prompts,
/// salvaged and hardened from archive/verity-prototype/src/git.mjs.
pub fn run_git(repo_dir: &Path, args: &[&str]) -> Result<Output, GitError> {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_dir)
        .stdin(Stdio::null())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .args(args);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.output().map_err(|e| GitError::Io(e.to_string()))
}

fn require_git_ok(repo_dir: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let out = run_git(repo_dir, args)?;
    if !out.status.success() {
        return Err(GitError::CommandFailed {
            args: args.iter().map(|s| s.to_string()).collect(),
            code: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(out.stdout)
}

pub fn git_common_dir(repo_dir: &Path) -> Result<PathBuf, GitError> {
    let bytes = require_git_ok(repo_dir, &["rev-parse", "--git-common-dir"])?;
    let raw = String::from_utf8_lossy(&bytes).trim().to_string();
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        Ok(p)
    } else {
        Ok(repo_dir.join(p))
    }
}

pub fn read_ref(repo_dir: &Path, ref_name: &str) -> Result<Option<CommitOid>, GitError> {
    let out = run_git(repo_dir, &["rev-parse", "--verify", "--quiet", ref_name])?;
    if !out.status.success() {
        return Ok(None);
    }
    let hex = String::from_utf8_lossy(&out.stdout).trim().to_string();
    CommitOid::parse(&hex)
        .map(Some)
        .map_err(|e| GitError::InvalidOutput(e.to_string()))
}

/// Idempotently creates a locked detached git worktree for a write subagent.
/// Reconciles stale worktree metadata first so crashes never wedge retries.
pub fn create_worktree(
    repo_dir: &Path,
    worktree_path: &Path,
    task_id: &TaskId,
    base: &CommitOid,
) -> Result<PathBuf, GitError> {
    let _ = run_git(repo_dir, &["worktree", "prune"]);
    if worktree_path.exists() {
        let _ = remove_worktree(repo_dir, worktree_path);
    }
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| GitError::Io(e.to_string()))?;
    }
    let wt_str = worktree_path
        .to_str()
        .ok_or_else(|| GitError::InvalidOutput("non-utf8 worktree path".to_string()))?;
    require_git_ok(
        repo_dir,
        &[
            "worktree",
            "add",
            "--detach",
            "--lock",
            "--reason",
            task_id.as_str(),
            wt_str,
            base.as_str(),
        ],
    )?;
    Ok(worktree_path.to_path_buf())
}

/// Idempotently removes a locked or unlocked worktree and prunes metadata.
pub fn remove_worktree(repo_dir: &Path, worktree_path: &Path) -> Result<(), GitError> {
    let wt_str = worktree_path
        .to_str()
        .ok_or_else(|| GitError::InvalidOutput("non-utf8 worktree path".to_string()))?;
    let _ = run_git(
        repo_dir,
        &["worktree", "remove", "--force", "--force", wt_str],
    );
    if worktree_path.exists() {
        std::fs::remove_dir_all(worktree_path).map_err(|e| GitError::Io(e.to_string()))?;
    }
    let _ = run_git(repo_dir, &["worktree", "prune"]);
    Ok(())
}

/// Stages all non-gitignored changes inside a worktree and creates a commit.
pub fn commit_worktree_changes(worktree_dir: &Path, message: &str) -> Result<CommitOid, GitError> {
    require_git_ok(worktree_dir, &["add", "--all"])?;
    require_git_ok(
        worktree_dir,
        &[
            "-c",
            "user.name=Verity Worker",
            "-c",
            "user.email=worker@verity.local",
            "commit",
            "--allow-empty",
            "-m",
            message,
        ],
    )?;
    read_ref(worktree_dir, "HEAD")?
        .ok_or_else(|| GitError::InvalidOutput("missing HEAD after commit".to_string()))
}

/// Extracts changed paths between two commits via NUL-delimited git diff.
pub fn changed_paths(
    repo_dir: &Path,
    base: &CommitOid,
    head: &CommitOid,
) -> Result<Vec<RelPath>, GitError> {
    let bytes = require_git_ok(
        repo_dir,
        &[
            "diff",
            "--no-ext-diff",
            "--no-renames",
            "--name-only",
            "-z",
            base.as_str(),
            head.as_str(),
        ],
    )?;
    let mut out = Vec::new();
    for raw in bytes.split(|&b| b == 0) {
        if raw.is_empty() {
            continue;
        }
        let s = std::str::from_utf8(raw)
            .map_err(|_| GitError::InvalidOutput("non-utf8 path in git diff".to_string()))?;
        let rel = RelPath::parse(s).map_err(|e| GitError::InvalidOutput(e.to_string()))?;
        out.push(rel);
    }
    Ok(out)
}

/// Lock-free 3-way merge via `git merge-tree --write-tree` (Git 2.38+).
/// Distinguishes clean merges from structured conflicts instead of throwing.
pub fn merge_tree(
    repo_dir: &Path,
    target_oid: &CommitOid,
    candidate_oid: &CommitOid,
) -> Result<MergeOutcome, GitError> {
    let out = run_git(
        repo_dir,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            target_oid.as_str(),
            candidate_oid.as_str(),
        ],
    )?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines().filter(|l| !l.trim().is_empty());
    let first = lines
        .next()
        .ok_or_else(|| GitError::InvalidOutput("empty merge-tree output".to_string()))?;

    if out.status.success() {
        let tree_oid = CommitOid::parse(first.trim())
            .map_err(|e| GitError::InvalidOutput(format!("bad tree oid: {}", e)))?;
        return Ok(MergeOutcome::Clean { tree_oid });
    }

    if out.status.code() == Some(1) {
        let mut conflicted = Vec::new();
        for line in lines {
            if let Ok(rel) = RelPath::parse(line.trim()) {
                conflicted.push(rel);
            }
        }
        return Ok(MergeOutcome::Conflict {
            conflicted_paths: conflicted,
        });
    }

    Err(GitError::CommandFailed {
        args: vec!["merge-tree".to_string()],
        code: out.status.code(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

pub fn commit_tree(
    repo_dir: &Path,
    tree_oid: &CommitOid,
    parents: &[&CommitOid],
    message: &str,
) -> Result<CommitOid, GitError> {
    let mut args: Vec<String> = vec!["commit-tree".to_string(), tree_oid.as_str().to_string()];
    for p in parents {
        args.push("-p".to_string());
        args.push(p.as_str().to_string());
    }
    args.push("-m".to_string());
    args.push(message.to_string());
    let str_args: Vec<&str> = args.iter().map(String::as_str).collect();
    let bytes = require_git_ok(repo_dir, &str_args)?;
    let hex = String::from_utf8_lossy(&bytes).trim().to_string();
    CommitOid::parse(&hex).map_err(|e| GitError::InvalidOutput(e.to_string()))
}

/// Atomic compare-and-swap ref update via `git update-ref --no-deref`.
pub fn update_ref_cas(
    repo_dir: &Path,
    ref_name: &str,
    new_oid: &CommitOid,
    expected_oid: Option<&CommitOid>,
) -> Result<(), GitError> {
    let zero_oid = "0".repeat(new_oid.as_str().len());
    let expected = expected_oid.map(|o| o.as_str()).unwrap_or(&zero_oid);
    require_git_ok(
        repo_dir,
        &[
            "update-ref",
            "--no-deref",
            ref_name,
            new_oid.as_str(),
            expected,
        ],
    )?;
    Ok(())
}
