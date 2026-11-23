use crate::verifiers::{hygiene_lint_lean, hygiene_lint_rust};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use verity_core::json::escape_json_string;
use verity_core::{
    CheckerVerdict, CommitOid, HygieneReport, RelPath, SpecDigest, SpecHandoff, VerifierKind,
};

/// Extracts ONLY formal definitions, public signatures, contracts, assume/unwind
/// lines, and reachability probes from changed files so the main agent reviews
/// the trust boundary without loading proof scripts or full implementation bodies.
pub fn extract_spec_only(verifier: VerifierKind, files: &[(RelPath, String)]) -> String {
    let mut sections = Vec::new();
    for (rel, content) in files {
        let mut kept = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            let include = match verifier {
                VerifierKind::RustKani => {
                    trimmed.starts_with("pub fn ")
                        || trimmed.starts_with("pub struct ")
                        || trimmed.starts_with("pub enum ")
                        || trimmed.starts_with("#[kani::")
                        || trimmed.contains("kani::assume(")
                        || trimmed.contains("kani::cover!(")
                        || trimmed.starts_with("assert!(")
                        || trimmed.starts_with("assert_eq!(")
                }
                VerifierKind::Lean4Comparator => {
                    trimmed.starts_with("structure ")
                        || trimmed.starts_with("inductive ")
                        || trimmed.starts_with("theorem ")
                        || trimmed.starts_with("lemma ")
                        || trimmed.starts_with("def ")
                        || trimmed.starts_with("axiom ")
                }
            };
            if include {
                let sanitized = match verifier {
                    VerifierKind::Lean4Comparator => {
                        if let Some(pos) = trimmed.find(":= by") {
                            trimmed[..pos].trim_end().to_string()
                        } else {
                            trimmed.to_string()
                        }
                    }
                    VerifierKind::RustKani => trimmed.to_string(),
                };
                kept.push(format!("{}: {}", idx + 1, sanitized));
            }
        }
        if !kept.is_empty() {
            sections.push(format!("// {}\n{}", rel.as_str(), kept.join("\n")));
        }
    }
    sections.join("\n\n")
}

/// Runs Layer 1 hygiene lints over all changed files, extracts the spec-only view,
/// computes the canonical SpecDigest, and returns a SpecHandoff bundle.
pub fn build_spec_handoff(
    candidate_commit: CommitOid,
    files: &[(RelPath, String)],
    verifier: VerifierKind,
    verdict: CheckerVerdict,
    back_translation: Option<String>,
) -> SpecHandoff {
    let mut violations = Vec::new();
    let mut changed_paths = Vec::with_capacity(files.len());

    for (rel, src) in files {
        changed_paths.push(rel.clone());
        let report = match verifier {
            VerifierKind::RustKani => hygiene_lint_rust(rel.as_str(), src),
            VerifierKind::Lean4Comparator => hygiene_lint_lean(rel.as_str(), src),
        };
        violations.extend(report.violations);
    }

    let extracted_spec = extract_spec_only(verifier, files);
    let spec_digest = SpecDigest::of_spec(&extracted_spec);

    SpecHandoff {
        candidate_commit,
        changed_paths,
        extracted_spec,
        back_translation,
        hygiene: HygieneReport { violations },
        verdict,
        spec_digest,
    }
}

/// Thin append-only JSONL run log replacing the 11-table SQLite journal.
#[derive(Debug, Clone)]
pub struct RunLog {
    path: PathBuf,
}

impl RunLog {
    pub fn open(repo_dir: &Path) -> Result<Self, std::io::Error> {
        let dir = repo_dir.join(".verity");
        fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join("run-log.jsonl"),
        })
    }

    pub fn append_event(
        &self,
        task_id: &str,
        kind: &str,
        detail: &str,
    ) -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = format!(
            "{{\"task_id\":{},\"kind\":{},\"detail\":{}}}\n",
            escape_json_string(task_id),
            escape_json_string(kind),
            escape_json_string(detail)
        );
        file.write_all(line.as_bytes())
    }

    pub fn read_lines(&self) -> Result<Vec<String>, std::io::Error> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&self.path)?;
        Ok(raw.lines().map(|l| l.to_string()).collect())
    }
}
