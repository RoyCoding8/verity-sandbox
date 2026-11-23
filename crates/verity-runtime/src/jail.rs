use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use verity_core::{within, RelPath};

pub const MAX_READ_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JailError {
    InvalidPath(String),
    ScopeDenied(String),
    SymlinkForbidden(String),
    FileTooLarge(String),
    Io(String),
}

impl fmt::Display for JailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JailError::InvalidPath(m) => write!(f, "invalid path: {}", m),
            JailError::ScopeDenied(m) => write!(f, "scope denied: {}", m),
            JailError::SymlinkForbidden(m) => write!(f, "symlink forbidden: {}", m),
            JailError::FileTooLarge(m) => write!(f, "file exceeds 256 KB cap: {}", m),
            JailError::Io(m) => write!(f, "jail io error: {}", m),
        }
    }
}

impl std::error::Error for JailError {}

/// Standalone workspace path jail salvaged from archive/verity-prototype/src/pi-worker.mjs
/// (lines 112-195) with O_NOFOLLOW, component-wise symlink checks, and safe search skipping.
#[derive(Debug, Clone)]
pub struct WorkspaceJail {
    root: PathBuf,
    read_scope: Vec<RelPath>,
    write_scope: Vec<RelPath>,
    forbidden: Vec<RelPath>,
}

impl WorkspaceJail {
    pub fn new(
        root: PathBuf,
        read_scope: Vec<RelPath>,
        write_scope: Vec<RelPath>,
        forbidden: Vec<RelPath>,
    ) -> Result<Self, JailError> {
        let canon = root.canonicalize().map_err(|e| JailError::Io(e.to_string()))?;
        Ok(Self {
            root: canon,
            read_scope,
            write_scope,
            forbidden,
        })
    }

    fn check_read_scope(&self, rel: &RelPath) -> Result<(), JailError> {
        if self.forbidden.iter().any(|f| within(rel, f)) {
            return Err(JailError::ScopeDenied(rel.as_str().to_string()));
        }
        if !self.read_scope.is_empty() && !self.read_scope.iter().any(|r| within(rel, r)) {
            return Err(JailError::ScopeDenied(rel.as_str().to_string()));
        }
        Ok(())
    }

    fn check_write_scope(&self, rel: &RelPath) -> Result<(), JailError> {
        if self.forbidden.iter().any(|f| within(rel, f)) {
            return Err(JailError::ScopeDenied(rel.as_str().to_string()));
        }
        if !self.write_scope.iter().any(|w| within(rel, w)) {
            return Err(JailError::ScopeDenied(rel.as_str().to_string()));
        }
        Ok(())
    }

    fn verify_no_symlinks(&self, rel: &RelPath) -> Result<PathBuf, JailError> {
        let mut cur = self.root.clone();
        for seg in rel.as_str().split('/') {
            cur.push(seg);
            match fs::symlink_metadata(&cur) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        return Err(JailError::SymlinkForbidden(rel.as_str().to_string()));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Remaining components do not exist yet (valid for writes).
                }
                Err(e) => return Err(JailError::Io(e.to_string())),
            }
        }
        Ok(cur)
    }

    pub fn read_text(&self, raw_rel: &str) -> Result<String, JailError> {
        let rel = RelPath::parse(raw_rel).map_err(|e| JailError::InvalidPath(e.to_string()))?;
        self.check_read_scope(&rel)?;
        let full = self.verify_no_symlinks(&rel)?;

        let mut opts = OpenOptions::new();
        opts.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(libc_o_nofollow());
        }
        let mut file = opts.open(&full).map_err(|e| JailError::Io(e.to_string()))?;
        let meta = file.metadata().map_err(|e| JailError::Io(e.to_string()))?;
        if !meta.is_file() {
            return Err(JailError::InvalidPath("not a regular file".to_string()));
        }
        if meta.len() as usize > MAX_READ_BYTES {
            return Err(JailError::FileTooLarge(rel.as_str().to_string()));
        }
        let mut buf = String::new();
        file.read_to_string(&mut buf)
            .map_err(|e| JailError::Io(e.to_string()))?;
        Ok(buf)
    }

    pub fn write_text(&self, raw_rel: &str, contents: &str) -> Result<(), JailError> {
        let rel = RelPath::parse(raw_rel).map_err(|e| JailError::InvalidPath(e.to_string()))?;
        self.check_write_scope(&rel)?;
        let full = self.verify_no_symlinks(&rel)?;

        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(|e| JailError::Io(e.to_string()))?;
            let canon_parent = parent
                .canonicalize()
                .map_err(|e| JailError::Io(e.to_string()))?;
            if !canon_parent.starts_with(&self.root) {
                return Err(JailError::SymlinkForbidden(rel.as_str().to_string()));
            }
        }

        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(libc_o_nofollow());
        }
        let mut file = opts.open(&full).map_err(|e| JailError::Io(e.to_string()))?;
        file.write_all(contents.as_bytes())
            .map_err(|e| JailError::Io(e.to_string()))
    }

    /// Searches files in scope, safely skipping oversized or binary files
    /// (fixes archive/verity-prototype/src/pi-worker.mjs line 244).
    pub fn search_substring(&self, needle: &str, limit: usize) -> Result<Vec<(String, usize, String)>, JailError> {
        let mut hits = Vec::new();
        self.visit_dir(&self.root, "", needle, limit, &mut hits)?;
        Ok(hits)
    }

    fn visit_dir(
        &self,
        dir: &Path,
        prefix: &str,
        needle: &str,
        limit: usize,
        hits: &mut Vec<(String, usize, String)>,
    ) -> Result<(), JailError> {
        if hits.len() >= limit {
            return Ok(());
        }
        let entries = fs::read_dir(dir).map_err(|e| JailError::Io(e.to_string()))?;
        for entry in entries {
            if hits.len() >= limit {
                break;
            }
            let entry = entry.map_err(|e| JailError::Io(e.to_string()))?;
            let name = match entry.file_name().into_string() {
                Ok(n) => n,
                Err(_) => continue,
            };
            if name.eq_ignore_ascii_case(".git") {
                continue;
            }
            let rel_str = if prefix.is_empty() {
                name
            } else {
                format!("{}/{}", prefix, name)
            };
            let meta = match fs::symlink_metadata(entry.path()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                self.visit_dir(&entry.path(), &rel_str, needle, limit, hits)?;
            } else if meta.is_file() {
                if let Ok(text) = self.read_text(&rel_str) {
                    for (idx, line) in text.lines().enumerate() {
                        if line.contains(needle) {
                            hits.push((rel_str.clone(), idx + 1, line.to_string()));
                            if hits.len() >= limit {
                                break;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn libc_o_nofollow() -> i32 {
    0x20000
}

#[cfg(target_os = "macos")]
fn libc_o_nofollow() -> i32 {
    0x0100
}

#[cfg(all(unix, not(target_os = "linux"), not(target_os = "macos")))]
fn libc_o_nofollow() -> i32 {
    0
}
