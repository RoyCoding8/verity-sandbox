use crate::types::RelPath;
use std::fmt;

#[cfg(verus_only)]
use self::proof::{
    approved_path, matches_prefix, path_bytes, scope_error, scope_result, stable_path_view,
    within_prefix,
};
#[cfg(verus_only)]
use vstd::prelude::*;
#[cfg(verus_only)]
use vstd::string::StringSliceAdditionalSpecFns;
#[cfg(verus_only)]
pub(crate) mod proof;

#[cfg_attr(verus_only, verus_verify)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    EmptyDiff,
    OutsideWriteScope(RelPath),
    ForbiddenPath(RelPath),
}

/// Allocation-free scope violation classification referencing the offending index.
#[cfg_attr(verus_only, verus_verify)]
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
                write!(
                    f,
                    "changed path '{}' is outside approved write scope",
                    p.as_str()
                )
            }
            ScopeError::ForbiddenPath(p) => {
                write!(f, "changed path '{}' hits a forbidden path", p.as_str())
            }
        }
    }
}

impl std::error::Error for ScopeError {}

/// Allocation-free byte-level containment check: returns true if `prefix` is non-empty
/// and `path` equals `prefix` or sits directly beneath `prefix/`.
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == within_prefix(path@, prefix@),
    no_unwind
))]
pub fn within_bytes(path: &[u8], prefix: &[u8]) -> bool {
    if prefix.is_empty() || path.len() < prefix.len() {
        return false;
    }
    let mut i = 0usize;
    #[cfg_attr(verus_only, verus_spec(
        invariant
            i <= prefix@.len() <= path@.len(),
            0 < prefix@.len(),
            forall|j: int| 0 <= j < i ==> path@[j] == prefix@[j],
        decreases prefix@.len() - i,
    ))]
    while i < prefix.len() {
        if path[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    path.len() == prefix.len() || path[prefix.len()] == b'/'
}

/// Allocation-free string containment check delegating to `within_bytes`.
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == within_prefix(path.spec_bytes(), prefix.spec_bytes()),
    no_unwind
))]
pub fn within_str(path: &str, prefix: &str) -> bool {
    within_bytes(path.as_bytes(), prefix.as_bytes())
}

/// Returns true if `path` equals `prefix` or sits directly beneath `prefix/`.
#[cfg_attr(verus_only, verus_spec(result =>
    ensures result == within_prefix(path_bytes(path), path_bytes(prefix)),
    no_unwind
))]
pub fn within(path: &RelPath, prefix: &RelPath) -> bool {
    within_str(path.as_str(), prefix.as_str())
}

#[cfg_attr(verus_only, verus_spec(result =>
    requires stable_path_view::<P>(),
    ensures result == matches_prefix(path.spec_bytes(), prefixes@),
    no_unwind
))]
fn matches_scope<P: AsRef<str>>(path: &str, prefixes: &[P]) -> bool {
    let mut index = 0usize;
    #[cfg_attr(verus_only, verus_spec(
        invariant
            stable_path_view::<P>(),
            index <= prefixes@.len(),
            forall|i: int| 0 <= i < index ==>
                !within_prefix(path.spec_bytes(), #[trigger] path_bytes(&prefixes@[i])),
        decreases prefixes@.len() - index,
    ))]
    while index < prefixes.len() {
        if within_str(path, prefixes[index].as_ref()) {
            #[cfg(verus_only)]
            proof! {
                assert(within_prefix(path.spec_bytes(), path_bytes(&prefixes@[index as int])));
            }
            return true;
        }
        index += 1;
    }
    false
}

/// Allocation-free core scope check.
#[cfg_attr(verus_only, verus_spec(result =>
    requires stable_path_view::<P>(),
    ensures scope_result(result, changed_paths@, write_scope@, forbidden@),
    no_unwind
))]
pub fn check_scope_core<P: AsRef<str>>(
    changed_paths: &[P],
    write_scope: &[P],
    forbidden: &[P],
) -> Result<(), ScopeViolationKind> {
    if changed_paths.is_empty() {
        return Err(ScopeViolationKind::EmptyDiff);
    }
    let mut index = 0usize;
    #[cfg_attr(verus_only, verus_spec(
        invariant
            stable_path_view::<P>(),
            0 < changed_paths@.len(),
            index <= changed_paths@.len(),
            forall|i: int| 0 <= i < index ==>
                approved_path(#[trigger] path_bytes(&changed_paths@[i]), write_scope@, forbidden@),
        decreases changed_paths@.len() - index,
    ))]
    while index < changed_paths.len() {
        let path = changed_paths[index].as_ref();
        if matches_scope(path, forbidden) {
            return Err(ScopeViolationKind::ForbiddenPath { index });
        }
        if !matches_scope(path, write_scope) {
            return Err(ScopeViolationKind::OutsideWriteScope { index });
        }
        index += 1;
    }
    Ok(())
}

/// Validates that a write task's diff is non-empty, contained in `write_scope`,
/// and disjoint from `forbidden` (with `forbidden` taking precedence).
#[cfg_attr(verus_only, verus_spec(result =>
    ensures scope_error(result, changed_paths@, write_scope@, forbidden@),
))]
pub fn check_scope(
    changed_paths: &[RelPath],
    write_scope: &[RelPath],
    forbidden: &[RelPath],
) -> Result<(), ScopeError> {
    match check_scope_core(changed_paths, write_scope, forbidden) {
        Ok(()) => Ok(()),
        Err(ScopeViolationKind::EmptyDiff) => Err(ScopeError::EmptyDiff),
        Err(ScopeViolationKind::ForbiddenPath { index }) => {
            Err(ScopeError::ForbiddenPath(changed_paths[index].clone()))
        }
        Err(ScopeViolationKind::OutsideWriteScope { index }) => {
            Err(ScopeError::OutsideWriteScope(changed_paths[index].clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{check_scope_core, ScopeViolationKind};

    fn cases() -> Vec<Vec<&'static str>> {
        const PATHS: [&str; 6] = ["a/b", "a/b/c", "a/b/s", "a/b/s/x", "a/b_2/c", "r"];
        let mut lists = vec![vec![]];
        for first in PATHS {
            lists.push(vec![first]);
            for second in PATHS {
                lists.push(vec![first, second]);
            }
        }
        lists
    }

    #[test]
    fn scope_decision_matches_the_policy_for_every_bounded_list() {
        let lists = cases();
        for changed in &lists {
            for allowed in &lists {
                for forbidden in &lists {
                    let expected = if changed.is_empty() {
                        Err(ScopeViolationKind::EmptyDiff)
                    } else {
                        let first_violation =
                            changed.iter().enumerate().find_map(|(index, path)| {
                                let under = |prefix: &&str| {
                                    path == prefix
                                        || path
                                            .strip_prefix(prefix)
                                            .is_some_and(|tail| tail.starts_with('/'))
                                };
                                if forbidden.iter().any(under) {
                                    Some(ScopeViolationKind::ForbiddenPath { index })
                                } else if !allowed.iter().any(under) {
                                    Some(ScopeViolationKind::OutsideWriteScope { index })
                                } else {
                                    None
                                }
                            });
                        first_violation.map_or(Ok(()), Err)
                    };
                    assert_eq!(
                        check_scope_core(changed, allowed, forbidden),
                        expected,
                        "changed={changed:?}, allowed={allowed:?}, forbidden={forbidden:?}"
                    );
                }
            }
        }
    }
}
