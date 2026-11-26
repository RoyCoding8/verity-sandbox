//! Formal verification of Verity core branded types (`TaskId`, `CommitOid`, `RelPath`, `SpecDigest`).

#[cfg(kani)]
use verity_core::types::{
    validate_commit_oid_bytes, validate_rel_path_bytes, validate_spec_digest_bytes,
    validate_task_id_bytes, TypeError,
};

#[cfg(kani)]
fn spec_eq_ascii_ci(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if !a[i].eq_ignore_ascii_case(&b[i]) {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(kani)]
fn spec_is_windows_reserved(seg: &[u8]) -> bool {
    let mut stem_len = 0;
    while stem_len < seg.len() && seg[stem_len] != b'.' {
        stem_len += 1;
    }
    while stem_len > 0 && seg[stem_len - 1] == b' ' {
        stem_len -= 1;
    }
    let stem = &seg[..stem_len];
    if stem.len() == 3 {
        spec_eq_ascii_ci(stem, b"CON")
            || spec_eq_ascii_ci(stem, b"PRN")
            || spec_eq_ascii_ci(stem, b"AUX")
            || spec_eq_ascii_ci(stem, b"NUL")
    } else if stem.len() == 4 || stem.len() == 5 {
        matches!(&stem[3..], [b'1'..=b'9'] | [0xc2, 0xb2 | 0xb3 | 0xb9])
            && (spec_eq_ascii_ci(&stem[..3], b"COM") || spec_eq_ascii_ci(&stem[..3], b"LPT"))
    } else if matches!(stem.len(), 6 | 7) {
        spec_eq_ascii_ci(stem, b"CONIN$") || spec_eq_ascii_ci(stem, b"CONOUT$")
    } else {
        false
    }
}

#[cfg(kani)]
#[kani::proof]
#[kani::unwind(66)]
pub fn prove_task_id_and_hex_digest_laws() {
    let len: usize = kani::any();
    kani::assume(len <= 8);
    let buf: [u8; 8] = kani::any();
    let slice = &buf[..len];

    let res = validate_task_id_bytes(slice);
    if res.is_ok() {
        assert!(!slice.is_empty());
        assert!(slice != b"." && slice != b"..");
        assert!(slice[slice.len() - 1] != b'.');
        assert!(!spec_eq_ascii_ci(slice, b".git"));
        assert!(!spec_is_windows_reserved(slice));
        let mut i = 0;
        while i < slice.len() {
            let b = slice[i];
            assert!(b != b'/' && b != b'\\' && b != 0);
            assert!(b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
            if i + 1 < slice.len() {
                assert!(!(b == b'.' && slice[i + 1] == b'.'));
            }
            i += 1;
        }
    }

    if spec_is_windows_reserved(slice) {
        assert!(res.is_err());
    }

    // Oversized TaskId (> 128 bytes) must always be rejected without panic.
    let oversized = [b'a'; 129];
    assert_eq!(
        validate_task_id_bytes(&oversized),
        Err(TypeError::EmptyTaskId)
    );

    kani::cover!(res.is_ok(), "task_id_valid_reachable");
    kani::cover!(
        res == Err(TypeError::EmptyTaskId),
        "task_id_empty_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::TraversalSegment),
        "task_id_traversal_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::InvalidTaskIdChar),
        "task_id_invalid_char_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::ReservedDeviceName),
        "task_id_reserved_device_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::DotGitSegment),
        "task_id_dotgit_reject_reachable"
    );

    // CommitOid & SpecDigest length laws: any short slice (<= 8 bytes) must fail.
    assert_eq!(
        validate_commit_oid_bytes(slice),
        Err(TypeError::InvalidCommitOid)
    );
    assert_eq!(
        validate_spec_digest_bytes(slice),
        Err(TypeError::InvalidSpecDigest)
    );

    // Symbolic 40-byte CommitOid: only lowercase hex passes; 40-byte SpecDigest always fails.
    let mut oid40 = [b'a'; 40];
    let idx40: usize = kani::any();
    kani::assume(idx40 < 40);
    let ch40: u8 = kani::any();
    oid40[idx40] = ch40;
    let oid40_res = validate_commit_oid_bytes(&oid40);
    let is_lower_hex_40 = matches!(ch40, b'0'..=b'9' | b'a'..=b'f');
    assert_eq!(oid40_res.is_ok(), is_lower_hex_40);
    assert_eq!(
        validate_spec_digest_bytes(&oid40),
        Err(TypeError::InvalidSpecDigest)
    );

    // Symbolic 64-byte CommitOid & SpecDigest: only lowercase hex passes for both.
    let mut hex64 = [b'0'; 64];
    let idx64: usize = kani::any();
    kani::assume(idx64 < 64);
    let ch64: u8 = kani::any();
    hex64[idx64] = ch64;
    let is_lower_hex_64 = matches!(ch64, b'0'..=b'9' | b'a'..=b'f');
    let oid64_res = validate_commit_oid_bytes(&hex64);
    let digest_res = validate_spec_digest_bytes(&hex64);
    assert_eq!(oid64_res.is_ok(), is_lower_hex_64);
    assert_eq!(digest_res.is_ok(), is_lower_hex_64);

    // Wrong-length hex slices (39, 41, 63, 65) are rejected.
    let hex65 = [b'f'; 65];
    assert_eq!(
        validate_commit_oid_bytes(&hex65[..39]),
        Err(TypeError::InvalidCommitOid)
    );
    assert_eq!(
        validate_commit_oid_bytes(&hex65[..41]),
        Err(TypeError::InvalidCommitOid)
    );
    assert_eq!(
        validate_commit_oid_bytes(&hex65[..63]),
        Err(TypeError::InvalidCommitOid)
    );
    assert_eq!(
        validate_commit_oid_bytes(&hex65),
        Err(TypeError::InvalidCommitOid)
    );
    assert_eq!(
        validate_spec_digest_bytes(&hex65[..63]),
        Err(TypeError::InvalidSpecDigest)
    );
    assert_eq!(
        validate_spec_digest_bytes(&hex65),
        Err(TypeError::InvalidSpecDigest)
    );

    kani::cover!(oid40_res.is_ok(), "commit_oid_40_valid_reachable");
    kani::cover!(oid64_res.is_ok(), "commit_oid_64_valid_reachable");
    kani::cover!(oid40_res.is_err(), "commit_oid_invalid_char_reachable");
    kani::cover!(digest_res.is_ok(), "spec_digest_valid_reachable");
    kani::cover!(digest_res.is_err(), "spec_digest_invalid_char_reachable");
}

#[cfg(kani)]
#[kani::proof]
#[kani::unwind(11)]
pub fn prove_rel_path_traversal_and_windows_laws() {
    let len: usize = kani::any();
    kani::assume(len <= 8);
    let buf: [u8; 8] = kani::any();
    let slice = &buf[..len];

    let res = validate_rel_path_bytes(slice);
    if !slice.is_empty() && slice[0] == b'/' {
        assert_eq!(res, Err(TypeError::AbsolutePath));
    }

    if let Ok(end) = res {
        assert!(end > 0 && end <= slice.len());
        let trimmed = &slice[..end];
        // Never absolute Unix, trailing slash, or Windows drive prefix
        assert!(trimmed[0] != b'/');
        assert!(trimmed[end - 1] != b'/');
        if trimmed.len() >= 2 {
            assert!(!(trimmed[0].is_ascii_alphabetic() && trimmed[1] == b':'));
        }
        // Never contains backslash, NUL, colon, or control chars
        let mut i = 0;
        while i < trimmed.len() {
            assert!(trimmed[i] != b'\\');
            assert!(trimmed[i] != b':');
            assert!(trimmed[i] != 0);
            assert!(trimmed[i] >= 0x20 && trimmed[i] != 0x7f);
            i += 1;
        }
        // One arbitrary segment represents all segments without unrolling a validator per segment.
        let start: usize = kani::any();
        let stop: usize = kani::any();
        if start <= stop
            && stop <= end
            && (start == 0 || trimmed[start - 1] == b'/')
            && (stop == end || trimmed[stop] == b'/')
        {
            let seg = &trimmed[start..stop];
            if seg.iter().all(|&byte| byte != b'/') {
                assert!(!seg.is_empty());
                assert!(seg != b".");
                assert!(seg != b"..");
                assert!(seg[seg.len() - 1] != b'.' && seg[seg.len() - 1] != b' ');
                assert!(!spec_eq_ascii_ci(seg, b".git"));
                assert!(!spec_is_windows_reserved(seg));
                kani::cover!(seg == b"src", "rel_path_arbitrary_segment_reachable");
            }
        }
    }

    kani::cover!(
        res.is_ok() && slice.len() >= 3 && slice[1] == b'/',
        "rel_path_nested_valid_reachable"
    );
    kani::cover!(
        res == Err(TypeError::TraversalSegment),
        "rel_path_traversal_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::AbsolutePath) && slice.len() >= 2 && slice[1] == b':',
        "rel_path_windows_drive_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::AbsolutePath) && !slice.is_empty() && slice[0] == b'/',
        "rel_path_unix_abs_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::AbsolutePath) && slice == b"/",
        "rel_path_root_slash_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::BackslashPath),
        "rel_path_backslash_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::ControlCharInPath),
        "rel_path_control_or_nul_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::DotGitSegment),
        "rel_path_dotgit_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::ReservedDeviceName),
        "rel_path_reserved_device_reject_reachable"
    );
}
