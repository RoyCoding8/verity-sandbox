use crate::types::RelPath;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    EmptyDiff,
    OutsideWriteScope(RelPath),
    ForbiddenPath(RelPath),
}

/// Allocation-free scope violation classification referencing the offending index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeViolationKind {
    EmptyDiff,
    OutsideWriteScope { index: usize },
    ForbiddenPath { index: usize },
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeError::EmptyDiff => write!(f, "write task produced an empty diff"),
            ScopeError::OutsideWriteScope(p) => {
                write!(f, "changed path '{}' is outside approved write scope", p.as_str())
            }
            ScopeError::ForbiddenPath(p) => {
                write!(f, "changed path '{}' hits a forbidden path", p.as_str())
            }
        }
    }
}

impl std::error::Error for ScopeError {}

/// Allocation-free byte-level containment check: returns true if `path` equals
/// `prefix` or sits directly beneath `prefix/`.
pub fn within_str(path: &str, prefix: &str) -> bool {
    let p = path.as_bytes();
    let pre = prefix.as_bytes();
    if p.len() < pre.len() {
        return false;
    }
    let mut i = 0usize;
    while i < pre.len() {
        if p[i] != pre[i] {
            return false;
        }
        i += 1;
    }
    p.len() == pre.len() || p[pre.len()] == b'/'
}

/// Returns true if `path` equals `prefix` or sits directly beneath `prefix/`.
pub fn within(path: &RelPath, prefix: &RelPath) -> bool {
    within_str(path.as_str(), prefix.as_str())
}

/// Allocation-free core scope check verified directly by Kani.
pub fn check_scope_core<P: AsRef<str>>(
    changed_paths: &[P],
    write_scope: &[P],
    forbidden: &[P],
) -> Result<(), ScopeViolationKind> {
    if changed_paths.is_empty() {
        return Err(ScopeViolationKind::EmptyDiff);
    }
    let mut index = 0usize;
    while index < changed_paths.len() {
        let path = changed_paths[index].as_ref();
        let mut f_idx = 0usize;
        while f_idx < forbidden.len() {
            if within_str(path, forbidden[f_idx].as_ref()) {
                return Err(ScopeViolationKind::ForbiddenPath { index });
            }
            f_idx += 1;
        }
        let mut allowed = false;
        let mut w_idx = 0usize;
        while w_idx < write_scope.len() {
            if within_str(path, write_scope[w_idx].as_ref()) {
                allowed = true;
                break;
            }
            w_idx += 1;
        }
        if !allowed {
            return Err(ScopeViolationKind::OutsideWriteScope { index });
        }
        index += 1;
    }
    Ok(())
}

/// Validates that a write task's diff is non-empty, contained in `write_scope`,
/// and disjoint from `forbidden` (with `forbidden` taking precedence).
pub fn check_scope(
    changed_paths: &[RelPath],
    write_scope: &[RelPath],
    forbidden: &[RelPath],
) -> Result<(), ScopeError> {
    check_scope_core(changed_paths, write_scope, forbidden).map_err(|err| match err {
        ScopeViolationKind::EmptyDiff => ScopeError::EmptyDiff,
        ScopeViolationKind::ForbiddenPath { index } => {
            ScopeError::ForbiddenPath(changed_paths[index].clone())
        }
        ScopeViolationKind::OutsideWriteScope { index } => {
            ScopeError::OutsideWriteScope(changed_paths[index].clone())
        }
    })
}
