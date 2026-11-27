use crate::projects::{default_verity_home, normalize_project_path};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use verity_core::types::sha256_bytes;

static SNAPSHOT_GIT_LOCK: Mutex<()> = Mutex::new(());
static CHECKPOINT_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const SHADOW_GIT_EXCLUDE: &str = ".git/\ntarget/\nnode_modules/\n.worktrees/\ndist/\n";
const UNRESOLVED_SESSION_ID: &str = "default-session";

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_tree_oid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointStore {
    #[serde(default)]
    pub checkpoints: Vec<TurnCheckpoint>,
    #[serde(default)]
    pub last_redo: Option<LastRedoState>,
}

fn sha256_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    sha256_bytes(data)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(&mut s, "{b:02x}");
            s
        })
}

pub fn project_snapshot_hash(project_dir: &Path) -> String {
    let canonical = normalize_project_path(project_dir);
    sha256_hex(canonical.to_string_lossy().as_bytes())[..16].to_string()
}

pub fn snapshot_dir_for_project(project_dir: &Path) -> PathBuf {
    default_verity_home()
        .join("snapshots")
        .join(project_snapshot_hash(project_dir))
}

pub fn shadow_git_dir_for_project(project_dir: &Path) -> PathBuf {
    snapshot_dir_for_project(project_dir).join("git")
}

pub fn checkpoints_path_for_project(project_dir: &Path) -> PathBuf {
    snapshot_dir_for_project(project_dir).join("checkpoints.json")
}

fn validate_tree_oid(oid: &str) -> Result<String, String> {
    let t = oid.trim();
    if t.len() != 40 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid git tree OID: '{t}'"));
    }
    Ok(t.to_ascii_lowercase())
}

fn is_dot_git(seg: &std::ffi::OsStr) -> bool {
    if cfg!(windows) {
        seg.to_string_lossy().eq_ignore_ascii_case(".git")
    } else {
        seg == ".git"
    }
}

fn validate_rel_path(rel_path: &str) -> Result<PathBuf, String> {
    if rel_path.is_empty() || rel_path.contains('\0') {
        return Err("invalid empty or NUL relative path in snapshot".into());
    }
    let p = Path::new(rel_path);
    if p.is_absolute() {
        return Err(format!("refusing absolute path in snapshot: {rel_path}"));
    }
    for comp in p.components() {
        match comp {
            Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_)
            | Component::CurDir => {
                return Err(format!(
                    "refusing unsafe path traversal in snapshot: {rel_path}"
                ));
            }
            Component::Normal(seg) if is_dot_git(seg) => {
                return Err(format!("refusing .git path in snapshot: {rel_path}"));
            }
            _ => {}
        }
    }
    Ok(p.to_path_buf())
}

fn fs_key(rel_path: &str) -> String {
    if cfg!(windows) {
        rel_path.to_ascii_lowercase()
    } else {
        rel_path.to_string()
    }
}

fn is_real_dir(meta: &fs::Metadata) -> bool {
    meta.is_dir() && !meta.file_type().is_symlink()
}

fn meta_opt(path: &Path, ctx: &str) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(m) => Ok(Some(m)),
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => Ok(None),
        Err(e) => Err(format!("{ctx}: {e}")),
    }
}

fn collect_ancestors(rel_path: &str) -> Vec<(String, PathBuf)> {
    let parts: Vec<&str> = rel_path.split('/').collect();
    if parts.len() <= 1 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(parts.len() - 1);
    let (mut rel, mut buf) = (String::new(), PathBuf::new());
    for part in &parts[..parts.len() - 1] {
        if !rel.is_empty() {
            rel.push('/');
        }
        rel.push_str(part);
        buf.push(part);
        out.push((rel.clone(), buf.clone()));
    }
    out
}

fn clean_git_command() -> Command {
    let mut cmd = Command::new("git");
    cmd.stdin(Stdio::null())
        .args(["-c", "core.autocrlf=false"])
        .env("GIT_TERMINAL_PROMPT", "0");
    for v in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        cmd.env_remove(v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn git_output(
    shadow_git_dir: Option<&Path>,
    work_tree: Option<&Path>,
    args: &[&str],
) -> Result<Output, String> {
    let mut cmd = clean_git_command();
    if let Some(d) = shadow_git_dir {
        cmd.arg("--git-dir").arg(d);
    }
    if let Some(w) = work_tree {
        cmd.arg("--work-tree").arg(w);
    }
    cmd.args(args)
        .output()
        .map_err(|e| format!("failed to execute git {}: {e}", args.join(" ")))
}

fn run_git(
    shadow_git_dir: Option<&Path>,
    work_tree: Option<&Path>,
    args: &[&str],
) -> Result<Vec<u8>, String> {
    let out = git_output(shadow_git_dir, work_tree, args)?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed (exit {:?}): {}",
            args.join(" "),
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn ensure_shadow_git_dir(project_dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    if !project_dir.is_dir() {
        return Err(format!(
            "project directory does not exist or is not a directory: {}",
            project_dir.display()
        ));
    }
    let canonical = normalize_project_path(project_dir);
    let shadow = shadow_git_dir_for_project(&canonical);
    if !shadow.join("HEAD").exists() {
        fs::create_dir_all(&shadow).map_err(|e| {
            format!(
                "failed to create shadow git directory {}: {e}",
                shadow.display()
            )
        })?;
        let out = clean_git_command()
            .args(["init", "--bare"])
            .arg(&shadow)
            .output()
            .map_err(|e| format!("failed to execute git init --bare: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git init --bare failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    let info = shadow.join("info");
    fs::create_dir_all(&info)
        .map_err(|e| format!("failed to create shadow git info directory: {e}"))?;
    fs::write(info.join("exclude"), SHADOW_GIT_EXCLUDE)
        .map_err(|e| format!("failed to write shadow git exclude file: {e}"))?;
    Ok((canonical, shadow))
}

fn capture_snapshot_in(project: &Path, shadow: &Path) -> Result<String, String> {
    run_git(Some(shadow), Some(project), &["add", "-A"])?;
    validate_tree_oid(&String::from_utf8_lossy(&run_git(
        Some(shadow),
        None,
        &["write-tree"],
    )?))
}

pub fn capture_snapshot(project_dir: &Path) -> Result<String, String> {
    let _guard = SNAPSHOT_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (project, shadow) = ensure_shadow_git_dir(project_dir)?;
    capture_snapshot_in(&project, &shadow)
}

fn verify_tree_exists(shadow: &Path, tree_oid: &str) -> Result<(), String> {
    let out = run_git(Some(shadow), None, &["cat-file", "-t", tree_oid])?;
    let kind = String::from_utf8_lossy(&out);
    if kind.trim() != "tree" {
        return Err(format!(
            "snapshot object {tree_oid} is '{}' rather than a git tree",
            kind.trim()
        ));
    }
    Ok(())
}

fn list_tree_entries(shadow: &Path, tree_oid: &str) -> Result<HashMap<String, String>, String> {
    let raw = run_git(Some(shadow), None, &["ls-tree", "-r", "-z", tree_oid])?;
    let mut entries = HashMap::new();
    for rec in raw.split(|&b| b == 0).filter(|r| !r.is_empty()) {
        let tab = rec
            .iter()
            .position(|&b| b == b'\t')
            .ok_or("malformed git ls-tree -z output")?;
        let header = std::str::from_utf8(&rec[..tab])
            .map_err(|e| format!("non-utf8 ls-tree header: {e}"))?;
        let path = std::str::from_utf8(&rec[tab + 1..])
            .map_err(|e| format!("non-utf8 ls-tree path: {e}"))?;
        validate_rel_path(path)?;
        let kind = header
            .split_whitespace()
            .nth(1)
            .ok_or("malformed git ls-tree -z header")?;
        entries.insert(path.to_string(), kind.to_string());
    }
    Ok(entries)
}

fn diff_trees(
    shadow: &Path,
    old_oid: &str,
    new_oid: &str,
) -> Result<Vec<(String, String)>, String> {
    let raw = run_git(
        Some(shadow),
        None,
        &[
            "diff-tree",
            "--no-renames",
            "-r",
            "--name-status",
            "-z",
            old_oid,
            new_oid,
        ],
    )?;
    let mut parts = raw.split(|&b| b == 0);
    let mut out = Vec::new();
    while let Some(st) = parts.next() {
        if st.is_empty() {
            continue;
        }
        let p = parts
            .next()
            .ok_or("malformed git diff-tree -z output: missing path")?;
        let status = String::from_utf8(st.to_vec())
            .map_err(|e| format!("non-utf8 diff-tree status: {e}"))?;
        let path =
            String::from_utf8(p.to_vec()).map_err(|e| format!("non-utf8 diff-tree path: {e}"))?;
        out.push((status, path));
    }
    Ok(out)
}

fn is_git_ignored(shadow: &Path, project: &Path, rel_path: &str) -> Result<bool, String> {
    let out = git_output(
        Some(shadow),
        Some(project),
        &["check-ignore", "--no-index", "-q", "--", rel_path],
    )?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        code => Err(format!(
            "shadow git check-ignore failed for {rel_path} (exit {code:?}): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

fn check_ancestors_no_symlink(project: &Path, rel_path: &str) -> Result<(), String> {
    for (anc_rel, anc_buf) in collect_ancestors(rel_path) {
        if meta_opt(
            &project.join(&anc_buf),
            &format!("preflight failed inspecting ancestor '{anc_rel}'"),
        )?
        .is_some_and(|m| m.file_type().is_symlink())
        {
            return Err(format!(
                "preflight collision: ancestor '{anc_rel}' of '{rel_path}' is a symlink"
            ));
        }
    }
    Ok(())
}

fn check_dir_all_descendants_removable(
    dir_abs: &Path,
    dir_rel: &str,
    removals_lookup: &HashSet<String>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir_abs)
        .map_err(|e| format!("preflight failed reading directory '{dir_rel}': {e}"))?;
    for entry_res in entries {
        let entry =
            entry_res.map_err(|e| format!("preflight failed reading entry in '{dir_rel}': {e}"))?;
        let os_name = entry.file_name();
        let name = os_name
            .to_str()
            .ok_or_else(|| format!("preflight collision: non-utf8 entry inside '{dir_rel}'"))?;
        let child_rel = if dir_rel.is_empty() {
            name.to_string()
        } else {
            format!("{dir_rel}/{name}")
        };
        if is_dot_git(&os_name) {
            return Err(format!(
                "preflight collision: refusing to disturb .git entry at '{child_rel}'"
            ));
        }
        let child = entry.path();
        let meta = fs::symlink_metadata(&child)
            .map_err(|e| format!("preflight failed inspecting '{child_rel}': {e}"))?;
        if is_real_dir(&meta) {
            check_dir_all_descendants_removable(&child, &child_rel, removals_lookup)?;
        } else if !removals_lookup.contains(&fs_key(&child_rel)) {
            return Err(format!("preflight collision: replacing directory '{dir_rel}' would lose ignored or unrelated file '{child_rel}'"));
        }
    }
    Ok(())
}

fn prune_empty_parent_dirs(project_root: &Path, file_path: &Path) {
    let mut cur = file_path.parent();
    while let Some(dir) = cur {
        if dir == project_root || !dir.starts_with(project_root) {
            break;
        }
        if fs::read_dir(dir).is_ok_and(|mut e| e.next().is_none()) {
            let _ = fs::remove_dir(dir);
            cur = dir.parent();
        } else {
            break;
        }
    }
}

fn remove_empty_dir_tree(dir: &Path) -> Result<(), String> {
    for entry_res in
        fs::read_dir(dir).map_err(|e| format!("failed to read directory {}: {e}", dir.display()))?
    {
        let child = entry_res
            .map_err(|e| format!("failed to iterate directory {}: {e}", dir.display()))?
            .path();
        let meta = fs::symlink_metadata(&child)
            .map_err(|e| format!("failed to inspect {}: {e}", child.display()))?;
        if is_real_dir(&meta) {
            remove_empty_dir_tree(&child)?;
        } else {
            return Err(format!(
                "refusing to delete non-empty directory {} containing {}",
                dir.display(),
                child.display()
            ));
        }
    }
    fs::remove_dir(dir).map_err(|e| format!("failed to remove directory {}: {e}", dir.display()))
}

fn remove_worktree_file(project_root: &Path, rel: &Path) -> Result<(), String> {
    let full = project_root.join(rel);
    if let Ok(meta) = fs::symlink_metadata(&full) {
        if is_real_dir(&meta) {
            remove_empty_dir_tree(&full)?;
        } else {
            fs::remove_file(&full)
                .map_err(|e| format!("failed to remove file {}: {e}", full.display()))?;
        }
        prune_empty_parent_dirs(project_root, &full);
    }
    Ok(())
}
struct RestorePlan {
    tree_oid: String,
    ordered_files: Vec<String>,
    removals: Vec<(String, PathBuf)>,
    restores: Vec<(String, PathBuf, bool)>,
    removals_lookup: HashSet<String>,
    restores_ancestors: HashSet<String>,
    check_ignored: bool,
}

fn apply_restore_plan(
    shadow: &Path,
    project: &Path,
    mut plan: RestorePlan,
) -> Result<Vec<String>, String> {
    plan.removals.sort_by_key(|a| std::cmp::Reverse(a.0.len()));

    for (rel, buf) in &plan.removals {
        check_ancestors_no_symlink(project, rel)?;
        let full = project.join(buf);
        if let Some(meta) = meta_opt(
            &full,
            &format!("preflight failed inspecting removal target '{rel}'"),
        )? {
            if is_real_dir(&meta) {
                check_dir_all_descendants_removable(&full, rel, &plan.removals_lookup)?;
            } else if plan.check_ignored && is_git_ignored(shadow, project, rel)? {
                return Err(format!(
                    "preflight collision: refusing to delete ignored file '{rel}'"
                ));
            }
        }
    }

    for (rel, buf, refuse_if_present) in &plan.restores {
        for (anc_rel, anc_buf) in collect_ancestors(rel) {
            if let Some(meta) = meta_opt(
                &project.join(&anc_buf),
                &format!("preflight failed inspecting ancestor '{anc_rel}' of '{rel}'"),
            )? {
                if !is_real_dir(&meta) && !plan.removals_lookup.contains(&fs_key(&anc_rel)) {
                    return Err(format!("preflight collision: cannot restore '{rel}' because ancestor '{anc_rel}' is an ignored or unrelated file/symlink"));
                }
            }
        }
        let full = project.join(buf);
        if let Some(meta) = meta_opt(
            &full,
            &format!("preflight failed inspecting restore target '{rel}'"),
        )? {
            if is_real_dir(&meta) {
                check_dir_all_descendants_removable(&full, rel, &plan.removals_lookup)?;
            } else if *refuse_if_present
                || (plan.check_ignored && is_git_ignored(shadow, project, rel)?)
            {
                return Err(format!(
                    "preflight collision: refusing to overwrite ignored or unindexed file '{rel}'"
                ));
            }
        }
    }

    for (rel, buf) in &plan.removals {
        if plan.restores_ancestors.contains(&fs_key(rel))
            && fs::symlink_metadata(project.join(buf)).is_ok_and(|m| is_real_dir(&m))
        {
            continue;
        }
        remove_worktree_file(project, buf)?;
    }
    for (rel, buf, _) in &plan.restores {
        let full = project.join(buf);
        if fs::symlink_metadata(&full).is_ok_and(|m| is_real_dir(&m)) {
            remove_empty_dir_tree(&full)?;
        }
        run_git(
            Some(shadow),
            Some(project),
            &["--literal-pathspecs", "checkout", &plan.tree_oid, "--", rel],
        )?;
    }
    Ok(plan.ordered_files)
}

/// Reverts ONLY the files differing between `target_tree_oid` and the working tree, preserving
/// exact `mtime` on untouched files and running preflight collision checks before disk mutation.
pub fn revert_to_snapshot(
    project_dir: &Path,
    target_tree_oid: &str,
) -> Result<TargetedRevertOutcome, String> {
    let target_tree_oid = validate_tree_oid(target_tree_oid)?;
    let _guard = SNAPSHOT_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (project, shadow) = ensure_shadow_git_dir(project_dir)?;
    verify_tree_exists(&shadow, &target_tree_oid)?;

    let redo_tree_oid = capture_snapshot_in(&project, &shadow)?;
    let entries = diff_trees(&shadow, &target_tree_oid, &redo_tree_oid)?;
    if entries.is_empty() {
        return Ok(TargetedRevertOutcome {
            reverted_files: Vec::new(),
            redo_tree_oid,
            target_tree_oid,
        });
    }

    let (mut ordered_files, mut removals, mut restores) =
        (Vec::with_capacity(entries.len()), Vec::new(), Vec::new());
    let (mut removals_lookup, mut restores_ancestors) = (HashSet::new(), HashSet::new());
    for (status, rel) in entries {
        let buf = validate_rel_path(&rel)?;
        if status.starts_with('A') {
            removals_lookup.insert(fs_key(&rel));
            removals.push((rel.clone(), buf));
        } else if status.starts_with('M') || status.starts_with('D') || status.starts_with('T') {
            for (anc, _) in collect_ancestors(&rel) {
                restores_ancestors.insert(fs_key(&anc));
            }
            restores.push((rel.clone(), buf, status.starts_with('D')));
        } else {
            return Err(format!(
                "unexpected git diff-tree status '{status}' for path '{rel}'"
            ));
        }
        ordered_files.push(rel);
    }

    if !restores.is_empty() {
        let target_entries = list_tree_entries(&shadow, &target_tree_oid)?;
        for (rel, _, _) in &restores {
            match target_entries.get(rel).map(String::as_str) {
                Some("blob") => {}
                Some(kind) => return Err(format!("preflight error: unsupported git tree entry type '{kind}' for '{rel}'")),
                None => return Err(format!("preflight error: target snapshot {target_tree_oid} missing expected leaf '{rel}'")),
            }
        }
    }

    let reverted_files = apply_restore_plan(
        &shadow,
        &project,
        RestorePlan {
            tree_oid: target_tree_oid.clone(),
            ordered_files,
            removals,
            restores,
            removals_lookup,
            restores_ancestors,
            check_ignored: true,
        },
    )?;
    Ok(TargetedRevertOutcome {
        reverted_files,
        redo_tree_oid,
        target_tree_oid,
    })
}

fn check_stale_redo(
    project: &Path,
    shadow: &Path,
    expected_tree_oid: &str,
    affected: &HashSet<String>,
    affected_ancestors: &HashSet<String>,
) -> Result<(), String> {
    let expected = validate_tree_oid(expected_tree_oid)?;
    verify_tree_exists(shadow, &expected)?;
    let current = capture_snapshot_in(project, shadow)?;
    for (_, changed) in diff_trees(shadow, &expected, &current)? {
        let key = fs_key(&changed);
        if affected.contains(&key)
            || affected_ancestors.contains(&key)
            || collect_ancestors(&changed)
                .iter()
                .any(|(anc, _)| affected.contains(&fs_key(anc)))
        {
            return Err(format!(
                "stale redo: affected path '{changed}' changed after revert"
            ));
        }
    }
    Ok(())
}

fn redo_targeted_revert_locked(
    project: &Path,
    shadow: &Path,
    redo_tree_oid: &str,
    files: &[String],
    expected_tree_oid: Option<&str>,
) -> Result<Vec<String>, String> {
    let redo_tree_oid = validate_tree_oid(redo_tree_oid)?;
    if files.is_empty() {
        return Ok(Vec::new());
    }
    verify_tree_exists(shadow, &redo_tree_oid)?;

    let (mut seen, mut validated, mut affected_ancestors) = (
        HashSet::new(),
        Vec::with_capacity(files.len()),
        HashSet::new(),
    );
    for f in files {
        let buf = validate_rel_path(f)?;
        if seen.insert(fs_key(f)) {
            for (anc, _) in collect_ancestors(f) {
                affected_ancestors.insert(fs_key(&anc));
            }
            validated.push((f.clone(), buf));
        }
    }
    if let Some(expected) = expected_tree_oid {
        check_stale_redo(project, shadow, expected, &seen, &affected_ancestors)?;
    }

    let redo_entries = list_tree_entries(shadow, &redo_tree_oid)?;
    let (mut ordered_files, mut removals, mut restores) =
        (Vec::with_capacity(validated.len()), Vec::new(), Vec::new());
    let (mut removals_lookup, mut restores_ancestors) = (HashSet::new(), HashSet::new());
    for (rel, buf) in validated {
        match redo_entries.get(&rel).map(String::as_str) {
            Some("blob") => {
                for (anc, _) in collect_ancestors(&rel) {
                    restores_ancestors.insert(fs_key(&anc));
                }
                restores.push((rel.clone(), buf, false));
            }
            Some(kind) => {
                return Err(format!(
                    "preflight error: unsupported git tree entry type '{kind}' for '{rel}'"
                ))
            }
            None => {
                removals_lookup.insert(fs_key(&rel));
                removals.push((rel.clone(), buf));
            }
        }
        ordered_files.push(rel);
    }

    apply_restore_plan(
        shadow,
        project,
        RestorePlan {
            tree_oid: redo_tree_oid,
            ordered_files,
            removals,
            restores,
            removals_lookup,
            restores_ancestors,
            check_ignored: true,
        },
    )
}

pub fn redo_targeted_revert_checked(
    project_dir: &Path,
    redo_tree_oid: &str,
    files: &[String],
    expected_tree_oid: Option<&str>,
) -> Result<Vec<String>, String> {
    if files.is_empty() {
        validate_tree_oid(redo_tree_oid)?;
        return Ok(Vec::new());
    }
    let _guard = SNAPSHOT_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (project, shadow) = ensure_shadow_git_dir(project_dir)?;
    redo_targeted_revert_locked(&project, &shadow, redo_tree_oid, files, expected_tree_oid)
}

pub fn redo_targeted_revert(
    project_dir: &Path,
    redo_tree_oid: &str,
    files: &[String],
) -> Result<Vec<String>, String> {
    redo_targeted_revert_checked(project_dir, redo_tree_oid, files, None)
}

pub fn normalize_prompt_preview(prompt: &str) -> String {
    prompt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect()
}

fn now_epoch_secs_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}

fn load_checkpoint_store_unlocked(project_dir: &Path) -> CheckpointStore {
    fs::read_to_string(checkpoints_path_for_project(project_dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn load_checkpoint_store(project_dir: &Path) -> CheckpointStore {
    let _guard = SNAPSHOT_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    load_checkpoint_store_unlocked(project_dir)
}

pub fn save_checkpoint_store(project_dir: &Path, store: &CheckpointStore) -> Result<(), String> {
    let path = checkpoints_path_for_project(project_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create checkpoint directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| format!("failed to serialize checkpoints: {e}"))?;
    let seq = CHECKPOINT_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.tmp.{}.{seq}", std::process::id()));
    fs::write(&tmp, json).map_err(|e| format!("failed to write temp checkpoints file: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("failed to commit checkpoints file: {e}")
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
    let _guard = SNAPSHOT_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
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

fn persist_turn_checkpoint_unlocked(
    project_dir: &Path,
    session_id: String,
    prompt_preview: String,
    tree_oid: String,
) -> Result<TurnCheckpoint, String> {
    let mut store = load_checkpoint_store_unlocked(project_dir);
    let turn_index = store
        .checkpoints
        .iter()
        .filter(|c| c.session_id == session_id)
        .count();
    let cp = TurnCheckpoint {
        session_id,
        entry_id: format!("turn-{}", store.checkpoints.len() + 1),
        turn_index: Some(turn_index),
        prompt_preview: normalize_prompt_preview(&prompt_preview),
        tree_oid,
        created_at: now_epoch_secs_string(),
    };
    store.checkpoints.push(cp.clone());
    save_checkpoint_store(project_dir, &store)?;
    Ok(cp)
}

pub fn record_captured_turn_checkpoint(
    project_dir: &Path,
    session_id: String,
    prompt_preview: String,
    tree_oid: String,
) -> Result<TurnCheckpoint, String> {
    let tree_oid = validate_tree_oid(&tree_oid)?;
    let _guard = SNAPSHOT_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    persist_turn_checkpoint_unlocked(project_dir, session_id, prompt_preview, tree_oid)
}

#[allow(dead_code)]
pub fn capture_and_record_turn_checkpoint(
    project_dir: &Path,
    session_id: String,
    prompt_preview: String,
) -> Result<TurnCheckpoint, String> {
    let _guard = SNAPSHOT_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (project, shadow) = ensure_shadow_git_dir(project_dir)?;
    let tree_oid = capture_snapshot_in(&project, &shadow)?;
    persist_turn_checkpoint_unlocked(project_dir, session_id, prompt_preview, tree_oid)
}

pub fn save_last_redo(
    project_dir: &Path,
    redo_tree_oid: String,
    files: Vec<String>,
    target_tree_oid: String,
    session_id: &str,
) -> Result<(), String> {
    let redo_tree_oid = validate_tree_oid(&redo_tree_oid)?;
    let target_tree_oid = Some(validate_tree_oid(&target_tree_oid)?);
    let session_id = Some(
        resolved_session_id(session_id)
            .ok_or("revert session is unresolved")?
            .to_string(),
    );
    let _guard = SNAPSHOT_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut store = load_checkpoint_store_unlocked(project_dir);
    store.last_redo = Some(LastRedoState {
        redo_tree_oid,
        files,
        target_tree_oid,
        session_id,
    });
    save_checkpoint_store(project_dir, &store)
}

pub fn redo_last_revert(
    project_dir: &Path,
    session_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let _guard = SNAPSHOT_GIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut store = load_checkpoint_store_unlocked(project_dir);
    let redo = store
        .last_redo
        .as_ref()
        .ok_or("No revert operation is available to redo.")?;
    let owner = redo
        .session_id
        .as_deref()
        .and_then(resolved_session_id)
        .ok_or("Redo checkpoint lacks a session identity.")?;
    let expected_tree = redo
        .target_tree_oid
        .as_deref()
        .ok_or("Redo checkpoint lacks an expected working tree.")?;
    if session_id.and_then(resolved_session_id) != Some(owner) {
        return Err(format!(
            "refusing redo: active session does not match revert owner '{owner}'"
        ));
    }
    let (project, shadow) = ensure_shadow_git_dir(project_dir)?;
    let restored = redo_targeted_revert_locked(
        &project,
        &shadow,
        &redo.redo_tree_oid,
        &redo.files,
        Some(expected_tree),
    )?;
    store.last_redo = None;
    save_checkpoint_store(project_dir, &store)?;
    Ok(restored)
}

fn resolved_session_id(session_id: &str) -> Option<&str> {
    let trimmed = session_id.trim();
    (!trimmed.is_empty() && trimmed != UNRESOLVED_SESSION_ID).then_some(trimmed)
}

fn prompts_compatible(recorded: &str, requested: &str) -> bool {
    recorded.is_empty()
        || recorded == requested
        || recorded.starts_with(requested)
        || requested.starts_with(recorded)
}

fn find_checkpoint_in_session(
    store: &CheckpointStore,
    sid: &str,
    entry_id: &str,
    turn_index: Option<usize>,
    prompt_text: Option<&str>,
) -> Option<TurnCheckpoint> {
    let cps: Vec<&TurnCheckpoint> = store
        .checkpoints
        .iter()
        .filter(|c| c.session_id.trim() == sid && validate_tree_oid(&c.tree_oid).is_ok())
        .collect();
    if cps.is_empty() {
        return None;
    }
    let norm_prompt = prompt_text
        .map(normalize_prompt_preview)
        .filter(|s| !s.is_empty());

    let trimmed_entry = entry_id.trim();
    if !trimmed_entry.is_empty() {
        let m: Vec<&TurnCheckpoint> = cps
            .iter()
            .copied()
            .filter(|c| c.entry_id.trim() == trimmed_entry)
            .collect();
        if m.len() == 1 {
            let cand = m[0];
            let conflict = turn_index.is_some_and(|idx| {
                cand.turn_index != Some(idx)
                    && cps
                        .iter()
                        .any(|o| !std::ptr::eq(*o, cand) && o.turn_index == Some(idx))
            });
            return (!conflict).then(|| cand.clone());
        }
        if m.len() > 1 {
            return None;
        }
    }
    if let Some(idx) = turn_index {
        let m: Vec<&TurnCheckpoint> = cps
            .iter()
            .copied()
            .filter(|c| c.turn_index == Some(idx))
            .collect();
        if m.len() == 1 {
            let cand = m[0];
            if norm_prompt
                .as_deref()
                .is_none_or(|norm| prompts_compatible(&cand.prompt_preview, norm))
            {
                return Some(cand.clone());
            }
        }
        if m.len() > 1 {
            if let Some(ref norm) = norm_prompt {
                let d: Vec<&TurnCheckpoint> = m
                    .into_iter()
                    .filter(|c| c.prompt_preview == *norm)
                    .collect();
                if d.len() == 1 {
                    return Some(d[0].clone());
                }
            }
            return None;
        }
    }
    if let Some(ref norm) = norm_prompt {
        let m: Vec<&TurnCheckpoint> = cps
            .iter()
            .copied()
            .filter(|c| c.prompt_preview == *norm)
            .collect();
        if m.len() == 1 {
            return Some(m[0].clone());
        }
    }
    None
}

/// Finds a checkpoint across an ordered session lineage (active session first, then parent
/// session(s)), stopping at the first resolved session that matches cleanly.
pub fn find_checkpoint_with_lineage(
    store: &CheckpointStore,
    session_ids: &[&str],
    entry_id: &str,
    turn_index: Option<usize>,
    prompt_preview: Option<&str>,
) -> Option<TurnCheckpoint> {
    let mut seen = HashSet::new();
    for raw_sid in session_ids {
        let Some(sid) = resolved_session_id(raw_sid) else {
            continue;
        };
        if !seen.insert(sid) {
            continue;
        }
        if let Some(cp) =
            find_checkpoint_in_session(store, sid, entry_id, turn_index, prompt_preview)
        {
            return Some(cp);
        }
    }
    None
}

/// Finds a checkpoint strictly bound to `session_id` (unique entry_id -> unique turn_index -> unique prompt).
#[allow(dead_code)]
pub fn find_checkpoint(
    store: &CheckpointStore,
    session_id: Option<&str>,
    entry_id: &str,
    turn_index: Option<usize>,
    prompt_text: Option<&str>,
) -> Option<TurnCheckpoint> {
    let sid = session_id?;
    find_checkpoint_with_lineage(store, &[sid], entry_id, turn_index, prompt_text)
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::env;
    use std::thread;
    use std::time::Duration;

    pub(crate) static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());
    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestEnv {
        root: PathBuf,
        project: PathBuf,
        prev_vh: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestEnv {
        fn new(prefix: &str, init_git: bool) -> Self {
            let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let n = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "verity-snap-{prefix}-{}-{ts}-{n}",
                std::process::id()
            ));
            let (home, project) = (root.join("home"), root.join("project"));
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&project).unwrap();
            let prev_vh = env::var("VERITY_HOME").ok();
            env::set_var("VERITY_HOME", &home);
            if init_git {
                run_git(None, None, &["init", project.to_str().unwrap()]).unwrap();
            }
            Self {
                root,
                project,
                prev_vh,
                _lock: lock,
            }
        }
        fn path(&self, rel: &str) -> PathBuf {
            self.project.join(rel)
        }
        fn write(&self, rel: &str, content: &str) {
            let full = self.path(rel);
            if let Some(p) = full.parent() {
                fs::create_dir_all(p).unwrap();
            }
            fs::write(full, content).unwrap();
        }
        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.path(rel)).unwrap()
        }
        fn rm(&self, rel: &str) {
            let full = self.path(rel);
            if full.is_dir() {
                fs::remove_dir_all(full).unwrap();
            } else {
                fs::remove_file(full).unwrap();
            }
        }
        fn capture(&self) -> String {
            capture_snapshot(&self.project).unwrap()
        }
        fn revert(&self, oid: &str) -> Result<TargetedRevertOutcome, String> {
            revert_to_snapshot(&self.project, oid)
        }
        fn redo(&self, o: &TargetedRevertOutcome) -> Result<Vec<String>, String> {
            redo_targeted_revert(&self.project, &o.redo_tree_oid, &o.reverted_files)
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            match &self.prev_vh {
                Some(v) => env::set_var("VERITY_HOME", v),
                None => env::remove_var("VERITY_HOME"),
            }
            let _ = fs::remove_dir_all(&self.root);
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
        let env = TestEnv::new("revert", false);
        env.write("untouched.txt", "never changed\n");
        env.write("src/app.rs", "fn main() { println!(\"v1\"); }\n");
        env.write("keep_me.txt", "important data\n");
        let mtime_before = fs::metadata(env.path("untouched.txt"))
            .unwrap()
            .modified()
            .unwrap();

        let snap1 = env.capture();
        assert_eq!(snap1.len(), 40);
        thread::sleep(Duration::from_millis(30));

        env.write("src/app.rs", "fn main() { println!(\"v2-broken\"); }\n");
        env.rm("keep_me.txt");
        env.write("generated/extra.txt", "unwanted file\n");

        let outcome = env.revert(&snap1).unwrap();
        assert_eq!(outcome.target_tree_oid, snap1);
        assert_ne!(outcome.redo_tree_oid, snap1);
        assert!(outcome.reverted_files.contains(&"src/app.rs".into()));
        assert!(outcome.reverted_files.contains(&"keep_me.txt".into()));
        assert!(outcome
            .reverted_files
            .contains(&"generated/extra.txt".into()));
        assert!(!outcome.reverted_files.contains(&"untouched.txt".into()));

        assert_eq!(env.read("src/app.rs"), "fn main() { println!(\"v1\"); }\n");
        assert_eq!(env.read("keep_me.txt"), "important data\n");
        assert!(!env.path("generated").exists());
        assert_eq!(
            mtime_before,
            fs::metadata(env.path("untouched.txt"))
                .unwrap()
                .modified()
                .unwrap()
        );

        thread::sleep(Duration::from_millis(30));
        assert_eq!(
            env.redo(&outcome).unwrap().len(),
            outcome.reverted_files.len()
        );
        assert_eq!(
            env.read("src/app.rs"),
            "fn main() { println!(\"v2-broken\"); }\n"
        );
        assert!(!env.path("keep_me.txt").exists());
        assert_eq!(env.read("generated/extra.txt"), "unwanted file\n");
        assert_eq!(
            mtime_before,
            fs::metadata(env.path("untouched.txt"))
                .unwrap()
                .modified()
                .unwrap()
        );
    }

    #[test]
    fn non_git_directory_works_via_shadow_gitdir_without_creating_project_dot_git() {
        let env = TestEnv::new("nongit", false);
        env.write("notes.md", "version 1\n");
        let oid = env.capture();
        assert_eq!(oid.len(), 40);
        assert!(!env.path(".git").exists());
        assert!(shadow_git_dir_for_project(&env.project)
            .join("HEAD")
            .exists());

        env.write("notes.md", "version 2\n");
        assert_eq!(
            env.revert(&oid).unwrap().reverted_files,
            vec!["notes.md".to_string()]
        );
        assert_eq!(env.read("notes.md"), "version 1\n");
        assert!(!env.path(".git").exists());
    }

    #[test]
    fn empty_diff_revert_touches_zero_files() {
        let env = TestEnv::new("noop", false);
        env.write("a.txt", "same\n");
        let mtime_before = fs::metadata(env.path("a.txt")).unwrap().modified().unwrap();
        let oid = env.capture();
        thread::sleep(Duration::from_millis(25));
        assert!(env.revert(&oid).unwrap().reverted_files.is_empty());
        assert_eq!(
            mtime_before,
            fs::metadata(env.path("a.txt")).unwrap().modified().unwrap()
        );
    }

    #[test]
    fn checkpoint_persistence_and_lookup_work_by_entry_id_and_prompt() {
        let env = TestEnv::new("cp", false);
        let oid = "0123456789abcdef0123456789abcdef01234567".to_string();
        record_turn_checkpoint(
            &env.project,
            "sess-1".into(),
            "turn-1".into(),
            Some(0),
            "  Fix   the login bug  ".into(),
            oid.clone(),
        )
        .unwrap();
        let store = load_checkpoint_store(&env.project);
        assert_eq!(store.checkpoints[0].prompt_preview, "Fix the login bug");
        assert_eq!(
            find_checkpoint(&store, Some("sess-1"), "turn-1", None, None)
                .unwrap()
                .tree_oid,
            oid
        );
        assert_eq!(
            find_checkpoint(&store, Some("sess-1"), "pi-99", Some(0), None)
                .unwrap()
                .tree_oid,
            oid
        );
        assert_eq!(
            find_checkpoint(
                &store,
                Some("sess-1"),
                "pi-99",
                Some(99),
                Some("Fix the login bug")
            )
            .unwrap()
            .tree_oid,
            oid
        );
        assert!(
            find_checkpoint(&store, Some("sess-1"), "unknown", Some(5), Some("other")).is_none()
        );
    }

    #[test]
    fn foreign_or_unresolved_session_checkpoint_never_applies() {
        let env = TestEnv::new("foreign-cp", false);
        record_turn_checkpoint(
            &env.project,
            "session-a".into(),
            "turn-1".into(),
            Some(0),
            "Shared".into(),
            "1111111111111111111111111111111111111111".into(),
        )
        .unwrap();
        record_turn_checkpoint(
            &env.project,
            "default-session".into(),
            "turn-2".into(),
            Some(0),
            "Default".into(),
            "2222222222222222222222222222222222222222".into(),
        )
        .unwrap();
        let store = load_checkpoint_store(&env.project);
        for (sid, eid, prompt) in [
            (Some("session-b"), "turn-1", "Shared"),
            (Some("session-b"), "turn-2", "Default"),
            (Some("   "), "turn-1", "Shared"),
            (Some("default-session"), "turn-2", "Default"),
            (None, "turn-1", "Shared"),
        ] {
            assert!(find_checkpoint(&store, sid, eid, Some(0), Some(prompt)).is_none());
        }
    }

    #[test]
    fn ambiguous_same_session_checkpoints_never_apply_without_unique_disambiguation() {
        let env = TestEnv::new("ambig-cp", false);
        let oid0 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let oid_b = "dddddddddddddddddddddddddddddddddddddddd".to_string();
        for (eid, idx, prompt, oid) in [
            ("turn-1", 0, "continue", oid0.clone()),
            (
                "turn-2",
                1,
                "continue",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            ),
            (
                "turn-3",
                2,
                "alpha task",
                "cccccccccccccccccccccccccccccccccccccccc".into(),
            ),
            ("turn-4", 2, "beta task", oid_b.clone()),
        ] {
            record_turn_checkpoint(
                &env.project,
                "session-a".into(),
                eid.into(),
                Some(idx),
                prompt.into(),
                oid,
            )
            .unwrap();
        }
        let store = load_checkpoint_store(&env.project);
        assert!(find_checkpoint(&store, Some("session-a"), "x", None, Some("continue")).is_none());
        assert_eq!(
            find_checkpoint(&store, Some("session-a"), "x", Some(0), Some("continue"))
                .unwrap()
                .tree_oid,
            oid0
        );
        assert!(find_checkpoint(&store, Some("session-a"), "x", Some(2), None).is_none());
        assert_eq!(
            find_checkpoint(&store, Some("session-a"), "x", Some(2), Some("beta task"))
                .unwrap()
                .tree_oid,
            oid_b
        );
        assert!(find_checkpoint(&store, Some("session-a"), "turn-1", Some(1), None).is_none());
    }

    #[test]
    fn revert_and_redo_preserve_sibling_ignored_file_inside_shared_directory() {
        let env = TestEnv::new("sib-ign", true);
        env.write(".gitignore", "*.ignored\n");
        env.write("shape/keep.txt", "v1");
        env.write("shape/local.ignored", "stay safe");
        let snap1 = env.capture();

        env.write("shape/keep.txt", "v2");
        env.write("shape/added.txt", "extra");
        let outcome = env.revert(&snap1).unwrap();
        assert_eq!(env.read("shape/keep.txt"), "v1");
        assert!(!env.path("shape/added.txt").exists());
        assert_eq!(env.read("shape/local.ignored"), "stay safe");

        env.redo(&outcome).unwrap();
        assert_eq!(env.read("shape/keep.txt"), "v2");
        assert_eq!(env.read("shape/added.txt"), "extra");
        assert_eq!(env.read("shape/local.ignored"), "stay safe");
    }

    #[test]
    fn revert_fails_safely_when_ignored_file_blocks_ancestor_or_exact_path() {
        let env = TestEnv::new("rev-anc-ign", true);
        env.write("aaa.txt", "before");
        env.write("shape/visible.txt", "v1");
        let snap1 = env.capture();
        env.rm("shape");
        let _snap2 = env.capture();
        env.write(".gitignore", "shape\n");
        env.write("aaa.txt", "after");
        env.write("shape", "ignored file blocking dir");
        assert!(env
            .revert(&snap1)
            .unwrap_err()
            .contains("preflight collision"));
        assert_eq!(env.read("aaa.txt"), "after");
        assert_eq!(env.read("shape"), "ignored file blocking dir");
        drop(env);

        let env2 = TestEnv::new("rev-exact-ign", true);
        env2.write("aaa.txt", "before");
        env2.write("secret.env", "tracked-v1");
        let s1 = env2.capture();
        env2.rm("secret.env");
        let _s2 = env2.capture();
        env2.write(".gitignore", "secret.env\n");
        env2.write("aaa.txt", "after");
        env2.write("secret.env", "local-secret-do-not-overwrite");
        assert!(env2
            .revert(&s1)
            .unwrap_err()
            .contains("preflight collision"));
        assert_eq!(env2.read("aaa.txt"), "after");
        assert_eq!(env2.read("secret.env"), "local-secret-do-not-overwrite");
    }

    #[test]
    fn revert_fails_safely_without_losing_nested_ignored_file_when_reverting_dir_to_file() {
        let env = TestEnv::new("rev-nested-ign", true);
        env.write(".gitignore", "*.ignored\n");
        env.write("aaa.txt", "before");
        env.write("shape", "original file");
        let snap1 = env.capture();

        env.write("aaa.txt", "after");
        env.rm("shape");
        env.write("shape/visible.txt", "current file");
        env.write("shape/local.ignored", "keep local ignored");
        env.write("shape/nested/deep.ignored", "keep deep ignored");

        assert!(env
            .revert(&snap1)
            .unwrap_err()
            .contains("preflight collision"));
        assert_eq!(env.read("aaa.txt"), "after");
        assert!(env.path("shape").is_dir());
        assert_eq!(env.read("shape/visible.txt"), "current file");
        assert_eq!(env.read("shape/local.ignored"), "keep local ignored");
        assert_eq!(env.read("shape/nested/deep.ignored"), "keep deep ignored");
    }

    #[test]
    fn revert_and_redo_succeed_for_dir_to_file_transition_without_ignored_files() {
        let env = TestEnv::new("shape-ok", true);
        env.write("aaa.txt", "before");
        env.write("shape", "original file");
        let snap1 = env.capture();
        env.write("aaa.txt", "after");
        env.rm("shape");
        env.write("shape/visible.txt", "dir file");

        let outcome = env.revert(&snap1).unwrap();
        assert_eq!(env.read("aaa.txt"), "before");
        assert_eq!(env.read("shape"), "original file");

        env.redo(&outcome).unwrap();
        assert_eq!(env.read("aaa.txt"), "after");
        assert_eq!(env.read("shape/visible.txt"), "dir file");
    }

    #[test]
    fn redo_fails_safely_without_losing_ignored_or_unrelated_files_on_collision() {
        let env = TestEnv::new("redo-coll", true);
        env.write(".gitignore", "*.ignored\n");
        env.write("aaa.txt", "before");
        env.write("shape/visible.txt", "original dir file");
        let snap1 = env.capture();
        env.write("aaa.txt", "after");
        env.rm("shape");
        env.write("shape", "current file");

        let outcome = env.revert(&snap1).unwrap();
        env.write("shape/local.ignored", "keep ignored");
        env.write("shape/unrelated.txt", "keep unrelated");

        assert!(env
            .redo(&outcome)
            .unwrap_err()
            .contains("preflight collision"));
        assert_eq!(env.read("aaa.txt"), "before");
        assert_eq!(env.read("shape/visible.txt"), "original dir file");
        assert_eq!(env.read("shape/local.ignored"), "keep ignored");
        assert_eq!(env.read("shape/unrelated.txt"), "keep unrelated");
    }

    #[test]
    fn redo_fails_safely_without_deleting_ignored_file_at_removal_path() {
        let env = TestEnv::new("redo-ign-rm", true);
        env.write("aaa.txt", "before");
        env.write("temp.dat", "tracked-v1");
        let snap1 = env.capture();
        env.write("aaa.txt", "after");
        env.rm("temp.dat");

        let outcome = env.revert(&snap1).unwrap();
        env.write(".gitignore", "temp.dat\n");
        env.write("temp.dat", "local-ignored-keep");

        assert!(env
            .redo(&outcome)
            .unwrap_err()
            .contains("preflight collision"));
        assert_eq!(env.read("aaa.txt"), "before");
        assert_eq!(env.read("temp.dat"), "local-ignored-keep");
    }

    #[test]
    fn redo_last_revert_enforces_session_ownership_and_refuses_stale_affected_paths() {
        let env = TestEnv::new("redo-stale", false);
        env.write("affected.txt", "v1");
        env.write("unrelated.txt", "u1");
        let snap1 = env.capture();
        env.write("affected.txt", "v2");
        let outcome = env.revert(&snap1).unwrap();
        save_last_redo(
            &env.project,
            outcome.redo_tree_oid.clone(),
            outcome.reverted_files.clone(),
            outcome.target_tree_oid.clone(),
            "sess-a",
        )
        .unwrap();

        for caller in [None, Some(""), Some("default-session"), Some("sess-b")] {
            assert!(redo_last_revert(&env.project, caller)
                .unwrap_err()
                .contains("refusing redo"));
        }

        env.write("affected.txt", "v3-later");
        assert!(redo_last_revert(&env.project, Some("sess-a"))
            .unwrap_err()
            .contains("stale redo"));
        assert_eq!(env.read("affected.txt"), "v3-later");
        assert!(load_checkpoint_store(&env.project).last_redo.is_some());

        env.write("affected.txt", "v1");
        env.write("unrelated.txt", "u2-unrelated");
        assert_eq!(
            redo_last_revert(&env.project, Some("sess-a")).unwrap(),
            vec!["affected.txt".to_string()]
        );
        assert_eq!(env.read("affected.txt"), "v2");
        assert_eq!(env.read("unrelated.txt"), "u2-unrelated");
        assert!(load_checkpoint_store(&env.project).last_redo.is_none());
    }

    #[test]
    fn revert_and_redo_succeed_for_dir_to_file_transition_with_empty_untracked_subdir() {
        let env = TestEnv::new("shape-empty-sub", true);
        env.write("aaa.txt", "before");
        env.write("shape", "original file");
        let snap1 = env.capture();

        env.write("aaa.txt", "after");
        env.rm("shape");
        env.write("shape/visible.txt", "dir file");
        fs::create_dir_all(env.path("shape/empty_sub")).unwrap();

        let outcome = env.revert(&snap1).unwrap();
        assert_eq!(env.read("aaa.txt"), "before");
        assert!(!env.path("shape").is_dir());
        assert_eq!(env.read("shape"), "original file");

        env.redo(&outcome).unwrap();
        assert_eq!(env.read("aaa.txt"), "after");
        assert_eq!(env.read("shape/visible.txt"), "dir file");
    }

    #[test]
    fn revert_fails_safely_when_previously_indexed_file_is_added_to_gitignore_and_edited() {
        let env = TestEnv::new("rev-reign", true);
        env.write("aaa.txt", "before");
        env.write("secret.env", "tracked-v1");
        let snap1 = env.capture();

        env.write(".gitignore", "secret.env\n");
        env.write("aaa.txt", "after");
        env.write("secret.env", "local-secret-edited");

        assert!(env
            .revert(&snap1)
            .unwrap_err()
            .contains("preflight collision"));
        assert_eq!(env.read("aaa.txt"), "after");
        assert_eq!(env.read("secret.env"), "local-secret-edited");
    }

    #[test]
    fn find_checkpoint_with_lineage_resolves_active_then_parent_and_blocks_foreign() {
        let env = TestEnv::new("lineage-cp", false);
        let oid_p0 = "1111111111111111111111111111111111111111".to_string();
        let oid_p1 = "2222222222222222222222222222222222222222".to_string();
        let oid_c0 = "3333333333333333333333333333333333333333".to_string();
        let oid_foreign = "4444444444444444444444444444444444444444".to_string();

        record_turn_checkpoint(
            &env.project,
            "session-parent".into(),
            "turn-1".into(),
            Some(0),
            "First prompt".into(),
            oid_p0.clone(),
        )
        .unwrap();
        record_turn_checkpoint(
            &env.project,
            "session-parent".into(),
            "turn-2".into(),
            Some(1),
            "Second prompt".into(),
            oid_p1.clone(),
        )
        .unwrap();
        record_turn_checkpoint(
            &env.project,
            "session-child".into(),
            "turn-3".into(),
            Some(0),
            "Revised second prompt".into(),
            oid_c0.clone(),
        )
        .unwrap();
        record_turn_checkpoint(
            &env.project,
            "session-foreign".into(),
            "turn-4".into(),
            Some(0),
            "Foreign prompt".into(),
            oid_foreign,
        )
        .unwrap();

        let store = load_checkpoint_store(&env.project);
        let lineage = ["session-child", "session-parent"];

        // Active child session checkpoint wins when matching its own turn.
        assert_eq!(
            find_checkpoint_with_lineage(
                &store,
                &lineage,
                "pi-c1",
                Some(0),
                Some("Revised second prompt"),
            )
            .unwrap()
            .tree_oid,
            oid_c0
        );

        // Chained revert to an earlier parent turn falls through cleanly to parent session.
        assert_eq!(
            find_checkpoint_with_lineage(&store, &lineage, "pi-p0", Some(0), Some("First prompt"))
                .unwrap()
                .tree_oid,
            oid_p0
        );
        assert_eq!(
            find_checkpoint_with_lineage(
                &store,
                &lineage,
                "pi-p1",
                Some(1),
                Some("Second prompt"),
            )
            .unwrap()
            .tree_oid,
            oid_p1
        );

        // Foreign session checkpoints are never matched.
        assert!(find_checkpoint_with_lineage(
            &store,
            &lineage,
            "turn-4",
            Some(0),
            Some("Foreign prompt"),
        )
        .is_none());
    }

    #[test]
    fn clean_git_command_suppresses_windows_console_and_terminal_prompts() {
        let cmd = clean_git_command();
        assert!(cmd
            .get_envs()
            .any(|(k, v)| k == "GIT_TERMINAL_PROMPT" && v == Some(std::ffi::OsStr::new("0"))));
        let source = include_str!("snapshots.rs");
        assert!(source.contains("CREATE_NO_WINDOW"));
        assert!(source.contains("Stdio::null()"));
    }
}
