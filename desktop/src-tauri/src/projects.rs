use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
use verity_core::json::{parse_json, JsonValue};

static PROJECTS_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static PROJECTS_WRITE_LOCK: Mutex<()> = Mutex::new(());
static SESSION_META_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiProject {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiSessionSummary {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub updated_at: String,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetaStore {
    #[serde(default)]
    pub archived_session_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRegistry {
    #[serde(default)]
    pub projects: Vec<PiProject>,
    #[serde(default)]
    pub active_project_path: Option<String>,
}

fn non_empty_env(var: &str) -> Option<String> {
    env::var(var)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Resolves the Verity config directory, respecting `VERITY_HOME` first for deterministic tests,
/// otherwise falling back to `~/.verity`.
pub fn default_verity_home() -> PathBuf {
    if let Some(vh) = non_empty_env("VERITY_HOME") {
        return PathBuf::from(vh);
    }
    if let Some(home) = non_empty_env("HOME").or_else(|| non_empty_env("USERPROFILE")) {
        return PathBuf::from(home).join(".verity");
    }
    if let (Some(drive), Some(path)) = (non_empty_env("HOMEDRIVE"), non_empty_env("HOMEPATH")) {
        return PathBuf::from(format!("{}{}", drive, path)).join(".verity");
    }
    PathBuf::from(".verity")
}

/// Default path for the persistent project registry (`~/.verity/projects.json` or `$VERITY_HOME/projects.json`).
pub fn default_projects_store_path() -> PathBuf {
    default_verity_home().join("projects.json")
}

/// Default path for persistent session archive metadata (`~/.verity/session-meta.json` or `$VERITY_HOME/session-meta.json`).
pub fn default_session_meta_path() -> PathBuf {
    default_verity_home().join("session-meta.json")
}

/// Loads archived session IDs from an explicit `session-meta.json` path.
pub fn load_archived_session_ids_from(meta_path: &Path) -> HashSet<String> {
    if !meta_path.exists() {
        return HashSet::new();
    }
    let Ok(raw) = fs::read_to_string(meta_path) else {
        return HashSet::new();
    };
    if raw.trim().is_empty() {
        return HashSet::new();
    }
    serde_json::from_str::<SessionMetaStore>(&raw)
        .map(|store| {
            store
                .archived_session_ids
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Loads archived session IDs from `~/.verity/session-meta.json` (respecting `VERITY_HOME`).
pub fn load_archived_session_ids() -> HashSet<String> {
    load_archived_session_ids_from(&default_session_meta_path())
}

/// Atomically updates the archived flag for `session_id` at an explicit `session-meta.json` path.
pub fn set_session_archived_at(
    meta_path: &Path,
    session_id: &str,
    archived: bool,
) -> Result<HashSet<String>, String> {
    let sid = session_id.trim();
    if sid.is_empty() {
        return Err("sessionId must not be empty".to_string());
    }
    let _guard = SESSION_META_WRITE_LOCK
        .lock()
        .expect("session meta write lock poisoned");

    let mut set = load_archived_session_ids_from(meta_path);
    if archived {
        set.insert(sid.to_string());
    } else {
        set.remove(sid);
    }

    let mut sorted: Vec<String> = set.iter().cloned().collect();
    sorted.sort();
    let store = SessionMetaStore {
        archived_session_ids: sorted,
    };

    if let Some(parent) = meta_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create session meta dir: {}", e))?;
    }
    let json = serde_json::to_string_pretty(&store)
        .map_err(|e| format!("failed to serialize session meta: {}", e))?;
    let n = PROJECTS_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(
        ".{}.tmp.{}.{}",
        meta_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("session-meta.json"),
        std::process::id(),
        n
    );
    let tmp_path = meta_path.with_file_name(tmp_name);
    fs::write(&tmp_path, json.as_bytes())
        .map_err(|e| format!("failed to write temporary session meta: {}", e))?;
    fs::rename(&tmp_path, meta_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("failed to commit session meta: {}", e)
    })?;
    Ok(set)
}

/// Persists the archived flag for `session_id` in `~/.verity/session-meta.json` (respecting `VERITY_HOME`).
pub fn set_session_archived(session_id: &str, archived: bool) -> Result<(), String> {
    set_session_archived_at(&default_session_meta_path(), session_id, archived).map(|_| ())
}

fn is_in_project_local_pi_sessions(canon_file: &Path) -> bool {
    let Some(parent) = canon_file.parent() else {
        return false;
    };
    let Some(grandparent) = parent.parent() else {
        return false;
    };
    parent.file_name().and_then(|n| n.to_str()) == Some("sessions")
        && grandparent.file_name().and_then(|n| n.to_str()) == Some(".pi")
}

/// Safely deletes a Pi `.jsonl` session file after verifying it resides inside Pi's sessions
/// directory or a project-local `.pi/sessions/` directory. Returns `Ok(true)` if deleted,
/// `Ok(false)` if the file did not exist or `session_path` was empty.
pub fn delete_session_file_with_root(
    session_path: &str,
    pi_sessions_root: Option<&Path>,
) -> Result<bool, String> {
    let trimmed = session_path.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    let path = Path::new(trimmed);
    let is_jsonl = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"));
    if !is_jsonl {
        return Err(format!(
            "refusing to delete non-jsonl session file: {}",
            trimmed
        ));
    }
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_file() {
        return Err(format!("session path is not a regular file: {}", trimmed));
    }
    let canon_file = fs::canonicalize(path)
        .map_err(|e| format!("failed to canonicalize session file path: {}", e))?;

    let in_global_root = pi_sessions_root
        .and_then(|root| fs::canonicalize(root).ok())
        .is_some_and(|canon_root| canon_file.starts_with(&canon_root));
    let in_project_local = is_in_project_local_pi_sessions(&canon_file);

    if !in_global_root && !in_project_local {
        return Err(format!(
            "refusing to delete file outside Pi sessions directory: {}",
            canon_file.display()
        ));
    }

    fs::remove_file(&canon_file).map_err(|e| format!("failed to delete session file: {}", e))?;
    Ok(true)
}

/// Safely deletes a Pi `.jsonl` session file using the default Pi sessions root.
pub fn delete_session_file(session_path: &str) -> Result<bool, String> {
    let global_root = default_pi_sessions_root();
    delete_session_file_with_root(session_path, global_root.as_deref())
}

/// Canonicalizes a path if possible, falling back to the original path if it cannot be canonicalized.
pub fn normalize_project_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Validates that `path` exists and is a directory, returning its canonicalized `PathBuf`.
pub fn validate_project_dir(path: &Path) -> Result<PathBuf, String> {
    let raw = path.to_string_lossy();
    if raw.trim().is_empty() {
        return Err("project path must not be empty".to_string());
    }
    if !path.exists() {
        return Err(format!("project directory does not exist: {}", raw));
    }
    if !path.is_dir() {
        return Err(format!("project path is not a directory: {}", raw));
    }
    fs::canonicalize(path).map_err(|e| format!("failed to canonicalize project path: {}", e))
}

/// Computes a deterministic 64-bit FNV-1a hash of the canonical path string.
fn fnv1a_64(input: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in input {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Derives a deterministic, collision-resistant project ID from a canonicalized path.
pub fn derive_project_id(canonical_path: &Path) -> String {
    let path_str = canonical_path.to_string_lossy();
    let name = canonical_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("workspace");
    let slug: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let hash = fnv1a_64(path_str.as_bytes());
    format!("proj-{}-{:08x}", slug.trim_matches('-'), hash & 0xffff_ffff)
}

/// Builds a `PiProject` record from a directory path (canonicalizing when possible).
pub fn project_from_path(path: &Path) -> PiProject {
    let canonical = normalize_project_path(path);
    let name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("workspace")
        .to_string();
    PiProject {
        id: derive_project_id(&canonical),
        name,
        path: canonical.to_string_lossy().to_string(),
    }
}

impl ProjectRegistry {
    /// Loads a `ProjectRegistry` from disk, returning default empty registry if file does not exist.
    pub fn load(store_path: &Path) -> Result<Self, String> {
        if !store_path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(store_path)
            .map_err(|e| format!("failed to read projects store: {}", e))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str::<Self>(&raw)
            .map_err(|e| format!("failed to parse projects store: {}", e))
    }

    /// Atomically persists the project registry to disk.
    pub fn save_atomic(&self, store_path: &Path) -> Result<(), String> {
        let _guard = PROJECTS_WRITE_LOCK
            .lock()
            .expect("projects write lock poisoned");
        if let Some(parent) = store_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create projects store dir: {}", e))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize projects store: {}", e))?;
        let n = PROJECTS_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_name = format!(
            ".{}.tmp.{}.{}",
            store_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("projects.json"),
            std::process::id(),
            n
        );
        let tmp_path = store_path.with_file_name(tmp_name);
        fs::write(&tmp_path, json.as_bytes())
            .map_err(|e| format!("failed to write temporary projects store: {}", e))?;
        fs::rename(&tmp_path, store_path).map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            format!("failed to commit projects store: {}", e)
        })
    }

    /// Returns the currently active `PiProject`, if any.
    pub fn active_project(&self) -> Option<&PiProject> {
        if let Some(active_path) = self.active_project_path.as_deref() {
            if let Some(found) = self.projects.iter().find(|p| p.path == active_path) {
                return Some(found);
            }
        }
        self.projects.first()
    }

    /// Validates, deduplicates, and adds a project directory, marking it active.
    pub fn add_project(&mut self, path: &Path) -> Result<PiProject, String> {
        let canonical = validate_project_dir(path)?;
        let canonical_str = canonical.to_string_lossy().to_string();

        if let Some(existing) = self.projects.iter().find(|p| {
            p.path == canonical_str || normalize_project_path(Path::new(&p.path)) == canonical
        }) {
            let project = existing.clone();
            self.active_project_path = Some(project.path.clone());
            return Ok(project);
        }

        let project = project_from_path(&canonical);
        self.active_project_path = Some(project.path.clone());
        self.projects.push(project.clone());
        Ok(project)
    }

    /// Removes a project matching `path`. If it was active and another project remains,
    /// switches `active_project_path` to the next available project.
    /// Returns `(removed, active_changed, new_active_project)`.
    pub fn remove_project(
        &mut self,
        path: &Path,
    ) -> Result<(bool, bool, Option<PiProject>), String> {
        let raw_str = path.to_string_lossy().to_string();
        if raw_str.trim().is_empty() {
            return Err("project path must not be empty".to_string());
        }
        let target_canonical = normalize_project_path(path);
        let target_str = target_canonical.to_string_lossy().to_string();

        let prev_active = self.active_project_path.clone();
        let before_len = self.projects.len();
        self.projects.retain(|p| {
            p.path != raw_str
                && p.path != target_str
                && normalize_project_path(Path::new(&p.path)) != target_canonical
        });
        let removed = self.projects.len() < before_len;

        let active_was_removed = prev_active.as_deref().is_some_and(|active| {
            active == raw_str
                || active == target_str
                || normalize_project_path(Path::new(active)) == target_canonical
        });

        if active_was_removed {
            self.active_project_path = self.projects.first().map(|p| p.path.clone());
        } else if self.active_project_path.is_none() {
            self.active_project_path = self.projects.first().map(|p| p.path.clone());
        }

        let new_active = self.active_project().cloned();
        Ok((removed, active_was_removed, new_active))
    }

    /// Sets the active project to `path` (validating that the directory exists).
    /// If the directory is already registered, activates it; otherwise registers and activates it.
    pub fn set_active_project(&mut self, path: &Path) -> Result<PiProject, String> {
        self.add_project(path)
    }
}

/// Loads the project registry from `store_path`, or initializes it with `default_cwd` if empty/missing.
pub fn load_or_init_projects(store_path: &Path, default_cwd: &Path) -> ProjectRegistry {
    let mut registry = ProjectRegistry::load(store_path).unwrap_or_default();

    // Deduplicate any stale duplicate entries while preserving order.
    let mut seen = HashSet::new();
    registry.projects.retain(|p| {
        let norm = normalize_project_path(Path::new(&p.path))
            .to_string_lossy()
            .to_string();
        seen.insert(norm)
    });

    if registry.projects.is_empty() {
        let initial = project_from_path(default_cwd);
        registry.active_project_path = Some(initial.path.clone());
        registry.projects.push(initial);
        let _ = registry.save_atomic(store_path);
        return registry;
    }

    let has_valid_active = registry
        .active_project_path
        .as_deref()
        .is_some_and(|active| registry.projects.iter().any(|p| p.path == active));
    if !has_valid_active {
        registry.active_project_path = registry.projects.first().map(|p| p.path.clone());
        let _ = registry.save_atomic(store_path);
    }

    registry
}

/// Convenience wrapper for `add_project` that loads, mutates, and persists `store_path`.
pub fn add_project(store_path: &Path, path: &Path) -> Result<(ProjectRegistry, PiProject), String> {
    let mut reg = ProjectRegistry::load(store_path).unwrap_or_default();
    let project = reg.add_project(path)?;
    reg.save_atomic(store_path)?;
    Ok((reg, project))
}

/// Convenience wrapper for `remove_project` that loads, mutates, and persists `store_path`.
pub fn remove_project(
    store_path: &Path,
    path: &Path,
) -> Result<(ProjectRegistry, Option<PiProject>), String> {
    let mut reg = ProjectRegistry::load(store_path).unwrap_or_default();
    let (_, _, next_active) = reg.remove_project(path)?;
    reg.save_atomic(store_path)?;
    Ok((reg, next_active))
}

/// Convenience wrapper for `set_active_project` that loads, mutates, and persists `store_path`.
pub fn set_active_project(
    store_path: &Path,
    path: &Path,
) -> Result<(ProjectRegistry, PiProject), String> {
    let mut reg = ProjectRegistry::load(store_path).unwrap_or_default();
    let project = reg.set_active_project(path)?;
    reg.save_atomic(store_path)?;
    Ok((reg, project))
}

// ============================================================
// Pi Disk Session Scanner & Live Session Merging
// ============================================================

/// Encodes a project `cwd` path into Pi's `~/.pi/agent/sessions/<encoded-cwd>` directory name.
pub fn encode_pi_session_dir_name(cwd: &Path) -> String {
    let raw = cwd.to_string_lossy();
    let (path, prefix) = if let Some(path) = raw.strip_prefix(r"\\?\UNC\") {
        (path, "-")
    } else {
        let path = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
        (path.strip_prefix(['/', '\\']).unwrap_or(path), "")
    };
    let replaced: String = path
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            other => other,
        })
        .collect();
    format!("--{prefix}{replaced}--")
}

/// Resolves Pi's global sessions root (`$PI_CODING_AGENT_DIR/sessions` or `~/.pi/agent/sessions`).
pub fn default_pi_sessions_root() -> Option<PathBuf> {
    if let Some(agent_dir) = non_empty_env("PI_CODING_AGENT_DIR") {
        return Some(PathBuf::from(agent_dir).join("sessions"));
    }
    non_empty_env("HOME")
        .or_else(|| non_empty_env("USERPROFILE"))
        .map(|home| {
            PathBuf::from(home)
                .join(".pi")
                .join("agent")
                .join("sessions")
        })
}

/// Scans Pi `.jsonl` session files on disk for a specific project `cwd`, tagging archived state.
pub fn scan_project_sessions(project_id: &str, project_cwd: &Path) -> Vec<PiSessionSummary> {
    let global_root = default_pi_sessions_root();
    let archived_ids = load_archived_session_ids();
    scan_project_sessions_with_root_and_archived(
        project_id,
        project_cwd,
        global_root.as_deref(),
        &archived_ids,
    )
}

/// Scans Pi `.jsonl` session files on disk for a specific project `cwd` with an explicit archived set.
pub fn scan_project_sessions_with_archived(
    project_id: &str,
    project_cwd: &Path,
    archived_ids: &HashSet<String>,
) -> Vec<PiSessionSummary> {
    let global_root = default_pi_sessions_root();
    scan_project_sessions_with_root_and_archived(
        project_id,
        project_cwd,
        global_root.as_deref(),
        archived_ids,
    )
}

/// Scans Pi `.jsonl` session files from `<project>/.pi/sessions/` and `<pi_sessions_root>/<encoded-cwd>/`
/// (plus any other `<pi_sessions_root>/*/` subdirectories whose session header `cwd` matches `project_cwd`).
pub fn scan_project_sessions_with_root(
    project_id: &str,
    project_cwd: &Path,
    pi_sessions_root: Option<&Path>,
) -> Vec<PiSessionSummary> {
    let archived_ids = load_archived_session_ids();
    scan_project_sessions_with_root_and_archived(
        project_id,
        project_cwd,
        pi_sessions_root,
        &archived_ids,
    )
}

/// Scans Pi `.jsonl` session files with explicit global sessions root and archived session IDs.
pub fn scan_project_sessions_with_root_and_archived(
    project_id: &str,
    project_cwd: &Path,
    pi_sessions_root: Option<&Path>,
    archived_ids: &HashSet<String>,
) -> Vec<PiSessionSummary> {
    let canonical_cwd = normalize_project_path(project_cwd);
    let mut candidate_dirs: Vec<(PathBuf, bool)> = Vec::new();
    let mut seen_dirs: HashSet<PathBuf> = HashSet::new();

    let project_local = canonical_cwd.join(".pi").join("sessions");
    if project_local.is_dir() && seen_dirs.insert(project_local.clone()) {
        candidate_dirs.push((project_local, false));
    }

    if let Some(root) = pi_sessions_root {
        if root.is_dir() {
            let encoded_dir = root.join(encode_pi_session_dir_name(&canonical_cwd));
            if encoded_dir.is_dir() && seen_dirs.insert(encoded_dir.clone()) {
                candidate_dirs.push((encoded_dir, true));
            }
            if let Ok(entries) = fs::read_dir(root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && seen_dirs.insert(path.clone()) {
                        candidate_dirs.push((path, true));
                    }
                }
            }
        }
    }

    let mut sessions = Vec::new();
    let mut seen_files: HashSet<PathBuf> = HashSet::new();

    for (dir, require_cwd_match) in candidate_dirs {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let file_path = entry.path();
            if !file_path.is_file() {
                continue;
            }
            if file_path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let norm_file = normalize_project_path(&file_path);
            if !seen_files.insert(norm_file) {
                continue;
            }
            if let Some(mut summary) =
                parse_jsonl_session_file(&file_path, project_id, &canonical_cwd, require_cwd_match)
            {
                summary.archived = archived_ids.contains(&summary.id);
                sessions.push(summary);
            }
        }
    }

    sessions.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.path.cmp(&a.path))
    });
    sessions
}

fn opt_json_str(val: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| val.get(k).and_then(JsonValue::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn file_mtime_string(path: &Path) -> String {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|| "0".to_string())
}

/// Parses a Pi `.jsonl` session file header and scans lightweight metadata (latest session name,
/// message count, parent session) without loading full conversation content into memory.
pub fn parse_jsonl_session_file(
    file_path: &Path,
    project_id: &str,
    expected_cwd: &Path,
    require_cwd_match: bool,
) -> Option<PiSessionSummary> {
    let file = File::open(file_path).ok()?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    // Find first non-empty line (session header).
    let header_line = lines.find_map(|l| l.ok().filter(|s| !s.trim().is_empty()))?;
    let header = parse_json(&header_line).ok()?;

    match opt_json_str(&header, &["cwd"]) {
        Some(cwd) if normalize_project_path(Path::new(&cwd)) != expected_cwd => return None,
        None if require_cwd_match => return None,
        _ => {}
    }

    let file_stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();

    let id = opt_json_str(&header, &["id", "sessionId", "session_id"])
        .unwrap_or_else(|| file_stem.clone());
    let mut name = opt_json_str(&header, &["name", "sessionName", "session_name", "title"]);
    let mut updated_at = opt_json_str(&header, &["timestamp", "updatedAt", "updated_at"])
        .unwrap_or_else(|| file_mtime_string(file_path));
    let parent_session_id = opt_json_str(
        &header,
        &["parentSession", "parentSessionId", "parent_session_id"],
    );

    let mut message_count: usize = 0;
    for line_res in lines {
        let Ok(line) = line_res else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = parse_json(trimmed) else {
            continue;
        };
        let entry_type = entry.get("type").and_then(JsonValue::as_str);
        match entry_type {
            Some("message") => {
                message_count += 1;
            }
            Some("session_info") => {
                if let Some(n) = opt_json_str(&entry, &["name", "sessionName", "title"]) {
                    name = Some(n);
                }
            }
            _ => {
                if entry.get("role").and_then(JsonValue::as_str).is_some() {
                    message_count += 1;
                }
            }
        }
        if let Some(ts) = opt_json_str(&entry, &["timestamp"]) {
            updated_at = ts;
        }
    }

    Some(PiSessionSummary {
        id,
        project_id: project_id.to_string(),
        name: name.unwrap_or(file_stem),
        path: file_path.to_string_lossy().to_string(),
        updated_at,
        message_count,
        parent_session_id,
        archived: false,
    })
}

/// Merges disk-scanned sessions with an active in-memory/live session summary so the live Pi
/// session always appears without creating duplicates.
pub fn merge_live_and_disk_sessions(
    disk_sessions: Vec<PiSessionSummary>,
    live_session: Option<PiSessionSummary>,
) -> Vec<PiSessionSummary> {
    let mut merged = Vec::new();
    if let Some(live) = live_session {
        let live_path_norm = if live.path.trim().is_empty() {
            None
        } else {
            Some(normalize_project_path(Path::new(&live.path)))
        };
        merged.push(live.clone());
        for s in disk_sessions {
            let same_id = !live.id.is_empty() && s.id == live.id;
            let same_path = live_path_norm.as_ref().is_some_and(|lp| {
                !s.path.trim().is_empty() && normalize_project_path(Path::new(&s.path)) == *lp
            });
            if !same_id && !same_path {
                merged.push(s);
            }
        }
    } else {
        merged = disk_sessions;
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "verity-projects-{}-{}-{}",
            label,
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn persists_deduplicates_switches_and_removes_projects() {
        let root = unique_temp_dir("registry");
        let store_path = root.join("projects.json");
        let proj_a = root.join("alpha");
        let proj_b = root.join("beta");
        fs::create_dir_all(&proj_a).unwrap();
        fs::create_dir_all(&proj_b).unwrap();

        let reg = load_or_init_projects(&store_path, &proj_a);
        assert_eq!(reg.projects.len(), 1);
        assert_eq!(reg.projects[0].name, "alpha");
        assert!(store_path.is_file());

        // Adding the same project again (even via non-canonical path) deduplicates.
        let non_canonical_a = proj_a.join(".").join("../alpha");
        let (reg2, added_a) = add_project(&store_path, &non_canonical_a).unwrap();
        assert_eq!(reg2.projects.len(), 1);
        assert_eq!(added_a.id, reg.projects[0].id);

        // Adding a second project persists both and activates beta.
        let (reg3, added_b) = add_project(&store_path, &proj_b).unwrap();
        assert_eq!(reg3.projects.len(), 2);
        assert_eq!(
            reg3.active_project_path.as_deref(),
            Some(added_b.path.as_str())
        );

        // Non-existent directory is rejected.
        assert!(add_project(&store_path, &root.join("does-not-exist")).is_err());

        // Switching active project persists across reload.
        let (reg4, switched) = set_active_project(&store_path, &proj_a).unwrap();
        assert_eq!(switched.id, added_a.id);
        assert_eq!(
            reg4.active_project_path.as_deref(),
            Some(added_a.path.as_str())
        );
        let reloaded = ProjectRegistry::load(&store_path).unwrap();
        assert_eq!(
            reloaded.active_project_path.as_deref(),
            Some(added_a.path.as_str())
        );

        // Removing the active project switches active to the remaining project (beta).
        let (reg5, next_active) = remove_project(&store_path, &proj_a).unwrap();
        assert_eq!(reg5.projects.len(), 1);
        assert_eq!(
            next_active.as_ref().map(|p| p.id.as_str()),
            Some(added_b.id.as_str())
        );
        assert_eq!(
            reg5.active_project_path.as_deref(),
            Some(added_b.path.as_str())
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn derives_distinct_deterministic_ids_for_same_basename_paths() {
        let root = unique_temp_dir("ids");
        let a = root.join("one").join("app");
        let b = root.join("two").join("app");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();

        let id_a1 = derive_project_id(&fs::canonicalize(&a).unwrap());
        let id_a2 = derive_project_id(&fs::canonicalize(&a).unwrap());
        let id_b = derive_project_id(&fs::canonicalize(&b).unwrap());

        assert_eq!(id_a1, id_a2);
        assert_ne!(id_a1, id_b);
        assert!(id_a1.starts_with("proj-app-"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn scans_pi_disk_sessions_from_encoded_and_local_dirs_and_merges_live() {
        let root = unique_temp_dir("scanner");
        let proj = root.join("my-repo");
        let other_proj = root.join("my").join("repo");
        let pi_sessions_root = root.join("pi-sessions");
        fs::create_dir_all(&proj).unwrap();
        fs::create_dir_all(&other_proj).unwrap();

        let canon_proj = fs::canonicalize(&proj).unwrap();
        let canon_other = fs::canonicalize(&other_proj).unwrap();

        // 1. Encoded Pi agent sessions subdir
        let encoded_dir = pi_sessions_root.join(encode_pi_session_dir_name(&canon_proj));
        fs::create_dir_all(&encoded_dir).unwrap();
        let s1_path = encoded_dir.join("2026-11-20_sess-1.jsonl");
        fs::write(
            &s1_path,
            format!(
                concat!(
                    r#"{{"type":"session","version":3,"id":"sess-1","timestamp":"2026-11-20T10:00:00Z","cwd":{}}}"#,
                    "\n",
                    r#"{{"type":"message","id":"m1","timestamp":"2026-11-20T10:01:00Z","message":{{"role":"user","content":"hi"}}}}"#,
                    "\n",
                    r#"{{"type":"session_info","name":"Auth Refactor","timestamp":"2026-11-20T10:02:00Z"}}"#,
                    "\n"
                ),
                verity_core::json::escape_json_string(&canon_proj.to_string_lossy())
            ),
        )
        .unwrap();

        // 2. Project-local .pi/sessions/
        let local_sessions = canon_proj.join(".pi").join("sessions");
        fs::create_dir_all(&local_sessions).unwrap();
        let s2_path = local_sessions.join("local-2.jsonl");
        fs::write(
            &s2_path,
            format!(
                r#"{{"type":"session","id":"sess-2","name":"Local Scratch","timestamp":"2026-11-21T12:00:00Z","cwd":{}}}"#,
                verity_core::json::escape_json_string(&canon_proj.to_string_lossy())
            ),
        )
        .unwrap();

        // 3. Another project's session in pi_sessions_root must NOT be included
        let other_encoded = pi_sessions_root.join(encode_pi_session_dir_name(&canon_other));
        assert_eq!(other_encoded, encoded_dir);
        fs::write(
            other_encoded.join("other.jsonl"),
            format!(
                r#"{{"type":"session","id":"sess-other","name":"Other","timestamp":"2026-11-22T00:00:00Z","cwd":{}}}"#,
                verity_core::json::escape_json_string(&canon_other.to_string_lossy())
            ),
        )
        .unwrap();

        let scanned =
            scan_project_sessions_with_root("proj-1", &canon_proj, Some(&pi_sessions_root));
        assert_eq!(scanned.len(), 2);
        assert_eq!(scanned[0].id, "sess-2");
        assert_eq!(scanned[0].name, "Local Scratch");
        assert_eq!(scanned[1].id, "sess-1");
        assert_eq!(scanned[1].name, "Auth Refactor");
        assert_eq!(scanned[1].message_count, 1);

        // Merging live session deduplicates by ID/path and puts live session first.
        let live = PiSessionSummary {
            id: "sess-1".to_string(),
            project_id: "proj-1".to_string(),
            name: "Auth Refactor (Live)".to_string(),
            path: s1_path.to_string_lossy().to_string(),
            updated_at: "Just now".to_string(),
            message_count: 3,
            parent_session_id: None,
            archived: false,
        };
        let merged = merge_live_and_disk_sessions(scanned, Some(live));
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "sess-1");
        assert_eq!(merged[0].name, "Auth Refactor (Live)");
        assert_eq!(merged[1].id, "sess-2");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn persists_archived_session_ids_and_tags_scanned_sessions() {
        let root = unique_temp_dir("archive-meta");
        let meta_path = root.join("session-meta.json");
        let proj = root.join("repo");
        let local_sessions = proj.join(".pi").join("sessions");
        fs::create_dir_all(&local_sessions).unwrap();

        let canon_proj = fs::canonicalize(&proj).unwrap();
        let s1 = local_sessions.join("s1.jsonl");
        let s2 = local_sessions.join("s2.jsonl");
        fs::write(
            &s1,
            format!(
                r#"{{"type":"session","id":"sess-a","name":"Alpha","timestamp":"2026-11-20T10:00:00Z","cwd":{}}}"#,
                verity_core::json::escape_json_string(&canon_proj.to_string_lossy())
            ),
        )
        .unwrap();
        fs::write(
            &s2,
            format!(
                r#"{{"type":"session","id":"sess-b","name":"Beta","timestamp":"2026-11-21T10:00:00Z","cwd":{}}}"#,
                verity_core::json::escape_json_string(&canon_proj.to_string_lossy())
            ),
        )
        .unwrap();

        assert!(load_archived_session_ids_from(&meta_path).is_empty());

        let archived = set_session_archived_at(&meta_path, "sess-a", true).unwrap();
        assert!(archived.contains("sess-a"));
        assert!(meta_path.is_file());

        // Reload from disk and verify scanned sessions get tagged properly.
        let reloaded = load_archived_session_ids_from(&meta_path);
        assert!(reloaded.contains("sess-a"));
        assert!(!reloaded.contains("sess-b"));

        let scanned =
            scan_project_sessions_with_root_and_archived("proj-x", &canon_proj, None, &reloaded);
        assert_eq!(scanned.len(), 2);
        let a = scanned.iter().find(|s| s.id == "sess-a").unwrap();
        let b = scanned.iter().find(|s| s.id == "sess-b").unwrap();
        assert_eq!(a.project_id, "proj-x");
        assert!(a.archived);
        assert_eq!(b.project_id, "proj-x");
        assert!(!b.archived);

        // Unarchive persists cleanly.
        let unarchived = set_session_archived_at(&meta_path, "sess-a", false).unwrap();
        assert!(!unarchived.contains("sess-a"));
        assert!(load_archived_session_ids_from(&meta_path).is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn safely_deletes_only_jsonl_files_inside_pi_sessions_directories() {
        let root = unique_temp_dir("safe-delete");
        let pi_root = root.join("pi-sessions");
        let proj = root.join("my-app");
        let global_dir = pi_root.join("--my-app--");
        let local_dir = proj.join(".pi").join("sessions");
        fs::create_dir_all(&global_dir).unwrap();
        fs::create_dir_all(&local_dir).unwrap();

        let global_sess = global_dir.join("g1.jsonl");
        let local_sess = local_dir.join("l1.jsonl");
        let outside_jsonl = proj.join("important.jsonl");
        let non_jsonl = local_dir.join("notes.txt");
        fs::write(
            &global_sess,
            "{}
",
        )
        .unwrap();
        fs::write(
            &local_sess,
            "{}
",
        )
        .unwrap();
        fs::write(
            &outside_jsonl,
            "{}
",
        )
        .unwrap();
        fs::write(
            &non_jsonl, "keep
",
        )
        .unwrap();

        // Rejects non-jsonl files even inside .pi/sessions/
        assert!(
            delete_session_file_with_root(&non_jsonl.to_string_lossy(), Some(&pi_root)).is_err()
        );
        assert!(non_jsonl.exists());

        // Rejects .jsonl files outside Pi sessions directories
        assert!(
            delete_session_file_with_root(&outside_jsonl.to_string_lossy(), Some(&pi_root))
                .is_err()
        );
        assert!(outside_jsonl.exists());

        // Allows deleting .jsonl inside global Pi sessions root
        assert!(
            delete_session_file_with_root(&global_sess.to_string_lossy(), Some(&pi_root)).unwrap()
        );
        assert!(!global_sess.exists());

        // Allows deleting .jsonl inside project-local .pi/sessions/
        assert!(
            delete_session_file_with_root(&local_sess.to_string_lossy(), Some(&pi_root)).unwrap()
        );
        assert!(!local_sess.exists());

        // Deleting an already-deleted file is idempotent Ok(false)
        assert!(
            !delete_session_file_with_root(&local_sess.to_string_lossy(), Some(&pi_root)).unwrap()
        );

        let _ = fs::remove_dir_all(&root);
    }
}
