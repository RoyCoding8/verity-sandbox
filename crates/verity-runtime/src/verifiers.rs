use crate::sandbox::{default_offline_config, run_sandboxed, SandboxConfig, SandboxError};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{env, fs};
use verity_core::json::{parse_json, JsonValue};
use verity_core::{
    classify_kani_report, CheckKind, CheckStatus, CheckerVerdict, HygieneReport, HygieneViolation,
    KaniCheck, KaniReport, RelPath, VerdictStatus,
};

/// A single source line split into executable code (with comments and string
/// literals replaced by whitespace) and extracted comment text.
struct ScannedLine {
    code: String,
    comments: String,
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn starts_with_ident(bytes: &[u8], pos: usize, kw: &[u8]) -> bool {
    if pos > 0 && is_ident_byte(bytes[pos - 1]) {
        return false;
    }
    if bytes.get(pos..pos + kw.len()) != Some(kw) {
        return false;
    }
    bytes.get(pos + kw.len()).is_none_or(|&b| !is_ident_byte(b))
}

/// Checks whether `bytes[pos..]` starts a Rust raw string literal (`r"..."`,
/// `r#"..."#`, `br#"..."#`, `cr#"..."#`) and returns `(content_start, hash_count)`.
fn match_rust_raw_string_start(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
    if pos > 0 && is_ident_byte(bytes[pos - 1]) {
        return None;
    }
    let mut i = pos;
    if matches!(bytes.get(i), Some(b'b' | b'c')) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'r') {
        return None;
    }
    i += 1;
    let mut hashes = 0usize;
    while bytes.get(i) == Some(&b'#') {
        hashes += 1;
        i += 1;
    }
    if bytes.get(i) == Some(&b'"') {
        Some((i + 1, hashes))
    } else {
        None
    }
}

/// Returns the byte index immediately after a raw string literal whose content
/// starts at `content_start` with `hashes` `#` delimiters.
fn find_rust_raw_string_end(bytes: &[u8], content_start: usize, hashes: usize) -> usize {
    let mut i = content_start;
    let len = bytes.len();
    while i < len {
        if bytes[i] == b'"' {
            let mut matched = 0usize;
            while matched < hashes && bytes.get(i + 1 + matched) == Some(&b'#') {
                matched += 1;
            }
            if matched == hashes {
                return i + 1 + hashes;
            }
        }
        i += 1;
    }
    len
}

/// Returns the byte index immediately after a normal `"..."` Rust string literal
/// starting at `pos` (where `bytes[pos] == b'"'`).
fn find_rust_string_end(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos + 1;
    let len = bytes.len();
    while i < len {
        if bytes[i] == b'\\' {
            i = (i + 2).min(len);
        } else if bytes[i] == b'"' {
            return i + 1;
        } else {
            i += 1;
        }
    }
    len
}

/// Checks whether `bytes[pos..]` starts a Rust char literal (as opposed to a
/// lifetime like `'a` or `'static`) and returns the byte index right after the
/// closing `'`.
fn match_rust_char_literal(bytes: &[u8], pos: usize) -> Option<usize> {
    if bytes.get(pos) != Some(&b'\'') {
        return None;
    }
    let next = *bytes.get(pos + 1)?;
    if next == b'\'' || next == b'\n' || next == b'\r' {
        return None;
    }
    if next == b'\\' {
        let mut i = pos + 2;
        if i >= bytes.len() {
            return None;
        }
        // Handle single-char escapes, \xNN, and \u{...} up to closing '\''.
        let limit = (pos + 14).min(bytes.len());
        i += 1;
        while i < limit {
            if bytes[i] == b'\'' {
                return Some(i + 1);
            }
            if bytes[i] == b'\n' || bytes[i] == b'\r' {
                return None;
            }
            i += 1;
        }
        return None;
    }
    // Decode one UTF-8 scalar after '\'' and require an immediate closing '\''.
    let rest = std::str::from_utf8(&bytes[pos + 1..]).ok()?;
    let ch = rest.chars().next()?;
    let after_ch = pos + 1 + ch.len_utf8();
    if bytes.get(after_ch) == Some(&b'\'') {
        Some(after_ch + 1)
    } else {
        None
    }
}

/// Strips Rust line comments (`//`) and nested block comments (`/* ... */`)
/// while preserving string and char literals intact.
fn strip_rust_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut out = String::with_capacity(len);

    while i < len {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(' ');
            continue;
        }

        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            let mut depth = 1usize;
            out.push(' ');
            while i < len && depth > 0 {
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                } else if bytes[i] == b'\n' {
                    out.push('\n');
                    i += 1;
                } else {
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }

        if let Some((content_start, hashes)) = match_rust_raw_string_start(bytes, i) {
            let end = find_rust_raw_string_end(bytes, content_start, hashes);
            out.push_str(&source[i..end]);
            i = end;
            continue;
        }

        if bytes[i] == b'"' {
            let end = find_rust_string_end(bytes, i);
            out.push_str(&source[i..end]);
            i = end;
            continue;
        }

        if let Some(after_char) = match_rust_char_literal(bytes, i) {
            out.push_str(&source[i..after_char]);
            i = after_char;
            continue;
        }

        let ch = source[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

/// Parses a Rust string literal (`"..."` or `r#"..."#`) at `pos` and returns
/// `(unescaped_value, next_index)`.
fn parse_rust_string_literal(source: &str, pos: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    if let Some((content_start, hashes)) = match_rust_raw_string_start(bytes, pos) {
        let end = find_rust_raw_string_end(bytes, content_start, hashes);
        let content_end = end.saturating_sub(1 + hashes);
        if content_end >= content_start && end <= bytes.len() {
            return Some((source[content_start..content_end].to_string(), end));
        }
        return None;
    }

    if bytes.get(pos) != Some(&b'"') {
        return None;
    }
    let mut i = pos + 1;
    let len = bytes.len();
    let mut out = String::new();
    while i < len {
        if bytes[i] == b'\\' {
            i += 1;
            if i >= len {
                return None;
            }
            match bytes[i] {
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'\\' => out.push('\\'),
                b'"' => out.push('"'),
                b'\'' => out.push('\''),
                other => out.push(other as char),
            }
            i += 1;
        } else if bytes[i] == b'"' {
            return Some((out, i + 1));
        } else {
            let ch = source[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    None
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Scans `kani::cover!(...)` starting at the opening delimiter `(`, `[`, or `{`
/// and returns `(optional_label, index_after_closing_delim)`.
fn extract_cover_label(source: &str, open_pos: usize) -> (Option<String>, usize) {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = open_pos + 1;
    let mut depth = 1usize;
    let mut after_first_comma: Option<usize> = None;

    while i < len && depth > 0 {
        if let Some((content_start, hashes)) = match_rust_raw_string_start(bytes, i) {
            i = find_rust_raw_string_end(bytes, content_start, hashes);
            continue;
        }
        if bytes[i] == b'"' {
            i = find_rust_string_end(bytes, i);
            continue;
        }
        if let Some(after_char) = match_rust_char_literal(bytes, i) {
            i = after_char;
            continue;
        }

        match bytes[i] {
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' | b'}' => {
                depth -= 1;
                i += 1;
            }
            b',' if depth == 1 && after_first_comma.is_none() => {
                after_first_comma = Some(i + 1);
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let label = after_first_comma.and_then(|comma_end| {
        let lit_start = skip_ws(bytes, comma_end);
        parse_rust_string_literal(source, lit_start).map(|(s, _)| s)
    });
    (label, i)
}

#[derive(Debug)]
pub(crate) struct KaniTarget {
    pub(crate) name: String,
    source: RelPath,
    line: usize,
    probes: Vec<String>,
}

/// Finds the first proof and its labeled reachability probes.
pub fn discover_kani_targets(files: &[(RelPath, String)]) -> (Option<String>, Vec<String>) {
    discover_kani_proofs(files).into_iter().next().map_or_else(
        || (None, Vec::new()),
        |target| (Some(target.name), target.probes),
    )
}

fn is_kani_proof_attr(compact: &str) -> bool {
    if compact == "kani::proof" {
        return true;
    }
    if let Some(inner) = compact
        .strip_prefix("cfg_attr(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        if let Some((pred, attrs)) = inner.split_once(',') {
            if pred == "kani" {
                return attrs.split(',').any(|attr| attr == "kani::proof");
            }
        }
    }
    false
}

pub(crate) fn discover_kani_proofs(files: &[(RelPath, String)]) -> Vec<KaniTarget> {
    let mut targets: Vec<KaniTarget> = Vec::new();

    for (rel, source) in files {
        if !rel.as_str().ends_with(".rs") {
            continue;
        }
        let stripped = strip_rust_comments(source);
        let bytes = stripped.as_bytes();
        let len = bytes.len();
        let mut i = 0usize;
        let mut pending_proof = false;
        let mut active_proof = None;
        let mut body_depth = 0usize;

        while i < len {
            // Skip literals so strings containing `#[kani::proof]` or `kani::cover!` never match.
            if let Some((content_start, hashes)) = match_rust_raw_string_start(bytes, i) {
                i = find_rust_raw_string_end(bytes, content_start, hashes);
                continue;
            }
            if bytes[i] == b'"' {
                i = find_rust_string_end(bytes, i);
                continue;
            }
            if let Some(after_char) = match_rust_char_literal(bytes, i) {
                i = after_char;
                continue;
            }

            // Detect outer attribute `#[...]` such as `#[kani::proof]`.
            if bytes[i] == b'#' && bytes.get(i + 1) == Some(&b'[') {
                let attr_start = i + 2;
                let mut j = attr_start;
                let mut depth = 1usize;
                while j < len && depth > 0 {
                    if let Some((cs, h)) = match_rust_raw_string_start(bytes, j) {
                        j = find_rust_raw_string_end(bytes, cs, h);
                        continue;
                    }
                    if bytes[j] == b'"' {
                        j = find_rust_string_end(bytes, j);
                        continue;
                    }
                    if let Some(ac) = match_rust_char_literal(bytes, j) {
                        j = ac;
                        continue;
                    }
                    match bytes[j] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                let attr_end = if depth == 0 { j - 1 } else { j };
                let compact: String = stripped[attr_start..attr_end]
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                if is_kani_proof_attr(&compact) && active_proof.is_none() {
                    pending_proof = true;
                }
                i = j;
                continue;
            }

            if pending_proof {
                if bytes[i] == b';' || bytes[i] == b'{' {
                    pending_proof = false;
                    i += 1;
                    continue;
                }
                if starts_with_ident(bytes, i, b"fn") {
                    let name_start = skip_ws(bytes, i + 2);
                    let mut name_end = name_start;
                    while name_end < len && is_ident_byte(bytes[name_end]) {
                        name_end += 1;
                    }
                    if name_end > name_start {
                        active_proof = Some(targets.len());
                        targets.push(KaniTarget {
                            name: stripped[name_start..name_end].to_string(),
                            source: rel.clone(),
                            line: 1 + bytes[..i].iter().filter(|&&b| b == b'\n').count(),
                            probes: Vec::new(),
                        });
                        pending_proof = false;
                        i = name_end;
                        continue;
                    }
                }
            }

            if let Some(target) = active_proof {
                match bytes[i] {
                    b'{' => body_depth += 1,
                    b'}' if body_depth > 0 => {
                        body_depth -= 1;
                        if body_depth == 0 {
                            active_proof = None;
                        }
                    }
                    b';' if body_depth == 0 => active_proof = None,
                    _ => {}
                }

                if body_depth > 0 && starts_with_ident(bytes, i, b"kani") {
                    let mut j = skip_ws(bytes, i + 4);
                    if bytes.get(j..j + 2) == Some(b"::") {
                        j = skip_ws(bytes, j + 2);
                        if starts_with_ident(bytes, j, b"cover") {
                            j = skip_ws(bytes, j + 5);
                            if bytes.get(j) == Some(&b'!') {
                                j = skip_ws(bytes, j + 1);
                                if matches!(bytes.get(j), Some(b'(' | b'[' | b'{')) {
                                    let (label, next_i) = extract_cover_label(&stripped, j);
                                    if let Some(desc) = label.filter(|label| !label.is_empty()) {
                                        targets[target].probes.push(desc);
                                    }
                                    i = next_i;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }

            let ch = stripped[i..].chars().next().unwrap();
            i += ch.len_utf8();
        }
    }

    targets
}

/// Strips Rust line comments (`//`), nested block comments (`/* ... */`),
/// string literals (`"..."`, `b"..."`, `c"..."`, `r#"..."#`), and char literals
/// while preserving 1-to-1 line numbering and retaining comment text per line.
fn scan_rust_lines(source: &str) -> Vec<ScannedLine> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut lines: Vec<ScannedLine> = Vec::new();
    let mut code = String::new();
    let mut comments = String::new();

    let flush_line = |lines: &mut Vec<ScannedLine>, code: &mut String, comments: &mut String| {
        lines.push(ScannedLine {
            code: std::mem::take(code),
            comments: std::mem::take(comments),
        });
    };

    while i < len {
        // Line comment: // ... \n
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            comments.push_str(&source[start..i]);
            code.push(' ');
            continue;
        }

        // Nested block comment: /* ... */
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            let mut depth = 1usize;
            code.push(' ');
            while i < len && depth > 0 {
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                } else if bytes[i] == b'\n' {
                    flush_line(&mut lines, &mut code, &mut comments);
                    i += 1;
                } else {
                    let ch = source[i..].chars().next().unwrap();
                    comments.push(ch);
                    i += ch.len_utf8();
                }
            }
            code.push(' ');
            continue;
        }

        // Raw string literal: r"...", r#"..."#, br#"..."#, cr#"..."#
        if let Some((content_start, hashes)) = match_rust_raw_string_start(bytes, i) {
            i = content_start;
            code.push(' ');
            while i < len {
                if bytes[i] == b'"' {
                    let mut matched = 0usize;
                    while matched < hashes && bytes.get(i + 1 + matched) == Some(&b'#') {
                        matched += 1;
                    }
                    if matched == hashes {
                        i += 1 + hashes;
                        break;
                    }
                }
                if bytes[i] == b'\n' {
                    flush_line(&mut lines, &mut code, &mut comments);
                }
                i += 1;
            }
            code.push(' ');
            continue;
        }

        // Normal string literal: "..."
        if bytes[i] == b'"' {
            i += 1;
            code.push(' ');
            while i < len {
                if bytes[i] == b'\\' {
                    i += 1;
                    if i < len {
                        if bytes[i] == b'\n' {
                            flush_line(&mut lines, &mut code, &mut comments);
                        }
                        i += 1;
                    }
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else if bytes[i] == b'\n' {
                    flush_line(&mut lines, &mut code, &mut comments);
                    i += 1;
                } else {
                    i += 1;
                }
            }
            code.push(' ');
            continue;
        }

        // Char literal: 'x' or '\n' (without misclassifying lifetimes like 'a)
        if let Some(after_char) = match_rust_char_literal(bytes, i) {
            i = after_char;
            code.push(' ');
            continue;
        }

        if bytes[i] == b'\n' {
            flush_line(&mut lines, &mut code, &mut comments);
            i += 1;
            continue;
        }

        let ch = source[i..].chars().next().unwrap();
        code.push(ch);
        i += ch.len_utf8();
    }

    if !code.is_empty() || !comments.is_empty() || lines.is_empty() {
        flush_line(&mut lines, &mut code, &mut comments);
    }
    lines
}

/// Strips Lean 4 line comments (`--`), nested block comments (`/- ... -/`),
/// and string literals (`"..."`) while preserving 1-to-1 line numbering.
fn scan_lean_lines(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut lines: Vec<String> = Vec::new();
    let mut code = String::new();

    while i < len {
        // Line comment: -- ... \n
        if bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            code.push(' ');
            continue;
        }

        // Nested block comment: /- ... -/
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            let mut depth = 1usize;
            code.push(' ');
            while i < len && depth > 0 {
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'-') {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                } else if bytes[i] == b'\n' {
                    lines.push(std::mem::take(&mut code));
                    i += 1;
                } else {
                    i += 1;
                }
            }
            code.push(' ');
            continue;
        }

        // String literal: "..."
        if bytes[i] == b'"' {
            i += 1;
            code.push(' ');
            while i < len {
                if bytes[i] == b'\\' {
                    i += 1;
                    if i < len {
                        if bytes[i] == b'\n' {
                            lines.push(std::mem::take(&mut code));
                        }
                        i += 1;
                    }
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else if bytes[i] == b'\n' {
                    lines.push(std::mem::take(&mut code));
                    i += 1;
                } else {
                    i += 1;
                }
            }
            code.push(' ');
            continue;
        }

        if bytes[i] == b'\n' {
            lines.push(std::mem::take(&mut code));
            i += 1;
            continue;
        }

        let ch = source[i..].chars().next().unwrap();
        code.push(ch);
        i += ch.len_utf8();
    }

    if !code.is_empty() || lines.is_empty() {
        lines.push(code);
    }
    lines
}

/// Collapses runs of horizontal whitespace to a single space so split tokens
/// like `partial /- c -/ def` cannot evade multi-word checks.
fn collapse_spaces(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut prev_ws = false;
    for ch in line.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out
}

/// Layer 1 Rust/Kani escape-hatch lint. Rejects stubbing, should_panic,
/// disabled unwinding assertions, and unannotated `kani::assume` calls using a
/// comment- and string-aware scanner so comments never trigger false positives
/// and block comments cannot smuggle banned constructs.
fn normalize_rust_path_spaces(line: &str) -> String {
    collapse_spaces(line)
        .replace(" :: ", "::")
        .replace(" ::", "::")
        .replace(":: ", "::")
}

fn contains_call(code: &str, target: &str, allow_path_prefix: bool) -> bool {
    let bytes = code.as_bytes();
    let target_bytes = target.as_bytes();
    if target_bytes.is_empty() || bytes.len() < target_bytes.len() {
        return false;
    }
    let mut i = 0;
    while i + target_bytes.len() <= bytes.len() {
        if &bytes[i..i + target_bytes.len()] == target_bytes {
            let prev_ok = i == 0
                || (!is_ident_byte(bytes[i - 1])
                    && (allow_path_prefix || (bytes[i - 1] != b':' && bytes[i - 1] != b'.')));
            if prev_ok {
                let after = skip_ws(bytes, i + target_bytes.len());
                if bytes.get(after) == Some(&b'(') {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

fn imports_kani_assume(scanned_lines: &[ScannedLine]) -> bool {
    let mut joined = String::new();
    for line in scanned_lines {
        joined.push_str(&line.code);
        joined.push(' ');
    }
    let normalized = normalize_rust_path_spaces(&joined);
    let bytes = normalized.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if starts_with_ident(bytes, i, b"use") {
            let start = i + 3;
            let end = normalized[start..]
                .find(';')
                .map(|off| start + off)
                .unwrap_or(bytes.len());
            let stmt = &normalized[start..end];
            let stmt_bytes = stmt.as_bytes();
            for pos in 0..stmt_bytes.len() {
                if stmt[pos..].starts_with("kani::assume")
                    && (pos == 0 || !is_ident_byte(stmt_bytes[pos - 1]))
                    && stmt_bytes
                        .get(pos + "kani::assume".len())
                        .is_none_or(|&b| !is_ident_byte(b))
                {
                    return true;
                }
                if stmt[pos..].starts_with("kani::*")
                    && (pos == 0 || !is_ident_byte(stmt_bytes[pos - 1]))
                {
                    return true;
                }
                if stmt[pos..].starts_with("kani::{")
                    && (pos == 0 || !is_ident_byte(stmt_bytes[pos - 1]))
                {
                    let rest = &stmt[pos + "kani::{".len()..];
                    if let Some(close) = rest.find('}') {
                        let items = &rest[..close];
                        if items.split(',').any(|item| {
                            let item = item.trim();
                            item == "assume"
                                || item == "*"
                                || item.starts_with("assume ")
                                || item == "self::assume"
                        }) {
                            return true;
                        }
                    }
                }
            }
            i = end;
        }
        i += 1;
    }
    false
}

pub fn hygiene_lint_rust(rel_path: &str, source: &str) -> HygieneReport {
    let mut violations = Vec::new();
    let scanned_lines = scan_rust_lines(source);
    let has_kani_assume_import = imports_kani_assume(&scanned_lines);
    for (idx, scanned) in scanned_lines.into_iter().enumerate() {
        let code = normalize_rust_path_spaces(&scanned.code);
        let line_no = idx + 1;
        if code.contains("kani::stub") {
            violations.push(HygieneViolation {
                rule: "no-kani-stub".to_string(),
                path: rel_path.to_string(),
                line: line_no,
                detail: "kani::stub mocks out real implementation logic".to_string(),
            });
        }
        if code.contains("kani::should_panic") {
            violations.push(HygieneViolation {
                rule: "no-should-panic".to_string(),
                path: rel_path.to_string(),
                line: line_no,
                detail: "kani::should_panic weakens proof obligations".to_string(),
            });
        }
        if code.contains("no-unwinding-assertions") {
            violations.push(HygieneViolation {
                rule: "require-unwinding-assertions".to_string(),
                path: rel_path.to_string(),
                line: line_no,
                detail: "disabling unwinding assertions allows vacuous loop bounds".to_string(),
            });
        }
        let has_assume_call = contains_call(&code, "kani::assume", true)
            || (has_kani_assume_import && contains_call(&code, "assume", false));
        if has_assume_call && !scanned.comments.contains("verity:trusted-assume") {
            violations.push(HygieneViolation {
                rule: "unreviewed-kani-assume".to_string(),
                path: rel_path.to_string(),
                line: line_no,
                detail:
                    "kani::assume must be marked with '// verity:trusted-assume' for spec review"
                        .to_string(),
            });
        }
    }
    HygieneReport { violations }
}

/// Layer 1 Lean 4 escape-hatch lint. Blocks sorry, custom axioms, and
/// compiler-divergence constructs (`partial`, `unsafe`, `@[implemented_by]`,
/// `@[extern]`, `@[csimp]`, `native_decide`) using a comment- and string-aware
/// scanner.
pub fn hygiene_lint_lean(rel_path: &str, source: &str) -> HygieneReport {
    let mut violations = Vec::new();
    let banned = [
        ("no-sorry", "sorry", "proof contains sorry"),
        (
            "no-custom-axiom",
            "axiom ",
            "custom axiom weakens logical soundness",
        ),
        (
            "no-partial",
            "partial def",
            "partial def allows compiled runtime divergence",
        ),
        ("no-unsafe", "unsafe ", "unsafe bypasses the Lean kernel"),
        (
            "no-implemented-by",
            "@[implemented_by",
            "@[implemented_by] diverges from proved definition",
        ),
        (
            "no-extern",
            "@[extern",
            "@[extern] diverges from proved definition",
        ),
        (
            "no-csimp",
            "@[csimp",
            "@[csimp] rewrites compiled code outside kernel",
        ),
        (
            "no-native-decide",
            "native_decide",
            "native_decide trusts the compiler",
        ),
    ];

    for (idx, raw_code) in scan_lean_lines(source).into_iter().enumerate() {
        let code = collapse_spaces(&raw_code);
        let line_no = idx + 1;
        for (rule, needle, detail) in banned {
            if code.contains(needle) {
                violations.push(HygieneViolation {
                    rule: rule.to_string(),
                    path: rel_path.to_string(),
                    line: line_no,
                    detail: detail.to_string(),
                });
            }
        }
    }
    HygieneReport { violations }
}

struct KaniExport {
    report: KaniReport,
    crate_name: String,
    source: String,
    start_line: usize,
    end_line: usize,
    succeeded: bool,
}

fn kani_text<'a>(object: &'a JsonValue, field: &str) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| format!("missing or invalid Kani field '{field}'"))
}

fn kani_bool(object: &JsonValue, field: &str) -> Result<bool, String> {
    object
        .get(field)
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| format!("missing or invalid Kani field '{field}'"))
}

fn kani_array<'a>(object: &'a JsonValue, field: &str) -> Result<&'a [JsonValue], String> {
    object
        .get(field)
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("missing or invalid Kani array '{field}'"))
}

fn kani_one<'a>(object: &'a JsonValue, field: &str) -> Result<&'a JsonValue, String> {
    match kani_array(object, field)? {
        [item] => Ok(item),
        _ => Err(format!("expected one item in Kani array '{field}'")),
    }
}

fn kani_line(source: &JsonValue, field: &str) -> Result<usize, String> {
    match source.get(field) {
        Some(JsonValue::Number(line))
            if *line >= 1.0 && *line < usize::MAX as f64 && line.fract() == 0.0 =>
        {
            Ok(*line as usize)
        }
        _ => Err(format!("missing or invalid Kani source line '{field}'")),
    }
}

/// Parses the single-harness export produced by Kani 0.68.0.
pub fn parse_kani_json(raw: &str) -> Result<KaniReport, String> {
    Ok(parse_kani_export(raw)?.report)
}

fn parse_kani_export(raw: &str) -> Result<KaniExport, String> {
    let root = parse_json(raw).map_err(|e| e.to_string())?;
    let metadata = root.get("metadata").ok_or("missing Kani export metadata")?;
    if kani_text(metadata, "version")? != "1.0" || kani_text(metadata, "kani_version")? != "0.68.0"
    {
        return Err("unsupported Kani export schema or version (expected Kani 0.68.0)".to_string());
    }

    let harness = kani_one(&root, "harness_metadata")?;
    let harness_name = kani_text(harness, "pretty_name")?;
    let crate_name = kani_text(harness, "crate_name")?;
    let attributes = harness
        .get("attributes")
        .ok_or("missing Kani harness attributes")?;
    if harness_name.is_empty()
        || crate_name.is_empty()
        || kani_text(attributes, "kind")? != "Proof"
        || kani_bool(harness, "is_automatically_generated")?
    {
        return Err("invalid Kani proof harness metadata".to_string());
    }
    let should_panic = kani_bool(attributes, "should_panic")?;
    let source = harness.get("source").ok_or("missing Kani harness source")?;
    let source_file = kani_text(source, "file")?;
    let start_line = kani_line(source, "start_line")?;
    let end_line = kani_line(source, "end_line")?;
    if source_file.is_empty() || start_line > end_line {
        return Err("invalid Kani harness source span".to_string());
    }

    let verification = root
        .get("verification_results")
        .ok_or("missing Kani verification results")?;
    let result = kani_one(verification, "results")?;
    let errors = kani_one(&root, "error_details")?;
    let properties = kani_one(&root, "property_details")?;
    if kani_text(result, "harness_id")? != harness_name
        || kani_text(errors, "harness_id")? != harness_name
        || kani_text(properties, "harness_id")? != harness_name
    {
        return Err("Kani export records disagree on harness identity".to_string());
    }
    let succeeded = match kani_text(result, "status")? {
        "Success" => true,
        "Failure" => false,
        other => return Err(format!("invalid Kani harness status '{other}'")),
    };
    let summary = verification
        .get("summary")
        .ok_or("missing Kani verification summary")?;
    for (field, count) in [
        ("total_harnesses", 1.0),
        ("executed", 1.0),
        ("successful", f64::from(succeeded)),
        ("failed", f64::from(!succeeded)),
    ] {
        if summary.get(field) != Some(&JsonValue::Number(count)) {
            return Err(format!("inconsistent Kani verification count '{field}'"));
        }
    }
    if kani_text(summary, "status")? != "completed" || kani_bool(errors, "has_errors")? == succeeded
    {
        return Err("incomplete or inconsistent Kani verification outcome".to_string());
    }

    let checks_array = kani_array(result, "checks")?;
    let property_count = properties
        .get("property_details")
        .and_then(|counts| counts.get("total_properties"));
    if property_count != Some(&JsonValue::Number(checks_array.len() as f64)) {
        return Err("Kani verification checks disagree with the property count".to_string());
    }
    let mut checks = Vec::with_capacity(checks_array.len());
    let mut solver_error = !succeeded
        && (kani_text(errors, "exit_status")? != "properties_failed"
            || !matches!(
                kani_text(errors, "error_type")?,
                "assertion_failure" | "verification_failure"
            ));
    for item in checks_array {
        let category = kani_text(item, "category")?;
        let kind = match category {
            "assertion" => CheckKind::Assertion,
            "cover" => CheckKind::Cover,
            _ => CheckKind::Safety,
        };
        let status = match kani_text(item, "status")? {
            "Success" => CheckStatus::Success,
            "Failure" if matches!(category, "unwind" | "recursion") => CheckStatus::Undetermined,
            "Failure" => CheckStatus::Failure,
            "Unreachable" => CheckStatus::Unreachable,
            "Satisfied" => CheckStatus::Satisfied,
            "Unsatisfiable" => CheckStatus::Unsatisfiable,
            "Undetermined" | "Unknown" | "Covered" | "Uncovered" => CheckStatus::Undetermined,
            "Error" => {
                solver_error = true;
                CheckStatus::Undetermined
            }
            other => return Err(format!("invalid Kani check status '{other}'")),
        };
        checks.push(KaniCheck {
            kind,
            description: kani_text(item, "description")?.to_string(),
            status,
        });
    }

    let has_failed_check = checks
        .iter()
        .any(|check| check.kind != CheckKind::Cover && check.status == CheckStatus::Failure);
    if succeeded && has_failed_check {
        return Err("Kani reported a successful harness with failed checks".to_string());
    }
    if !succeeded
        && !has_failed_check
        && !checks
            .iter()
            .any(|check| check.status == CheckStatus::Undetermined)
    {
        solver_error = true;
    }

    Ok(KaniExport {
        report: KaniReport {
            harness_name: harness_name.to_string(),
            should_panic,
            solver_error,
            checks,
        },
        crate_name: crate_name.to_string(),
        source: source_file.to_string(),
        start_line,
        end_line,
        succeeded,
    })
}

fn no_checker_evidence(status: VerdictStatus, reason: impl Into<String>) -> CheckerVerdict {
    CheckerVerdict {
        status,
        reason: reason.into(),
        checks_passed: 0,
        probes_satisfied: 0,
    }
}

#[cfg(test)]
fn classify_kani_export(
    raw: &str,
    exit_code: Option<i32>,
    worktree_dir: &Path,
    target: &KaniTarget,
) -> CheckerVerdict {
    classify_kani_export_with_crate(raw, exit_code, worktree_dir, target, Some(KANI_CRATE))
}

fn classify_kani_export_with_crate(
    raw: &str,
    exit_code: Option<i32>,
    worktree_dir: &Path,
    target: &KaniTarget,
    expected_crate: Option<&str>,
) -> CheckerVerdict {
    let export = match parse_kani_export(raw) {
        Ok(export) => export,
        Err(reason) => return no_checker_evidence(VerdictStatus::InvalidReport, reason),
    };
    if expected_crate.is_some_and(|expected| export.crate_name != expected)
        || export.report.harness_name.rsplit("::").next() != Some(target.name.as_str())
        || worktree_dir.join(&export.source) != worktree_dir.join(target.source.as_str())
        || !(export.start_line..=export.end_line).contains(&target.line)
    {
        return no_checker_evidence(
            VerdictStatus::InvalidReport,
            format!(
                "Kani report does not identify requested harness '{}' in {}",
                target.name,
                target.source.as_str()
            ),
        );
    }
    if exit_code != Some(i32::from(!export.succeeded)) {
        return no_checker_evidence(
            VerdictStatus::Inconclusive,
            format!("Kani exit {exit_code:?} disagrees with the verification report"),
        );
    }
    let expected_probes: Vec<&str> = target.probes.iter().map(String::as_str).collect();
    let verdict = classify_kani_report(&export.report, &expected_probes);
    if verdict.status == VerdictStatus::Pass
        && !export
            .report
            .checks
            .iter()
            .any(|check| check.kind == CheckKind::Assertion && check.status == CheckStatus::Success)
    {
        return no_checker_evidence(
            VerdictStatus::InvalidReport,
            "Kani did not report a successful assertion for the requested proof",
        );
    }
    verdict
}

const KANI_CRATE: &str = "verity_candidate";
static NEXT_KANI_RUN: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn run_kani_proof_sandboxed(
    worktree_dir: &Path,
    target: &KaniTarget,
) -> Result<CheckerVerdict, SandboxError> {
    let worktree_dir = canonical_path(worktree_dir)?;
    let program = configured_checker_path("VERITY_KANI", &worktree_dir)?;
    let source_path = worktree_dir.join(target.source.as_str());
    if canonical_path(&source_path)? != source_path || !source_path.is_file() {
        return Ok(no_checker_evidence(
            VerdictStatus::InvalidReport,
            "Kani proof source must be a regular file in the worktree",
        ));
    }

    let run_dir = worktree_dir.join(format!(
        ".verity-kani-{}-{}",
        std::process::id(),
        NEXT_KANI_RUN.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir(&run_dir).map_err(sandbox_io)?;
    let verdict = capture_kani_verdict(&program, &worktree_dir, &source_path, &run_dir, target);
    fs::remove_dir_all(&run_dir).map_err(sandbox_io)?;
    verdict
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KaniMode {
    Standalone,
    Cargo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KaniInvocation {
    mode: KaniMode,
    program: PathBuf,
    args: Vec<String>,
}

fn binary_name_matches(path: &Path, name: &str) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|base| {
            base.eq_ignore_ascii_case(name) || base.eq_ignore_ascii_case(&format!("{name}.exe"))
        })
}

fn build_kani_invocation(
    worktree_dir: &Path,
    source_path: &Path,
    run_dir: &Path,
    report_path: &Path,
    target: &KaniTarget,
    kani_program: &Path,
    cargo_kani_env: Option<&str>,
) -> Result<KaniInvocation, SandboxError> {
    let manifest_path = worktree_dir.join("Cargo.toml");
    let has_cargo_manifest = manifest_path.is_file();
    let in_src = target.source.as_str().starts_with("src/");
    let is_cargo_kani_bin = binary_name_matches(kani_program, "cargo-kani");
    let cargo_env_enabled = cargo_kani_env
        .is_some_and(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"));

    if has_cargo_manifest && in_src && (cargo_env_enabled || is_cargo_kani_bin) {
        let program = if let Some(raw_path) = cargo_kani_env.filter(|v| Path::new(v).is_absolute())
        {
            independent_checker_file(Path::new(raw_path), worktree_dir, "VERITY_CARGO_KANI")?
        } else if is_cargo_kani_bin || binary_name_matches(kani_program, "cargo") {
            kani_program.to_path_buf()
        } else if let Some(sibling) = kani_program
            .parent()
            .map(|dir| dir.join("cargo-kani"))
            .filter(|p| p.is_file())
        {
            sibling
        } else {
            PathBuf::from("cargo")
        };

        let mut args = Vec::new();
        if binary_name_matches(&program, "cargo") {
            args.push("kani".to_string());
        }
        args.extend([
            "--manifest-path".to_string(),
            path_arg(&manifest_path)?.to_string(),
            "-Z".to_string(),
            "unstable-options".to_string(),
            "--harness".to_string(),
            target.name.clone(),
            "--target-dir".to_string(),
            path_arg(run_dir)?.to_string(),
            "--export-json".to_string(),
            path_arg(report_path)?.to_string(),
        ]);
        return Ok(KaniInvocation {
            mode: KaniMode::Cargo,
            program,
            args,
        });
    }

    Ok(KaniInvocation {
        mode: KaniMode::Standalone,
        program: kani_program.to_path_buf(),
        args: vec![
            "-Z".to_string(),
            "unstable-options".to_string(),
            "--harness".to_string(),
            target.name.clone(),
            "--crate-name".to_string(),
            KANI_CRATE.to_string(),
            "--target-dir".to_string(),
            path_arg(run_dir)?.to_string(),
            "--export-json".to_string(),
            path_arg(report_path)?.to_string(),
            path_arg(source_path)?.to_string(),
        ],
    })
}

fn capture_kani_verdict(
    program: &Path,
    worktree_dir: &Path,
    source_path: &Path,
    run_dir: &Path,
    target: &KaniTarget,
) -> Result<CheckerVerdict, SandboxError> {
    let report_path = run_dir.join("report.json");
    let cargo_kani_env = env::var("VERITY_CARGO_KANI").ok();
    let invocation = build_kani_invocation(
        worktree_dir,
        source_path,
        run_dir,
        &report_path,
        target,
        program,
        cargo_kani_env.as_deref(),
    )?;
    let tool_anchor = if invocation.program.is_absolute() {
        invocation.program.as_path()
    } else {
        program
    };
    let cfg = checker_sandbox(worktree_dir, tool_anchor)?;
    let arg_refs: Vec<&str> = invocation.args.iter().map(String::as_str).collect();
    let out = run_sandboxed(&cfg, path_arg(&invocation.program)?, &arg_refs)?;
    let expected_crate = match invocation.mode {
        KaniMode::Standalone => Some(KANI_CRATE),
        KaniMode::Cargo => None,
    };
    match fs::symlink_metadata(&report_path) {
        Ok(metadata) if metadata.is_file() => {
            let raw = fs::read_to_string(&report_path).map_err(sandbox_io)?;
            Ok(classify_kani_export_with_crate(
                &raw,
                out.status.code(),
                worktree_dir,
                target,
                expected_crate,
            ))
        }
        Ok(_) => Ok(no_checker_evidence(
            VerdictStatus::InvalidReport,
            "Kani report is not a regular file",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(no_checker_evidence(
            VerdictStatus::Inconclusive,
            format!(
                "Kani produced no report (exit {:?}): {}",
                out.status.code(),
                checker_diagnostic(&out),
            ),
        )),
        Err(error) => Err(sandbox_io(error)),
    }
}

fn checker_diagnostic(out: &Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let output = if stderr.trim().is_empty() {
        &stdout
    } else {
        &stderr
    };
    output.trim().chars().take(1024).collect()
}

fn sandbox_io(error: std::io::Error) -> SandboxError {
    SandboxError::Io(error.to_string())
}

fn canonical_path(path: &Path) -> Result<PathBuf, SandboxError> {
    path.canonicalize()
        .map_err(|error| SandboxError::Io(format!("{}: {error}", path.display())))
}

fn path_arg(path: &Path) -> Result<&str, SandboxError> {
    path.to_str()
        .ok_or_else(|| SandboxError::Io("checker path is not UTF-8".to_string()))
}

fn independent_checker_file(
    path: &Path,
    worktree_dir: &Path,
    role: &str,
) -> Result<PathBuf, SandboxError> {
    if path.is_absolute() {
        let canonical = canonical_path(path)?;
        if canonical.is_file() && !canonical.starts_with(worktree_dir) {
            return Ok(canonical);
        }
    }
    Err(SandboxError::NotAvailable(format!(
        "{role} must be an absolute path to a regular file outside the candidate worktree"
    )))
}

fn configured_checker_path(variable: &str, worktree_dir: &Path) -> Result<PathBuf, SandboxError> {
    let path = env::var_os(variable)
        .ok_or_else(|| SandboxError::NotAvailable(format!("{variable} is unset")))?;
    independent_checker_file(Path::new(&path), worktree_dir, variable)
}

fn checker_sandbox(worktree_dir: &Path, program: &Path) -> Result<SandboxConfig, SandboxError> {
    let bin_dir = program
        .parent()
        .ok_or_else(|| SandboxError::Io("checker has no tool directory".to_string()))?;
    let tool_path = env::join_paths([
        bin_dir,
        Path::new("/usr/local/bin"),
        Path::new("/usr/bin"),
        Path::new("/bin"),
    ])
    .map_err(|error| SandboxError::Io(error.to_string()))?;
    let mut cfg = default_offline_config(worktree_dir, None);
    cfg.read_only_paths.push(program.to_path_buf());
    cfg.env_passthrough.push((
        "PATH".to_string(),
        path_arg(Path::new(&tool_path))?.to_string(),
    ));
    Ok(cfg)
}

/// Runs a trusted Lake comparator against an independently maintained configuration and export.
pub fn run_lean_comparator_sandboxed(
    worktree_dir: &Path,
    challenge_export: &Path,
) -> Result<CheckerVerdict, SandboxError> {
    let worktree_dir = canonical_path(worktree_dir)?;
    let program = configured_checker_path("VERITY_LEAN_COMPARATOR", &worktree_dir)?;
    let config = configured_checker_path("VERITY_LEAN_COMPARATOR_CONFIG", &worktree_dir)?;
    let challenge_export =
        independent_checker_file(challenge_export, &worktree_dir, "Lean challenge export")?;
    let config_flag = format!("--config={}", path_arg(&config)?);
    let challenge_flag = format!("--challenge-from-export={}", path_arg(&challenge_export)?);
    let mut sandbox = checker_sandbox(&worktree_dir, &program)?;
    sandbox.read_only_paths.extend([config, challenge_export]);
    let out = run_sandboxed(
        &sandbox,
        path_arg(&program)?,
        &["comparator", &config_flag, &challenge_flag, "--paranoid"],
    )?;
    if out.status.success() {
        return Ok(CheckerVerdict {
            status: VerdictStatus::Pass,
            reason: "Lean comparator --paranoid accepted the proof".to_string(),
            checks_passed: 1,
            probes_satisfied: 0,
        });
    }
    Ok(no_checker_evidence(
        VerdictStatus::Inconclusive,
        format!(
            "Lean comparator did not accept the proof (exit {:?}): {}",
            out.status.code(),
            checker_diagnostic(&out),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KANI_PASS: &str = include_str!("../tests/fixtures/kani-0.68.0-pass.json");

    #[test]
    fn pinned_kani_exports_distinguish_assertion_failure_from_insufficient_unwind() {
        let cases = [
            (
                include_str!("../tests/fixtures/kani-0.68.0-failure.json"),
                "verify_failure",
                10,
                VerdictStatus::Counterexample,
            ),
            (
                include_str!("../tests/fixtures/kani-0.68.0-bound.json"),
                "verify_bound",
                18,
                VerdictStatus::Inconclusive,
            ),
        ];
        for (raw, name, line, expected) in cases {
            let target = KaniTarget {
                name: name.to_string(),
                source: RelPath::parse("proof.rs").unwrap(),
                line,
                probes: Vec::new(),
            };
            let verdict = classify_kani_export(raw, Some(1), Path::new("/worktree"), &target);
            assert_eq!(verdict.status, expected, "{}", verdict.reason);
        }
    }

    #[test]
    fn pinned_kani_export_requires_requested_source_and_successful_process() {
        let mut target = KaniTarget {
            name: "verify_pass".to_string(),
            source: RelPath::parse("proof.rs").unwrap(),
            line: 3,
            probes: vec!["forty_two".to_string()],
        };
        let classify = |raw, exit_code, target: &KaniTarget| {
            classify_kani_export(raw, exit_code, Path::new("/worktree"), target).status
        };
        assert_eq!(classify(KANI_PASS, Some(0), &target), VerdictStatus::Pass);
        assert_eq!(
            classify(KANI_PASS, Some(1), &target),
            VerdictStatus::Inconclusive
        );
        let other_crate = KANI_PASS.replace(
            r#""crate_name": "verity_candidate""#,
            r#""crate_name": "other_crate""#,
        );
        assert_eq!(
            classify(&other_crate, Some(0), &target),
            VerdictStatus::InvalidReport
        );
        target.name = "pass".to_string();
        assert_eq!(
            classify(KANI_PASS, Some(0), &target),
            VerdictStatus::InvalidReport
        );
        target.name = "verify_pass".to_string();
        target.line = 8;
        assert_eq!(
            classify(KANI_PASS, Some(0), &target),
            VerdictStatus::InvalidReport
        );
        target.line = 3;
        target.source = RelPath::parse("src/other.rs").unwrap();
        assert_eq!(
            classify(KANI_PASS, Some(0), &target),
            VerdictStatus::InvalidReport
        );
        target.source = RelPath::parse("proof.rs").unwrap();
        target.probes.push("missing_probe".to_string());
        assert_eq!(
            classify(KANI_PASS, Some(0), &target),
            VerdictStatus::InvalidReport
        );
    }

    #[test]
    fn pinned_kani_export_requires_consistent_completed_harness_evidence() {
        assert!(parse_kani_json(r#"{"harness":"verify_pass","checks":[]}"#).is_err());
        for (from, to) in [
            (r#""executed": 1"#, r#""executed": 0"#),
            (r#""total_properties": 2"#, r#""total_properties": 3"#),
            (r#""status": "completed""#, r#""status": "incomplete""#),
            (
                r#""harness_id": "proofs::verify_pass""#,
                r#""harness_id": "proofs::other""#,
            ),
            (r#""kani_version": "0.68.0""#, r#""kani_version": "0.67.0""#),
        ] {
            let raw = KANI_PASS.replacen(from, to, 1);
            assert_ne!(raw, KANI_PASS, "fixture field {from} is missing");
            assert!(parse_kani_json(&raw).is_err(), "accepted {to}");
        }
    }

    #[test]
    fn candidate_kani_report_requires_more_than_implicit_safety_and_cover_checks() {
        let target = KaniTarget {
            name: "verify_pass".into(),
            source: RelPath::parse("proof.rs").unwrap(),
            line: 3,
            probes: vec!["forty_two".into()],
        };
        let safety_only =
            KANI_PASS.replace(r#""category": "assertion""#, r#""category": "overflow""#);
        assert_ne!(safety_only, KANI_PASS);
        let verdict = classify_kani_export(&safety_only, Some(0), Path::new("/worktree"), &target);
        assert_eq!(verdict.status, VerdictStatus::InvalidReport);
        assert!(verdict.reason.contains("successful assertion"));
    }

    #[test]
    fn checker_files_require_independent_absolute_paths() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let worktree = canonical_path(&manifest_dir.join("src")).unwrap();
        assert!(independent_checker_file(Path::new("Cargo.toml"), &worktree, "checker").is_err());
        assert!(
            independent_checker_file(&worktree.join("verifiers.rs"), &worktree, "checker").is_err()
        );
        let independent = manifest_dir.join("Cargo.toml");
        assert_eq!(
            independent_checker_file(&independent, &worktree, "checker").unwrap(),
            canonical_path(&independent).unwrap(),
        );
    }

    #[test]
    fn rust_scanner_ignores_comments_and_strings_but_catches_smuggled_code() {
        let clean = r##"
// kani::assume(false); kani::stub(a, b);
/* nested /* kani::assume(false) */ still comment: kani::should_panic */
pub fn msg<'a, 'b>(_a: &'a str, _b: &'b str) -> &'static str {
    let _s = "kani::assume(false) \" kani::stub";
    let _r = r#"kani::should_panic and kani::assume(false)"#;
    let _c = '"';
    "ok"
}
"##;
        assert!(hygiene_lint_rust("src/clean.rs", clean).is_clean());

        let smuggled = "/* harmless /* nested */ */ kani::assume(false);\n\
                        let s = \"verity:trusted-assume\"; kani::assume(x > 0);\n";
        let report = hygiene_lint_rust("src/smuggled.rs", smuggled);
        assert_eq!(report.violations.len(), 2);
        assert_eq!(report.violations[0].line, 1);
        assert_eq!(report.violations[0].rule, "unreviewed-kani-assume");
        assert_eq!(report.violations[1].line, 2);
        assert_eq!(report.violations[1].rule, "unreviewed-kani-assume");
    }

    #[test]
    fn lean_scanner_ignores_nested_comments_and_strings_but_catches_smuggling() {
        let clean = r#"
-- sorry axiom partial def
/- outer /- inner sorry -/ still comment: unsafe native_decide -/
def note : String := "sorry and partial def"
"#;
        assert!(hygiene_lint_lean("Spec.lean", clean).is_clean());

        let smuggled = "/- ok /- nested -/ -/ sorry\n\
                        partial /- gap -/ def loop (n : Nat) : Nat := loop n\n";
        let report = hygiene_lint_lean("Bad.lean", smuggled);
        assert_eq!(report.violations.len(), 2);
        assert_eq!(report.violations[0].line, 1);
        assert_eq!(report.violations[0].rule, "no-sorry");
        assert_eq!(report.violations[1].line, 2);
        assert_eq!(report.violations[1].rule, "no-partial");
    }

    #[test]
    fn discover_kani_targets_ignores_comments_and_strings_and_extracts_real_targets() {
        let src = r##"
// #[kani::proof]
// fn commented_line_proof() {
//     kani::cover!(true, "line_comment_probe");
// }

/*
#[kani::proof]
/* nested #[kani::proof] fn nested_comment_proof() {} */
fn block_comment_proof() {
    kani::cover!(true, "block_comment_probe");
}
*/

pub fn decoy() -> &'static str {
    let _s = "#[kani::proof]\nfn string_proof() { kani::cover!(true, \"string_probe\"); }";
    let _r = r#"#[kani::proof] fn raw_proof() { kani::cover!(true, "raw_probe"); }"#;
    "ok"
}

#[kani::proof]
#[kani::unwind(8)]
pub(crate) fn verify_safe_sub() {
    let a: u32 = kani::any();
    let b: u32 = kani::any();
    kani::cover!(a > b, "positive_diff");
    kani::cover!(
        a.checked_sub(b).is_none() && helper(1, 2) == 3,
        "underflow_saturates",
    );
    kani::cover!(a == 0);
}

#[kani::proof]
fn second_harness() {
    kani::cover!(true, r#"raw_label_probe"#);
}

fn helper() {
    kani::cover!(true, "outside_probe");
}
"##;
        let files = vec![
            (
                RelPath::parse("README.md").unwrap(),
                "#[kani::proof]\nfn md_fake() { kani::cover!(true, \"md_probe\"); }\n".to_string(),
            ),
            (RelPath::parse("src/math.rs").unwrap(), src.to_string()),
        ];

        let targets = discover_kani_proofs(&files);
        let [first, second] = targets.as_slice() else {
            panic!("expected two proof harnesses, got {targets:?}");
        };
        assert_eq!(first.name, "verify_safe_sub");
        assert_eq!(first.source.as_str(), "src/math.rs");
        assert_eq!(first.probes, ["positive_diff", "underflow_saturates"]);
        assert_eq!(second.name, "second_harness");
        assert_eq!(second.probes, ["raw_label_probe"]);
    }

    #[test]
    fn discover_kani_targets_returns_none_when_only_commented_proofs_exist() {
        let src = r#"
// #[kani::proof]
// fn fake() {}
/* #[kani::proof] fn fake2() { kani::cover!(true, "hidden"); } */
pub fn add(a: u32, b: u32) -> u32 { a.saturating_add(b) }
"#;
        let files = vec![(RelPath::parse("src/lib.rs").unwrap(), src.to_string())];
        let (harness, probes) = discover_kani_targets(&files);
        assert_eq!(harness, None);
        assert!(probes.is_empty());
    }

    #[test]
    fn discover_kani_proofs_recognizes_cfg_attr_kani_proof() {
        let src = r#"
#[cfg_attr(kani, kani::proof)]
fn verify_cfg_attr_harness() {
    kani::cover!(true, "cfg_probe");
}
"#;
        let files = vec![(RelPath::parse("src/cfg.rs").unwrap(), src.to_string())];
        let targets = discover_kani_proofs(&files);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "verify_cfg_attr_harness");
        assert_eq!(targets[0].probes, ["cfg_probe"]);
    }

    #[test]
    fn hygiene_lint_rust_catches_whitespace_split_and_imported_assume() {
        let split_src = "fn harness() {
    kani :: assume(false);
}
";
        let split_report = hygiene_lint_rust("src/split.rs", split_src);
        assert_eq!(split_report.violations.len(), 1);
        assert_eq!(split_report.violations[0].rule, "unreviewed-kani-assume");
        assert_eq!(split_report.violations[0].line, 2);

        let imported_src = "use kani::assume;
fn harness() {
    assume(false);
}
";
        let imported_report = hygiene_lint_rust("src/imported.rs", imported_src);
        assert_eq!(imported_report.violations.len(), 1);
        assert_eq!(imported_report.violations[0].rule, "unreviewed-kani-assume");
        assert_eq!(imported_report.violations[0].line, 3);

        let grouped_import_src = "use kani::{any, assume};
fn harness() {
    let _x: u8 = any();
    assume(false);
}
";
        let grouped_report = hygiene_lint_rust("src/grouped.rs", grouped_import_src);
        assert_eq!(grouped_report.violations.len(), 1);
        assert_eq!(grouped_report.violations[0].rule, "unreviewed-kani-assume");
        assert_eq!(grouped_report.violations[0].line, 4);
    }

    #[test]
    fn kani_invocation_selects_cargo_or_standalone_mode() {
        let base = env::temp_dir().join(format!("verity-kani-inv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let wt = base.join("wt");
        let bin_dir = base.join("bin");
        fs::create_dir_all(wt.join("src")).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(wt.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        let src_file = wt.join("src/lib.rs");
        fs::write(&src_file, "#[kani::proof] fn verify_ok() {}\n").unwrap();
        let root_file = wt.join("proof.rs");
        fs::write(&root_file, "#[kani::proof] fn verify_ok() {}\n").unwrap();
        let fake_cargo = bin_dir.join("cargo");
        fs::write(&fake_cargo, "#!/bin/sh\n").unwrap();

        let run_dir = wt.join(".verity-kani-0");
        let report_path = run_dir.join("report.json");
        let src_target = KaniTarget {
            name: "verify_ok".to_string(),
            source: RelPath::parse("src/lib.rs").unwrap(),
            line: 1,
            probes: Vec::new(),
        };
        let root_target = KaniTarget {
            name: "verify_ok".to_string(),
            source: RelPath::parse("proof.rs").unwrap(),
            line: 1,
            probes: Vec::new(),
        };

        // 1. Standalone kani without VERITY_CARGO_KANI uses standalone mode.
        let standalone = build_kani_invocation(
            &wt,
            &src_file,
            &run_dir,
            &report_path,
            &src_target,
            Path::new("/usr/local/bin/kani"),
            None,
        )
        .unwrap();
        assert_eq!(standalone.mode, KaniMode::Standalone);
        assert!(standalone.args.contains(&"--crate-name".to_string()));
        assert!(standalone
            .args
            .contains(&src_file.to_string_lossy().into_owned()));

        // 2. cargo-kani binary with Cargo.toml and src/ proof selects Cargo mode.
        let cargo_bin = build_kani_invocation(
            &wt,
            &src_file,
            &run_dir,
            &report_path,
            &src_target,
            Path::new("/usr/local/bin/cargo-kani"),
            None,
        )
        .unwrap();
        assert_eq!(cargo_bin.mode, KaniMode::Cargo);
        assert_eq!(
            cargo_bin.program,
            PathBuf::from("/usr/local/bin/cargo-kani")
        );
        assert!(cargo_bin.args.contains(&"--manifest-path".to_string()));
        assert!(!cargo_bin.args.contains(&"--crate-name".to_string()));
        assert!(!cargo_bin.args.contains(&"kani".to_string()));

        // 3. VERITY_CARGO_KANI pointing at cargo prepends `kani` subcommand.
        let cargo_env = build_kani_invocation(
            &wt,
            &src_file,
            &run_dir,
            &report_path,
            &src_target,
            Path::new("/usr/local/bin/kani"),
            Some(fake_cargo.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(cargo_env.mode, KaniMode::Cargo);
        assert_eq!(cargo_env.args.first().map(String::as_str), Some("kani"));
        assert!(cargo_env.args.contains(&"--manifest-path".to_string()));

        // 4. Proof outside src/ stays in standalone mode even with cargo-kani.
        let outside_src = build_kani_invocation(
            &wt,
            &root_file,
            &run_dir,
            &report_path,
            &root_target,
            Path::new("/usr/local/bin/cargo-kani"),
            Some("1"),
        )
        .unwrap();
        assert_eq!(outside_src.mode, KaniMode::Standalone);

        // 5. Repo without Cargo.toml stays in standalone mode.
        fs::remove_file(wt.join("Cargo.toml")).unwrap();
        let no_manifest = build_kani_invocation(
            &wt,
            &src_file,
            &run_dir,
            &report_path,
            &src_target,
            Path::new("/usr/local/bin/cargo-kani"),
            Some("1"),
        )
        .unwrap();
        assert_eq!(no_manifest.mode, KaniMode::Standalone);

        let _ = fs::remove_dir_all(&base);
    }
}
