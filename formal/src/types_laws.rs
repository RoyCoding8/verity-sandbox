//! Formal verification of Verity core branded types (`TaskId`, `CommitOid`, `RelPath`).

#[cfg(any(kani, test))]
use verity_core::types::{
    validate_commit_oid_bytes, validate_rel_path_bytes, validate_task_id_bytes, TypeError,
};

#[cfg(kani)]
#[kani::proof]
#[kani::unwind(42)]
pub fn prove_task_id_and_commit_oid_laws() {
    let len: usize = kani::any();
    kani::assume(len <= 8);
    let buf: [u8; 8] = kani::any();
    let slice = &buf[..len];

    let res = validate_task_id_bytes(slice);
    if res.is_ok() {
        assert!(!slice.is_empty());
        assert!(slice != b"." && slice != b"..");
        let mut i = 0;
        while i < slice.len() {
            let b = slice[i];
            assert!(b != b'/' && b != b'\\' && b != 0);
            if i + 1 < slice.len() {
                assert!(!(b == b'.' && slice[i + 1] == b'.'));
            }
            i += 1;
        }
    }

    kani::cover!(res.is_ok(), "task_id_valid_reachable");
    kani::cover!(
        res == Err(TypeError::TraversalSegment),
        "task_id_traversal_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::InvalidTaskIdChar),
        "task_id_invalid_char_reject_reachable"
    );

    // CommitOid length law: any slice shorter than 40 bytes must fail.
    assert!(validate_commit_oid_bytes(slice) == Err(TypeError::InvalidCommitOid));

    // Symbolic 40-byte CommitOid: only lowercase hex passes.
    let mut oid = [b'a'; 40];
    let idx: usize = kani::any();
    kani::assume(idx < 40);
    let ch: u8 = kani::any();
    oid[idx] = ch;
    let oid_res = validate_commit_oid_bytes(&oid);
    let is_lower_hex = matches!(ch, b'0'..=b'9' | b'a'..=b'f');
    assert_eq!(oid_res.is_ok(), is_lower_hex);
    kani::cover!(oid_res.is_ok(), "commit_oid_valid_reachable");
    kani::cover!(oid_res.is_err(), "commit_oid_invalid_char_reachable");
}

#[cfg(kani)]
#[kani::proof]
#[kani::unwind(10)]
pub fn prove_rel_path_traversal_and_windows_laws() {
    let len: usize = kani::any();
    kani::assume(len <= 8);
    let buf: [u8; 8] = kani::any();
    let slice = &buf[..len];

    let res = validate_rel_path_bytes(slice);
    if let Ok(end) = res {
        assert!(end > 0 && end <= slice.len());
        let trimmed = &slice[..end];
        // Never absolute Unix or Windows drive prefix
        assert!(trimmed[0] != b'/');
        if trimmed.len() >= 2 {
            assert!(!(trimmed[0].is_ascii_alphabetic() && trimmed[1] == b':'));
        }
        // Never contains backslash, NUL, or control chars
        let mut i = 0;
        while i < trimmed.len() {
            assert!(trimmed[i] != b'\\');
            assert!(trimmed[i] >= 0x20 && trimmed[i] != 0x7f);
            i += 1;
        }
        // Never contains empty segment (`//`)
        let mut j = 0;
        while j + 1 < trimmed.len() {
            assert!(!(trimmed[j] == b'/' && trimmed[j + 1] == b'/'));
            j += 1;
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
        res == Err(TypeError::BackslashPath),
        "rel_path_backslash_reject_reachable"
    );
    kani::cover!(
        res == Err(TypeError::DotGitSegment),
        "rel_path_dotgit_reject_reachable"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_and_rel_path_reject_traversal_and_windows_drives() {
        assert_eq!(validate_task_id_bytes(b".."), Err(TypeError::TraversalSegment));
        assert_eq!(validate_task_id_bytes(b"a..b"), Err(TypeError::TraversalSegment));
        assert_eq!(validate_task_id_bytes(b"a/b"), Err(TypeError::InvalidTaskIdChar));
        assert!(validate_task_id_bytes(b"task-1.v2").is_ok());

        assert!(validate_commit_oid_bytes(&[b'a'; 40]).is_ok());
        assert_eq!(
            validate_commit_oid_bytes(&[b'A'; 40]),
            Err(TypeError::InvalidCommitOid)
        );
        assert_eq!(
            validate_commit_oid_bytes(b"abc"),
            Err(TypeError::InvalidCommitOid)
        );

        assert_eq!(validate_rel_path_bytes(b"C:foo"), Err(TypeError::AbsolutePath));
        assert_eq!(
            validate_rel_path_bytes(b"c:/Windows/System32"),
            Err(TypeError::AbsolutePath)
        );
        assert_eq!(
            validate_rel_path_bytes(b"src\\lib.rs"),
            Err(TypeError::BackslashPath)
        );
        assert_eq!(
            validate_rel_path_bytes(b"src/../secret"),
            Err(TypeError::TraversalSegment)
        );
        assert_eq!(
            validate_rel_path_bytes(b"src/.git/config"),
            Err(TypeError::DotGitSegment)
        );
        assert_eq!(
            validate_rel_path_bytes(b"src/CON.txt"),
            Err(TypeError::ReservedDeviceName)
        );
        assert_eq!(validate_rel_path_bytes(b"src/lib.rs"), Ok(10));
    }
}
