// GENERATED native Rust types + codec — do not edit.
#![allow(dead_code)]
use crate::cbor::{Cbor, DecodeError};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ActionKind {
    #[default] CreateWorkspace,
    InitFromSources,
    AddExistingRepo,
    CreateRepo,
    Materialize,
    Status,
    Snapshot,
    Tag,
    PullHead,
    PullSnapshot,
    Push,
    Capture,
    Commit,
    Stage,
    Ls,
    Forall,
    RepoSync,
    Stash,
    Branch,
    CloneWorkspace,
    ListSnapshots,
    Diff,
    CloneRepoMember,
    DetachRepoMember,
    AttachRepoMember,
    Merge,
}
impl ActionKind {
    pub fn wire(self) -> i64 { match self {
        Self::CreateWorkspace => 0,
        Self::InitFromSources => 1,
        Self::AddExistingRepo => 2,
        Self::CreateRepo => 3,
        Self::Materialize => 4,
        Self::Status => 5,
        Self::Snapshot => 6,
        Self::Tag => 7,
        Self::PullHead => 8,
        Self::PullSnapshot => 9,
        Self::Push => 10,
        Self::Capture => 11,
        Self::Commit => 12,
        Self::Stage => 13,
        Self::Ls => 14,
        Self::Forall => 15,
        Self::RepoSync => 16,
        Self::Stash => 17,
        Self::Branch => 18,
        Self::CloneWorkspace => 19,
        Self::ListSnapshots => 20,
        Self::Diff => 21,
        Self::CloneRepoMember => 22,
        Self::DetachRepoMember => 23,
        Self::AttachRepoMember => 24,
        Self::Merge => 25,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::CreateWorkspace,
        1 => Self::InitFromSources,
        2 => Self::AddExistingRepo,
        3 => Self::CreateRepo,
        4 => Self::Materialize,
        5 => Self::Status,
        6 => Self::Snapshot,
        7 => Self::Tag,
        8 => Self::PullHead,
        9 => Self::PullSnapshot,
        10 => Self::Push,
        11 => Self::Capture,
        12 => Self::Commit,
        13 => Self::Stage,
        14 => Self::Ls,
        15 => Self::Forall,
        16 => Self::RepoSync,
        17 => Self::Stash,
        18 => Self::Branch,
        19 => Self::CloneWorkspace,
        20 => Self::ListSnapshots,
        21 => Self::Diff,
        22 => Self::CloneRepoMember,
        23 => Self::DetachRepoMember,
        24 => Self::AttachRepoMember,
        25 => Self::Merge,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "ActionKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum TagOp {
    #[default] Create,
    List,
    Fetch,
    Push,
    Delete,
}
impl TagOp {
    pub fn wire(self) -> i64 { match self {
        Self::Create => 0,
        Self::List => 1,
        Self::Fetch => 2,
        Self::Push => 3,
        Self::Delete => 4,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Create,
        1 => Self::List,
        2 => Self::Fetch,
        3 => Self::Push,
        4 => Self::Delete,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "TagOp", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum StashOp {
    #[default] Push,
    List,
    Apply,
    Pop,
    Drop,
}
impl StashOp {
    pub fn wire(self) -> i64 { match self {
        Self::Push => 0,
        Self::List => 1,
        Self::Apply => 2,
        Self::Pop => 3,
        Self::Drop => 4,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Push,
        1 => Self::List,
        2 => Self::Apply,
        3 => Self::Pop,
        4 => Self::Drop,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "StashOp", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum StashParticipation {
    #[default] Stashed,
    Empty,
    Skipped,
}
impl StashParticipation {
    pub fn wire(self) -> i64 { match self {
        Self::Stashed => 0,
        Self::Empty => 1,
        Self::Skipped => 2,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Stashed,
        1 => Self::Empty,
        2 => Self::Skipped,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "StashParticipation", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum StashPushLifecycle {
    #[default] Unattempted,
    Saving,
    Saved,
    Empty,
    Failed,
}
impl StashPushLifecycle {
    pub fn wire(self) -> i64 { match self {
        Self::Unattempted => 0,
        Self::Saving => 1,
        Self::Saved => 2,
        Self::Empty => 3,
        Self::Failed => 4,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Unattempted,
        1 => Self::Saving,
        2 => Self::Saved,
        3 => Self::Empty,
        4 => Self::Failed,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "StashPushLifecycle", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum StashRestoreState {
    #[default] Pending,
    Applied,
    Popped,
    Dropped,
    Noop,
    Missing,
}
impl StashRestoreState {
    pub fn wire(self) -> i64 { match self {
        Self::Pending => 0,
        Self::Applied => 1,
        Self::Popped => 2,
        Self::Dropped => 3,
        Self::Noop => 4,
        Self::Missing => 5,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Pending,
        1 => Self::Applied,
        2 => Self::Popped,
        3 => Self::Dropped,
        4 => Self::Noop,
        5 => Self::Missing,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "StashRestoreState", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BranchOp {
    #[default] List,
    Create,
    Delete,
    Merge,
}
impl BranchOp {
    pub fn wire(self) -> i64 { match self {
        Self::List => 0,
        Self::Create => 1,
        Self::Delete => 2,
        Self::Merge => 3,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::List,
        1 => Self::Create,
        2 => Self::Delete,
        3 => Self::Merge,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "BranchOp", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergeOp {
    #[default] Start,
    Resume,
    Abort,
    Status,
    Gc,
}
impl MergeOp {
    pub fn wire(self) -> i64 { match self {
        Self::Start => 0,
        Self::Resume => 1,
        Self::Abort => 2,
        Self::Status => 3,
        Self::Gc => 4,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Start,
        1 => Self::Resume,
        2 => Self::Abort,
        3 => Self::Status,
        4 => Self::Gc,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergeOp", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergeMode {
    #[default] Normal,
    FfOnly,
    NoFf,
}
impl MergeMode {
    pub fn wire(self) -> i64 { match self {
        Self::Normal => 0,
        Self::FfOnly => 1,
        Self::NoFf => 2,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Normal,
        1 => Self::FfOnly,
        2 => Self::NoFf,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergeMode", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergeAnalysisKind {
    #[default] UpToDate,
    FastForward,
    TrueMerge,
    Unknown,
}
impl MergeAnalysisKind {
    pub fn wire(self) -> i64 { match self {
        Self::UpToDate => 0,
        Self::FastForward => 1,
        Self::TrueMerge => 2,
        Self::Unknown => 3,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::UpToDate,
        1 => Self::FastForward,
        2 => Self::TrueMerge,
        3 => Self::Unknown,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergeAnalysisKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergePendingActionKind {
    #[default] VerifyUpToDate,
    FastForward,
    TrueMerge,
    ResolveConflict,
}
impl MergePendingActionKind {
    pub fn wire(self) -> i64 { match self {
        Self::VerifyUpToDate => 0,
        Self::FastForward => 1,
        Self::TrueMerge => 2,
        Self::ResolveConflict => 3,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::VerifyUpToDate,
        1 => Self::FastForward,
        2 => Self::TrueMerge,
        3 => Self::ResolveConflict,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergePendingActionKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergePendingActionState {
    #[default] NotStarted,
    ExpectedConflict,
    CompletedExactly,
    Ambiguous,
}
impl MergePendingActionState {
    pub fn wire(self) -> i64 { match self {
        Self::NotStarted => 0,
        Self::ExpectedConflict => 1,
        Self::CompletedExactly => 2,
        Self::Ambiguous => 3,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::NotStarted,
        1 => Self::ExpectedConflict,
        2 => Self::CompletedExactly,
        3 => Self::Ambiguous,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergePendingActionState", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergeParticipantState {
    #[default] Planned,
    UpToDate,
    FastForwarded,
    Merged,
    Conflicted,
    Failed,
    Unattempted,
    Continued,
    Aborted,
    RolledBack,
}
impl MergeParticipantState {
    pub fn wire(self) -> i64 { match self {
        Self::Planned => 0,
        Self::UpToDate => 1,
        Self::FastForwarded => 2,
        Self::Merged => 3,
        Self::Conflicted => 4,
        Self::Failed => 5,
        Self::Unattempted => 6,
        Self::Continued => 7,
        Self::Aborted => 8,
        Self::RolledBack => 9,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Planned,
        1 => Self::UpToDate,
        2 => Self::FastForwarded,
        3 => Self::Merged,
        4 => Self::Conflicted,
        5 => Self::Failed,
        6 => Self::Unattempted,
        7 => Self::Continued,
        8 => Self::Aborted,
        9 => Self::RolledBack,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergeParticipantState", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergeOperationState {
    #[default] Executing,
    AwaitingResolution,
    Halted,
    Finalizing,
    Preserving,
    RollingBack,
    Completed,
    Aborted,
    RecoveryRequired,
    Idle,
}
impl MergeOperationState {
    pub fn wire(self) -> i64 { match self {
        Self::Executing => 0,
        Self::AwaitingResolution => 1,
        Self::Halted => 2,
        Self::Finalizing => 3,
        Self::Preserving => 4,
        Self::RollingBack => 5,
        Self::Completed => 6,
        Self::Aborted => 7,
        Self::RecoveryRequired => 8,
        Self::Idle => 9,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Executing,
        1 => Self::AwaitingResolution,
        2 => Self::Halted,
        3 => Self::Finalizing,
        4 => Self::Preserving,
        5 => Self::RollingBack,
        6 => Self::Completed,
        7 => Self::Aborted,
        8 => Self::RecoveryRequired,
        9 => Self::Idle,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergeOperationState", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergeParticipantDriftKind {
    #[default] BranchChanged,
    HeadAdvanced,
    HeadRewound,
    TargetRefChanged,
    WorktreeModified,
    IndexModified,
    MergeStateMissing,
    MergeHeadChanged,
    NewIntegrationState,
    RepositoryMissing,
    HeadDiverged,
    ObjectMissing,
    ForeignIntegrationState,
    PendingActionAmbiguous,
}
impl MergeParticipantDriftKind {
    pub fn wire(self) -> i64 { match self {
        Self::BranchChanged => 0,
        Self::HeadAdvanced => 1,
        Self::HeadRewound => 2,
        Self::TargetRefChanged => 3,
        Self::WorktreeModified => 4,
        Self::IndexModified => 5,
        Self::MergeStateMissing => 6,
        Self::MergeHeadChanged => 7,
        Self::NewIntegrationState => 8,
        Self::RepositoryMissing => 9,
        Self::HeadDiverged => 10,
        Self::ObjectMissing => 11,
        Self::ForeignIntegrationState => 12,
        Self::PendingActionAmbiguous => 13,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::BranchChanged,
        1 => Self::HeadAdvanced,
        2 => Self::HeadRewound,
        3 => Self::TargetRefChanged,
        4 => Self::WorktreeModified,
        5 => Self::IndexModified,
        6 => Self::MergeStateMissing,
        7 => Self::MergeHeadChanged,
        8 => Self::NewIntegrationState,
        9 => Self::RepositoryMissing,
        10 => Self::HeadDiverged,
        11 => Self::ObjectMissing,
        12 => Self::ForeignIntegrationState,
        13 => Self::PendingActionAmbiguous,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergeParticipantDriftKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergeOperationDriftKind {
    #[default] BaselineLockChanged,
    BaselineManifestChanged,
    RootCandidateMetadataInvalid,
    RootCandidateStateChanged,
    RecordUnreadable,
}
impl MergeOperationDriftKind {
    pub fn wire(self) -> i64 { match self {
        Self::BaselineLockChanged => 0,
        Self::BaselineManifestChanged => 1,
        Self::RootCandidateMetadataInvalid => 2,
        Self::RootCandidateStateChanged => 3,
        Self::RecordUnreadable => 4,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::BaselineLockChanged,
        1 => Self::BaselineManifestChanged,
        2 => Self::RootCandidateMetadataInvalid,
        3 => Self::RootCandidateStateChanged,
        4 => Self::RecordUnreadable,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergeOperationDriftKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MergePublicationStep {
    #[default] NotStarted,
    ValidatingResults,
    PreparingCandidate,
    CommittingEvidence,
    PublishingCandidate,
    VerifyingPublication,
    Complete,
}
impl MergePublicationStep {
    pub fn wire(self) -> i64 { match self {
        Self::NotStarted => 0,
        Self::ValidatingResults => 1,
        Self::PreparingCandidate => 2,
        Self::CommittingEvidence => 3,
        Self::PublishingCandidate => 4,
        Self::VerifyingPublication => 5,
        Self::Complete => 6,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::NotStarted,
        1 => Self::ValidatingResults,
        2 => Self::PreparingCandidate,
        3 => Self::CommittingEvidence,
        4 => Self::PublishingCandidate,
        5 => Self::VerifyingPublication,
        6 => Self::Complete,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MergePublicationStep", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BranchActionResult {
    #[default] Listed,
    Created,
    Exists,
    Deleted,
    Switched,
    Noop,
    Skipped,
    Merged,
    Conflicted,
}
impl BranchActionResult {
    pub fn wire(self) -> i64 { match self {
        Self::Listed => 0,
        Self::Created => 1,
        Self::Exists => 2,
        Self::Deleted => 3,
        Self::Switched => 4,
        Self::Noop => 5,
        Self::Skipped => 6,
        Self::Merged => 7,
        Self::Conflicted => 8,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Listed,
        1 => Self::Created,
        2 => Self::Exists,
        3 => Self::Deleted,
        4 => Self::Switched,
        5 => Self::Noop,
        6 => Self::Skipped,
        7 => Self::Merged,
        8 => Self::Conflicted,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "BranchActionResult", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ExecMode {
    #[default] Argv,
    Shell,
}
impl ExecMode {
    pub fn wire(self) -> i64 { match self {
        Self::Argv => 0,
        Self::Shell => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Argv,
        1 => Self::Shell,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "ExecMode", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SourceKind {
    #[default] Git,
    Archive,
    Package,
    Local,
    Generated,
}
impl SourceKind {
    pub fn wire(self) -> i64 { match self {
        Self::Git => 0,
        Self::Archive => 1,
        Self::Package => 2,
        Self::Local => 3,
        Self::Generated => 4,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Git,
        1 => Self::Archive,
        2 => Self::Package,
        3 => Self::Local,
        4 => Self::Generated,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "SourceKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum TargetKind {
    #[default] Root,
    Member,
}
impl TargetKind {
    pub fn wire(self) -> i64 { match self {
        Self::Root => 0,
        Self::Member => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Root,
        1 => Self::Member,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "TargetKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum AggregateStatus {
    #[default] Accepted,
    Ok,
    Noop,
    Rejected,
    Partial,
    Failed,
    Dirty,
    Conflicted,
}
impl AggregateStatus {
    pub fn wire(self) -> i64 { match self {
        Self::Accepted => 0,
        Self::Ok => 1,
        Self::Noop => 2,
        Self::Rejected => 3,
        Self::Partial => 4,
        Self::Failed => 5,
        Self::Dirty => 6,
        Self::Conflicted => 7,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Accepted,
        1 => Self::Ok,
        2 => Self::Noop,
        3 => Self::Rejected,
        4 => Self::Partial,
        5 => Self::Failed,
        6 => Self::Dirty,
        7 => Self::Conflicted,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "AggregateStatus", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MemberStatus {
    #[default] Planned,
    Ok,
    Noop,
    Skipped,
    Rejected,
    Failed,
    Conflicted,
}
impl MemberStatus {
    pub fn wire(self) -> i64 { match self {
        Self::Planned => 0,
        Self::Ok => 1,
        Self::Noop => 2,
        Self::Skipped => 3,
        Self::Rejected => 4,
        Self::Failed => 5,
        Self::Conflicted => 6,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Planned,
        1 => Self::Ok,
        2 => Self::Noop,
        3 => Self::Skipped,
        4 => Self::Rejected,
        5 => Self::Failed,
        6 => Self::Conflicted,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MemberStatus", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MaterializeTargetKind {
    #[default] Lock,
    Head,
    Snapshot,
    Tag,
    Commit,
    Branch,
}
impl MaterializeTargetKind {
    pub fn wire(self) -> i64 { match self {
        Self::Lock => 0,
        Self::Head => 1,
        Self::Snapshot => 2,
        Self::Tag => 3,
        Self::Commit => 4,
        Self::Branch => 5,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Lock,
        1 => Self::Head,
        2 => Self::Snapshot,
        3 => Self::Tag,
        4 => Self::Commit,
        5 => Self::Branch,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "MaterializeTargetKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SnapshotSourceKind {
    #[default] Current,
    Branch,
}
impl SnapshotSourceKind {
    pub fn wire(self) -> i64 { match self {
        Self::Current => 0,
        Self::Branch => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Current,
        1 => Self::Branch,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "SnapshotSourceKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SyncBehavior {
    #[default] FetchOnly,
    FfOnly,
    Merge,
    Rebase,
    Reset,
    DriverSelected,
}
impl SyncBehavior {
    pub fn wire(self) -> i64 { match self {
        Self::FetchOnly => 0,
        Self::FfOnly => 1,
        Self::Merge => 2,
        Self::Rebase => 3,
        Self::Reset => 4,
        Self::DriverSelected => 5,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::FetchOnly,
        1 => Self::FfOnly,
        2 => Self::Merge,
        3 => Self::Rebase,
        4 => Self::Reset,
        5 => Self::DriverSelected,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "SyncBehavior", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum PartialBehavior {
    #[default] Atomic,
    Partial,
}
impl PartialBehavior {
    pub fn wire(self) -> i64 { match self {
        Self::Atomic => 0,
        Self::Partial => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Atomic,
        1 => Self::Partial,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "PartialBehavior", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DestructiveBehavior {
    #[default] Refuse,
    Allow,
}
impl DestructiveBehavior {
    pub fn wire(self) -> i64 { match self {
        Self::Refuse => 0,
        Self::Allow => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Refuse,
        1 => Self::Allow,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DestructiveBehavior", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum UnsupportedMemberBehavior {
    #[default] Fail,
    Skip,
}
impl UnsupportedMemberBehavior {
    pub fn wire(self) -> i64 { match self {
        Self::Fail => 0,
        Self::Skip => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Fail,
        1 => Self::Skip,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "UnsupportedMemberBehavior", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum PlannedAction {
    #[default] Noop,
    Clone,
    Fetch,
    FastForward,
    Checkout,
    InitRepo,
    AddManifestMember,
    WriteManifest,
    WriteLock,
    WriteSnapshot,
    WriteTag,
    Push,
    Merge,
    Rebase,
    Reset,
    DetachMember,
    AttachMember,
}
impl PlannedAction {
    pub fn wire(self) -> i64 { match self {
        Self::Noop => 0,
        Self::Clone => 1,
        Self::Fetch => 2,
        Self::FastForward => 3,
        Self::Checkout => 4,
        Self::InitRepo => 5,
        Self::AddManifestMember => 6,
        Self::WriteManifest => 7,
        Self::WriteLock => 8,
        Self::WriteSnapshot => 9,
        Self::WriteTag => 10,
        Self::Push => 11,
        Self::Merge => 12,
        Self::Rebase => 13,
        Self::Reset => 14,
        Self::DetachMember => 15,
        Self::AttachMember => 16,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Noop,
        1 => Self::Clone,
        2 => Self::Fetch,
        3 => Self::FastForward,
        4 => Self::Checkout,
        5 => Self::InitRepo,
        6 => Self::AddManifestMember,
        7 => Self::WriteManifest,
        8 => Self::WriteLock,
        9 => Self::WriteSnapshot,
        10 => Self::WriteTag,
        11 => Self::Push,
        12 => Self::Merge,
        13 => Self::Rebase,
        14 => Self::Reset,
        15 => Self::DetachMember,
        16 => Self::AttachMember,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "PlannedAction", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum LockMatch {
    #[default] Unknown,
    Matches,
    Differs,
    Missing,
}
impl LockMatch {
    pub fn wire(self) -> i64 { match self {
        Self::Unknown => 0,
        Self::Matches => 1,
        Self::Differs => 2,
        Self::Missing => 3,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Unknown,
        1 => Self::Matches,
        2 => Self::Differs,
        3 => Self::Missing,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "LockMatch", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum GitProgressPhase {
    #[default] Enumerating,
    Counting,
    Compressing,
    Receiving,
    Resolving,
    CheckingOut,
    Writing,
}
impl GitProgressPhase {
    pub fn wire(self) -> i64 { match self {
        Self::Enumerating => 0,
        Self::Counting => 1,
        Self::Compressing => 2,
        Self::Receiving => 3,
        Self::Resolving => 4,
        Self::CheckingOut => 5,
        Self::Writing => 6,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Enumerating,
        1 => Self::Counting,
        2 => Self::Compressing,
        3 => Self::Receiving,
        4 => Self::Resolving,
        5 => Self::CheckingOut,
        6 => Self::Writing,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "GitProgressPhase", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum StatusMode {
    #[default] Summary,
    Combined,
}
impl StatusMode {
    pub fn wire(self) -> i64 { match self {
        Self::Summary => 0,
        Self::Combined => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Summary,
        1 => Self::Combined,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "StatusMode", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum StatusPathStyle {
    #[default] MemberRelative,
    WorkspaceRelative,
}
impl StatusPathStyle {
    pub fn wire(self) -> i64 { match self {
        Self::MemberRelative => 0,
        Self::WorkspaceRelative => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::MemberRelative,
        1 => Self::WorkspaceRelative,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "StatusPathStyle", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum EventKind {
    #[default] OperationStarted,
    MemberStarted,
    MemberProgress,
    MemberFinished,
    ArtifactWritten,
    OperationFinished,
    Reset,
    OperationStateChanged,
}
impl EventKind {
    pub fn wire(self) -> i64 { match self {
        Self::OperationStarted => 0,
        Self::MemberStarted => 1,
        Self::MemberProgress => 2,
        Self::MemberFinished => 3,
        Self::ArtifactWritten => 4,
        Self::OperationFinished => 5,
        Self::Reset => 6,
        Self::OperationStateChanged => 7,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::OperationStarted,
        1 => Self::MemberStarted,
        2 => Self::MemberProgress,
        3 => Self::MemberFinished,
        4 => Self::ArtifactWritten,
        5 => Self::OperationFinished,
        6 => Self::Reset,
        7 => Self::OperationStateChanged,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "EventKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Severity {
    #[default] Debug,
    Info,
    Warn,
    Error,
}
impl Severity {
    pub fn wire(self) -> i64 { match self {
        Self::Debug => 0,
        Self::Info => 1,
        Self::Warn => 2,
        Self::Error => 3,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Debug,
        1 => Self::Info,
        2 => Self::Warn,
        3 => Self::Error,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "Severity", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum GwzErrorCode {
    #[default] Ok,
    InvalidRequest,
    WorkspaceNotFound,
    WorkspaceAlreadyExists,
    NestedWorkspace,
    ManifestNotFound,
    ManifestInvalid,
    SchemaUnsupported,
    MemberNotFound,
    MemberInactive,
    PathEscape,
    PathCollision,
    PathReserved,
    UnsupportedSourceKind,
    UnsupportedOperation,
    DirtyMember,
    DivergedMember,
    MissingRemote,
    SnapshotNotFound,
    LockNotFound,
    TagNotFound,
    TagInvalid,
    RemoteRejected,
    GitCommandFailed,
    ExternalToolMissing,
    OperationNotFound,
    AttributionDenied,
    PermissionDenied,
    IoError,
    InternalError,
    BranchDetachedHead,
    BranchUnbornHead,
    BranchMixed,
    StashNotFound,
    StashIncomplete,
    StashConflict,
    SourceIdentityMismatch,
    DeprecatedOperation,
    MergeValidationFailed,
    MergeIdMismatch,
    MergeDrift,
    OpenOperation,
    MergeRecoveryRequired,
    MergePhaseUnsupported,
    RootMergeNotYetSupported,
    MergeRecordUnreadable,
}
impl GwzErrorCode {
    pub fn wire(self) -> i64 { match self {
        Self::Ok => 0,
        Self::InvalidRequest => 1,
        Self::WorkspaceNotFound => 2,
        Self::WorkspaceAlreadyExists => 3,
        Self::NestedWorkspace => 4,
        Self::ManifestNotFound => 5,
        Self::ManifestInvalid => 6,
        Self::SchemaUnsupported => 7,
        Self::MemberNotFound => 8,
        Self::MemberInactive => 9,
        Self::PathEscape => 10,
        Self::PathCollision => 11,
        Self::PathReserved => 12,
        Self::UnsupportedSourceKind => 13,
        Self::UnsupportedOperation => 14,
        Self::DirtyMember => 15,
        Self::DivergedMember => 16,
        Self::MissingRemote => 17,
        Self::SnapshotNotFound => 18,
        Self::LockNotFound => 19,
        Self::TagNotFound => 20,
        Self::TagInvalid => 21,
        Self::RemoteRejected => 22,
        Self::GitCommandFailed => 23,
        Self::ExternalToolMissing => 24,
        Self::OperationNotFound => 25,
        Self::AttributionDenied => 26,
        Self::PermissionDenied => 27,
        Self::IoError => 28,
        Self::InternalError => 29,
        Self::BranchDetachedHead => 30,
        Self::BranchUnbornHead => 31,
        Self::BranchMixed => 32,
        Self::StashNotFound => 33,
        Self::StashIncomplete => 34,
        Self::StashConflict => 35,
        Self::SourceIdentityMismatch => 36,
        Self::DeprecatedOperation => 37,
        Self::MergeValidationFailed => 38,
        Self::MergeIdMismatch => 39,
        Self::MergeDrift => 40,
        Self::OpenOperation => 41,
        Self::MergeRecoveryRequired => 42,
        Self::MergePhaseUnsupported => 43,
        Self::RootMergeNotYetSupported => 44,
        Self::MergeRecordUnreadable => 45,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Ok,
        1 => Self::InvalidRequest,
        2 => Self::WorkspaceNotFound,
        3 => Self::WorkspaceAlreadyExists,
        4 => Self::NestedWorkspace,
        5 => Self::ManifestNotFound,
        6 => Self::ManifestInvalid,
        7 => Self::SchemaUnsupported,
        8 => Self::MemberNotFound,
        9 => Self::MemberInactive,
        10 => Self::PathEscape,
        11 => Self::PathCollision,
        12 => Self::PathReserved,
        13 => Self::UnsupportedSourceKind,
        14 => Self::UnsupportedOperation,
        15 => Self::DirtyMember,
        16 => Self::DivergedMember,
        17 => Self::MissingRemote,
        18 => Self::SnapshotNotFound,
        19 => Self::LockNotFound,
        20 => Self::TagNotFound,
        21 => Self::TagInvalid,
        22 => Self::RemoteRejected,
        23 => Self::GitCommandFailed,
        24 => Self::ExternalToolMissing,
        25 => Self::OperationNotFound,
        26 => Self::AttributionDenied,
        27 => Self::PermissionDenied,
        28 => Self::IoError,
        29 => Self::InternalError,
        30 => Self::BranchDetachedHead,
        31 => Self::BranchUnbornHead,
        32 => Self::BranchMixed,
        33 => Self::StashNotFound,
        34 => Self::StashIncomplete,
        35 => Self::StashConflict,
        36 => Self::SourceIdentityMismatch,
        37 => Self::DeprecatedOperation,
        38 => Self::MergeValidationFailed,
        39 => Self::MergeIdMismatch,
        40 => Self::MergeDrift,
        41 => Self::OpenOperation,
        42 => Self::MergeRecoveryRequired,
        43 => Self::MergePhaseUnsupported,
        44 => Self::RootMergeNotYetSupported,
        45 => Self::MergeRecordUnreadable,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "GwzErrorCode", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DiffComparisonKind {
    #[default] WorktreeVsIndex,
    IndexVsTree,
    WorktreeVsTree,
    TreeVsTree,
}
impl DiffComparisonKind {
    pub fn wire(self) -> i64 { match self {
        Self::WorktreeVsIndex => 0,
        Self::IndexVsTree => 1,
        Self::WorktreeVsTree => 2,
        Self::TreeVsTree => 3,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::WorktreeVsIndex,
        1 => Self::IndexVsTree,
        2 => Self::WorktreeVsTree,
        3 => Self::TreeVsTree,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DiffComparisonKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DiffOutputFormat {
    #[default] Patch,
    Raw,
    NameOnly,
    NameStatus,
    Stat,
    Numstat,
    Shortstat,
    Summary,
    PatchWithRaw,
    PatchWithStat,
    NoPatch,
}
impl DiffOutputFormat {
    pub fn wire(self) -> i64 { match self {
        Self::Patch => 0,
        Self::Raw => 1,
        Self::NameOnly => 2,
        Self::NameStatus => 3,
        Self::Stat => 4,
        Self::Numstat => 5,
        Self::Shortstat => 6,
        Self::Summary => 7,
        Self::PatchWithRaw => 8,
        Self::PatchWithStat => 9,
        Self::NoPatch => 10,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Patch,
        1 => Self::Raw,
        2 => Self::NameOnly,
        3 => Self::NameStatus,
        4 => Self::Stat,
        5 => Self::Numstat,
        6 => Self::Shortstat,
        7 => Self::Summary,
        8 => Self::PatchWithRaw,
        9 => Self::PatchWithStat,
        10 => Self::NoPatch,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DiffOutputFormat", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DiffManifestMode {
    #[default] Full,
    AnyDifference,
}
impl DiffManifestMode {
    pub fn wire(self) -> i64 { match self {
        Self::Full => 0,
        Self::AnyDifference => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Full,
        1 => Self::AnyDifference,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DiffManifestMode", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DiffAlgorithm {
    #[default] Default,
    Myers,
    Minimal,
    Patience,
}
impl DiffAlgorithm {
    pub fn wire(self) -> i64 { match self {
        Self::Default => 0,
        Self::Myers => 1,
        Self::Minimal => 2,
        Self::Patience => 3,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Default,
        1 => Self::Myers,
        2 => Self::Minimal,
        3 => Self::Patience,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DiffAlgorithm", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DiffWhitespaceMode {
    #[default] Default,
    IgnoreAll,
    IgnoreChange,
    IgnoreEol,
    IgnoreBlankLines,
}
impl DiffWhitespaceMode {
    pub fn wire(self) -> i64 { match self {
        Self::Default => 0,
        Self::IgnoreAll => 1,
        Self::IgnoreChange => 2,
        Self::IgnoreEol => 3,
        Self::IgnoreBlankLines => 4,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Default,
        1 => Self::IgnoreAll,
        2 => Self::IgnoreChange,
        3 => Self::IgnoreEol,
        4 => Self::IgnoreBlankLines,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DiffWhitespaceMode", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DiffStatus {
    #[default] Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
}
impl DiffStatus {
    pub fn wire(self) -> i64 { match self {
        Self::Added => 0,
        Self::Modified => 1,
        Self::Deleted => 2,
        Self::Renamed => 3,
        Self::Copied => 4,
        Self::TypeChanged => 5,
        Self::Unmerged => 6,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Added,
        1 => Self::Modified,
        2 => Self::Deleted,
        3 => Self::Renamed,
        4 => Self::Copied,
        5 => Self::TypeChanged,
        6 => Self::Unmerged,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DiffStatus", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DiffChunkEncoding {
    #[default] Utf8,
    Bytes,
}
impl DiffChunkEncoding {
    pub fn wire(self) -> i64 { match self {
        Self::Utf8 => 0,
        Self::Bytes => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Utf8,
        1 => Self::Bytes,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DiffChunkEncoding", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DiffOutputRecordKind {
    #[default] PatchBytes,
    FileStarted,
    FileFinished,
    StaleFile,
    Diagnostic,
}
impl DiffOutputRecordKind {
    pub fn wire(self) -> i64 { match self {
        Self::PatchBytes => 0,
        Self::FileStarted => 1,
        Self::FileFinished => 2,
        Self::StaleFile => 3,
        Self::Diagnostic => 4,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::PatchBytes,
        1 => Self::FileStarted,
        2 => Self::FileFinished,
        3 => Self::StaleFile,
        4 => Self::Diagnostic,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DiffOutputRecordKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DiffTargetExclusionReason {
    #[default] SnapshotMissing,
    SnapshotMissingCommit,
    RootNotInSnapshot,
}
impl DiffTargetExclusionReason {
    pub fn wire(self) -> i64 { match self {
        Self::SnapshotMissing => 0,
        Self::SnapshotMissingCommit => 1,
        Self::RootNotInSnapshot => 2,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::SnapshotMissing,
        1 => Self::SnapshotMissingCommit,
        2 => Self::RootNotInSnapshot,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "DiffTargetExclusionReason", value: v }),
    }) }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct WorkspaceRef {
    pub root: Option<String>,
    pub workspace_id: Option<String>,
}
impl WorkspaceRef {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, match &self.root { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (2, match &self.workspace_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            root: { let v = c.try_get(1)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            workspace_id: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct OperationActor {
    pub actor_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub authority: Option<String>,
}
impl OperationActor {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.actor_id.clone())),
            (2, match &self.display_name { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, match &self.email { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.authority { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            actor_id: c.try_get(1)?.try_text()?,
            display_name: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            email: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            authority: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GitObjectIdentity {
    pub name: String,
    pub email: String,
    pub time_ms: Option<i64>,
    pub timezone_offset_minutes: Option<i64>,
}
impl GitObjectIdentity {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.name.clone())),
            (2, Cbor::Text(self.email.clone())),
            (3, match &self.time_ms { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (4, match &self.timezone_offset_minutes { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            name: c.try_get(1)?.try_text()?,
            email: c.try_get(2)?.try_text()?,
            time_ms: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            timezone_offset_minutes: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_int()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct OperationAttribution {
    pub actor: Option<OperationActor>,
    pub git_author: Option<GitObjectIdentity>,
    pub git_committer: Option<GitObjectIdentity>,
    pub credential_ref: Option<String>,
}
impl OperationAttribution {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, match &self.actor { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (2, match &self.git_author { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (3, match &self.git_committer { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (4, match &self.credential_ref { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            actor: { let v = c.try_get(1)?; if v.is_null() { None } else { Some(OperationActor::from_cbor(v)?) } },
            git_author: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(GitObjectIdentity::from_cbor(v)?) } },
            git_committer: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(GitObjectIdentity::from_cbor(v)?) } },
            credential_ref: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Selection {
    pub all: Option<bool>,
    pub member_ids: Vec<String>,
    pub paths: Vec<String>,
    pub targets: Vec<String>,
    pub exclude_targets: Vec<String>,
}
impl Selection {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, match &self.all { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (2, Cbor::Array(self.member_ids.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (3, Cbor::Array(self.paths.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (4, Cbor::Array(self.targets.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (5, Cbor::Array(self.exclude_targets.iter().map(|x| Cbor::Text(x.clone())).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            all: { let v = c.try_get(1)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            member_ids: c.try_get(2)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            paths: c.try_get(3)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            targets: c.try_get(4)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            exclude_targets: c.try_get(5)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct OperationPolicy {
    pub partial: Option<PartialBehavior>,
    pub destructive: Option<DestructiveBehavior>,
    pub sync: Option<SyncBehavior>,
    pub unsupported_member: Option<UnsupportedMemberBehavior>,
    pub remote: Option<String>,
    pub concurrency: Option<i64>,
    pub progress_min_interval_ms: Option<i64>,
    pub max_connections_per_host: Option<i64>,
}
impl OperationPolicy {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, match &self.partial { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (2, match &self.destructive { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (3, match &self.sync { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (4, match &self.unsupported_member { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (5, match &self.remote { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.concurrency { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (7, match &self.progress_min_interval_ms { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (8, match &self.max_connections_per_host { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            partial: { let v = c.try_get(1)?; if v.is_null() { None } else { Some(PartialBehavior::from_wire(v.try_int()?)?) } },
            destructive: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(DestructiveBehavior::from_wire(v.try_int()?)?) } },
            sync: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(SyncBehavior::from_wire(v.try_int()?)?) } },
            unsupported_member: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(UnsupportedMemberBehavior::from_wire(v.try_int()?)?) } },
            remote: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            concurrency: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            progress_min_interval_ms: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            max_connections_per_host: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_int()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct RequestMeta {
    pub request_id: String,
    pub schema_version: String,
    pub workspace: Option<WorkspaceRef>,
    pub selection: Option<Selection>,
    pub policy: Option<OperationPolicy>,
    pub dry_run: Option<bool>,
    pub attribution: Option<OperationAttribution>,
}
impl RequestMeta {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.request_id.clone())),
            (2, Cbor::Text(self.schema_version.clone())),
            (3, match &self.workspace { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (4, match &self.selection { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (5, match &self.policy { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (6, match &self.dry_run { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (7, match &self.attribution { Some(v) => v.to_cbor(), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            request_id: c.try_get(1)?.try_text()?,
            schema_version: c.try_get(2)?.try_text()?,
            workspace: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(WorkspaceRef::from_cbor(v)?) } },
            selection: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(Selection::from_cbor(v)?) } },
            policy: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(OperationPolicy::from_cbor(v)?) } },
            dry_run: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            attribution: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(OperationAttribution::from_cbor(v)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ResponseMeta {
    pub request_id: String,
    pub schema_version: String,
    pub action: ActionKind,
    pub aggregate_status: AggregateStatus,
    pub operation_id: Option<String>,
    pub message: Option<String>,
    pub attribution: Option<OperationAttribution>,
}
impl ResponseMeta {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.request_id.clone())),
            (2, Cbor::Text(self.schema_version.clone())),
            (3, Cbor::Int(self.action.wire())),
            (4, Cbor::Int(self.aggregate_status.wire())),
            (5, match &self.operation_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.message { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (7, match &self.attribution { Some(v) => v.to_cbor(), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            request_id: c.try_get(1)?.try_text()?,
            schema_version: c.try_get(2)?.try_text()?,
            action: ActionKind::from_wire(c.try_get(3)?.try_int()?)?,
            aggregate_status: AggregateStatus::from_wire(c.try_get(4)?.try_int()?)?,
            operation_id: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            message: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            attribution: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(OperationAttribution::from_cbor(v)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GwzError {
    pub code: GwzErrorCode,
    pub message: String,
    pub member_id: Option<String>,
    pub member_path: Option<String>,
    pub detail: Option<String>,
    pub target_kind: Option<TargetKind>,
}
impl GwzError {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.code.wire())),
            (2, Cbor::Text(self.message.clone())),
            (3, match &self.member_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.member_path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.detail { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.target_kind { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            code: GwzErrorCode::from_wire(c.try_get(1)?.try_int()?)?,
            message: c.try_get(2)?.try_text()?,
            member_id: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            member_path: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            detail: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            target_kind: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(TargetKind::from_wire(v.try_int()?)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct RemoteSpec {
    pub name: String,
    pub url: String,
    pub fetch: Option<bool>,
    pub push: Option<bool>,
}
impl RemoteSpec {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.name.clone())),
            (2, Cbor::Text(self.url.clone())),
            (3, match &self.fetch { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (4, match &self.push { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            name: c.try_get(1)?.try_text()?,
            url: c.try_get(2)?.try_text()?,
            fetch: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            push: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DesiredRef {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub git_tag: Option<String>,
    pub local_only: Option<bool>,
}
impl DesiredRef {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, match &self.branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (2, match &self.commit { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, match &self.git_tag { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.local_only { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            branch: { let v = c.try_get(1)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            commit: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            git_tag: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            local_only: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SourceUrl {
    pub url: String,
    pub path: Option<String>,
    pub remote_name: Option<String>,
    pub branch: Option<String>,
}
impl SourceUrl {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.url.clone())),
            (2, match &self.path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, match &self.remote_name { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            url: c.try_get(1)?.try_text()?,
            path: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            remote_name: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            branch: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MemberSpec {
    pub member_id: String,
    pub path: String,
    pub source_id: String,
    pub source_kind: SourceKind,
    pub active: bool,
    pub desired: Option<DesiredRef>,
    pub remotes: Vec<RemoteSpec>,
}
impl MemberSpec {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.member_id.clone())),
            (2, Cbor::Text(self.path.clone())),
            (3, Cbor::Text(self.source_id.clone())),
            (4, Cbor::Int(self.source_kind.wire())),
            (5, Cbor::Bool(self.active)),
            (6, match &self.desired { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (7, Cbor::Array(self.remotes.iter().map(|x| x.to_cbor()).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            member_id: c.try_get(1)?.try_text()?,
            path: c.try_get(2)?.try_text()?,
            source_id: c.try_get(3)?.try_text()?,
            source_kind: SourceKind::from_wire(c.try_get(4)?.try_int()?)?,
            active: c.try_get(5)?.try_bool()?,
            desired: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(DesiredRef::from_cbor(v)?) } },
            remotes: c.try_get(7)?.try_array()?.iter().map(|x| RemoteSpec::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MaterializeTarget {
    pub kind: MaterializeTargetKind,
    pub name: Option<String>,
    pub commit: Option<String>,
}
impl MaterializeTarget {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.kind.wire())),
            (2, match &self.name { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, match &self.commit { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            kind: MaterializeTargetKind::from_wire(c.try_get(1)?.try_int()?)?,
            name: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            commit: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SnapshotSource {
    pub kind: SnapshotSourceKind,
    pub branch: Option<String>,
}
impl SnapshotSource {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.kind.wire())),
            (2, match &self.branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            kind: SnapshotSourceKind::from_wire(c.try_get(1)?.try_int()?)?,
            branch: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ResolvedMemberState {
    pub member_id: String,
    pub path: String,
    pub source_id: String,
    pub source_kind: SourceKind,
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub detached: Option<bool>,
    pub upstream: Option<String>,
    pub dirty: Option<bool>,
    pub materialized: bool,
    pub remotes: Vec<RemoteSpec>,
}
impl ResolvedMemberState {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.member_id.clone())),
            (2, Cbor::Text(self.path.clone())),
            (3, Cbor::Text(self.source_id.clone())),
            (4, Cbor::Int(self.source_kind.wire())),
            (5, match &self.commit { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (7, match &self.detached { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (8, match &self.upstream { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (9, match &self.dirty { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (10, Cbor::Bool(self.materialized)),
            (11, Cbor::Array(self.remotes.iter().map(|x| x.to_cbor()).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            member_id: c.try_get(1)?.try_text()?,
            path: c.try_get(2)?.try_text()?,
            source_id: c.try_get(3)?.try_text()?,
            source_kind: SourceKind::from_wire(c.try_get(4)?.try_int()?)?,
            commit: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            branch: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            detached: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            upstream: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            dirty: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            materialized: c.try_get(10)?.try_bool()?,
            remotes: c.try_get(11)?.try_array()?.iter().map(|x| RemoteSpec::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GitStatus {
    pub member_id: String,
    pub branch: Option<String>,
    pub detached: bool,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
    pub dirty: bool,
}
impl GitStatus {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.member_id.clone())),
            (2, match &self.branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, Cbor::Bool(self.detached)),
            (4, match &self.head { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.upstream { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.ahead { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (7, match &self.behind { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (8, Cbor::Int(self.staged)),
            (9, Cbor::Int(self.unstaged)),
            (10, Cbor::Int(self.untracked)),
            (11, Cbor::Bool(self.dirty)),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            member_id: c.try_get(1)?.try_text()?,
            branch: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            detached: c.try_get(3)?.try_bool()?,
            head: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            upstream: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            ahead: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            behind: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            staged: c.try_get(8)?.try_int()?,
            unstaged: c.try_get(9)?.try_int()?,
            untracked: c.try_get(10)?.try_int()?,
            dirty: c.try_get(11)?.try_bool()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GitFileChange {
    pub member_id: String,
    pub member_path: String,
    pub repo_path: String,
    pub workspace_path: String,
    pub index_status: String,
    pub worktree_status: String,
    pub original_repo_path: Option<String>,
}
impl GitFileChange {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.member_id.clone())),
            (2, Cbor::Text(self.member_path.clone())),
            (3, Cbor::Text(self.repo_path.clone())),
            (4, Cbor::Text(self.workspace_path.clone())),
            (5, Cbor::Text(self.index_status.clone())),
            (6, Cbor::Text(self.worktree_status.clone())),
            (7, match &self.original_repo_path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            member_id: c.try_get(1)?.try_text()?,
            member_path: c.try_get(2)?.try_text()?,
            repo_path: c.try_get(3)?.try_text()?,
            workspace_path: c.try_get(4)?.try_text()?,
            index_status: c.try_get(5)?.try_text()?,
            worktree_status: c.try_get(6)?.try_text()?,
            original_repo_path: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GitTransferProgress {
    pub phase: GitProgressPhase,
    pub received_objects: Option<i64>,
    pub total_objects: Option<i64>,
    pub received_bytes: Option<i64>,
    pub indexed_deltas: Option<i64>,
    pub total_deltas: Option<i64>,
}
impl GitTransferProgress {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.phase.wire())),
            (2, match &self.received_objects { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (3, match &self.total_objects { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (4, match &self.received_bytes { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (5, match &self.indexed_deltas { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (6, match &self.total_deltas { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            phase: GitProgressPhase::from_wire(c.try_get(1)?.try_int()?)?,
            received_objects: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            total_objects: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            received_bytes: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            indexed_deltas: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            total_deltas: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_int()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct WorkspaceRootGitStatus {
    pub branch: Option<String>,
    pub detached: bool,
    pub head: Option<String>,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
    pub dirty: bool,
    pub unborn: bool,
}
impl WorkspaceRootGitStatus {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, match &self.branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (2, Cbor::Bool(self.detached)),
            (3, match &self.head { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, Cbor::Int(self.staged)),
            (5, Cbor::Int(self.unstaged)),
            (6, Cbor::Int(self.untracked)),
            (7, Cbor::Bool(self.dirty)),
            (8, Cbor::Bool(self.unborn)),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            branch: { let v = c.try_get(1)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            detached: c.try_get(2)?.try_bool()?,
            head: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            staged: c.try_get(4)?.try_int()?,
            unstaged: c.try_get(5)?.try_int()?,
            untracked: c.try_get(6)?.try_int()?,
            dirty: c.try_get(7)?.try_bool()?,
            unborn: c.try_get(8)?.try_bool()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct WorkspaceRootFileChange {
    pub repo_path: String,
    pub workspace_path: String,
    pub index_status: String,
    pub worktree_status: String,
    pub original_repo_path: Option<String>,
}
impl WorkspaceRootFileChange {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.repo_path.clone())),
            (2, Cbor::Text(self.workspace_path.clone())),
            (3, Cbor::Text(self.index_status.clone())),
            (4, Cbor::Text(self.worktree_status.clone())),
            (5, match &self.original_repo_path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            repo_path: c.try_get(1)?.try_text()?,
            workspace_path: c.try_get(2)?.try_text()?,
            index_status: c.try_get(3)?.try_text()?,
            worktree_status: c.try_get(4)?.try_text()?,
            original_repo_path: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GitMemberBranchStatus {
    pub member_id: String,
    pub member_path: String,
    pub label: String,
    pub branch: Option<String>,
    pub detached: bool,
    pub unborn: bool,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
}
impl GitMemberBranchStatus {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.member_id.clone())),
            (2, Cbor::Text(self.member_path.clone())),
            (3, Cbor::Text(self.label.clone())),
            (4, match &self.branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, Cbor::Bool(self.detached)),
            (6, Cbor::Bool(self.unborn)),
            (7, match &self.head { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (8, match &self.upstream { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (9, match &self.ahead { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (10, match &self.behind { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            member_id: c.try_get(1)?.try_text()?,
            member_path: c.try_get(2)?.try_text()?,
            label: c.try_get(3)?.try_text()?,
            branch: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            detached: c.try_get(5)?.try_bool()?,
            unborn: c.try_get(6)?.try_bool()?,
            head: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            upstream: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            ahead: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            behind: { let v = c.try_get(10)?; if v.is_null() { None } else { Some(v.try_int()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GitBranchGroup {
    pub label: String,
    pub member_ids: Vec<String>,
    pub member_paths: Vec<String>,
}
impl GitBranchGroup {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.label.clone())),
            (2, Cbor::Array(self.member_ids.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (3, Cbor::Array(self.member_paths.iter().map(|x| Cbor::Text(x.clone())).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            label: c.try_get(1)?.try_text()?,
            member_ids: c.try_get(2)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            member_paths: c.try_get(3)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GitBranchDifference {
    pub label: String,
    pub majority_label: Option<String>,
    pub member_ids: Vec<String>,
    pub member_paths: Vec<String>,
    pub message: Option<String>,
}
impl GitBranchDifference {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.label.clone())),
            (2, match &self.majority_label { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, Cbor::Array(self.member_ids.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (4, Cbor::Array(self.member_paths.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (5, match &self.message { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            label: c.try_get(1)?.try_text()?,
            majority_label: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            member_ids: c.try_get(3)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            member_paths: c.try_get(4)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            message: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct WorkspaceGitStatus {
    pub clean: bool,
    pub file_changes: Vec<GitFileChange>,
    pub branches: Vec<GitMemberBranchStatus>,
    pub branch_groups: Vec<GitBranchGroup>,
    pub branch_differences: Vec<GitBranchDifference>,
    pub root_status: Option<WorkspaceRootGitStatus>,
    pub root_file_changes: Vec<WorkspaceRootFileChange>,
}
impl WorkspaceGitStatus {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bool(self.clean)),
            (2, Cbor::Array(self.file_changes.iter().map(|x| x.to_cbor()).collect())),
            (3, Cbor::Array(self.branches.iter().map(|x| x.to_cbor()).collect())),
            (4, Cbor::Array(self.branch_groups.iter().map(|x| x.to_cbor()).collect())),
            (5, Cbor::Array(self.branch_differences.iter().map(|x| x.to_cbor()).collect())),
            (6, match &self.root_status { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (7, Cbor::Array(self.root_file_changes.iter().map(|x| x.to_cbor()).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            clean: c.try_get(1)?.try_bool()?,
            file_changes: c.try_get(2)?.try_array()?.iter().map(|x| GitFileChange::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            branches: c.try_get(3)?.try_array()?.iter().map(|x| GitMemberBranchStatus::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            branch_groups: c.try_get(4)?.try_array()?.iter().map(|x| GitBranchGroup::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            branch_differences: c.try_get(5)?.try_array()?.iter().map(|x| GitBranchDifference::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            root_status: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(WorkspaceRootGitStatus::from_cbor(v)?) } },
            root_file_changes: c.try_get(7)?.try_array()?.iter().map(|x| WorkspaceRootFileChange::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StashDirtySummary {
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub ignored: bool,
}
impl StashDirtySummary {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bool(self.staged)),
            (2, Cbor::Bool(self.unstaged)),
            (3, Cbor::Bool(self.untracked)),
            (4, Cbor::Bool(self.ignored)),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            staged: c.try_get(1)?.try_bool()?,
            unstaged: c.try_get(2)?.try_bool()?,
            untracked: c.try_get(3)?.try_bool()?,
            ignored: c.try_get(4)?.try_bool()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StashErrorDetail {
    pub code: String,
    pub message: String,
}
impl StashErrorDetail {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.code.clone())),
            (2, Cbor::Text(self.message.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            code: c.try_get(1)?.try_text()?,
            message: c.try_get(2)?.try_text()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StashWarning {
    pub code: String,
    pub message: String,
    pub member_id: Option<String>,
}
impl StashWarning {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.code.clone())),
            (2, Cbor::Text(self.message.clone())),
            (3, match &self.member_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            code: c.try_get(1)?.try_text()?,
            message: c.try_get(2)?.try_text()?,
            member_id: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StashDrift {
    pub code: String,
    pub message: String,
    pub member_id: String,
}
impl StashDrift {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.code.clone())),
            (2, Cbor::Text(self.message.clone())),
            (3, Cbor::Text(self.member_id.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            code: c.try_get(1)?.try_text()?,
            message: c.try_get(2)?.try_text()?,
            member_id: c.try_get(3)?.try_text()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StashBundleMember {
    pub member_id: String,
    pub path: String,
    pub participation: StashParticipation,
    pub push_lifecycle: StashPushLifecycle,
    pub restore_state: StashRestoreState,
    pub branch_before: Option<String>,
    pub head_before: Option<String>,
    pub full_stash_message: String,
    pub dirty_summary: StashDirtySummary,
    pub native_stash_object_id: Option<String>,
    pub native_stash_display_ref: Option<String>,
    pub error: Option<StashErrorDetail>,
}
impl StashBundleMember {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.member_id.clone())),
            (2, Cbor::Text(self.path.clone())),
            (3, Cbor::Int(self.participation.wire())),
            (4, Cbor::Int(self.push_lifecycle.wire())),
            (5, Cbor::Int(self.restore_state.wire())),
            (6, match &self.branch_before { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (7, match &self.head_before { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (8, Cbor::Text(self.full_stash_message.clone())),
            (9, self.dirty_summary.to_cbor()),
            (10, match &self.native_stash_object_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (11, match &self.native_stash_display_ref { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (12, match &self.error { Some(v) => v.to_cbor(), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            member_id: c.try_get(1)?.try_text()?,
            path: c.try_get(2)?.try_text()?,
            participation: StashParticipation::from_wire(c.try_get(3)?.try_int()?)?,
            push_lifecycle: StashPushLifecycle::from_wire(c.try_get(4)?.try_int()?)?,
            restore_state: StashRestoreState::from_wire(c.try_get(5)?.try_int()?)?,
            branch_before: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            head_before: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            full_stash_message: c.try_get(8)?.try_text()?,
            dirty_summary: StashDirtySummary::from_cbor(c.try_get(9)?)?,
            native_stash_object_id: { let v = c.try_get(10)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            native_stash_display_ref: { let v = c.try_get(11)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            error: { let v = c.try_get(12)?; if v.is_null() { None } else { Some(StashErrorDetail::from_cbor(v)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StashBundle {
    pub schema: String,
    pub workspace_id: String,
    pub stash_id: String,
    pub created_at: String,
    pub message_suffix: String,
    pub include_untracked: bool,
    pub include_ignored: bool,
    pub members: Vec<StashBundleMember>,
    pub warnings: Vec<StashWarning>,
    pub drift: Vec<StashDrift>,
    pub selected_members: Vec<String>,
}
impl StashBundle {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.schema.clone())),
            (2, Cbor::Text(self.workspace_id.clone())),
            (3, Cbor::Text(self.stash_id.clone())),
            (4, Cbor::Text(self.created_at.clone())),
            (5, Cbor::Text(self.message_suffix.clone())),
            (6, Cbor::Bool(self.include_untracked)),
            (7, Cbor::Bool(self.include_ignored)),
            (8, Cbor::Array(self.members.iter().map(|x| x.to_cbor()).collect())),
            (9, Cbor::Array(self.warnings.iter().map(|x| x.to_cbor()).collect())),
            (10, Cbor::Array(self.drift.iter().map(|x| x.to_cbor()).collect())),
            (11, Cbor::Array(self.selected_members.iter().map(|x| Cbor::Text(x.clone())).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            schema: c.try_get(1)?.try_text()?,
            workspace_id: c.try_get(2)?.try_text()?,
            stash_id: c.try_get(3)?.try_text()?,
            created_at: c.try_get(4)?.try_text()?,
            message_suffix: c.try_get(5)?.try_text()?,
            include_untracked: c.try_get(6)?.try_bool()?,
            include_ignored: c.try_get(7)?.try_bool()?,
            members: c.try_get(8)?.try_array()?.iter().map(|x| StashBundleMember::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            warnings: c.try_get(9)?.try_array()?.iter().map(|x| StashWarning::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            drift: c.try_get(10)?.try_array()?.iter().map(|x| StashDrift::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            selected_members: c.try_get(11)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct BranchRepoSummary {
    pub member_id: String,
    pub member_path: String,
    pub source_kind: SourceKind,
    pub result: BranchActionResult,
    pub branch: Option<String>,
    pub current_branch: Option<String>,
    pub detached: bool,
    pub unborn: bool,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub source_ref: Option<String>,
    pub target_branch: Option<String>,
    pub resulting_commit: Option<String>,
    pub conflict_paths: Vec<String>,
}
impl BranchRepoSummary {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.member_id.clone())),
            (2, Cbor::Text(self.member_path.clone())),
            (3, Cbor::Int(self.source_kind.wire())),
            (4, Cbor::Int(self.result.wire())),
            (5, match &self.branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.current_branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (7, Cbor::Bool(self.detached)),
            (8, Cbor::Bool(self.unborn)),
            (9, match &self.head { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (10, match &self.upstream { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (11, match &self.ahead { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (12, match &self.behind { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (13, match &self.source_ref { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (14, match &self.target_branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (15, match &self.resulting_commit { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (16, Cbor::Array(self.conflict_paths.iter().map(|x| Cbor::Text(x.clone())).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            member_id: c.try_get(1)?.try_text()?,
            member_path: c.try_get(2)?.try_text()?,
            source_kind: SourceKind::from_wire(c.try_get(3)?.try_int()?)?,
            result: BranchActionResult::from_wire(c.try_get(4)?.try_int()?)?,
            branch: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            current_branch: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            detached: c.try_get(7)?.try_bool()?,
            unborn: c.try_get(8)?.try_bool()?,
            head: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            upstream: { let v = c.try_get(10)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            ahead: { let v = c.try_get(11)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            behind: { let v = c.try_get(12)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            source_ref: { let v = c.try_get(13)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            target_branch: { let v = c.try_get(14)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            resulting_commit: { let v = c.try_get(15)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            conflict_paths: c.try_get(16)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergeParticipantCounts {
    pub total: i64,
    pub planned: i64,
    pub up_to_date: i64,
    pub fast_forwarded: i64,
    pub merged: i64,
    pub conflicted: i64,
    pub failed: i64,
    pub unattempted: i64,
    pub continued: i64,
    pub aborted: i64,
    pub rolled_back: i64,
}
impl MergeParticipantCounts {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.total)),
            (2, Cbor::Int(self.planned)),
            (3, Cbor::Int(self.up_to_date)),
            (4, Cbor::Int(self.fast_forwarded)),
            (5, Cbor::Int(self.merged)),
            (6, Cbor::Int(self.conflicted)),
            (7, Cbor::Int(self.failed)),
            (8, Cbor::Int(self.unattempted)),
            (9, Cbor::Int(self.continued)),
            (10, Cbor::Int(self.aborted)),
            (11, Cbor::Int(self.rolled_back)),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            total: c.try_get(1)?.try_int()?,
            planned: c.try_get(2)?.try_int()?,
            up_to_date: c.try_get(3)?.try_int()?,
            fast_forwarded: c.try_get(4)?.try_int()?,
            merged: c.try_get(5)?.try_int()?,
            conflicted: c.try_get(6)?.try_int()?,
            failed: c.try_get(7)?.try_int()?,
            unattempted: c.try_get(8)?.try_int()?,
            continued: c.try_get(9)?.try_int()?,
            aborted: c.try_get(10)?.try_int()?,
            rolled_back: c.try_get(11)?.try_int()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergeParticipantDrift {
    pub kind: MergeParticipantDriftKind,
    pub message: String,
    pub expected_branch: Option<String>,
    pub live_branch: Option<String>,
    pub expected_head: Option<String>,
    pub live_head: Option<String>,
    pub expected_merge_head: Option<String>,
    pub live_merge_head: Option<String>,
}
impl MergeParticipantDrift {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.kind.wire())),
            (2, Cbor::Text(self.message.clone())),
            (3, match &self.expected_branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.live_branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.expected_head { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.live_head { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (7, match &self.expected_merge_head { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (8, match &self.live_merge_head { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            kind: MergeParticipantDriftKind::from_wire(c.try_get(1)?.try_int()?)?,
            message: c.try_get(2)?.try_text()?,
            expected_branch: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            live_branch: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            expected_head: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            live_head: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            expected_merge_head: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            live_merge_head: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergeOperationDrift {
    pub kind: MergeOperationDriftKind,
    pub message: String,
}
impl MergeOperationDrift {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.kind.wire())),
            (2, Cbor::Text(self.message.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            kind: MergeOperationDriftKind::from_wire(c.try_get(1)?.try_int()?)?,
            message: c.try_get(2)?.try_text()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergePreservation {
    pub target_id: String,
    pub path: String,
    pub backup_ref: Option<String>,
    pub backup_commit: Option<String>,
    pub stash_id: Option<String>,
    pub stash_object_id: Option<String>,
}
impl MergePreservation {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.target_id.clone())),
            (2, Cbor::Text(self.path.clone())),
            (3, match &self.backup_ref { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.backup_commit { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.stash_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.stash_object_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            target_id: c.try_get(1)?.try_text()?,
            path: c.try_get(2)?.try_text()?,
            backup_ref: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            backup_commit: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            stash_id: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            stash_object_id: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergePendingActionSummary {
    pub kind: MergePendingActionKind,
    pub state: MergePendingActionState,
    pub message: Option<String>,
}
impl MergePendingActionSummary {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.kind.wire())),
            (2, Cbor::Int(self.state.wire())),
            (3, match &self.message { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            kind: MergePendingActionKind::from_wire(c.try_get(1)?.try_int()?)?,
            state: MergePendingActionState::from_wire(c.try_get(2)?.try_int()?)?,
            message: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergeRepoSummary {
    pub target_id: String,
    pub target_kind: TargetKind,
    pub path: String,
    pub source_ref: String,
    pub source_commit: String,
    pub target_branch: String,
    pub before_commit: String,
    pub resulting_commit: Option<String>,
    pub live_commit: Option<String>,
    pub state: MergeParticipantState,
    pub predicted: Option<MergeAnalysisKind>,
    pub prediction_complete: Option<bool>,
    pub conflict_paths: Vec<String>,
    pub continue_eligible: Option<bool>,
    pub abort_eligible: Option<bool>,
    pub drift: Vec<MergeParticipantDrift>,
    pub error: Option<GwzError>,
    pub pending_action: Option<MergePendingActionSummary>,
}
impl MergeRepoSummary {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.target_id.clone())),
            (2, Cbor::Int(self.target_kind.wire())),
            (3, Cbor::Text(self.path.clone())),
            (4, Cbor::Text(self.source_ref.clone())),
            (5, Cbor::Text(self.source_commit.clone())),
            (6, Cbor::Text(self.target_branch.clone())),
            (7, Cbor::Text(self.before_commit.clone())),
            (8, match &self.resulting_commit { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (9, match &self.live_commit { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (10, Cbor::Int(self.state.wire())),
            (11, match &self.predicted { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (12, match &self.prediction_complete { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (13, Cbor::Array(self.conflict_paths.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (14, match &self.continue_eligible { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (15, match &self.abort_eligible { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (16, Cbor::Array(self.drift.iter().map(|x| x.to_cbor()).collect())),
            (17, match &self.error { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (18, match &self.pending_action { Some(v) => v.to_cbor(), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            target_id: c.try_get(1)?.try_text()?,
            target_kind: TargetKind::from_wire(c.try_get(2)?.try_int()?)?,
            path: c.try_get(3)?.try_text()?,
            source_ref: c.try_get(4)?.try_text()?,
            source_commit: c.try_get(5)?.try_text()?,
            target_branch: c.try_get(6)?.try_text()?,
            before_commit: c.try_get(7)?.try_text()?,
            resulting_commit: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            live_commit: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            state: MergeParticipantState::from_wire(c.try_get(10)?.try_int()?)?,
            predicted: { let v = c.try_get(11)?; if v.is_null() { None } else { Some(MergeAnalysisKind::from_wire(v.try_int()?)?) } },
            prediction_complete: { let v = c.try_get(12)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            conflict_paths: c.try_get(13)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            continue_eligible: { let v = c.try_get(14)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            abort_eligible: { let v = c.try_get(15)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            drift: c.try_get(16)?.try_array()?.iter().map(|x| MergeParticipantDrift::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            error: { let v = c.try_get(17)?; if v.is_null() { None } else { Some(GwzError::from_cbor(v)?) } },
            pending_action: { let v = c.try_get(18)?; if v.is_null() { None } else { Some(MergePendingActionSummary::from_cbor(v)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PlannedChange {
    pub action: PlannedAction,
    pub from_ref: Option<String>,
    pub to_ref: Option<String>,
    pub message: Option<String>,
}
impl PlannedChange {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.action.wire())),
            (2, match &self.from_ref { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, match &self.to_ref { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.message { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            action: PlannedAction::from_wire(c.try_get(1)?.try_int()?)?,
            from_ref: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            to_ref: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            message: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MemberResponse {
    pub member_id: String,
    pub member_path: String,
    pub source_kind: SourceKind,
    pub status: MemberStatus,
    pub error: Option<GwzError>,
    pub planned: Option<PlannedChange>,
    pub state: Option<ResolvedMemberState>,
    pub git_status: Option<GitStatus>,
    pub lock_match: Option<LockMatch>,
    pub target_kind: Option<TargetKind>,
}
impl MemberResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.member_id.clone())),
            (2, Cbor::Text(self.member_path.clone())),
            (3, Cbor::Int(self.source_kind.wire())),
            (4, Cbor::Int(self.status.wire())),
            (5, match &self.error { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (6, match &self.planned { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (7, match &self.state { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (8, match &self.git_status { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (9, match &self.lock_match { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (10, match &self.target_kind { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            member_id: c.try_get(1)?.try_text()?,
            member_path: c.try_get(2)?.try_text()?,
            source_kind: SourceKind::from_wire(c.try_get(3)?.try_int()?)?,
            status: MemberStatus::from_wire(c.try_get(4)?.try_int()?)?,
            error: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(GwzError::from_cbor(v)?) } },
            planned: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(PlannedChange::from_cbor(v)?) } },
            state: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(ResolvedMemberState::from_cbor(v)?) } },
            git_status: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(GitStatus::from_cbor(v)?) } },
            lock_match: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(LockMatch::from_wire(v.try_int()?)?) } },
            target_kind: { let v = c.try_get(10)?; if v.is_null() { None } else { Some(TargetKind::from_wire(v.try_int()?)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ResponseEnvelope {
    pub meta: ResponseMeta,
    pub members: Vec<MemberResponse>,
    pub errors: Vec<GwzError>,
}
impl ResponseEnvelope {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Array(self.members.iter().map(|x| x.to_cbor()).collect())),
            (3, Cbor::Array(self.errors.iter().map(|x| x.to_cbor()).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: ResponseMeta::from_cbor(c.try_get(1)?)?,
            members: c.try_get(2)?.try_array()?.iter().map(|x| MemberResponse::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            errors: c.try_get(3)?.try_array()?.iter().map(|x| GwzError::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct OperationEvent {
    pub operation_id: String,
    pub request_id: String,
    pub sequence: i64,
    pub timestamp_ms: i64,
    pub kind: EventKind,
    pub severity: Severity,
    pub member_id: Option<String>,
    pub member_path: Option<String>,
    pub message: Option<String>,
    pub member: Option<MemberResponse>,
    pub error: Option<GwzError>,
    pub attribution: Option<OperationAttribution>,
    pub progress: Option<GitTransferProgress>,
    pub target_kind: Option<TargetKind>,
    pub merge_state: Option<MergeOperationState>,
    pub merge_member: Option<MergeRepoSummary>,
    pub artifact_path: Option<String>,
}
impl OperationEvent {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.operation_id.clone())),
            (2, Cbor::Text(self.request_id.clone())),
            (3, Cbor::Int(self.sequence)),
            (4, Cbor::Int(self.timestamp_ms)),
            (5, Cbor::Int(self.kind.wire())),
            (6, Cbor::Int(self.severity.wire())),
            (7, match &self.member_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (8, match &self.member_path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (9, match &self.message { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (10, match &self.member { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (11, match &self.error { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (12, match &self.attribution { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (13, match &self.progress { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (14, match &self.target_kind { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (15, match &self.merge_state { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (16, match &self.merge_member { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (17, match &self.artifact_path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            operation_id: c.try_get(1)?.try_text()?,
            request_id: c.try_get(2)?.try_text()?,
            sequence: c.try_get(3)?.try_int()?,
            timestamp_ms: c.try_get(4)?.try_int()?,
            kind: EventKind::from_wire(c.try_get(5)?.try_int()?)?,
            severity: Severity::from_wire(c.try_get(6)?.try_int()?)?,
            member_id: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            member_path: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            message: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            member: { let v = c.try_get(10)?; if v.is_null() { None } else { Some(MemberResponse::from_cbor(v)?) } },
            error: { let v = c.try_get(11)?; if v.is_null() { None } else { Some(GwzError::from_cbor(v)?) } },
            attribution: { let v = c.try_get(12)?; if v.is_null() { None } else { Some(OperationAttribution::from_cbor(v)?) } },
            progress: { let v = c.try_get(13)?; if v.is_null() { None } else { Some(GitTransferProgress::from_cbor(v)?) } },
            target_kind: { let v = c.try_get(14)?; if v.is_null() { None } else { Some(TargetKind::from_wire(v.try_int()?)?) } },
            merge_state: { let v = c.try_get(15)?; if v.is_null() { None } else { Some(MergeOperationState::from_wire(v.try_int()?)?) } },
            merge_member: { let v = c.try_get(16)?; if v.is_null() { None } else { Some(MergeRepoSummary::from_cbor(v)?) } },
            artifact_path: { let v = c.try_get(17)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct OperationResult {
    pub operation_id: String,
    pub request_id: String,
    pub action: ActionKind,
    pub aggregate_status: AggregateStatus,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub members: Vec<MemberResponse>,
    pub errors: Vec<GwzError>,
    pub attribution: Option<OperationAttribution>,
}
impl OperationResult {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.operation_id.clone())),
            (2, Cbor::Text(self.request_id.clone())),
            (3, Cbor::Int(self.action.wire())),
            (4, Cbor::Int(self.aggregate_status.wire())),
            (5, Cbor::Int(self.started_at_ms)),
            (6, Cbor::Int(self.finished_at_ms)),
            (7, Cbor::Array(self.members.iter().map(|x| x.to_cbor()).collect())),
            (8, Cbor::Array(self.errors.iter().map(|x| x.to_cbor()).collect())),
            (9, match &self.attribution { Some(v) => v.to_cbor(), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            operation_id: c.try_get(1)?.try_text()?,
            request_id: c.try_get(2)?.try_text()?,
            action: ActionKind::from_wire(c.try_get(3)?.try_int()?)?,
            aggregate_status: AggregateStatus::from_wire(c.try_get(4)?.try_int()?)?,
            started_at_ms: c.try_get(5)?.try_int()?,
            finished_at_ms: c.try_get(6)?.try_int()?,
            members: c.try_get(7)?.try_array()?.iter().map(|x| MemberResponse::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            errors: c.try_get(8)?.try_array()?.iter().map(|x| GwzError::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            attribution: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(OperationAttribution::from_cbor(v)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CreateWorkspaceRequest {
    pub meta: RequestMeta,
    pub workspace_root: String,
    pub workspace_id: Option<String>,
}
impl CreateWorkspaceRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.workspace_root.clone())),
            (3, match &self.workspace_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            workspace_root: c.try_get(2)?.try_text()?,
            workspace_id: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct InitFromSourcesRequest {
    pub meta: RequestMeta,
    pub workspace_root: String,
    pub sources: Vec<SourceUrl>,
    pub target: Option<MaterializeTarget>,
    pub workspace_id: Option<String>,
}
impl InitFromSourcesRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.workspace_root.clone())),
            (3, Cbor::Array(self.sources.iter().map(|x| x.to_cbor()).collect())),
            (4, match &self.target { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (5, match &self.workspace_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            workspace_root: c.try_get(2)?.try_text()?,
            sources: c.try_get(3)?.try_array()?.iter().map(|x| SourceUrl::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            target: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(MaterializeTarget::from_cbor(v)?) } },
            workspace_id: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CloneWorkspaceRequest {
    pub meta: RequestMeta,
    pub url: String,
    pub target: String,
}
impl CloneWorkspaceRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.url.clone())),
            (3, Cbor::Text(self.target.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            url: c.try_get(2)?.try_text()?,
            target: c.try_get(3)?.try_text()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct AddExistingRepoRequest {
    pub meta: RequestMeta,
    pub repository_path: String,
    pub member_path: Option<String>,
    pub member_id: Option<String>,
    pub source_id: Option<String>,
}
impl AddExistingRepoRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.repository_path.clone())),
            (3, match &self.member_path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.member_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.source_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            repository_path: c.try_get(2)?.try_text()?,
            member_path: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            member_id: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            source_id: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CreateRepoRequest {
    pub meta: RequestMeta,
    pub member_path: String,
    pub initial_branch: Option<String>,
    pub member_id: Option<String>,
    pub source_id: Option<String>,
}
impl CreateRepoRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.member_path.clone())),
            (3, match &self.initial_branch { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.member_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.source_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            member_path: c.try_get(2)?.try_text()?,
            initial_branch: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            member_id: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            source_id: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct RepoSyncRequest {
    pub meta: RequestMeta,
}
impl RepoSyncRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CloneRepoMemberRequest {
    pub meta: RequestMeta,
    pub source: SourceUrl,
    pub member_id: Option<String>,
    pub source_id: Option<String>,
}
impl CloneRepoMemberRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, self.source.to_cbor()),
            (3, match &self.member_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.source_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            source: SourceUrl::from_cbor(c.try_get(2)?)?,
            member_id: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            source_id: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DetachRepoMemberRequest {
    pub meta: RequestMeta,
}
impl DetachRepoMemberRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct AttachRepoMemberRequest {
    pub meta: RequestMeta,
}
impl AttachRepoMemberRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MaterializeRequest {
    pub meta: RequestMeta,
    pub target: MaterializeTarget,
}
impl MaterializeRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, self.target.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            target: MaterializeTarget::from_cbor(c.try_get(2)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StatusRequest {
    pub meta: RequestMeta,
    pub mode: Option<StatusMode>,
    pub include_file_changes: Option<bool>,
    pub include_branch_summary: Option<bool>,
    pub path_style: Option<StatusPathStyle>,
}
impl StatusRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, match &self.mode { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (3, match &self.include_file_changes { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (4, match &self.include_branch_summary { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (5, match &self.path_style { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            mode: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(StatusMode::from_wire(v.try_int()?)?) } },
            include_file_changes: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            include_branch_summary: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            path_style: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(StatusPathStyle::from_wire(v.try_int()?)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct LsRequest {
    pub meta: RequestMeta,
    pub include_unmaterialized: Option<bool>,
}
impl LsRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, match &self.include_unmaterialized { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            include_unmaterialized: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MemberEntry {
    pub id: String,
    pub path: String,
    pub abspath: String,
    pub materialized: bool,
    pub target_kind: Option<TargetKind>,
}
impl MemberEntry {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.id.clone())),
            (2, Cbor::Text(self.path.clone())),
            (3, Cbor::Text(self.abspath.clone())),
            (4, Cbor::Bool(self.materialized)),
            (5, match &self.target_kind { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            id: c.try_get(1)?.try_text()?,
            path: c.try_get(2)?.try_text()?,
            abspath: c.try_get(3)?.try_text()?,
            materialized: c.try_get(4)?.try_bool()?,
            target_kind: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(TargetKind::from_wire(v.try_int()?)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct LsResponse {
    pub response: ResponseEnvelope,
    pub members: Option<Vec<MemberEntry>>,
}
impl LsResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
            (2, match &self.members { Some(v) => Cbor::Array(v.iter().map(|x| x.to_cbor()).collect()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
            members: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_array()?.iter().map(|x| MemberEntry::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExecResult {
    pub id: String,
    pub path: String,
    pub exit_code: Option<i64>,
    pub signal: Option<i64>,
    pub spawn_error: Option<String>,
}
impl ExecResult {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.id.clone())),
            (2, Cbor::Text(self.path.clone())),
            (3, match &self.exit_code { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (4, match &self.signal { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (5, match &self.spawn_error { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            id: c.try_get(1)?.try_text()?,
            path: c.try_get(2)?.try_text()?,
            exit_code: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            signal: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            spawn_error: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExecRequest {
    pub meta: RequestMeta,
    pub mode: ExecMode,
    pub command: Vec<String>,
    pub members: Vec<MemberEntry>,
    pub continue_on_fail: Option<bool>,
}
impl ExecRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Int(self.mode.wire())),
            (3, Cbor::Array(self.command.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (4, Cbor::Array(self.members.iter().map(|x| x.to_cbor()).collect())),
            (5, match &self.continue_on_fail { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            mode: ExecMode::from_wire(c.try_get(2)?.try_int()?)?,
            command: c.try_get(3)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            members: c.try_get(4)?.try_array()?.iter().map(|x| MemberEntry::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            continue_on_fail: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExecResponse {
    pub response: ResponseEnvelope,
    pub results: Option<Vec<ExecResult>>,
}
impl ExecResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
            (2, match &self.results { Some(v) => Cbor::Array(v.iter().map(|x| x.to_cbor()).collect()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
            results: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_array()?.iter().map(|x| ExecResult::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SnapshotRequest {
    pub meta: RequestMeta,
    pub snapshot_id: String,
    pub source: Option<SnapshotSource>,
}
impl SnapshotRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.snapshot_id.clone())),
            (3, match &self.source { Some(v) => v.to_cbor(), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            snapshot_id: c.try_get(2)?.try_text()?,
            source: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(SnapshotSource::from_cbor(v)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ListSnapshotsRequest {
    pub meta: RequestMeta,
}
impl ListSnapshotsRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TagRequest {
    pub meta: RequestMeta,
    pub op: TagOp,
    pub name: Option<String>,
    pub message: Option<String>,
    pub signed: Option<bool>,
    pub remote: Option<String>,
    pub all: Option<bool>,
}
impl TagRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Int(self.op.wire())),
            (3, match &self.name { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.message { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.signed { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (6, match &self.remote { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (7, match &self.all { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            op: TagOp::from_wire(c.try_get(2)?.try_int()?)?,
            name: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            message: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            signed: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            remote: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            all: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CaptureRequest {
    pub meta: RequestMeta,
}
impl CaptureRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommitRequest {
    pub meta: RequestMeta,
    pub message: String,
    pub all: Option<bool>,
    pub commit_marker: Option<bool>,
}
impl CommitRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.message.clone())),
            (3, match &self.all { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (4, match &self.commit_marker { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            message: c.try_get(2)?.try_text()?,
            all: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            commit_marker: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StageRequest {
    pub meta: RequestMeta,
    pub cwd: String,
    pub pathspecs: Vec<String>,
    pub all: Option<bool>,
}
impl StageRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.cwd.clone())),
            (3, Cbor::Array(self.pathspecs.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (4, match &self.all { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            cwd: c.try_get(2)?.try_text()?,
            pathspecs: c.try_get(3)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            all: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PullHeadRequest {
    pub meta: RequestMeta,
}
impl PullHeadRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PullSnapshotRequest {
    pub meta: RequestMeta,
    pub snapshot_id: String,
}
impl PullSnapshotRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.snapshot_id.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            snapshot_id: c.try_get(2)?.try_text()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PushRequest {
    pub meta: RequestMeta,
    pub remote: Option<String>,
    pub refspec: Option<String>,
}
impl PushRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, match &self.remote { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, match &self.refspec { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            remote: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            refspec: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StashRequest {
    pub meta: RequestMeta,
    pub op: StashOp,
    pub stash_id: Option<String>,
    pub message: Option<String>,
    pub include_untracked: Option<bool>,
    pub include_ignored: Option<bool>,
    pub expanded: Option<bool>,
    pub preserve_index: Option<bool>,
}
impl StashRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Int(self.op.wire())),
            (3, match &self.stash_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.message { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.include_untracked { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (6, match &self.include_ignored { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (7, match &self.expanded { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (8, match &self.preserve_index { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            op: StashOp::from_wire(c.try_get(2)?.try_int()?)?,
            stash_id: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            message: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            include_untracked: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            include_ignored: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            expanded: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            preserve_index: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct BranchRequest {
    pub meta: RequestMeta,
    pub op: BranchOp,
    pub name: Option<String>,
    pub start_ref: Option<String>,
    pub switch_after_create: Option<bool>,
}
impl BranchRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Int(self.op.wire())),
            (3, match &self.name { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.start_ref { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.switch_after_create { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            op: BranchOp::from_wire(c.try_get(2)?.try_int()?)?,
            name: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            start_ref: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            switch_after_create: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergeRequest {
    pub meta: RequestMeta,
    pub op: MergeOp,
    pub source_ref: Option<String>,
    pub merge_id: Option<String>,
    pub mode: Option<MergeMode>,
    pub message: Option<String>,
    pub preserve: Option<bool>,
}
impl MergeRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Int(self.op.wire())),
            (3, match &self.source_ref { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.merge_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.mode { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (6, match &self.message { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (7, match &self.preserve { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            op: MergeOp::from_wire(c.try_get(2)?.try_int()?)?,
            source_ref: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            merge_id: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            mode: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(MergeMode::from_wire(v.try_int()?)?) } },
            message: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            preserve: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CreateWorkspaceResponse {
    pub response: ResponseEnvelope,
}
impl CreateWorkspaceResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct InitFromSourcesResponse {
    pub response: ResponseEnvelope,
}
impl InitFromSourcesResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CloneWorkspaceResponse {
    pub response: ResponseEnvelope,
}
impl CloneWorkspaceResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct AddExistingRepoResponse {
    pub response: ResponseEnvelope,
}
impl AddExistingRepoResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CreateRepoResponse {
    pub response: ResponseEnvelope,
}
impl CreateRepoResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct RepoSyncResponse {
    pub response: ResponseEnvelope,
}
impl RepoSyncResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CloneRepoMemberResponse {
    pub response: ResponseEnvelope,
}
impl CloneRepoMemberResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DetachRepoMemberResponse {
    pub response: ResponseEnvelope,
}
impl DetachRepoMemberResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct AttachRepoMemberResponse {
    pub response: ResponseEnvelope,
}
impl AttachRepoMemberResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MaterializeResponse {
    pub response: ResponseEnvelope,
}
impl MaterializeResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StatusResponse {
    pub response: ResponseEnvelope,
    pub workspace_git_status: Option<WorkspaceGitStatus>,
}
impl StatusResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
            (2, match &self.workspace_git_status { Some(v) => v.to_cbor(), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
            workspace_git_status: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(WorkspaceGitStatus::from_cbor(v)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SnapshotResponse {
    pub response: ResponseEnvelope,
}
impl SnapshotResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SnapshotInfo {
    pub name: String,
    pub created_at: String,
    pub created_by: String,
    pub members: i64,
}
impl SnapshotInfo {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.name.clone())),
            (2, Cbor::Text(self.created_at.clone())),
            (3, Cbor::Text(self.created_by.clone())),
            (4, Cbor::Int(self.members)),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            name: c.try_get(1)?.try_text()?,
            created_at: c.try_get(2)?.try_text()?,
            created_by: c.try_get(3)?.try_text()?,
            members: c.try_get(4)?.try_int()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ListSnapshotsResponse {
    pub response: ResponseEnvelope,
    pub snapshots: Option<Vec<SnapshotInfo>>,
}
impl ListSnapshotsResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
            (2, match &self.snapshots { Some(v) => Cbor::Array(v.iter().map(|x| x.to_cbor()).collect()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
            snapshots: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_array()?.iter().map(|x| SnapshotInfo::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TagInfo {
    pub name: String,
    pub members: i64,
}
impl TagInfo {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.name.clone())),
            (2, Cbor::Int(self.members)),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            name: c.try_get(1)?.try_text()?,
            members: c.try_get(2)?.try_int()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TagResponse {
    pub response: ResponseEnvelope,
    pub tags: Option<Vec<TagInfo>>,
}
impl TagResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
            (2, match &self.tags { Some(v) => Cbor::Array(v.iter().map(|x| x.to_cbor()).collect()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
            tags: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_array()?.iter().map(|x| TagInfo::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CaptureResponse {
    pub response: ResponseEnvelope,
}
impl CaptureResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommitResponse {
    pub response: ResponseEnvelope,
}
impl CommitResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StageResponse {
    pub response: ResponseEnvelope,
}
impl StageResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PullHeadResponse {
    pub response: ResponseEnvelope,
}
impl PullHeadResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PullSnapshotResponse {
    pub response: ResponseEnvelope,
}
impl PullSnapshotResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PushResponse {
    pub response: ResponseEnvelope,
}
impl PushResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct StashResponse {
    pub response: ResponseEnvelope,
    pub bundles: Option<Vec<StashBundle>>,
}
impl StashResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
            (2, match &self.bundles { Some(v) => Cbor::Array(v.iter().map(|x| x.to_cbor()).collect()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
            bundles: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_array()?.iter().map(|x| StashBundle::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct BranchResponse {
    pub response: ResponseEnvelope,
    pub repos: Option<Vec<BranchRepoSummary>>,
}
impl BranchResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
            (2, match &self.repos { Some(v) => Cbor::Array(v.iter().map(|x| x.to_cbor()).collect()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
            repos: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_array()?.iter().map(|x| BranchRepoSummary::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergeResponse {
    pub response: ResponseEnvelope,
    pub merge_id: Option<String>,
    pub state: MergeOperationState,
    pub open: bool,
    pub participant_counts: MergeParticipantCounts,
    pub repos: Vec<MergeRepoSummary>,
    pub operation_drift: Vec<MergeOperationDrift>,
    pub preservation: Option<Vec<MergePreservation>>,
    pub publication_step: Option<MergePublicationStep>,
}
impl MergeResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
            (2, match &self.merge_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, Cbor::Int(self.state.wire())),
            (4, Cbor::Bool(self.open)),
            (5, self.participant_counts.to_cbor()),
            (6, Cbor::Array(self.repos.iter().map(|x| x.to_cbor()).collect())),
            (7, Cbor::Array(self.operation_drift.iter().map(|x| x.to_cbor()).collect())),
            (8, match &self.preservation { Some(v) => Cbor::Array(v.iter().map(|x| x.to_cbor()).collect()), None => Cbor::Null }),
            (9, match &self.publication_step { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
            merge_id: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            state: MergeOperationState::from_wire(c.try_get(3)?.try_int()?)?,
            open: c.try_get(4)?.try_bool()?,
            participant_counts: MergeParticipantCounts::from_cbor(c.try_get(5)?)?,
            repos: c.try_get(6)?.try_array()?.iter().map(|x| MergeRepoSummary::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            operation_drift: c.try_get(7)?.try_array()?.iter().map(|x| MergeOperationDrift::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            preservation: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_array()?.iter().map(|x| MergePreservation::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?) } },
            publication_step: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(MergePublicationStep::from_wire(v.try_int()?)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffComparison {
    pub kind: DiffComparisonKind,
    pub left: Option<String>,
    pub right: Option<String>,
    pub merge_base: Option<bool>,
}
impl DiffComparison {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.kind.wire())),
            (2, match &self.left { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, match &self.right { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.merge_base { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            kind: DiffComparisonKind::from_wire(c.try_get(1)?.try_int()?)?,
            left: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            right: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            merge_base: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffOptions {
    pub output_format: Option<DiffOutputFormat>,
    pub context_lines: Option<i64>,
    pub interhunk_lines: Option<i64>,
    pub algorithm: Option<DiffAlgorithm>,
    pub whitespace: Option<DiffWhitespaceMode>,
    pub find_renames: Option<bool>,
    pub find_copies: Option<bool>,
    pub rename_threshold: Option<i64>,
    pub rename_limit: Option<i64>,
    pub binary: Option<bool>,
    pub text: Option<bool>,
    pub full_index: Option<bool>,
    pub abbrev: Option<i64>,
    pub reverse: Option<bool>,
    pub null_terminated: Option<bool>,
    pub src_prefix: Option<String>,
    pub dst_prefix: Option<String>,
    pub no_prefix: Option<bool>,
    pub line_prefix: Option<String>,
    pub ignore_submodules: Option<String>,
    pub diff_filter: Option<String>,
    pub manifest_mode: Option<DiffManifestMode>,
    pub echo_manifest_entries: Option<bool>,
}
impl DiffOptions {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, match &self.output_format { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (2, match &self.context_lines { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (3, match &self.interhunk_lines { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (4, match &self.algorithm { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (5, match &self.whitespace { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (6, match &self.find_renames { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (7, match &self.find_copies { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (8, match &self.rename_threshold { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (9, match &self.rename_limit { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (10, match &self.binary { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (11, match &self.text { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (12, match &self.full_index { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (13, match &self.abbrev { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (14, match &self.reverse { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (15, match &self.null_terminated { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (16, match &self.src_prefix { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (17, match &self.dst_prefix { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (18, match &self.no_prefix { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (19, match &self.line_prefix { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (20, match &self.ignore_submodules { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (21, match &self.diff_filter { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (22, match &self.manifest_mode { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (23, match &self.echo_manifest_entries { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            output_format: { let v = c.try_get(1)?; if v.is_null() { None } else { Some(DiffOutputFormat::from_wire(v.try_int()?)?) } },
            context_lines: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            interhunk_lines: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            algorithm: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(DiffAlgorithm::from_wire(v.try_int()?)?) } },
            whitespace: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(DiffWhitespaceMode::from_wire(v.try_int()?)?) } },
            find_renames: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            find_copies: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            rename_threshold: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            rename_limit: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            binary: { let v = c.try_get(10)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            text: { let v = c.try_get(11)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            full_index: { let v = c.try_get(12)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            abbrev: { let v = c.try_get(13)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            reverse: { let v = c.try_get(14)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            null_terminated: { let v = c.try_get(15)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            src_prefix: { let v = c.try_get(16)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            dst_prefix: { let v = c.try_get(17)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            no_prefix: { let v = c.try_get(18)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            line_prefix: { let v = c.try_get(19)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            ignore_submodules: { let v = c.try_get(20)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            diff_filter: { let v = c.try_get(21)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            manifest_mode: { let v = c.try_get(22)?; if v.is_null() { None } else { Some(DiffManifestMode::from_wire(v.try_int()?)?) } },
            echo_manifest_entries: { let v = c.try_get(23)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffRequest {
    pub meta: RequestMeta,
    pub workspace_cwd: Option<String>,
    pub operands: Vec<String>,
    pub explicit_pathspecs: Vec<String>,
    pub options: Option<DiffOptions>,
    pub cached: Option<bool>,
    pub merge_base: Option<bool>,
}
impl DiffRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, match &self.workspace_cwd { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, Cbor::Array(self.operands.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (4, Cbor::Array(self.explicit_pathspecs.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (5, match &self.options { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (6, match &self.cached { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (7, match &self.merge_base { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            meta: RequestMeta::from_cbor(c.try_get(1)?)?,
            workspace_cwd: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            operands: c.try_get(3)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            explicit_pathspecs: c.try_get(4)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            options: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(DiffOptions::from_cbor(v)?) } },
            cached: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            merge_base: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffRepoScope {
    pub root: Option<bool>,
    pub member_id: Option<String>,
    pub member_path: Option<String>,
    pub source_kind: Option<SourceKind>,
}
impl DiffRepoScope {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, match &self.root { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (2, match &self.member_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (3, match &self.member_path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.source_kind { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            root: { let v = c.try_get(1)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            member_id: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            member_path: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            source_kind: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(SourceKind::from_wire(v.try_int()?)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffExcludedTarget {
    pub scope: DiffRepoScope,
    pub reason: DiffTargetExclusionReason,
    pub snapshot_id: Option<String>,
    pub message: Option<String>,
}
impl DiffExcludedTarget {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.scope.to_cbor()),
            (2, Cbor::Int(self.reason.wire())),
            (3, match &self.snapshot_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.message { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            scope: DiffRepoScope::from_cbor(c.try_get(1)?)?,
            reason: DiffTargetExclusionReason::from_wire(c.try_get(2)?.try_int()?)?,
            snapshot_id: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            message: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffParsedTarget {
    pub target_id: String,
    pub scope: DiffRepoScope,
    pub comparison: DiffComparison,
    pub pathspecs: Vec<String>,
    pub left_oid: Option<String>,
    pub right_oid: Option<String>,
    pub merge_base_oid: Option<String>,
    pub left_snapshot_id: Option<String>,
    pub right_snapshot_id: Option<String>,
}
impl DiffParsedTarget {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.target_id.clone())),
            (2, self.scope.to_cbor()),
            (3, self.comparison.to_cbor()),
            (4, Cbor::Array(self.pathspecs.iter().map(|x| Cbor::Text(x.clone())).collect())),
            (5, match &self.left_oid { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.right_oid { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (7, match &self.merge_base_oid { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (8, match &self.left_snapshot_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (9, match &self.right_snapshot_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            target_id: c.try_get(1)?.try_text()?,
            scope: DiffRepoScope::from_cbor(c.try_get(2)?)?,
            comparison: DiffComparison::from_cbor(c.try_get(3)?)?,
            pathspecs: c.try_get(4)?.try_array()?.iter().map(|x| Ok(x.try_text()?)).collect::<Result<Vec<_>, DecodeError>>()?,
            left_oid: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            right_oid: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            merge_base_oid: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            left_snapshot_id: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            right_snapshot_id: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffFileEntry {
    pub file_id: String,
    pub scope: DiffRepoScope,
    pub status: DiffStatus,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_mode: Option<i64>,
    pub new_mode: Option<i64>,
    pub similarity: Option<i64>,
    pub insertions: Option<i64>,
    pub deletions: Option<i64>,
    pub is_binary: Option<bool>,
}
impl DiffFileEntry {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.file_id.clone())),
            (2, self.scope.to_cbor()),
            (3, Cbor::Int(self.status.wire())),
            (4, match &self.old_path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (5, match &self.new_path { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (6, match &self.old_mode { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (7, match &self.new_mode { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (8, match &self.similarity { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (9, match &self.insertions { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (10, match &self.deletions { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (11, match &self.is_binary { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            file_id: c.try_get(1)?.try_text()?,
            scope: DiffRepoScope::from_cbor(c.try_get(2)?)?,
            status: DiffStatus::from_wire(c.try_get(3)?.try_int()?)?,
            old_path: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            new_path: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            old_mode: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            new_mode: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            similarity: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            insertions: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            deletions: { let v = c.try_get(10)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            is_binary: { let v = c.try_get(11)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffRepoSummary {
    pub scope: DiffRepoScope,
    pub has_differences: bool,
    pub files_changed: i64,
    pub insertions: i64,
    pub deletions: i64,
    pub files_manifested: i64,
}
impl DiffRepoSummary {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.scope.to_cbor()),
            (2, Cbor::Bool(self.has_differences)),
            (3, Cbor::Int(self.files_changed)),
            (4, Cbor::Int(self.insertions)),
            (5, Cbor::Int(self.deletions)),
            (6, Cbor::Int(self.files_manifested)),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            scope: DiffRepoScope::from_cbor(c.try_get(1)?)?,
            has_differences: c.try_get(2)?.try_bool()?,
            files_changed: c.try_get(3)?.try_int()?,
            insertions: c.try_get(4)?.try_int()?,
            deletions: c.try_get(5)?.try_int()?,
            files_manifested: c.try_get(6)?.try_int()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffSummary {
    pub has_differences: bool,
    pub repos_examined: i64,
    pub repos_with_differences: i64,
    pub files_changed: i64,
    pub insertions: i64,
    pub deletions: i64,
    pub repo_summaries: Vec<DiffRepoSummary>,
}
impl DiffSummary {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bool(self.has_differences)),
            (2, Cbor::Int(self.repos_examined)),
            (3, Cbor::Int(self.repos_with_differences)),
            (4, Cbor::Int(self.files_changed)),
            (5, Cbor::Int(self.insertions)),
            (6, Cbor::Int(self.deletions)),
            (7, Cbor::Array(self.repo_summaries.iter().map(|x| x.to_cbor()).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            has_differences: c.try_get(1)?.try_bool()?,
            repos_examined: c.try_get(2)?.try_int()?,
            repos_with_differences: c.try_get(3)?.try_int()?,
            files_changed: c.try_get(4)?.try_int()?,
            insertions: c.try_get(5)?.try_int()?,
            deletions: c.try_get(6)?.try_int()?,
            repo_summaries: c.try_get(7)?.try_array()?.iter().map(|x| DiffRepoSummary::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffOutputLogRef {
    pub log_id: String,
    pub format: DiffOutputFormat,
    pub encoding: Option<DiffChunkEncoding>,
}
impl DiffOutputLogRef {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Text(self.log_id.clone())),
            (2, Cbor::Int(self.format.wire())),
            (3, match &self.encoding { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            log_id: c.try_get(1)?.try_text()?,
            format: DiffOutputFormat::from_wire(c.try_get(2)?.try_int()?)?,
            encoding: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(DiffChunkEncoding::from_wire(v.try_int()?)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffManifestResponse {
    pub response: ResponseEnvelope,
    pub files: Vec<DiffFileEntry>,
    pub summary: Option<DiffSummary>,
    pub targets: Vec<DiffParsedTarget>,
    pub output: Option<DiffOutputLogRef>,
    pub excluded_targets: Vec<DiffExcludedTarget>,
}
impl DiffManifestResponse {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.response.to_cbor()),
            (2, Cbor::Array(self.files.iter().map(|x| x.to_cbor()).collect())),
            (3, match &self.summary { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (4, Cbor::Array(self.targets.iter().map(|x| x.to_cbor()).collect())),
            (5, match &self.output { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (6, Cbor::Array(self.excluded_targets.iter().map(|x| x.to_cbor()).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            response: ResponseEnvelope::from_cbor(c.try_get(1)?)?,
            files: c.try_get(2)?.try_array()?.iter().map(|x| DiffFileEntry::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            summary: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(DiffSummary::from_cbor(v)?) } },
            targets: c.try_get(4)?.try_array()?.iter().map(|x| DiffParsedTarget::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            output: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(DiffOutputLogRef::from_cbor(v)?) } },
            excluded_targets: c.try_get(6)?.try_array()?.iter().map(|x| DiffExcludedTarget::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffOutputRecord {
    pub kind: DiffOutputRecordKind,
    pub scope: Option<DiffRepoScope>,
    pub file_id: Option<String>,
    pub entry: Option<DiffFileEntry>,
    pub data: Option<Vec<u8>>,
    pub stale: Option<bool>,
    pub diagnostic: Option<String>,
}
impl DiffOutputRecord {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.kind.wire())),
            (2, match &self.scope { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (3, match &self.file_id { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
            (4, match &self.entry { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (5, match &self.data { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (6, match &self.stale { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
            (7, match &self.diagnostic { Some(v) => Cbor::Text(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            kind: DiffOutputRecordKind::from_wire(c.try_get(1)?.try_int()?)?,
            scope: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(DiffRepoScope::from_cbor(v)?) } },
            file_id: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_text()?) } },
            entry: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(DiffFileEntry::from_cbor(v)?) } },
            data: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            stale: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_bool()?) } },
            diagnostic: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_text()?) } },
        })
    }
}
