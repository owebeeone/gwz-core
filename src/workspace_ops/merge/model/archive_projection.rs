#![allow(
    dead_code,
    reason = "the neutral model includes test-gated v1 archive variants"
)]

use std::collections::BTreeSet;

use crate::artifact::ArtifactSourceKind;

use super::ParticipantState;

/// Wire version of the checked archive source.
///
/// This discriminator is intentionally independent of the disabled v1 model so
/// archived v0 records can be decoded and projected by ordinary builds without
/// making the v1 body reader reachable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArchiveSourceVersion {
    V0,
    V1,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ArchivedMergeProjection {
    pub(crate) source_version: ArchiveSourceVersion,
    pub(crate) terminal_outcome: ArchivedTerminalOutcome,
    pub(crate) acceptance: ArchivedAcceptanceProjection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArchivedTerminalOutcome {
    Completed,
    Aborted,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ArchivedAcceptanceProjection {
    SupportedPersisted {
        workspace: InstalledAcceptedWorkspaceProjection,
    },
    LegacyComplete {
        workspace: ArchivedAcceptedWorkspace,
        source: LegacyAcceptanceSource,
    },
    LegacyUnavailable {
        available: LegacyAcceptanceEvidence,
        missing: BTreeSet<LegacyAcceptanceGap>,
    },
    NotAccepted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArchivedAcceptanceKind {
    SupportedPersisted,
    LegacyComplete,
    LegacyUnavailable,
    NotAccepted,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InstalledAcceptedWorkspaceProjection {
    V1(AcceptedWorkspaceV1Projection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InstalledAcceptedWorkspaceKind {
    V1,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum LegacyAcceptanceSource {
    Candidate,
    BaselineNoPublication,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum LegacyAcceptanceGap {
    ExactLockBytes,
    CompleteMemberAudit,
    AcceptedRootInput,
    PublicationEvidence,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcceptedWorkspaceV1Projection {
    pub(crate) operation_baseline_lock_sha256: String,
    pub(crate) metadata_base: AcceptedMetadataBaseProjection,
    pub(crate) lock_yaml: String,
    pub(crate) lock_sha256: String,
    pub(crate) members: Vec<AcceptedMemberV1Projection>,
    pub(crate) root: AcceptedRootProjection,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcceptedMetadataBaseProjection {
    pub(crate) source: AcceptedMetadataSource,
    pub(crate) source_commit: Option<String>,
    pub(crate) manifest_yaml: String,
    pub(crate) manifest_sha256: String,
    pub(crate) lock_yaml: String,
    pub(crate) lock_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AcceptedMetadataSource {
    OperationBaseline,
    SelectedRootResult,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcceptedMemberV1Projection {
    pub(crate) member_id: String,
    pub(crate) kind: AcceptedMemberKind,
    pub(crate) integration: Option<AcceptedIntegrationProjection>,
    pub(crate) final_checkout: Option<AcceptedCheckoutProjection>,
    pub(crate) lock_member: Option<AcceptedLockMemberProjection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AcceptedMemberKind {
    Selected,
    UnselectedPresent,
    Absent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AcceptedIntegrationProjection {
    pub(crate) branch: String,
    pub(crate) before_commit: String,
    pub(crate) resulting_commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AcceptedCheckoutProjection {
    pub(crate) branch: String,
    pub(crate) commit: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcceptedLockMemberProjection {
    pub(crate) path: String,
    pub(crate) source_id: String,
    pub(crate) source_kind: ArtifactSourceKind,
    pub(crate) commit: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) detached: Option<bool>,
    pub(crate) upstream: Option<String>,
    pub(crate) dirty: Option<bool>,
    pub(crate) materialized: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AcceptedRootProjection {
    pub(crate) kind: AcceptedRootKind,
    pub(crate) commit: Option<String>,
    pub(crate) symbolic_branch: Option<String>,
    pub(crate) publication_branch: Option<String>,
    pub(crate) lock_worktree_sha256: String,
    pub(crate) manifest_worktree_sha256: String,
    pub(crate) lock_commit_sha256: Option<String>,
    pub(crate) manifest_commit_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AcceptedRootKind {
    BornAttached,
    BornDetached,
    UnbornAttached,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ArchivedAcceptedWorkspace {
    pub(crate) baseline_lock_sha256: String,
    pub(crate) lock_yaml: String,
    pub(crate) lock_sha256: String,
    pub(crate) members: Vec<AcceptedMemberV1Projection>,
    pub(crate) root: AcceptedRootProjection,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LegacyAcceptanceEvidence {
    pub(crate) lock_yaml: Option<String>,
    pub(crate) lock_sha256: Option<String>,
    pub(crate) members: Vec<LegacyMemberEvidence>,
    pub(crate) root: Option<AcceptedRootProjection>,
    pub(crate) composition_commit: Option<String>,
    pub(crate) composition_tree: Option<String>,
    pub(crate) candidate_hashes: Vec<AcceptedCandidateHashProjection>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LegacyMemberEvidence {
    pub(crate) member_id: String,
    pub(crate) selected: bool,
    pub(crate) state: Option<ParticipantState>,
    pub(crate) integration: Option<AcceptedIntegrationProjection>,
    pub(crate) lock_member: Option<AcceptedLockMemberProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AcceptedCandidateHashProjection {
    pub(crate) path: String,
    pub(crate) sha256: String,
}

macro_rules! pinned_value {
    ($name:ident { $($variant:ident = $value:literal),+ $(,)? }) => {
        impl $name {
            pub(crate) const fn pinned_value(self) -> u32 {
                match self { $(Self::$variant => $value),+ }
            }
        }
    };
}

pinned_value!(ArchivedTerminalOutcome { Completed = 0, Aborted = 1 });
pinned_value!(ArchiveSourceVersion { V0 = 0, V1 = 1 });
pinned_value!(ArchivedAcceptanceKind {
    SupportedPersisted = 0,
    LegacyComplete = 1,
    LegacyUnavailable = 2,
    NotAccepted = 3,
});
pinned_value!(InstalledAcceptedWorkspaceKind { V1 = 0 });
pinned_value!(LegacyAcceptanceSource { Candidate = 0, BaselineNoPublication = 1 });
pinned_value!(LegacyAcceptanceGap {
    ExactLockBytes = 0,
    CompleteMemberAudit = 1,
    AcceptedRootInput = 2,
    PublicationEvidence = 3,
});
pinned_value!(AcceptedMemberKind { Selected = 0, UnselectedPresent = 1, Absent = 2 });
pinned_value!(AcceptedRootKind { BornAttached = 0, BornDetached = 1, UnbornAttached = 2 });
pinned_value!(AcceptedMetadataSource { OperationBaseline = 0, SelectedRootResult = 1 });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2_archive_discriminants_are_pinned() {
        assert_eq!(
            [
                ArchiveSourceVersion::V0.pinned_value(),
                ArchiveSourceVersion::V1.pinned_value()
            ],
            [0, 1]
        );
        assert_eq!(
            [
                ArchivedTerminalOutcome::Completed.pinned_value(),
                ArchivedTerminalOutcome::Aborted.pinned_value()
            ],
            [0, 1]
        );
        assert_eq!(
            [
                ArchivedAcceptanceKind::SupportedPersisted.pinned_value(),
                ArchivedAcceptanceKind::LegacyComplete.pinned_value(),
                ArchivedAcceptanceKind::LegacyUnavailable.pinned_value(),
                ArchivedAcceptanceKind::NotAccepted.pinned_value(),
            ],
            [0, 1, 2, 3]
        );
        assert_eq!(InstalledAcceptedWorkspaceKind::V1.pinned_value(), 0);
        assert_eq!(
            [
                LegacyAcceptanceSource::Candidate.pinned_value(),
                LegacyAcceptanceSource::BaselineNoPublication.pinned_value()
            ],
            [0, 1]
        );
        assert_eq!(
            [
                LegacyAcceptanceGap::ExactLockBytes.pinned_value(),
                LegacyAcceptanceGap::CompleteMemberAudit.pinned_value(),
                LegacyAcceptanceGap::AcceptedRootInput.pinned_value(),
                LegacyAcceptanceGap::PublicationEvidence.pinned_value(),
            ],
            [0, 1, 2, 3]
        );
        assert_eq!(
            [
                AcceptedMemberKind::Selected.pinned_value(),
                AcceptedMemberKind::UnselectedPresent.pinned_value(),
                AcceptedMemberKind::Absent.pinned_value()
            ],
            [0, 1, 2]
        );
        assert_eq!(
            [
                AcceptedRootKind::BornAttached.pinned_value(),
                AcceptedRootKind::BornDetached.pinned_value(),
                AcceptedRootKind::UnbornAttached.pinned_value()
            ],
            [0, 1, 2]
        );
        assert_eq!(
            [
                AcceptedMetadataSource::OperationBaseline.pinned_value(),
                AcceptedMetadataSource::SelectedRootResult.pinned_value()
            ],
            [0, 1]
        );
    }
}
