use crate::sandbox::{default_offline_config, run_sandboxed, SandboxError};
use std::path::Path;
use verity_core::json::parse_json;
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

/// Scans comment-stripped Rust files for the first `#[kani::proof]` function
/// name and all `kani::cover!(..., "label")` reachability probe descriptions.
pub fn discover_kani_targets(files: &[(RelPath, String)]) -> (Option<String>, Vec<String>) {
    let mut first_harness: Option<String> = None;
    let mut probes: Vec<String> = Vec::new();

    for (rel, source) in files {
        if !rel.as_str().ends_with(".rs") {
            continue;
        }
        let stripped = strip_rust_comments(source);
        let bytes = stripped.as_bytes();
        let len = bytes.len();
        let mut i = 0usize;
        let mut pending_proof = false;

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
                if compact == "kani::proof" && first_harness.is_none() {
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
                        first_harness = Some(stripped[name_start..name_end].to_string());
                        pending_proof = false;
                        i = name_end;
                        continue;
                    }
                }
            }

            // Detect `kani::cover!(..., "label")`.
            if starts_with_ident(bytes, i, b"kani") {
                let mut j = skip_ws(bytes, i + 4);
                if bytes.get(j..j + 2) == Some(b"::") {
                    j = skip_ws(bytes, j + 2);
                    if starts_with_ident(bytes, j, b"cover") {
                        j = skip_ws(bytes, j + 5);
                        if bytes.get(j) == Some(&b'!') {
                            j = skip_ws(bytes, j + 1);
                            if matches!(bytes.get(j), Some(b'(' | b'[' | b'{')) {
                                let (label, next_i) = extract_cover_label(&stripped, j);
                                if let Some(desc) = label {
                                    if !desc.is_empty() {
                                        probes.push(desc);
                                    }
                                }
                                i = next_i;
                                continue;
                            }
                        }
                    }
                }
            }

            let ch = stripped[i..].chars().next().unwrap();
            i += ch.len_utf8();
        }
    }

    (first_harness, probes)
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
pub fn hygiene_lint_rust(rel_path: &str, source: &str) -> HygieneReport {
    let mut violations = Vec::new();
    for (idx, scanned) in scan_rust_lines(source).into_iter().enumerate() {
        let code = collapse_spaces(&scanned.code);
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
        if code.contains("kani::assume(") && !scanned.comments.contains("verity:trusted-assume") {
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

/// Parses Kani JSON report output (`kani --export-json`) into `KaniReport`.
pub fn parse_kani_json(raw: &str) -> Result<KaniReport, String> {
    let root = parse_json(raw).map_err(|e| e.to_string())?;
    let harness_name = root
        .get("harness")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let should_panic = root
        .get("should_panic")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let solver_error = root
        .get("solver_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let checks_arr = root
        .get("checks")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing checks array in Kani JSON".to_string())?;

    let mut checks = Vec::with_capacity(checks_arr.len());
    for item in checks_arr {
        let class_str = item
            .get("property_class")
            .and_then(|v| v.as_str())
            .unwrap_or("safety");
        let kind = match class_str {
            "assertion" => CheckKind::Assertion,
            "cover" => CheckKind::Cover,
            _ => CheckKind::Safety,
        };
        let description = item
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let status_str = item
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("UNDETERMINED");
        let status = match status_str {
            "SUCCESS" => CheckStatus::Success,
            "FAILURE" => CheckStatus::Failure,
            "UNREACHABLE" => CheckStatus::Unreachable,
            "SATISFIED" => CheckStatus::Satisfied,
            "UNSATISFIABLE" => CheckStatus::Unsatisfiable,
            _ => CheckStatus::Undetermined,
        };
        checks.push(KaniCheck {
            kind,
            description,
            status,
        });
    }

    Ok(KaniReport {
        harness_name,
        should_panic,
        solver_error,
        checks,
    })
}

/// Runs `cargo kani` inside an offline OS sandbox and classifies its JSON report.
pub fn run_kani_sandboxed(
    worktree_dir: &Path,
    harness_name: &str,
    expected_probes: &[&str],
) -> Result<CheckerVerdict, SandboxError> {
    let cfg = default_offline_config(worktree_dir, None);
    let report_path = worktree_dir.join(".verity-kani-report.json");
    // Remove any stale report from an interrupted prior run before executing.
    let _ = std::fs::remove_file(&report_path);
    let report_str = report_path
        .to_str()
        .ok_or_else(|| SandboxError::Io("non-utf8 report path".to_string()))?;

    let out = run_sandboxed(
        &cfg,
        "cargo",
        &[
            "kani",
            "-Z",
            "unstable-options",
            "--harness",
            harness_name,
            "--export-json",
            report_str,
        ],
    )?;

    if !report_path.exists() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let missing_bin = stderr.contains("No such file or directory")
            || stderr.contains("no such command: `kani`")
            || stderr.contains("no such subcommand: `kani`")
            || stderr.contains("not found");
        let reason = if missing_bin {
            format!(
                "cargo kani binary unavailable in sandbox (exit {:?}): {} — install cargo-kani or ensure it is on PATH",
                out.status.code(),
                stderr
            )
        } else {
            format!(
                "kani produced no report (exit {:?}): {}",
                out.status.code(),
                stderr
            )
        };
        return Ok(CheckerVerdict {
            status: VerdictStatus::Inconclusive,
            reason,
            checks_passed: 0,
            probes_satisfied: 0,
        });
    }

    let raw = std::fs::read_to_string(&report_path).map_err(|e| SandboxError::Io(e.to_string()))?;
    let _ = std::fs::remove_file(&report_path);

    match parse_kani_json(&raw) {
        Ok(report) => Ok(classify_kani_report(&report, expected_probes)),
        Err(msg) => Ok(CheckerVerdict {
            status: VerdictStatus::InvalidReport,
            reason: msg,
            checks_passed: 0,
            probes_satisfied: 0,
        }),
    }
}

/// Runs Lean 4 `lake comparator --paranoid` inside an offline OS sandbox.
pub fn run_lean_comparator_sandboxed(
    worktree_dir: &Path,
    challenge_export: &Path,
) -> Result<CheckerVerdict, SandboxError> {
    let cfg = default_offline_config(worktree_dir, None);
    let flag = format!("--challenge-from-export={}", challenge_export.display());
    let out = run_sandboxed(
        &cfg,
        "lake",
        &[
            "comparator",
            &flag,
            "--paranoid",
            "--permitted-axioms=propext,Quot.sound,Classical.choice",
        ],
    )?;

    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if out.status.success() {
        return Ok(CheckerVerdict {
            status: VerdictStatus::Pass,
            reason: "lean comparator --paranoid accepted proof".to_string(),
            checks_passed: 1,
            probes_satisfied: 1,
        });
    }
    if stderr.contains("bwrap:")
        || stderr.contains("No such file or directory")
        || stderr.contains("not found")
    {
        return Ok(CheckerVerdict {
            status: VerdictStatus::Inconclusive,
            reason: format!(
                "lake binary unavailable in sandbox: {} — install Lean 4 / lake and ensure it is on PATH",
                stderr
            ),
            checks_passed: 0,
            probes_satisfied: 0,
        });
    }
    Ok(CheckerVerdict {
        status: VerdictStatus::Counterexample,
        reason: format!("lean comparator rejected proof: {}", stderr),
        checks_passed: 0,
        probes_satisfied: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
"##;
        let files = vec![
            (
                RelPath::parse("README.md").unwrap(),
                "#[kani::proof]\nfn md_fake() { kani::cover!(true, \"md_probe\"); }\n".to_string(),
            ),
            (RelPath::parse("src/math.rs").unwrap(), src.to_string()),
        ];

        let (harness, probes) = discover_kani_targets(&files);
        assert_eq!(harness.as_deref(), Some("verify_safe_sub"));
        assert_eq!(
            probes,
            vec![
                "positive_diff".to_string(),
                "underflow_saturates".to_string(),
                "raw_label_probe".to_string(),
            ]
        );
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
}
