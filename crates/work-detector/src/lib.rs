//! `gwz-work-detector`: pure classification of unsaved work (lane W).
//!
//! [`classify_work`] turns one repository's observed on-disk work
//! (`gwz_repo_contract::WorkObservation`) and core's decoded GWZ evidence
//! ([`GwzEvidence`]) into a [`WorkReport`]: clean, dirty with named hazards,
//! or unknown with reasons. It reads nothing and holds no core or protocol
//! type; core decodes merge/stash records into plain evidence first, and an
//! unsupported or malformed record is [`EvidenceState::Unknown`], never
//! silently clean. Diagnostic truncation never removes a hazard.
//!
//! LCM1.0c checkpoint state: the vocabulary is frozen; `classify_work`
//! returns [`WorkVerdict::Unknown`] with an `Unimplemented` reason. Lane W's
//! implementation starts from plain-value tests (staged/unstaged, binary,
//! renames, ignored data, nested repositories, suppressed flags, open
//! operations) failing against this function.

#![forbid(unsafe_code)]

use gwz_repo_contract::{BytePath, UnknownReason, WorkKind, WorkObservation};

/// Core-decoded evidence of GWZ coordination state for one repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GwzEvidence {
    /// The workspace's merge record, as it concerns this repository.
    pub merge: EvidenceState,
    /// GWZ stash bundle records naming this repository.
    pub stash: EvidenceState,
    /// Other coordination records core chose to surface.
    pub other: Vec<EvidenceItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum EvidenceState {
    /// No record.
    #[default]
    None,
    /// An open or unfinished record.
    Open { detail: String },
    /// A record exists but could not be interpreted; a hazard, not a pass.
    Unknown { detail: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceItem {
    pub kind: String,
    pub state: EvidenceState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkVerdict {
    Clean,
    Dirty,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HazardKind {
    /// On-disk work of the named kind.
    Work(WorkKind),
    /// A tracked path whose suppressed status was observed to differ or
    /// could not be observed.
    Suppressed,
    OpenNativeOperation,
    OpenGwzMerge,
    OpenGwzStash,
    /// Evidence that exists but cannot be interpreted.
    UninterpretableEvidence,
    /// Native stash entries present.
    NativeStash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hazard {
    pub kind: HazardKind,
    pub path: Option<BytePath>,
    pub detail: String,
}

/// The classification. `hazards` is complete even when `truncated` says the
/// diagnostic list was cut for presentation: truncation drops detail text,
/// never entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkReport {
    pub verdict: WorkVerdict,
    pub hazards: Vec<Hazard>,
    pub unknown: Vec<UnknownReason>,
    pub truncated: bool,
}

impl WorkReport {
    pub fn unknown(reasons: Vec<UnknownReason>) -> Self {
        Self {
            verdict: WorkVerdict::Unknown,
            hazards: Vec::new(),
            unknown: reasons,
            truncated: false,
        }
    }
}

/// Classify one repository's work. Pure and deterministic.
pub fn classify_work(_observation: &WorkObservation, _evidence: &GwzEvidence) -> WorkReport {
    WorkReport::unknown(vec![UnknownReason::unimplemented(
        "gwz-work-detector: classify_work",
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_repo_contract::UnknownKind;

    #[test]
    fn checkpoint_classifier_is_unknown_never_clean() {
        let report = classify_work(&WorkObservation::default(), &GwzEvidence::default());
        assert_eq!(report.verdict, WorkVerdict::Unknown);
        assert!(report.hazards.is_empty());
        assert_eq!(report.unknown[0].kind, UnknownKind::Unimplemented);
        assert!(!report.truncated);
        assert_ne!(report.verdict, WorkVerdict::Clean);
    }

    #[test]
    fn evidence_defaults_to_no_record() {
        assert_eq!(GwzEvidence::default().merge, EvidenceState::None);
        let unknown = EvidenceState::Unknown {
            detail: "record version 9".to_owned(),
        };
        assert_ne!(unknown, EvidenceState::None);
    }
}
