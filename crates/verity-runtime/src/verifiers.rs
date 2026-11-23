use crate::sandbox::{default_offline_config, run_sandboxed, SandboxError};
use std::path::Path;
use verity_core::json::parse_json;
use verity_core::{
    classify_kani_report, CheckKind, CheckStatus, CheckerVerdict, HygieneReport, HygieneViolation,
    KaniCheck, KaniReport, VerdictStatus,
};

/// Layer 1 Rust/Kani escape-hatch lint. Rejects stubbing, should_panic,
/// disabled unwinding assertions, and unannotated `kani::assume` calls.
pub fn hygiene_lint_rust(rel_path: &str, source: &str) -> HygieneReport {
    let mut violations = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        let line_no = idx + 1;
        if trimmed.contains("kani::stub") {
            violations.push(HygieneViolation {
                rule: "no-kani-stub".to_string(),
                path: rel_path.to_string(),
                line: line_no,
                detail: "kani::stub mocks out real implementation logic".to_string(),
            });
        }
        if trimmed.contains("kani::should_panic") {
            violations.push(HygieneViolation {
                rule: "no-should-panic".to_string(),
                path: rel_path.to_string(),
                line: line_no,
                detail: "kani::should_panic weakens proof obligations".to_string(),
            });
        }
        if trimmed.contains("no-unwinding-assertions") {
            violations.push(HygieneViolation {
                rule: "require-unwinding-assertions".to_string(),
                path: rel_path.to_string(),
                line: line_no,
                detail: "disabling unwinding assertions allows vacuous loop bounds".to_string(),
            });
        }
        if trimmed.contains("kani::assume(") && !line.contains("verity:trusted-assume") {
            violations.push(HygieneViolation {
                rule: "unreviewed-kani-assume".to_string(),
                path: rel_path.to_string(),
                line: line_no,
                detail: "kani::assume must be marked with '// verity:trusted-assume' for spec review"
                    .to_string(),
            });
        }
    }
    HygieneReport { violations }
}

/// Layer 1 Lean 4 escape-hatch lint. Blocks sorry, custom axioms, and
/// compiler-divergence constructs (`partial`, `unsafe`, `@[implemented_by]`, `@[extern]`, `@[csimp]`).
pub fn hygiene_lint_lean(rel_path: &str, source: &str) -> HygieneReport {
    let mut violations = Vec::new();
    let banned = [
        ("no-sorry", "sorry", "proof contains sorry"),
        ("no-custom-axiom", "axiom ", "custom axiom weakens logical soundness"),
        ("no-partial", "partial def", "partial def allows compiled runtime divergence"),
        ("no-unsafe", "unsafe ", "unsafe bypasses the Lean kernel"),
        ("no-implemented-by", "@[implemented_by", "@[implemented_by] diverges from proved definition"),
        ("no-extern", "@[extern", "@[extern] diverges from proved definition"),
        ("no-csimp", "@[csimp", "@[csimp] rewrites compiled code outside kernel"),
        ("no-native-decide", "native_decide", "native_decide trusts the compiler"),
    ];

    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            continue;
        }
        let line_no = idx + 1;
        for (rule, needle, detail) in banned {
            if trimmed.contains(needle) {
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
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Ok(CheckerVerdict {
            status: VerdictStatus::Inconclusive,
            reason: format!("kani produced no report (exit {:?}): {}", out.status.code(), stderr.trim()),
            checks_passed: 0,
            probes_satisfied: 0,
        });
    }

    let raw = std::fs::read_to_string(&report_path)
        .map_err(|e| SandboxError::Io(e.to_string()))?;
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
    if stderr.contains("bwrap:") || stderr.contains("No such file or directory") {
        return Ok(CheckerVerdict {
            status: VerdictStatus::Inconclusive,
            reason: format!("environment error running lean comparator: {}", stderr),
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
