#[cfg(kani)]
use verity_core::{
    check_scope_core, scope::within_bytes, types::validate_rel_path_bytes, ScopeViolationKind,
};

/// Law 3a: Write-scope containment and forbidden-path precedence across concrete cases.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(8)]
pub fn prove_scope_containment() {
    let write_scope = ["a/b"];
    let forbidden = ["a/b/s"];

    let choice: u8 = kani::any();
    kani::assume(choice <= 5);
    let empty: [&str; 0] = [];
    let ok_path = ["a/b/c"];
    let secret_path = ["a/b/s"];
    let secret_nested = ["a/b/s/x"];
    let sibling_path = ["a/b_2/c"];
    let mixed_paths = ["a/b/c", "r"];

    let changed: &[&str] = match choice {
        0 => &empty,
        1 => &ok_path,
        2 => &secret_path,
        3 => &secret_nested,
        4 => &sibling_path,
        _ => &mixed_paths,
    };

    let res = check_scope_core(changed, &write_scope, &forbidden);
    match choice {
        0 => assert_eq!(res, Err(ScopeViolationKind::EmptyDiff)),
        1 => assert!(res.is_ok()),
        2 | 3 => assert_eq!(res, Err(ScopeViolationKind::ForbiddenPath { index: 0 })),
        4 => assert_eq!(res, Err(ScopeViolationKind::OutsideWriteScope { index: 0 })),
        5 => assert_eq!(res, Err(ScopeViolationKind::OutsideWriteScope { index: 1 })),
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

/// Law 3b: Symbolic prefix-boundary and traversal-escape containment proof for `within_bytes`.
#[cfg(kani)]
#[kani::proof]
#[kani::unwind(10)]
pub fn prove_within_prefix_boundary_laws() {
    let p_len: usize = kani::any();
    let pre_len: usize = kani::any();
    kani::assume(p_len <= 8);
    kani::assume(pre_len <= 6);
    let p_buf: [u8; 8] = kani::any();
    let pre_buf: [u8; 6] = kani::any();
    let path = &p_buf[..p_len];
    let prefix = &pre_buf[..pre_len];

    let inside = within_bytes(path, prefix);
    if prefix.is_empty() {
        assert!(!inside);
    }
    if inside {
        assert!(!prefix.is_empty());
        assert!(path.len() >= prefix.len());
        let mut i = 0;
        while i < prefix.len() {
            assert_eq!(path[i], prefix[i]);
            i += 1;
        }
        assert!(path.len() == prefix.len() || path[prefix.len()] == b'/');
    }

    // Prefix-confusion law: appending any non-'/' byte to a non-empty prefix is never within it.
    if !prefix.is_empty() && pre_len < 6 {
        let suffix: u8 = kani::any();
        kani::assume(suffix != b'/');
        let mut sibling = [0u8; 7];
        let mut j = 0;
        while j < pre_len {
            sibling[j] = prefix[j];
            j += 1;
        }
        sibling[pre_len] = suffix;
        assert!(!within_bytes(&sibling[..pre_len + 1], prefix));
    }

    // Directory-escape law: a validated RelPath contained in prefix cannot contain `..` segments.
    if let Ok(end) = validate_rel_path_bytes(path) {
        let clean = &path[..end];
        if within_bytes(clean, prefix) {
            let mut seg_start = 0;
            let mut idx = 0;
            while idx <= clean.len() {
                if idx == clean.len() || clean[idx] == b'/' {
                    assert!(&clean[seg_start..idx] != b"..");
                    seg_start = idx + 1;
                }
                idx += 1;
            }
        }
    }

    kani::cover!(
        inside && path.len() == prefix.len(),
        "within_exact_match_reachable"
    );
    kani::cover!(
        inside && path.len() > prefix.len(),
        "within_child_match_reachable"
    );
    kani::cover!(
        !inside && !prefix.is_empty() && path.len() > prefix.len() && path[0] == prefix[0],
        "within_sibling_prefix_rejected_reachable"
    );
    kani::cover!(
        !inside && prefix.is_empty(),
        "within_empty_prefix_rejected_reachable"
    );
}
