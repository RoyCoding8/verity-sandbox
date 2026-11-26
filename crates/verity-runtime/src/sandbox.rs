use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub worktree_dir: PathBuf,
    pub git_common_dir: Option<PathBuf>,
    pub allow_network: bool,
    /// Additional trusted installation files visible to a Pi worker.
    pub read_only_paths: Vec<PathBuf>,
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
    bwrap_command()
        .and_then(|mut cmd| {
            cmd.args(["--ro-bind", "/", "/", "--unshare-net", "--", "/bin/true"])
                .output()
                .map_err(|e| SandboxError::Io(e.to_string()))
        })
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Whether this host can start a Pi worker with the Linux isolation profile.
pub fn pi_sandbox_available() -> bool {
    cfg!(target_os = "linux")
        && pi_linux_command(false)
            .and_then(|mut cmd| {
                cmd.args(["--remount-ro", "/", "--chdir", "/", "--", "/bin/true"])
                    .output()
                    .map_err(|e| SandboxError::Io(e.to_string()))
            })
            .is_ok_and(|output| output.status.success())
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
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        return Err(SandboxError::NotAvailable(
            "unsupported OS for native sandbox".to_string(),
        ));
    }
    let worktree = canonical_dir(&cfg.worktree_dir)?;
    let git_dir = cfg
        .git_common_dir
        .as_deref()
        .map(canonical_dir)
        .transpose()?;
    let mut cmd = if cfg!(target_os = "linux") {
        linux_command(cfg.allow_network, &worktree, git_dir.as_deref())?
    } else {
        macos_command(cfg.allow_network, &worktree, git_dir.as_deref())?
    };
    configure_environment(&mut cmd, cfg, &worktree);
    cmd.arg(program)
        .args(args)
        .output()
        .map_err(|e| SandboxError::Io(e.to_string()))
}

pub(crate) fn pi_sandboxed_command(
    cfg: &SandboxConfig,
    program: &str,
    args: &[&str],
) -> Result<Command, SandboxError> {
    if !cfg!(target_os = "linux") {
        return Err(SandboxError::NotAvailable(
            "Pi workers require Linux bubblewrap with PID namespace support".into(),
        ));
    }
    let mut cmd = pi_linux_command(cfg.allow_network)?;
    let worktree = canonical_dir(&cfg.worktree_dir)?;
    if worktree == Path::new("/") {
        return Err(SandboxError::Io("invalid Pi worktree root".into()));
    }
    cmd.arg("--bind").arg(&worktree).arg(&worktree);
    for path in &cfg.read_only_paths {
        bind_read_only(&mut cmd, &pi_installation_path(path, &worktree)?);
    }
    let program = Path::new(program);
    let executable = if program.is_absolute() {
        let executable = pi_installation_path(program, &worktree)?;
        if !executable.is_file() {
            return Err(SandboxError::Io("Pi executable must be a file".into()));
        }
        if !PI_RUNTIME_PATHS
            .iter()
            .any(|path| executable.starts_with(path))
        {
            bind_read_only(&mut cmd, &executable);
        }
        executable
    } else {
        program.to_owned()
    };
    if let Some(git_dir) = &cfg.git_common_dir {
        bind_read_only(&mut cmd, &canonical_dir(git_dir)?);
    }
    let git_marker = worktree.join(".git");
    match git_marker.symlink_metadata() {
        Ok(meta) if meta.is_file() || meta.is_dir() => bind_read_only(&mut cmd, &git_marker),
        Ok(_) => {
            return Err(SandboxError::Io(
                "unexpected Pi worktree .git file type".into(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(SandboxError::Io(error.to_string())),
    }
    cmd.args(["--remount-ro", "/", "--chdir"])
        .arg(&worktree)
        .arg("--")
        .arg(executable)
        .args(args);
    configure_environment(&mut cmd, cfg, &worktree);
    if !cfg
        .env_passthrough
        .iter()
        .any(|(key, _)| key == "PI_CODING_AGENT_DIR")
    {
        cmd.env("PI_CODING_AGENT_DIR", "/tmp/verity-pi-agent");
    }
    Ok(cmd)
}

fn configure_environment(cmd: &mut Command, cfg: &SandboxConfig, worktree: &Path) {
    cmd.current_dir(worktree)
        .envs(
            cfg.env_passthrough
                .iter()
                .filter(|(key, _)| !key.starts_with("LD_") && !key.starts_with("DYLD_"))
                .map(|(key, value)| (key, value)),
        )
        .env("HOME", worktree)
        .env("TMPDIR", "/tmp");
}

const PI_RUNTIME_PATHS: &[&str] = &[
    "/usr",
    "/bin",
    "/lib",
    "/lib64",
    "/etc/group",
    "/etc/hosts",
    "/etc/ld.so.cache",
    "/etc/nsswitch.conf",
    "/etc/passwd",
    "/etc/pki",
    "/etc/resolv.conf",
    "/etc/ssl",
];

fn pi_linux_command(allow_network: bool) -> Result<Command, SandboxError> {
    let mut cmd = bwrap_command()?;
    cmd.args(["--unshare-user", "--disable-userns", "--cap-drop", "ALL"])
        .args(["--dev", "/dev", "--proc", "/proc", "--remount-ro", "/proc"])
        .args(["--tmpfs", "/tmp"]);
    for path in PI_RUNTIME_PATHS {
        cmd.args(["--ro-bind-try", path, path]);
    }
    if !allow_network {
        cmd.arg("--unshare-net");
    }
    Ok(cmd)
}

fn pi_installation_path(path: &Path, worktree: &Path) -> Result<PathBuf, SandboxError> {
    if !path.is_absolute() {
        return Err(SandboxError::Io(
            "Pi installation paths must be absolute".into(),
        ));
    }
    let path = path
        .canonicalize()
        .map_err(|e| SandboxError::Io(format!("{}: {e}", path.display())))?;
    let host_home = std::env::var_os("HOME").and_then(|home| Path::new(&home).canonicalize().ok());
    if ["/dev", "/proc", "/run", "/sys"]
        .iter()
        .any(|root| path.starts_with(root))
        || (path.is_dir()
            && (path.components().count() < 3
                || worktree.starts_with(&path)
                || path.parent() == Some(Path::new("/home"))
                || host_home.is_some_and(|home| home.starts_with(&path))))
        || !(path.is_file() || path.is_dir())
    {
        return Err(SandboxError::Io(format!(
            "invalid Pi installation path: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn bind_read_only(cmd: &mut Command, path: &Path) {
    cmd.arg("--ro-bind").arg(path).arg(path);
}

fn canonical_dir(path: &Path) -> Result<PathBuf, SandboxError> {
    let canonical = path
        .canonicalize()
        .map_err(|e| SandboxError::Io(format!("{}: {e}", path.display())))?;
    if !canonical.is_dir() {
        return Err(SandboxError::Io(format!(
            "{} is not a directory",
            path.display()
        )));
    }
    Ok(canonical)
}

fn bwrap_command() -> Result<Command, SandboxError> {
    let mut cmd = launcher(&["/usr/bin/bwrap", "/usr/local/bin/bwrap"])?;
    cmd.env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .args(["--unshare-pid", "--unshare-ipc", "--die-with-parent"]);
    Ok(cmd)
}

fn launcher(paths: &[&str]) -> Result<Command, SandboxError> {
    paths
        .iter()
        .find(|path| Path::new(path).is_file())
        .map(Command::new)
        .ok_or_else(|| {
            SandboxError::NotAvailable(format!("launcher missing at {}", paths.join(" or ")))
        })
}

fn linux_command(
    allow_network: bool,
    worktree: &Path,
    git_dir: Option<&Path>,
) -> Result<Command, SandboxError> {
    let mut cmd = bwrap_command()?;
    cmd.args(["--ro-bind", "/", "/"])
        .args(["--dev", "/dev"])
        .args(["--proc", "/proc"])
        .args(["--tmpfs", "/tmp"])
        .arg("--bind")
        .arg(worktree)
        .arg(worktree)
        .arg("--chdir")
        .arg(worktree);

    if !allow_network {
        cmd.arg("--unshare-net");
    }
    if let Some(git_dir) = git_dir {
        cmd.arg("--ro-bind").arg(git_dir).arg(git_dir);
    }
    cmd.arg("--");
    Ok(cmd)
}

fn macos_command(
    allow_network: bool,
    worktree: &Path,
    git_dir: Option<&Path>,
) -> Result<Command, SandboxError> {
    let mut cmd = launcher(&["/usr/bin/sandbox-exec"])?;
    let mut profile = format!(
        "(version 1)\n(deny default)\n(allow process*)\n(allow file-read*)\n(allow file-write* (subpath {}) (subpath \"/private/tmp\"))\n(allow file-write-data (literal \"/dev/null\"))\n",
        seatbelt_path(worktree)?,
    );
    if let Some(git_dir) = git_dir {
        profile.push_str(&format!(
            "(deny file-write* (subpath {}))\n",
            seatbelt_path(git_dir)?
        ));
    }
    if allow_network {
        profile.push_str("(allow network*)\n");
    }
    cmd.env_clear()
        .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
        .args(["-p", &profile]);
    Ok(cmd)
}

fn seatbelt_path(path: &Path) -> Result<String, SandboxError> {
    let path = path
        .to_str()
        .filter(|s| !s.chars().any(char::is_control))
        .ok_or_else(|| {
            SandboxError::Io("path cannot be represented in the Seatbelt profile".into())
        })?;
    Ok(format!(
        "\"{}\"",
        path.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

pub fn default_offline_config(
    worktree_dir: &Path,
    git_common_dir: Option<PathBuf>,
) -> SandboxConfig {
    SandboxConfig {
        worktree_dir: worktree_dir.to_path_buf(),
        git_common_dir,
        allow_network: false,
        read_only_paths: Vec::new(),
        env_passthrough: Vec::new(),
    }
}
