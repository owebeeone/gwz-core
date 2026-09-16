//! Root-preservation vocabulary: the working-tree image that preservation
//! captures, the specification it works towards, and the physical steps,
//! guards and observations that carry it there.

use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GitPreservationDirtySummary {
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPreservationImage {
    pub preimage_sha256: String,
    pub dirty: GitPreservationDirtySummary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPreservationStashEvidence {
    pub object_id: String,
    pub message: String,
    pub head_commit: String,
    pub image: GitPreservationImage,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRootPreservationSpec {
    pub attached_branch: String,
    pub attached_commit: String,
    pub restore_commit: String,
    pub managed_marker_path: String,
    pub attached_clean_form: GitRootManagedForm,
    pub restore_clean_form: GitRootManagedForm,
    pub handoff_form: GitRootManagedForm,
    pub handoff_boundary: Vec<u8>,
    /// Nested member roots are never part of the root repository checkout,
    /// even while the publication boundary itself is being normalized.
    pub excluded_worktree_paths: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitRootPreservationPhysicalStep {
    Managed(GitRootManagedTransition),
    CreateStash { merge_id: String },
    ResetAttachedRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitRootPreservationGuard {
    NormalizedPreimage { sha256: String },
    OtherwiseClean,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitRootPreservationStepObservation {
    Before,
    After,
    AfterNeedsDurability,
    Ambiguous,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitCheckedPreservationMutation {
    Applied,
    AlreadyComplete,
    StashCreated(GitStashPushResult),
    RefReset(GitUpdateResult),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPreparedRootStash {
    pub normalized_image: GitPreservationImage,
}
