use crate::types::RelPath;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    EmptyDiff,
    OutsideWriteScope(RelPath),
    ForbiddenPath(RelPath),
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

/// Returns true if `path` equals `prefix` or sits directly beneath `prefix/`.
pub fn within(path: &RelPath, prefix: &RelPath) -> bool {
    let p = path.as_str();
    let pre = prefix.as_str();
    if p == pre {
        return true;
    }
    p.len() > pre.len() && p.starts_with(pre) && p.as_bytes()[pre.len()] == b'/'
}

/// Validates that a write task's diff is non-empty, contained in `write_scope`,
/// and disjoint from `forbidden` (with `forbidden` taking precedence).
pub fn check_scope(
    changed_paths: &[RelPath],
    write_scope: &[RelPath],
    forbidden: &[RelPath],
) -> Result<(), ScopeError> {
    if changed_paths.is_empty() {
        return Err(ScopeError::EmptyDiff);
    }
    for path in changed_paths {
        if forbidden.iter().any(|f| within(path, f)) {
            return Err(ScopeError::ForbiddenPath(path.clone()));
        }
        if !write_scope.iter().any(|w| within(path, w)) {
            return Err(ScopeError::OutsideWriteScope(path.clone()));
        }
    }
    Ok(())
}
