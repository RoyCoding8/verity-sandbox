use crate::types::{CommitOid, RelPath, SpecDigest};
use crate::verdict::{can_approve_gate, CheckerVerdict, GateError, HygieneReport};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecHandoff {
    pub candidate_commit: CommitOid,
    pub changed_paths: Vec<RelPath>,
    pub extracted_spec: String,
    pub back_translation: Option<String>,
    pub hygiene: HygieneReport,
    pub verdict: CheckerVerdict,
    pub spec_digest: SpecDigest,
}

pub fn can_approve(handoff: &SpecHandoff) -> Result<(), GateError> {
    can_approve_gate(&handoff.hygiene, &handoff.verdict)
}
