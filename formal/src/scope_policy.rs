use verity_core::scope::within_bytes;

fn under_prefix(path: &[u8], prefix: &[u8]) -> bool {
    !prefix.is_empty() && matches!(path.strip_prefix(prefix), Some([] | [b'/', ..]))
}

/// Exact byte-prefix containment for path lengths up to eight and prefix lengths up to six.
#[kani::proof]
#[kani::unwind(10)]
pub fn prove_within_prefix_boundary_laws() {
    let path_len: usize = kani::any();
    let prefix_len: usize = kani::any();
    kani::assume(path_len <= 8 && prefix_len <= 6);
    let path_buf: [u8; 8] = kani::any();
    let prefix_buf: [u8; 6] = kani::any();
    let path = &path_buf[..path_len];
    let prefix = &prefix_buf[..prefix_len];

    let inside = within_bytes(path, prefix);
    assert_eq!(inside, under_prefix(path, prefix));

    kani::cover!(inside && path_len == prefix_len, "within_exact_match");
    kani::cover!(inside && path_len > prefix_len, "within_child_match");
    kani::cover!(
        !inside && !prefix.is_empty() && path_len > prefix_len && path.starts_with(prefix),
        "within_sibling_prefix_rejected"
    );
    kani::cover!(!inside && prefix.is_empty(), "within_empty_prefix_rejected");
}
