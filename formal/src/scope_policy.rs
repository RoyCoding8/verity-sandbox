#[cfg(kani)]
use verity_core::{check_scope_core, ScopeViolationKind};
#[cfg(test)]
use verity_core::{check_scope, within, RelPath, ScopeError};

/// Law 3: Write-scope containment and forbidden-path precedence.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(7)]
pub fn prove_scope_containment() {
    let write_scope = ["a/b"];
    let forbidden = ["a/b/s"];

    let choice: u8 = kani::any();
    kani::assume(choice <= 4);
    let empty: [&str; 0] = [];
    let ok_path = ["a/b/c"];
    let secret_path = ["a/b/s"];
    let sibling_path = ["a/b2/c"];
    let mixed_paths = ["a/b/c", "r"];

    let changed: &[&str] = match choice {
        0 => &empty,
        1 => &ok_path,
        2 => &secret_path,
        3 => &sibling_path,
        _ => &mixed_paths,
    };

    let res = check_scope_core(changed, &write_scope, &forbidden);
    match choice {
        0 => assert_eq!(res, Err(ScopeViolationKind::EmptyDiff)),
        1 => {
            assert!(res.is_ok());
        }
        2 => assert_eq!(res, Err(ScopeViolationKind::ForbiddenPath { index: 0 })),
        3 => assert_eq!(res, Err(ScopeViolationKind::OutsideWriteScope { index: 0 })),
        4 => assert_eq!(res, Err(ScopeViolationKind::OutsideWriteScope { index: 1 })),
        _ => unreachable!(),
    }

    kani::cover!(res.is_ok(), "scope_accept_reachable");
    kani::cover!(
        matches!(res, Err(ScopeViolationKind::ForbiddenPath { .. })),
        "scope_forbidden_reachable"
    );
    kani::cover!(
        matches!(res, Err(ScopeViolationKind::OutsideWriteScope { .. })),
        "scope_outside_reachable"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_boundary_does_not_match_sibling_directory() {
        let dir = RelPath::parse("crates/core").unwrap();
        let inside = RelPath::parse("crates/core/src/lib.rs").unwrap();
        let sibling = RelPath::parse("crates/core_extra/src/lib.rs").unwrap();
        assert!(within(&inside, &dir));
        assert!(!within(&sibling, &dir));
    }

    #[test]
    fn forbidden_overrides_write_scope() {
        let write_scope = vec![RelPath::parse("src").unwrap()];
        let forbidden = vec![RelPath::parse("src/locked.rs").unwrap()];
        let changed = vec![RelPath::parse("src/locked.rs").unwrap()];
        assert_eq!(
            check_scope(&changed, &write_scope, &forbidden),
            Err(ScopeError::ForbiddenPath(
                RelPath::parse("src/locked.rs").unwrap()
            ))
        );
    }
}
