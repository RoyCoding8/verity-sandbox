use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub worktree_dir: PathBuf,
    pub git_common_dir: Option<PathBuf>,
    pub allow_network: bool,
    pub env_passthrough: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    NotAvailable(String),
    Io(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::NotAvailable(msg) => write!(f, "sandbox unavailable: {}", msg),
            SandboxError::Io(msg) => write!(f, "sandbox io error: {}", msg),
        }
    }
}

impl std::error::Error for SandboxError {}

pub fn bwrap_available() -> bool {
    Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--", "/bin/true"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Runs `program` inside an OS sandbox:
/// - Linux: `bubblewrap` with read-only root, writable `worktree_dir`,
///   read-only `$GIT_COMMON_DIR` (preventing `.git/hooks` tampering), and
///   `--unshare-net` when `allow_network` is false.
/// - macOS: `sandbox-exec` Seatbelt profile.
pub fn run_sandboxed(
    cfg: &SandboxConfig,
    program: &str,
    args: &[&str],
) -> Result<Output, SandboxError> {
    if cfg!(target_os = "linux") {
        run_bwrap(cfg, program, args)
    } else if cfg!(target_os = "macos") {
        run_seatbelt(cfg, program, args)
    } else {
        Err(SandboxError::NotAvailable(
            "unsupported OS for native sandbox".to_string(),
        ))
    }
}

fn run_bwrap(cfg: &SandboxConfig, program: &str, args: &[&str]) -> Result<Output, SandboxError> {
    let wt = cfg
        .worktree_dir
        .canonicalize()
        .map_err(|e| SandboxError::Io(e.to_string()))?;
    let wt_str = wt
        .to_str()
        .ok_or_else(|| SandboxError::Io("non-utf8 worktree path".to_string()))?;

    let mut cmd = Command::new("bwrap");
    cmd.env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", wt_str)
        .env("TMPDIR", "/tmp")
        .args(["--ro-bind", "/", "/"])
        .args(["--dev", "/dev"])
        .args(["--proc", "/proc"])
        .args(["--tmpfs", "/tmp"])
        .args(["--bind", wt_str, wt_str])
        .args(["--chdir", wt_str])
        .arg("--die-with-parent");

    if !cfg.allow_network {
        cmd.arg("--unshare-net");
    }

    if let Some(git_dir) = &cfg.git_common_dir {
        if let Ok(canon) = git_dir.canonicalize() {
            if let Some(s) = canon.to_str() {
                cmd.args(["--ro-bind", s, s]);
            }
        }
    }

    for (k, v) in &cfg.env_passthrough {
        cmd.env(k, v);
    }

    cmd.arg("--").arg(program).args(args);
    cmd.output().map_err(|e| SandboxError::Io(e.to_string()))
}

fn run_seatbelt(cfg: &SandboxConfig, program: &str, args: &[&str]) -> Result<Output, SandboxError> {
    let wt = cfg
        .worktree_dir
        .canonicalize()
        .map_err(|e| SandboxError::Io(e.to_string()))?;
    let wt_str = wt
        .to_str()
        .ok_or_else(|| SandboxError::Io("non-utf8 worktree path".to_string()))?;
    let net_rule = if cfg.allow_network {
        "(allow network*)"
    } else {
        "(deny network*)"
    };
    let profile = format!(
        "(version 1)\n(deny default)\n(allow process*)\n(allow file-read*)\n(allow file-write* (subpath \"{}\") (subpath \"/tmp\"))\n{}\n",
        wt_str, net_rule
    );

    let mut cmd = Command::new("sandbox-exec");
    cmd.current_dir(&wt)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", wt_str)
        .args(["-p", &profile, program])
        .args(args);

    for (k, v) in &cfg.env_passthrough {
        cmd.env(k, v);
    }

    cmd.output().map_err(|e| SandboxError::Io(e.to_string()))
}

pub fn default_offline_config(worktree_dir: &Path, git_common_dir: Option<PathBuf>) -> SandboxConfig {
    SandboxConfig {
        worktree_dir: worktree_dir.to_path_buf(),
        git_common_dir,
        allow_network: false,
        env_passthrough: Vec::new(),
    }
}
