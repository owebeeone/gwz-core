//! Decoded GWZ record evidence: what the caller hands in beside the
//! repository's own work observation.

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
