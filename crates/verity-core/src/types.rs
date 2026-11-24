use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeError {
    EmptyTaskId,
    InvalidTaskIdChar,
    InvalidCommitOid,
    InvalidSpecDigest,
    EmptyPath,
    AbsolutePath,
    BackslashPath,
    TraversalSegment,
    DotGitSegment,
    ReservedDeviceName,
    ControlCharInPath,
    EmptyWriteScope,
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeError::EmptyTaskId => write!(f, "task id cannot be empty"),
            TypeError::InvalidTaskIdChar => write!(f, "task id contains invalid character"),
            TypeError::InvalidCommitOid => {
                write!(f, "commit oid must be 40 or 64 lowercase hex chars")
            }
            TypeError::InvalidSpecDigest => {
                write!(f, "spec digest must be 64 lowercase hex chars")
            }
            TypeError::EmptyPath => write!(f, "relative path cannot be empty"),
            TypeError::AbsolutePath => write!(f, "path must be relative"),
            TypeError::BackslashPath => write!(f, "path must use forward slashes"),
            TypeError::TraversalSegment => write!(f, "path cannot contain '.' or '..' segments"),
            TypeError::DotGitSegment => write!(f, "path cannot reference '.git'"),
            TypeError::ReservedDeviceName => write!(f, "path segment uses a reserved device name"),
            TypeError::ControlCharInPath => write!(f, "path contains control characters"),
            TypeError::EmptyWriteScope => {
                write!(f, "write task requires at least one write scope path")
            }
        }
    }
}

impl std::error::Error for TypeError {}

/// Pure, allocation-free byte validator for `TaskId` (used by both runtime and Kani proofs).
/// Rejects empty/oversized IDs, `.`, `..`, any `..` traversal substring, and non-safe characters.
pub fn validate_task_id_bytes(bytes: &[u8]) -> Result<(), TypeError> {
    if bytes.is_empty() || bytes.len() > 128 {
        return Err(TypeError::EmptyTaskId);
    }
    if bytes == b"." || bytes == b".." {
        return Err(TypeError::TraversalSegment);
    }
    let mut prev_dot = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.') {
            return Err(TypeError::InvalidTaskIdChar);
        }
        if b == b'.' && prev_dot {
            return Err(TypeError::TraversalSegment);
        }
        prev_dot = b == b'.';
        i += 1;
    }
    Ok(())
}

/// Pure, allocation-free byte validator for 40-char SHA-1 or 64-char SHA-256 lowercase hex OIDs.
pub fn validate_commit_oid_bytes(bytes: &[u8]) -> Result<(), TypeError> {
    let len = bytes.len();
    if len != 40 && len != 64 {
        return Err(TypeError::InvalidCommitOid);
    }
    let mut i = 0;
    while i < len {
        if !matches!(bytes[i], b'0'..=b'9' | b'a'..=b'f') {
            return Err(TypeError::InvalidCommitOid);
        }
        i += 1;
    }
    Ok(())
}

/// Pure, allocation-free byte validator for 64-char SHA-256 lowercase hex spec digests.
pub fn validate_spec_digest_bytes(bytes: &[u8]) -> Result<(), TypeError> {
    if bytes.len() != 64 {
        return Err(TypeError::InvalidSpecDigest);
    }
    let mut i = 0;
    while i < 64 {
        if !matches!(bytes[i], b'0'..=b'9' | b'a'..=b'f') {
            return Err(TypeError::InvalidSpecDigest);
        }
        i += 1;
    }
    Ok(())
}

fn hex_nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        _ => 0,
    }
}

fn eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
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

fn is_windows_reserved_bytes(segment: &[u8]) -> bool {
    let mut stem_len = 0;
    while stem_len < segment.len() && segment[stem_len] != b'.' {
        stem_len += 1;
    }
    let stem = &segment[..stem_len];
    if stem.len() == 3 {
        eq_ignore_ascii_case(stem, b"CON")
            || eq_ignore_ascii_case(stem, b"PRN")
            || eq_ignore_ascii_case(stem, b"AUX")
            || eq_ignore_ascii_case(stem, b"NUL")
    } else if stem.len() == 4 {
        let digit = stem[3];
        if !matches!(digit, b'1'..=b'9') {
            return false;
        }
        eq_ignore_ascii_case(&stem[..3], b"COM") || eq_ignore_ascii_case(&stem[..3], b"LPT")
    } else {
        false
    }
}

/// Pure, allocation-free byte validator for `RelPath` (used by both runtime and Kani proofs).
/// Rejects empty paths, leading `/`, Windows drive prefixes (`C:`), backslashes `\`,
/// ASCII control chars (`< 0x20` or `0x7f`), empty/`.`/`..` segments, `.git`, and Windows reserved device names.
pub fn validate_rel_path_bytes(raw: &[u8]) -> Result<usize, TypeError> {
    let mut end = raw.len();
    while end > 0 && raw[end - 1] == b'/' {
        end -= 1;
    }
    if end == 0 {
        return Err(TypeError::EmptyPath);
    }
    let trimmed = &raw[..end];
    if trimmed[0] == b'/' {
        return Err(TypeError::AbsolutePath);
    }
    if trimmed.len() >= 2 && trimmed[0].is_ascii_alphabetic() && trimmed[1] == b':' {
        return Err(TypeError::AbsolutePath);
    }
    let mut i = 0;
    while i < trimmed.len() {
        let b = trimmed[i];
        if b == b'\\' {
            return Err(TypeError::BackslashPath);
        }
        if b < 0x20 || b == 0x7f {
            return Err(TypeError::ControlCharInPath);
        }
        i += 1;
    }

    let mut seg_start = 0;
    let mut idx = 0;
    while idx <= trimmed.len() {
        if idx == trimmed.len() || trimmed[idx] == b'/' {
            let segment = &trimmed[seg_start..idx];
            if segment.is_empty() || segment == b"." || segment == b".." {
                return Err(TypeError::TraversalSegment);
            }
            if eq_ignore_ascii_case(segment, b".git") {
                return Err(TypeError::DotGitSegment);
            }
            if is_windows_reserved_bytes(segment) {
                return Err(TypeError::ReservedDeviceName);
            }
            seg_start = idx + 1;
        }
        idx += 1;
    }

    Ok(end)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskId(String);

impl TaskId {
    pub fn parse(raw: &str) -> Result<Self, TypeError> {
        validate_task_id_bytes(raw.as_bytes())?;
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitOid(String);

impl CommitOid {
    pub fn parse(raw: &str) -> Result<Self, TypeError> {
        validate_commit_oid_bytes(raw.as_bytes())?;
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(String);

impl AsRef<str> for RelPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl RelPath {
    pub fn parse(raw: &str) -> Result<Self, TypeError> {
        let end = validate_rel_path_bytes(raw.as_bytes())?;
        Ok(Self(raw[..end].to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpecDigest([u8; 32]);

impl SpecDigest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn parse_hex(raw: &str) -> Result<Self, TypeError> {
        let bytes = raw.as_bytes();
        validate_spec_digest_bytes(bytes)?;
        let mut out = [0u8; 32];
        let mut i = 0;
        while i < 32 {
            out[i] = (hex_nibble(bytes[i * 2]) << 4) | hex_nibble(bytes[i * 2 + 1]);
            i += 1;
        }
        Ok(Self(out))
    }

    pub fn of_spec(spec_text: &str) -> Self {
        Self(sha256_bytes(spec_text.as_bytes()))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for &b in &self.0 {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifierKind {
    RustKani,
    Lean4Comparator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskMode {
    ReadOnly {
        read_scope: Vec<RelPath>,
    },
    Write {
        base: CommitOid,
        write_scope: Vec<RelPath>,
        forbidden: Vec<RelPath>,
        verifier: VerifierKind,
    },
}

impl TaskMode {
    pub fn read_only(read_scope: Vec<RelPath>) -> Self {
        Self::ReadOnly { read_scope }
    }

    pub fn write(
        base: CommitOid,
        write_scope: Vec<RelPath>,
        forbidden: Vec<RelPath>,
        verifier: VerifierKind,
    ) -> Result<Self, TypeError> {
        if write_scope.is_empty() {
            return Err(TypeError::EmptyWriteScope);
        }
        Ok(Self::Write {
            base,
            write_scope,
            forbidden,
            verifier,
        })
    }

    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::ReadOnly { .. })
    }

    pub fn needs_worktree(&self) -> bool {
        matches!(self, Self::Write { .. })
    }
}

/// Pure FIPS 180-4 SHA-256 implementation so verity-core stays zero-dependency.
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((data.len() + 9).div_ceil(64) * 64);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}
