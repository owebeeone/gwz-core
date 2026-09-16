#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;

/// Which side of the action namespace an edge is crossing.
///
/// The two edges are physically one move — a source-associated, no-replace
/// rename between two deterministic slot names under the same retained action
/// directory — so they share one primitive call site. The role selects the
/// stable `namespace.*` boundaries and the diagnostic label, exactly as
/// `AdmissionRecordRowV1` selects `admission.*` for the three admission rows
/// that share one write helper (`admission_mutation.rs:315-370`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ActionNamespaceEdgeV1 {
    /// Edge E12 — a scheduled scratch role published onto its active role.
    Publish,
    /// Edge E13 — a scheduled active role retired onto its retirement role.
    Retire,
}

impl ActionNamespaceEdgeV1 {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Publish => "publish action namespace slot",
            Self::Retire => "retire action namespace slot",
        }
    }

    pub(crate) const fn flush_label(self) -> &'static str {
        match self {
            Self::Publish => "flush action namespace publication",
            Self::Retire => "flush action namespace retirement",
        }
    }

    /// The reserve / pre-edge / edge / post-edge boundaries this role crosses.
    #[cfg(test)]
    pub(crate) const fn faults(self) -> [CheckedArtifactFaultKeyV1; 4] {
        match self {
            Self::Publish => [
                CheckedArtifactFaultKeyV1::NamespaceDestinationReserve,
                CheckedArtifactFaultKeyV1::NamespacePrePublishReobserve,
                CheckedArtifactFaultKeyV1::NamespacePublishNoReplace,
                CheckedArtifactFaultKeyV1::NamespacePublishedReobserve,
            ],
            Self::Retire => [
                CheckedArtifactFaultKeyV1::NamespaceRetirementReserve,
                CheckedArtifactFaultKeyV1::NamespacePreRetireReobserve,
                CheckedArtifactFaultKeyV1::NamespaceRetireExact,
                CheckedArtifactFaultKeyV1::NamespaceRetiredReobserve,
            ],
        }
    }
}
