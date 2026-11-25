use crate::projects::{default_verity_home, normalize_project_path};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static SNAPSHOT_GIT_LOCK: Mutex<()> = Mutex::new(());
static CHECKPOINT_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const SHADOW_GIT_EXCLUDE: &str = ".git/\ntarget/\nnode_modules/\n.worktrees/\ndist/\n";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetedRevertOutcome {
    pub reverted_files: Vec<String>,
    pub redo_tree_oid: String,
    pub target_tree_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCheckpoint {
    pub session_id: String,
    pub entry_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_index: Option<usize>,
    pub prompt_preview: String,
    pub tree_oid: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRedoState {
    pub redo_tree_oid: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointStore {
    #[serde(default)]
    pub checkpoints: Vec<TurnCheckpoint>,
    #[serde(default)]
    pub last_redo: Option<LastRedoState>,
}

/// Pure-Rust FIPS 180-4 SHA-256 implementation (zero external crate dependencies).
fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, &word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256_bytes(data);
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}

/// Returns the 16-character SHA-256 hex prefix for a canonicalized project path.
pub fn project_snapshot_hash(project_dir: &Path) -> String {
    let canonical = normalize_project_path(project_dir);
    let full = sha256_hex(canonical.to_string_lossy().as_bytes());
    full[..16].to_string()
}

/// Root snapshot directory for a project: `~/.verity/snapshots/<sha256_prefix_16>`
/// (respecting `VERITY_HOME`).
pub fn snapshot_dir_for_project(project_dir: &Path) -> PathBuf {
    default_verity_home()
        .join("snapshots")
        .join(project_snapshot_hash(project_dir))
}

/// Isolated bare shadow gitdir for a project: `~/.verity/snapshots/<sha256_prefix_16>/git`.
pub fn shadow_git_dir_for_project(project_dir: &Path) -> PathBuf {
    snapshot_dir_for_project(project_dir).join("git")
}

/// Persistent checkpoint ledger path: `~/.verity/snapshots/<sha256_prefix_16>/checkpoints.json`.
pub fn checkpoints_path_for_project(project_dir: &Path) -> PathBuf {
    snapshot_dir_for_project(project_dir).join("checkpoints.json")
}

fn validate_tree_oid(oid: &str) -> Result<String, String> {
    let trimmed = oid.trim();
    if trimmed.len() != 40 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid git tree OID: '{}'", trimmed));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn validate_rel_path(rel_path: &str) -> Result<PathBuf, String> {
    if rel_path.is_empty() {
        return Err("empty relative path in snapshot diff".to_string());
    }
    let p = Path::new(rel_path);
    if p.is_absolute() {
        return Err(format!("refusing absolute path in snapshot: {}", rel_path));
    }
    for comp in p.components() {
        match comp {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "refusing unsafe path traversal in snapshot: {}",
                    rel_path
                ));
            }
            Component::Normal(seg) if seg == ".git" => {
                return Err(format!("refusing .git path in snapshot: {}", rel_path));
            }
            _ => {}
        }
    }
    Ok(p.to_path_buf())
}

fn clean_git_command() -> Command {
    let mut cmd = Command::new("git");
    cmd.stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn ensure_shadow_git_dir(project_dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    if !project_dir.exists() || !project_dir.is_dir() {
        return Err(format!(
            "project directory does not exist or is not a directory: {}",
            project_dir.display()
        ));
    }
    let canonical_project = normalize_project_path(project_dir);
    let shadow_git_dir = shadow_git_dir_for_project(&canonical_project);

    if !shadow_git_dir.join("HEAD").exists() {
        fs::create_dir_all(&shadow_git_dir).map_err(|e| {
            format!(
                "failed to create shadow git directory {}: {}",
                shadow_git_dir.display(),
                e
            )
        })?;
        let out = clean_git_command()
            .arg("init")
            .arg("--bare")
            .arg(&shadow_git_dir)
            .output()
            .map_err(|e| format!("failed to execute git init --bare: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "git init --bare failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }

    let info_dir = shadow_git_dir.join("info");
    fs::create_dir_all(&info_dir)
        .map_err(|e| format!("failed to create shadow git info directory: {}", e))?;
    fs::write(info_dir.join("exclude"), SHADOW_GIT_EXCLUDE)
        .map_err(|e| format!("failed to write shadow git exclude file: {}", e))?;

    Ok((canonical_project, shadow_git_dir))
}

fn capture_snapshot_locked(project_dir: &Path) -> Result<(PathBuf, PathBuf, String), String> {
    let (canonical_project, shadow_git_dir) = ensure_shadow_git_dir(project_dir)?;

    let add_out = clean_git_command()
        .arg("--git-dir")
        .arg(&shadow_git_dir)
        .arg("--work-tree")
        .arg(&canonical_project)
        .arg("add")
        .arg("-A")
        .output()
        .map_err(|e| format!("failed to execute shadow git add -A: {}", e))?;
    if !add_out.status.success() {
        return Err(format!(
            "shadow git add -A failed (exit {:?}): {}",
            add_out.status.code(),
            String::from_utf8_lossy(&add_out.stderr).trim()
        ));
    }

    let tree_out = clean_git_command()
        .arg("--git-dir")
        .arg(&shadow_git_dir)
        .arg("write-tree")
        .output()
        .map_err(|e| format!("failed to execute shadow git write-tree: {}", e))?;
    if !tree_out.status.success() {
        return Err(format!(
            "shadow git write-tree failed (exit {:?}): {}",
            tree_out.status.code(),
            String::from_utf8_lossy(&tree_out.stderr).trim()
        ));
    }

    let raw_oid = String::from_utf8_lossy(&tree_out.stdout);
    let tree_oid = validate_tree_oid(&raw_oid)?;
    Ok((canonical_project, shadow_git_dir, tree_oid))
}

/// Captures a complete working-tree snapshot into the project's isolated bare shadow gitdir
/// without modifying the user's `.git/`, staging area, commits, or stash.
pub fn capture_snapshot(project_dir: &Path) -> Result<String, String> {
    let _guard = SNAPSHOT_GIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (_, _, tree_oid) = capture_snapshot_locked(project_dir)?;
    Ok(tree_oid)
}

fn parse_diff_tree_z(raw: &[u8]) -> Result<Vec<(String, String)>, String> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts = raw.split(|&b| b == 0);
    let mut entries = Vec::new();
    while let Some(status_bytes) = parts.next() {
        if status_bytes.is_empty() {
            continue;
        }
        let path_bytes = parts
            .next()
            .ok_or_else(|| "malformed git diff-tree -z output: missing path".to_string())?;
        let status = String::from_utf8(status_bytes.to_vec())
            .map_err(|e| format!("non-utf8 status in diff-tree: {}", e))?;
        let path = String::from_utf8(path_bytes.to_vec())
            .map_err(|e| format!("non-utf8 path in diff-tree: {}", e))?;
        entries.push((status, path));
    }
    Ok(entries)
}

fn prune_empty_parent_dirs(project_root: &Path, file_path: &Path) {
    let mut cur = file_path.parent();
    while let Some(dir) = cur {
        if dir == project_root || !dir.starts_with(project_root) {
            break;
        }
        match fs::read_dir(dir) {
            Ok(mut entries) => {
                if entries.next().is_none() {
                    let _ = fs::remove_dir(dir);
                    cur = dir.parent();
                } else {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn remove_worktree_file(project_root: &Path, rel: &Path) -> Result<(), String> {
    let full = project_root.join(rel);
    if let Ok(meta) = fs::symlink_metadata(&full) {
        if meta.is_dir() && !meta.file_type().is_symlink() {
            fs::remove_dir_all(&full)
                .map_err(|e| format!("failed to remove directory {}: {}", full.display(), e))?;
        } else {
            fs::remove_file(&full)
                .map_err(|e| format!("failed to remove file {}: {}", full.display(), e))?;
        }
        prune_empty_parent_dirs(project_root, &full);
    }
    Ok(())
}

fn checkout_single_file(
    shadow_git_dir: &Path,
    project_root: &Path,
    tree_oid: &str,
    rel_path: &str,
) -> Result<(), String> {
    let out = clean_git_command()
        .arg("--git-dir")
        .arg(shadow_git_dir)
        .arg("--work-tree")
        .arg(project_root)
        .arg("checkout")
        .arg(tree_oid)
        .arg("--")
        .arg(rel_path)
        .output()
        .map_err(|e| {
            format!(
                "failed to execute shadow git checkout for {}: {}",
                rel_path, e
            )
        })?;
    if !out.status.success() {
        return Err(format!(
            "shadow git checkout failed for {}: {}",
            rel_path,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Reverts ONLY the files that differ between `target_tree_oid` and the current working tree,
/// preserving exact `mtime` on untouched files and capturing a `redo_tree_oid` beforehand.
pub fn revert_to_snapshot(
    project_dir: &Path,
    target_tree_oid: &str,
) -> Result<TargetedRevertOutcome, String> {
    let target_tree_oid = validate_tree_oid(target_tree_oid)?;
    let _guard = SNAPSHOT_GIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Step 1: Capture redo snapshot BEFORE mutating any file on disk.
    let (canonical_project, shadow_git_dir, redo_tree_oid) = capture_snapshot_locked(project_dir)?;

    // Step 2: Compute exact diff list between target snapshot and current (redo) snapshot.
    let diff_out = clean_git_command()
        .arg("--git-dir")
        .arg(&shadow_git_dir)
        .arg("diff-tree")
        .arg("--no-renames")
        .arg("-r")
        .arg("--name-status")
        .arg("-z")
        .arg(&target_tree_oid)
        .arg(&redo_tree_oid)
        .output()
        .map_err(|e| format!("failed to execute shadow git diff-tree: {}", e))?;
    if !diff_out.status.success() {
        return Err(format!(
            "shadow git diff-tree failed: {}",
            String::from_utf8_lossy(&diff_out.stderr).trim()
        ));
    }

    let entries = parse_diff_tree_z(&diff_out.stdout)?;

    // Step 3: If diff list is empty, touch ZERO files on disk.
    if entries.is_empty() {
        return Ok(TargetedRevertOutcome {
            reverted_files: Vec::new(),
            redo_tree_oid,
            target_tree_oid,
        });
    }

    // Step 4: Validate all paths before mutating anything on disk.
    let mut validated: Vec<(String, String, PathBuf)> = Vec::with_capacity(entries.len());
    for (status, rel_path) in entries {
        let rel_buf = validate_rel_path(&rel_path)?;
        validated.push((status, rel_path, rel_buf));
    }

    let mut reverted_files = Vec::with_capacity(validated.len());
    for (status, rel_path, rel_buf) in validated {
        if status.starts_with('A') {
            // File was added after target_tree_oid -> delete ONLY this file.
            remove_worktree_file(&canonical_project, &rel_buf)?;
        } else if status.starts_with('M') || status.starts_with('D') || status.starts_with('T') {
            // File existed in target_tree_oid -> restore ONLY this file.
            checkout_single_file(
                &shadow_git_dir,
                &canonical_project,
                &target_tree_oid,
                &rel_path,
            )?;
        } else {
            return Err(format!(
                "unexpected git diff-tree status '{}' for path '{}'",
                status, rel_path
            ));
        }
        reverted_files.push(rel_path);
    }

    Ok(TargetedRevertOutcome {
        reverted_files,
        redo_tree_oid,
        target_tree_oid,
    })
}

/// Restores ONLY the exact `files` list from `redo_tree_oid` (removing any file that did not
/// exist in `redo_tree_oid`). Never touches any other file on disk.
pub fn redo_targeted_revert(
    project_dir: &Path,
    redo_tree_oid: &str,
    files: &[String],
) -> Result<Vec<String>, String> {
    let redo_tree_oid = validate_tree_oid(redo_tree_oid)?;
    if files.is_empty() {
        return Ok(Vec::new());
    }

    let _guard = SNAPSHOT_GIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (canonical_project, shadow_git_dir) = ensure_shadow_git_dir(project_dir)?;

    let mut validated: Vec<(String, PathBuf)> = Vec::with_capacity(files.len());
    for f in files {
        let rel_buf = validate_rel_path(f)?;
        validated.push((f.clone(), rel_buf));
    }

    let mut restored_files = Vec::with_capacity(validated.len());
    for (rel_path, rel_buf) in validated {
        let ls_out = clean_git_command()
            .arg("--git-dir")
            .arg(&shadow_git_dir)
            .arg("ls-tree")
            .arg("-r")
            .arg("--name-only")
            .arg("-z")
            .arg(&redo_tree_oid)
            .arg("--")
            .arg(&rel_path)
            .output()
            .map_err(|e| {
                format!(
                    "failed to execute shadow git ls-tree for {}: {}",
                    rel_path, e
                )
            })?;
        if !ls_out.status.success() {
            return Err(format!(
                "shadow git ls-tree failed for {}: {}",
                rel_path,
                String::from_utf8_lossy(&ls_out.stderr).trim()
            ));
        }

        if !ls_out.stdout.is_empty() {
            checkout_single_file(
                &shadow_git_dir,
                &canonical_project,
                &redo_tree_oid,
                &rel_path,
            )?;
        } else {
            remove_worktree_file(&canonical_project, &rel_buf)?;
        }
        restored_files.push(rel_path);
    }

    Ok(restored_files)
}

pub fn normalize_prompt_preview(prompt: &str) -> String {
    let collapsed = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (idx, ch) in collapsed.chars().enumerate() {
        if idx >= 160 {
            break;
        }
        out.push(ch);
    }
    out
}

fn now_epoch_secs_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn load_checkpoint_store_unlocked(project_dir: &Path) -> CheckpointStore {
    let path = checkpoints_path_for_project(project_dir);
    let Ok(raw) = fs::read_to_string(&path) else {
        return CheckpointStore::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn load_checkpoint_store(project_dir: &Path) -> CheckpointStore {
    let _guard = SNAPSHOT_GIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    load_checkpoint_store_unlocked(project_dir)
}

pub fn save_checkpoint_store(project_dir: &Path, store: &CheckpointStore) -> Result<(), String> {
    let path = checkpoints_path_for_project(project_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create checkpoint directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| format!("failed to serialize checkpoints: {}", e))?;
    let seq = CHECKPOINT_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.tmp.{}.{}", std::process::id(), seq));
    fs::write(&tmp, json).map_err(|e| format!("failed to write temp checkpoints file: {}", e))?;
    fs::rename(&tmp, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("failed to commit checkpoints file: {}", e)
    })
}

#[allow(dead_code)]
pub fn record_turn_checkpoint(
    project_dir: &Path,
    session_id: String,
    entry_id: String,
    turn_index: Option<usize>,
    prompt_preview: String,
    tree_oid: String,
) -> Result<TurnCheckpoint, String> {
    let tree_oid = validate_tree_oid(&tree_oid)?;
    let _guard = SNAPSHOT_GIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_checkpoint_store_unlocked(project_dir);
    let cp = TurnCheckpoint {
        session_id,
        entry_id,
        turn_index,
        prompt_preview: normalize_prompt_preview(&prompt_preview),
        tree_oid,
        created_at: now_epoch_secs_string(),
    };
    store.checkpoints.push(cp.clone());
    save_checkpoint_store(project_dir, &store)?;
    Ok(cp)
}

/// Captures a working-tree snapshot and atomically records a 0-based turn checkpoint
/// under `SNAPSHOT_GIT_LOCK` so any subsequent revert serializes behind an in-flight capture.
pub fn capture_and_record_turn_checkpoint(
    project_dir: &Path,
    session_id: String,
    prompt_preview: String,
) -> Result<TurnCheckpoint, String> {
    let _guard = SNAPSHOT_GIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (_, _, tree_oid) = capture_snapshot_locked(project_dir)?;
    let mut store = load_checkpoint_store_unlocked(project_dir);
    let turn_index = store
        .checkpoints
        .iter()
        .filter(|c| c.session_id == session_id)
        .count();
    let entry_id = format!("turn-{}", store.checkpoints.len() + 1);
    let cp = TurnCheckpoint {
        session_id,
        entry_id,
        turn_index: Some(turn_index),
        prompt_preview: normalize_prompt_preview(&prompt_preview),
        tree_oid,
        created_at: now_epoch_secs_string(),
    };
    store.checkpoints.push(cp.clone());
    save_checkpoint_store(project_dir, &store)?;
    Ok(cp)
}

pub fn save_last_redo(
    project_dir: &Path,
    redo_tree_oid: String,
    files: Vec<String>,
) -> Result<(), String> {
    let redo_tree_oid = validate_tree_oid(&redo_tree_oid)?;
    let _guard = SNAPSHOT_GIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_checkpoint_store_unlocked(project_dir);
    store.last_redo = Some(LastRedoState {
        redo_tree_oid,
        files,
    });
    save_checkpoint_store(project_dir, &store)
}

pub fn load_last_redo(project_dir: &Path) -> Option<LastRedoState> {
    load_checkpoint_store(project_dir).last_redo
}

pub fn clear_last_redo(project_dir: &Path) -> Result<(), String> {
    let _guard = SNAPSHOT_GIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_checkpoint_store_unlocked(project_dir);
    store.last_redo = None;
    save_checkpoint_store(project_dir, &store)
}

/// Finds a turn checkpoint in priority order:
/// 1. `(session_id, entry_id)` (then any session with `entry_id`)
/// 2. `(session_id, turn_index)` (falling back to `"default-session"`, or any session when `session_id` is absent)
/// 3. `(session_id, normalized_prompt_preview)` (then any session with `normalized_prompt_preview`)
pub fn find_checkpoint(
    store: &CheckpointStore,
    session_id: Option<&str>,
    entry_id: &str,
    turn_index: Option<usize>,
    prompt_text: Option<&str>,
) -> Option<TurnCheckpoint> {
    let trimmed_entry = entry_id.trim();
    let active_sid = session_id.filter(|s| !s.trim().is_empty());

    if let Some(sid) = active_sid {
        if let Some(cp) = store
            .checkpoints
            .iter()
            .rev()
            .find(|c| c.session_id == sid && c.entry_id == trimmed_entry)
        {
            return Some(cp.clone());
        }
    }

    if let Some(cp) = store
        .checkpoints
        .iter()
        .rev()
        .find(|c| c.entry_id == trimmed_entry)
    {
        return Some(cp.clone());
    }

    if let Some(idx) = turn_index {
        if let Some(sid) = active_sid {
            if let Some(cp) = store
                .checkpoints
                .iter()
                .rev()
                .find(|c| c.session_id == sid && c.turn_index == Some(idx))
            {
                return Some(cp.clone());
            }
            if let Some(cp) = store
                .checkpoints
                .iter()
                .rev()
                .find(|c| c.session_id == "default-session" && c.turn_index == Some(idx))
            {
                return Some(cp.clone());
            }
        } else if let Some(cp) = store
            .checkpoints
            .iter()
            .rev()
            .find(|c| c.turn_index == Some(idx))
        {
            return Some(cp.clone());
        }
    }

    if let Some(prompt) = prompt_text {
        let norm = normalize_prompt_preview(prompt);
        if !norm.is_empty() {
            if let Some(sid) = active_sid {
                if let Some(cp) = store
                    .checkpoints
                    .iter()
                    .rev()
                    .find(|c| c.session_id == sid && c.prompt_preview == norm)
                {
                    return Some(cp.clone());
                }
            }
            if let Some(cp) = store
                .checkpoints
                .iter()
                .rev()
                .find(|c| c.prompt_preview == norm)
            {
                return Some(cp.clone());
            }
        }
    }

    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::env;
    use std::thread;
    use std::time::Duration;

    pub(crate) static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());
    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let n = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "verity-snap-{}-{}-{}-{}",
                prefix,
                std::process::id(),
                ts,
                n
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    struct VerityHomeGuard {
        prev: Option<String>,
    }

    impl VerityHomeGuard {
        fn set(path: &Path) -> Self {
            let prev = env::var("VERITY_HOME").ok();
            env::set_var("VERITY_HOME", path);
            Self { prev }
        }
    }

    impl Drop for VerityHomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => env::set_var("VERITY_HOME", v),
                None => env::remove_var("VERITY_HOME"),
            }
        }
    }

    #[test]
    fn sha256_matches_fips_180_4_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn targeted_revert_restores_modified_deletes_new_and_preserves_untouched_mtime() {
        let _env_lock = TEST_ENV_LOCK.lock().unwrap();
        let home = TempDirGuard::new("home-revert");
        let _vh = VerityHomeGuard::set(&home.path);
        let project = TempDirGuard::new("proj-revert");

        let untouched = project.path.join("untouched.txt");
        let modified = project.path.join("src").join("app.rs");
        let deleted = project.path.join("keep_me.txt");
        fs::create_dir_all(modified.parent().unwrap()).unwrap();
        fs::write(&untouched, "never changed\n").unwrap();
        fs::write(&modified, "fn main() { println!(\"v1\"); }\n").unwrap();
        fs::write(&deleted, "important data\n").unwrap();

        let untouched_mtime_before = fs::metadata(&untouched).unwrap().modified().unwrap();

        let snap1 = capture_snapshot(&project.path).expect("capture initial snapshot");
        assert_eq!(snap1.len(), 40);

        // Ensure filesystem clock advances so any accidental rewrite would change mtime.
        thread::sleep(Duration::from_millis(50));

        // Mutate working tree: modify one file, delete one file, create a new file in a nested dir.
        fs::write(&modified, "fn main() { println!(\"v2-broken\"); }\n").unwrap();
        fs::remove_file(&deleted).unwrap();
        let added = project.path.join("generated").join("extra.txt");
        fs::create_dir_all(added.parent().unwrap()).unwrap();
        fs::write(&added, "unwanted file\n").unwrap();

        let outcome = revert_to_snapshot(&project.path, &snap1).expect("targeted revert");
        assert_eq!(outcome.target_tree_oid, snap1);
        assert_ne!(outcome.redo_tree_oid, snap1);
        assert!(outcome.reverted_files.contains(&"src/app.rs".to_string()));
        assert!(outcome.reverted_files.contains(&"keep_me.txt".to_string()));
        assert!(outcome
            .reverted_files
            .contains(&"generated/extra.txt".to_string()));
        assert!(!outcome
            .reverted_files
            .contains(&"untouched.txt".to_string()));

        // Verify disk state after revert.
        assert_eq!(
            fs::read_to_string(&modified).unwrap(),
            "fn main() { println!(\"v1\"); }\n"
        );
        assert_eq!(fs::read_to_string(&deleted).unwrap(), "important data\n");
        assert!(!added.exists(), "newly added file must be deleted");
        assert!(
            !added.parent().unwrap().exists(),
            "empty parent directory must be pruned"
        );

        // Verify untouched file mtime is strictly preserved (OpenCode #15391 regression guard).
        let untouched_mtime_after_revert = fs::metadata(&untouched).unwrap().modified().unwrap();
        assert_eq!(untouched_mtime_before, untouched_mtime_after_revert);

        // Symmetric targeted redo restores pre-revert state cleanly without touching untouched.txt.
        thread::sleep(Duration::from_millis(50));
        let restored = redo_targeted_revert(
            &project.path,
            &outcome.redo_tree_oid,
            &outcome.reverted_files,
        )
        .expect("targeted redo");
        assert_eq!(restored.len(), outcome.reverted_files.len());
        assert_eq!(
            fs::read_to_string(&modified).unwrap(),
            "fn main() { println!(\"v2-broken\"); }\n"
        );
        assert!(
            !deleted.exists(),
            "deleted file must be removed again on redo"
        );
        assert_eq!(fs::read_to_string(&added).unwrap(), "unwanted file\n");

        let untouched_mtime_after_redo = fs::metadata(&untouched).unwrap().modified().unwrap();
        assert_eq!(untouched_mtime_before, untouched_mtime_after_redo);
    }

    #[test]
    fn non_git_directory_works_via_shadow_gitdir_without_creating_project_dot_git() {
        let _env_lock = TEST_ENV_LOCK.lock().unwrap();
        let home = TempDirGuard::new("home-nongit");
        let _vh = VerityHomeGuard::set(&home.path);
        let project = TempDirGuard::new("proj-nongit");

        let note = project.path.join("notes.md");
        fs::write(&note, "version 1\n").unwrap();
        assert!(!project.path.join(".git").exists());

        let oid = capture_snapshot(&project.path).expect("snapshot in non-git dir");
        assert_eq!(oid.len(), 40);
        assert!(
            !project.path.join(".git").exists(),
            "shadow git must never create .git inside project directory"
        );
        assert!(shadow_git_dir_for_project(&project.path)
            .join("HEAD")
            .exists());

        fs::write(&note, "version 2\n").unwrap();
        let outcome = revert_to_snapshot(&project.path, &oid).expect("revert in non-git dir");
        assert_eq!(outcome.reverted_files, vec!["notes.md".to_string()]);
        assert_eq!(fs::read_to_string(&note).unwrap(), "version 1\n");
        assert!(!project.path.join(".git").exists());
    }

    #[test]
    fn empty_diff_revert_touches_zero_files() {
        let _env_lock = TEST_ENV_LOCK.lock().unwrap();
        let home = TempDirGuard::new("home-noop");
        let _vh = VerityHomeGuard::set(&home.path);
        let project = TempDirGuard::new("proj-noop");

        let file = project.path.join("a.txt");
        fs::write(&file, "same\n").unwrap();
        let mtime_before = fs::metadata(&file).unwrap().modified().unwrap();

        let oid = capture_snapshot(&project.path).unwrap();
        thread::sleep(Duration::from_millis(30));
        let outcome = revert_to_snapshot(&project.path, &oid).unwrap();
        assert!(outcome.reverted_files.is_empty());
        assert_eq!(
            mtime_before,
            fs::metadata(&file).unwrap().modified().unwrap()
        );
    }

    #[test]
    fn checkpoint_persistence_and_lookup_work_by_entry_id_and_prompt() {
        let _env_lock = TEST_ENV_LOCK.lock().unwrap();
        let home = TempDirGuard::new("home-cp");
        let _vh = VerityHomeGuard::set(&home.path);
        let project = TempDirGuard::new("proj-cp");

        let fake_oid = "0123456789abcdef0123456789abcdef01234567".to_string();
        record_turn_checkpoint(
            &project.path,
            "sess-1".to_string(),
            "turn-1".to_string(),
            Some(0),
            "  Fix   the login bug  ".to_string(),
            fake_oid.clone(),
        )
        .unwrap();

        let store = load_checkpoint_store(&project.path);
        assert_eq!(store.checkpoints.len(), 1);
        assert_eq!(store.checkpoints[0].prompt_preview, "Fix the login bug");
        assert_eq!(store.checkpoints[0].turn_index, Some(0));

        let by_id = find_checkpoint(&store, Some("sess-1"), "turn-1", None, None).unwrap();
        assert_eq!(by_id.tree_oid, fake_oid);

        let by_turn_idx =
            find_checkpoint(&store, Some("sess-1"), "pi-entry-99", Some(0), None).unwrap();
        assert_eq!(by_turn_idx.tree_oid, fake_oid);

        let by_prompt = find_checkpoint(
            &store,
            Some("sess-1"),
            "pi-entry-99",
            Some(99),
            Some("Fix the login bug"),
        )
        .unwrap();
        assert_eq!(by_prompt.tree_oid, fake_oid);

        assert!(
            find_checkpoint(&store, Some("sess-1"), "unknown", Some(5), Some("other")).is_none()
        );
    }

    #[test]
    fn clean_git_command_suppresses_windows_console_and_terminal_prompts() {
        let cmd = clean_git_command();
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(
            envs.iter()
                .any(|(k, v)| *k == "GIT_TERMINAL_PROMPT" && *v == Some(std::ffi::OsStr::new("0"))),
            "clean_git_command must set GIT_TERMINAL_PROMPT=0"
        );
        let source = include_str!("snapshots.rs");
        assert!(
            source.contains("CREATE_NO_WINDOW"),
            "snapshots.rs must configure CREATE_NO_WINDOW on Windows"
        );
        assert!(
            source.contains("Stdio::null()"),
            "snapshots.rs must attach Stdio::null() to git stdin"
        );
    }
}
