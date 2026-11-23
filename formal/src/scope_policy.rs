use verity_core::{check_scope, within, RelPath, ScopeError};

/// Law 3: Write-scope containment and forbidden-path precedence.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(4)]
pub fn prove_scope_containment() {
    let write_scope = vec![RelPath::parse("crates/core").unwrap()];
    let forbidden = vec![RelPath::parse("crates/core/secret.rs").unwrap()];

    let choice: u8 = kani::any();
    kani::assume(choice <= 4);
    let changed = match choice {
        0 => vec![],
        1 => vec![RelPath::parse("crates/core/lib.rs").unwrap()],
        2 => vec![RelPath::parse("crates/core/secret.rs").unwrap()],
        3 => vec![RelPath::parse("crates/core2/lib.rs").unwrap()],
        _ => vec![
            RelPath::parse("crates/core/lib.rs").unwrap(),
            RelPath::parse("README.md").unwrap(),
        ],
    };

    let res = check_scope(&changed, &write_scope, &forbidden);
    match choice {
        0 => assert_eq!(res, Err(ScopeError::EmptyDiff)),
        1 => assert!(res.is_ok()),
        2 => assert!(matches!(res, Err(ScopeError::ForbiddenPath(_)))),
        3 | 4 => assert!(matches!(res, Err(ScopeError::OutsideWriteScope(_)))),
        _ => unreachable!(),
    }

    kani::cover!(res.is_ok(), "scope_accept_reachable");
    kani::cover!(matches!(res, Err(ScopeError::ForbiddenPath(_))), "scope_forbidden_reachable");
    kani::cover!(matches!(res, Err(ScopeError::OutsideWriteScope(_))), "scope_outside_reachable");
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
