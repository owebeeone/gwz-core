// GENERATED native Rust types + codec — do not edit.
#![allow(dead_code)]
use crate::cbor::Cbor;

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
    } }
    pub fn from_wire(v: i64) -> Self { match v {
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
        _ => panic!("bad ActionKind wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Create,
        1 => Self::List,
        2 => Self::Fetch,
        3 => Self::Push,
        4 => Self::Delete,
        _ => panic!("bad TagOp wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Push,
        1 => Self::List,
        2 => Self::Apply,
        3 => Self::Pop,
        4 => Self::Drop,
        _ => panic!("bad StashOp wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Stashed,
        1 => Self::Empty,
        2 => Self::Skipped,
        _ => panic!("bad StashParticipation wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Unattempted,
        1 => Self::Saving,
        2 => Self::Saved,
        3 => Self::Empty,
        4 => Self::Failed,
        _ => panic!("bad StashPushLifecycle wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Pending,
        1 => Self::Applied,
        2 => Self::Popped,
        3 => Self::Dropped,
        4 => Self::Noop,
        5 => Self::Missing,
        _ => panic!("bad StashRestoreState wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::List,
        1 => Self::Create,
        2 => Self::Delete,
        3 => Self::Merge,
        _ => panic!("bad BranchOp wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Listed,
        1 => Self::Created,
        2 => Self::Exists,
        3 => Self::Deleted,
        4 => Self::Switched,
        5 => Self::Noop,
        6 => Self::Skipped,
        7 => Self::Merged,
        8 => Self::Conflicted,
        _ => panic!("bad BranchActionResult wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Argv,
        1 => Self::Shell,
        _ => panic!("bad ExecMode wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Git,
        1 => Self::Archive,
        2 => Self::Package,
        3 => Self::Local,
        4 => Self::Generated,
        _ => panic!("bad SourceKind wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Root,
        1 => Self::Member,
        _ => panic!("bad TargetKind wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Accepted,
        1 => Self::Ok,
        2 => Self::Noop,
        3 => Self::Rejected,
        4 => Self::Partial,
        5 => Self::Failed,
        6 => Self::Dirty,
        7 => Self::Conflicted,
        _ => panic!("bad AggregateStatus wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Planned,
        1 => Self::Ok,
        2 => Self::Noop,
        3 => Self::Skipped,
        4 => Self::Rejected,
        5 => Self::Failed,
        6 => Self::Conflicted,
        _ => panic!("bad MemberStatus wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Lock,
        1 => Self::Head,
        2 => Self::Snapshot,
        3 => Self::Tag,
        4 => Self::Commit,
        5 => Self::Branch,
        _ => panic!("bad MaterializeTargetKind wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Current,
        1 => Self::Branch,
        _ => panic!("bad SnapshotSourceKind wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::FetchOnly,
        1 => Self::FfOnly,
        2 => Self::Merge,
        3 => Self::Rebase,
        4 => Self::Reset,
        5 => Self::DriverSelected,
        _ => panic!("bad SyncBehavior wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Atomic,
        1 => Self::Partial,
        _ => panic!("bad PartialBehavior wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Refuse,
        1 => Self::Allow,
        _ => panic!("bad DestructiveBehavior wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Fail,
        1 => Self::Skip,
        _ => panic!("bad UnsupportedMemberBehavior wire value {}", v),
    } }
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
    } }
    pub fn from_wire(v: i64) -> Self { match v {
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
        _ => panic!("bad PlannedAction wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Unknown,
        1 => Self::Matches,
        2 => Self::Differs,
        3 => Self::Missing,
        _ => panic!("bad LockMatch wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Enumerating,
        1 => Self::Counting,
        2 => Self::Compressing,
        3 => Self::Receiving,
        4 => Self::Resolving,
        5 => Self::CheckingOut,
        6 => Self::Writing,
        _ => panic!("bad GitProgressPhase wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Summary,
        1 => Self::Combined,
        _ => panic!("bad StatusMode wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::MemberRelative,
        1 => Self::WorkspaceRelative,
        _ => panic!("bad StatusPathStyle wire value {}", v),
    } }
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
    } }
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::OperationStarted,
        1 => Self::MemberStarted,
        2 => Self::MemberProgress,
        3 => Self::MemberFinished,
        4 => Self::ArtifactWritten,
        5 => Self::OperationFinished,
        6 => Self::Reset,
        _ => panic!("bad EventKind wire value {}", v),
    } }
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
    pub fn from_wire(v: i64) -> Self { match v {
        0 => Self::Debug,
        1 => Self::Info,
        2 => Self::Warn,
        3 => Self::Error,
        _ => panic!("bad Severity wire value {}", v),
    } }
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
    } }
    pub fn from_wire(v: i64) -> Self { match v {
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
        _ => panic!("bad GwzErrorCode wire value {}", v),
    } }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            root: { let v = c.get(1); if v.is_null() { None } else { Some(v.text()) } },
            workspace_id: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            actor_id: c.get(1).text(),
            display_name: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
            email: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            authority: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            name: c.get(1).text(),
            email: c.get(2).text(),
            time_ms: { let v = c.get(3); if v.is_null() { None } else { Some(v.int()) } },
            timezone_offset_minutes: { let v = c.get(4); if v.is_null() { None } else { Some(v.int()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            actor: { let v = c.get(1); if v.is_null() { None } else { Some(OperationActor::from_cbor(v)) } },
            git_author: { let v = c.get(2); if v.is_null() { None } else { Some(GitObjectIdentity::from_cbor(v)) } },
            git_committer: { let v = c.get(3); if v.is_null() { None } else { Some(GitObjectIdentity::from_cbor(v)) } },
            credential_ref: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            all: { let v = c.get(1); if v.is_null() { None } else { Some(v.boolean()) } },
            member_ids: c.get(2).array().iter().map(|x| x.text()).collect(),
            paths: c.get(3).array().iter().map(|x| x.text()).collect(),
            targets: c.get(4).array().iter().map(|x| x.text()).collect(),
            exclude_targets: c.get(5).array().iter().map(|x| x.text()).collect(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            partial: { let v = c.get(1); if v.is_null() { None } else { Some(PartialBehavior::from_wire(v.int())) } },
            destructive: { let v = c.get(2); if v.is_null() { None } else { Some(DestructiveBehavior::from_wire(v.int())) } },
            sync: { let v = c.get(3); if v.is_null() { None } else { Some(SyncBehavior::from_wire(v.int())) } },
            unsupported_member: { let v = c.get(4); if v.is_null() { None } else { Some(UnsupportedMemberBehavior::from_wire(v.int())) } },
            remote: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
            concurrency: { let v = c.get(6); if v.is_null() { None } else { Some(v.int()) } },
            progress_min_interval_ms: { let v = c.get(7); if v.is_null() { None } else { Some(v.int()) } },
            max_connections_per_host: { let v = c.get(8); if v.is_null() { None } else { Some(v.int()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            request_id: c.get(1).text(),
            schema_version: c.get(2).text(),
            workspace: { let v = c.get(3); if v.is_null() { None } else { Some(WorkspaceRef::from_cbor(v)) } },
            selection: { let v = c.get(4); if v.is_null() { None } else { Some(Selection::from_cbor(v)) } },
            policy: { let v = c.get(5); if v.is_null() { None } else { Some(OperationPolicy::from_cbor(v)) } },
            dry_run: { let v = c.get(6); if v.is_null() { None } else { Some(v.boolean()) } },
            attribution: { let v = c.get(7); if v.is_null() { None } else { Some(OperationAttribution::from_cbor(v)) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            request_id: c.get(1).text(),
            schema_version: c.get(2).text(),
            action: ActionKind::from_wire(c.get(3).int()),
            aggregate_status: AggregateStatus::from_wire(c.get(4).int()),
            operation_id: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
            message: { let v = c.get(6); if v.is_null() { None } else { Some(v.text()) } },
            attribution: { let v = c.get(7); if v.is_null() { None } else { Some(OperationAttribution::from_cbor(v)) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            code: GwzErrorCode::from_wire(c.get(1).int()),
            message: c.get(2).text(),
            member_id: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            member_path: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
            detail: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
            target_kind: { let v = c.get(6); if v.is_null() { None } else { Some(TargetKind::from_wire(v.int())) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            name: c.get(1).text(),
            url: c.get(2).text(),
            fetch: { let v = c.get(3); if v.is_null() { None } else { Some(v.boolean()) } },
            push: { let v = c.get(4); if v.is_null() { None } else { Some(v.boolean()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            branch: { let v = c.get(1); if v.is_null() { None } else { Some(v.text()) } },
            commit: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
            git_tag: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            local_only: { let v = c.get(4); if v.is_null() { None } else { Some(v.boolean()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            url: c.get(1).text(),
            path: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
            remote_name: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            branch: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            member_id: c.get(1).text(),
            path: c.get(2).text(),
            source_id: c.get(3).text(),
            source_kind: SourceKind::from_wire(c.get(4).int()),
            active: c.get(5).boolean(),
            desired: { let v = c.get(6); if v.is_null() { None } else { Some(DesiredRef::from_cbor(v)) } },
            remotes: c.get(7).array().iter().map(|x| RemoteSpec::from_cbor(x)).collect(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            kind: MaterializeTargetKind::from_wire(c.get(1).int()),
            name: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
            commit: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            kind: SnapshotSourceKind::from_wire(c.get(1).int()),
            branch: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            member_id: c.get(1).text(),
            path: c.get(2).text(),
            source_id: c.get(3).text(),
            source_kind: SourceKind::from_wire(c.get(4).int()),
            commit: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
            branch: { let v = c.get(6); if v.is_null() { None } else { Some(v.text()) } },
            detached: { let v = c.get(7); if v.is_null() { None } else { Some(v.boolean()) } },
            upstream: { let v = c.get(8); if v.is_null() { None } else { Some(v.text()) } },
            dirty: { let v = c.get(9); if v.is_null() { None } else { Some(v.boolean()) } },
            materialized: c.get(10).boolean(),
            remotes: c.get(11).array().iter().map(|x| RemoteSpec::from_cbor(x)).collect(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            member_id: c.get(1).text(),
            branch: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
            detached: c.get(3).boolean(),
            head: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
            upstream: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
            ahead: { let v = c.get(6); if v.is_null() { None } else { Some(v.int()) } },
            behind: { let v = c.get(7); if v.is_null() { None } else { Some(v.int()) } },
            staged: c.get(8).int(),
            unstaged: c.get(9).int(),
            untracked: c.get(10).int(),
            dirty: c.get(11).boolean(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            member_id: c.get(1).text(),
            member_path: c.get(2).text(),
            repo_path: c.get(3).text(),
            workspace_path: c.get(4).text(),
            index_status: c.get(5).text(),
            worktree_status: c.get(6).text(),
            original_repo_path: { let v = c.get(7); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            phase: GitProgressPhase::from_wire(c.get(1).int()),
            received_objects: { let v = c.get(2); if v.is_null() { None } else { Some(v.int()) } },
            total_objects: { let v = c.get(3); if v.is_null() { None } else { Some(v.int()) } },
            received_bytes: { let v = c.get(4); if v.is_null() { None } else { Some(v.int()) } },
            indexed_deltas: { let v = c.get(5); if v.is_null() { None } else { Some(v.int()) } },
            total_deltas: { let v = c.get(6); if v.is_null() { None } else { Some(v.int()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            branch: { let v = c.get(1); if v.is_null() { None } else { Some(v.text()) } },
            detached: c.get(2).boolean(),
            head: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            staged: c.get(4).int(),
            unstaged: c.get(5).int(),
            untracked: c.get(6).int(),
            dirty: c.get(7).boolean(),
            unborn: c.get(8).boolean(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            repo_path: c.get(1).text(),
            workspace_path: c.get(2).text(),
            index_status: c.get(3).text(),
            worktree_status: c.get(4).text(),
            original_repo_path: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            member_id: c.get(1).text(),
            member_path: c.get(2).text(),
            label: c.get(3).text(),
            branch: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
            detached: c.get(5).boolean(),
            unborn: c.get(6).boolean(),
            head: { let v = c.get(7); if v.is_null() { None } else { Some(v.text()) } },
            upstream: { let v = c.get(8); if v.is_null() { None } else { Some(v.text()) } },
            ahead: { let v = c.get(9); if v.is_null() { None } else { Some(v.int()) } },
            behind: { let v = c.get(10); if v.is_null() { None } else { Some(v.int()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            label: c.get(1).text(),
            member_ids: c.get(2).array().iter().map(|x| x.text()).collect(),
            member_paths: c.get(3).array().iter().map(|x| x.text()).collect(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            label: c.get(1).text(),
            majority_label: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
            member_ids: c.get(3).array().iter().map(|x| x.text()).collect(),
            member_paths: c.get(4).array().iter().map(|x| x.text()).collect(),
            message: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            clean: c.get(1).boolean(),
            file_changes: c.get(2).array().iter().map(|x| GitFileChange::from_cbor(x)).collect(),
            branches: c.get(3).array().iter().map(|x| GitMemberBranchStatus::from_cbor(x)).collect(),
            branch_groups: c.get(4).array().iter().map(|x| GitBranchGroup::from_cbor(x)).collect(),
            branch_differences: c.get(5).array().iter().map(|x| GitBranchDifference::from_cbor(x)).collect(),
            root_status: { let v = c.get(6); if v.is_null() { None } else { Some(WorkspaceRootGitStatus::from_cbor(v)) } },
            root_file_changes: c.get(7).array().iter().map(|x| WorkspaceRootFileChange::from_cbor(x)).collect(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            staged: c.get(1).boolean(),
            unstaged: c.get(2).boolean(),
            untracked: c.get(3).boolean(),
            ignored: c.get(4).boolean(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            code: c.get(1).text(),
            message: c.get(2).text(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            code: c.get(1).text(),
            message: c.get(2).text(),
            member_id: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            code: c.get(1).text(),
            message: c.get(2).text(),
            member_id: c.get(3).text(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            member_id: c.get(1).text(),
            path: c.get(2).text(),
            participation: StashParticipation::from_wire(c.get(3).int()),
            push_lifecycle: StashPushLifecycle::from_wire(c.get(4).int()),
            restore_state: StashRestoreState::from_wire(c.get(5).int()),
            branch_before: { let v = c.get(6); if v.is_null() { None } else { Some(v.text()) } },
            head_before: { let v = c.get(7); if v.is_null() { None } else { Some(v.text()) } },
            full_stash_message: c.get(8).text(),
            dirty_summary: StashDirtySummary::from_cbor(c.get(9)),
            native_stash_object_id: { let v = c.get(10); if v.is_null() { None } else { Some(v.text()) } },
            native_stash_display_ref: { let v = c.get(11); if v.is_null() { None } else { Some(v.text()) } },
            error: { let v = c.get(12); if v.is_null() { None } else { Some(StashErrorDetail::from_cbor(v)) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            schema: c.get(1).text(),
            workspace_id: c.get(2).text(),
            stash_id: c.get(3).text(),
            created_at: c.get(4).text(),
            message_suffix: c.get(5).text(),
            include_untracked: c.get(6).boolean(),
            include_ignored: c.get(7).boolean(),
            members: c.get(8).array().iter().map(|x| StashBundleMember::from_cbor(x)).collect(),
            warnings: c.get(9).array().iter().map(|x| StashWarning::from_cbor(x)).collect(),
            drift: c.get(10).array().iter().map(|x| StashDrift::from_cbor(x)).collect(),
            selected_members: c.get(11).array().iter().map(|x| x.text()).collect(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            member_id: c.get(1).text(),
            member_path: c.get(2).text(),
            source_kind: SourceKind::from_wire(c.get(3).int()),
            result: BranchActionResult::from_wire(c.get(4).int()),
            branch: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
            current_branch: { let v = c.get(6); if v.is_null() { None } else { Some(v.text()) } },
            detached: c.get(7).boolean(),
            unborn: c.get(8).boolean(),
            head: { let v = c.get(9); if v.is_null() { None } else { Some(v.text()) } },
            upstream: { let v = c.get(10); if v.is_null() { None } else { Some(v.text()) } },
            ahead: { let v = c.get(11); if v.is_null() { None } else { Some(v.int()) } },
            behind: { let v = c.get(12); if v.is_null() { None } else { Some(v.int()) } },
            source_ref: { let v = c.get(13); if v.is_null() { None } else { Some(v.text()) } },
            target_branch: { let v = c.get(14); if v.is_null() { None } else { Some(v.text()) } },
            resulting_commit: { let v = c.get(15); if v.is_null() { None } else { Some(v.text()) } },
            conflict_paths: c.get(16).array().iter().map(|x| x.text()).collect(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            action: PlannedAction::from_wire(c.get(1).int()),
            from_ref: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
            to_ref: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            message: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            member_id: c.get(1).text(),
            member_path: c.get(2).text(),
            source_kind: SourceKind::from_wire(c.get(3).int()),
            status: MemberStatus::from_wire(c.get(4).int()),
            error: { let v = c.get(5); if v.is_null() { None } else { Some(GwzError::from_cbor(v)) } },
            planned: { let v = c.get(6); if v.is_null() { None } else { Some(PlannedChange::from_cbor(v)) } },
            state: { let v = c.get(7); if v.is_null() { None } else { Some(ResolvedMemberState::from_cbor(v)) } },
            git_status: { let v = c.get(8); if v.is_null() { None } else { Some(GitStatus::from_cbor(v)) } },
            lock_match: { let v = c.get(9); if v.is_null() { None } else { Some(LockMatch::from_wire(v.int())) } },
            target_kind: { let v = c.get(10); if v.is_null() { None } else { Some(TargetKind::from_wire(v.int())) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: ResponseMeta::from_cbor(c.get(1)),
            members: c.get(2).array().iter().map(|x| MemberResponse::from_cbor(x)).collect(),
            errors: c.get(3).array().iter().map(|x| GwzError::from_cbor(x)).collect(),
        }
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
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            operation_id: c.get(1).text(),
            request_id: c.get(2).text(),
            sequence: c.get(3).int(),
            timestamp_ms: c.get(4).int(),
            kind: EventKind::from_wire(c.get(5).int()),
            severity: Severity::from_wire(c.get(6).int()),
            member_id: { let v = c.get(7); if v.is_null() { None } else { Some(v.text()) } },
            member_path: { let v = c.get(8); if v.is_null() { None } else { Some(v.text()) } },
            message: { let v = c.get(9); if v.is_null() { None } else { Some(v.text()) } },
            member: { let v = c.get(10); if v.is_null() { None } else { Some(MemberResponse::from_cbor(v)) } },
            error: { let v = c.get(11); if v.is_null() { None } else { Some(GwzError::from_cbor(v)) } },
            attribution: { let v = c.get(12); if v.is_null() { None } else { Some(OperationAttribution::from_cbor(v)) } },
            progress: { let v = c.get(13); if v.is_null() { None } else { Some(GitTransferProgress::from_cbor(v)) } },
            target_kind: { let v = c.get(14); if v.is_null() { None } else { Some(TargetKind::from_wire(v.int())) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            operation_id: c.get(1).text(),
            request_id: c.get(2).text(),
            action: ActionKind::from_wire(c.get(3).int()),
            aggregate_status: AggregateStatus::from_wire(c.get(4).int()),
            started_at_ms: c.get(5).int(),
            finished_at_ms: c.get(6).int(),
            members: c.get(7).array().iter().map(|x| MemberResponse::from_cbor(x)).collect(),
            errors: c.get(8).array().iter().map(|x| GwzError::from_cbor(x)).collect(),
            attribution: { let v = c.get(9); if v.is_null() { None } else { Some(OperationAttribution::from_cbor(v)) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            workspace_root: c.get(2).text(),
            workspace_id: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            workspace_root: c.get(2).text(),
            sources: c.get(3).array().iter().map(|x| SourceUrl::from_cbor(x)).collect(),
            target: { let v = c.get(4); if v.is_null() { None } else { Some(MaterializeTarget::from_cbor(v)) } },
            workspace_id: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            url: c.get(2).text(),
            target: c.get(3).text(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            repository_path: c.get(2).text(),
            member_path: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            member_id: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
            source_id: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            member_path: c.get(2).text(),
            initial_branch: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            member_id: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
            source_id: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            target: MaterializeTarget::from_cbor(c.get(2)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            mode: { let v = c.get(2); if v.is_null() { None } else { Some(StatusMode::from_wire(v.int())) } },
            include_file_changes: { let v = c.get(3); if v.is_null() { None } else { Some(v.boolean()) } },
            include_branch_summary: { let v = c.get(4); if v.is_null() { None } else { Some(v.boolean()) } },
            path_style: { let v = c.get(5); if v.is_null() { None } else { Some(StatusPathStyle::from_wire(v.int())) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            include_unmaterialized: { let v = c.get(2); if v.is_null() { None } else { Some(v.boolean()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            id: c.get(1).text(),
            path: c.get(2).text(),
            abspath: c.get(3).text(),
            materialized: c.get(4).boolean(),
            target_kind: { let v = c.get(5); if v.is_null() { None } else { Some(TargetKind::from_wire(v.int())) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
            members: { let v = c.get(2); if v.is_null() { None } else { Some(v.array().iter().map(|x| MemberEntry::from_cbor(x)).collect()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            id: c.get(1).text(),
            path: c.get(2).text(),
            exit_code: { let v = c.get(3); if v.is_null() { None } else { Some(v.int()) } },
            signal: { let v = c.get(4); if v.is_null() { None } else { Some(v.int()) } },
            spawn_error: { let v = c.get(5); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            mode: ExecMode::from_wire(c.get(2).int()),
            command: c.get(3).array().iter().map(|x| x.text()).collect(),
            members: c.get(4).array().iter().map(|x| MemberEntry::from_cbor(x)).collect(),
            continue_on_fail: { let v = c.get(5); if v.is_null() { None } else { Some(v.boolean()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
            results: { let v = c.get(2); if v.is_null() { None } else { Some(v.array().iter().map(|x| ExecResult::from_cbor(x)).collect()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            snapshot_id: c.get(2).text(),
            source: { let v = c.get(3); if v.is_null() { None } else { Some(SnapshotSource::from_cbor(v)) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            op: TagOp::from_wire(c.get(2).int()),
            name: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            message: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
            signed: { let v = c.get(5); if v.is_null() { None } else { Some(v.boolean()) } },
            remote: { let v = c.get(6); if v.is_null() { None } else { Some(v.text()) } },
            all: { let v = c.get(7); if v.is_null() { None } else { Some(v.boolean()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommitRequest {
    pub meta: RequestMeta,
    pub message: String,
    pub all: Option<bool>,
}
impl CommitRequest {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.meta.to_cbor()),
            (2, Cbor::Text(self.message.clone())),
            (3, match &self.all { Some(v) => Cbor::Bool(*v), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            message: c.get(2).text(),
            all: { let v = c.get(3); if v.is_null() { None } else { Some(v.boolean()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            cwd: c.get(2).text(),
            pathspecs: c.get(3).array().iter().map(|x| x.text()).collect(),
            all: { let v = c.get(4); if v.is_null() { None } else { Some(v.boolean()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            snapshot_id: c.get(2).text(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            remote: { let v = c.get(2); if v.is_null() { None } else { Some(v.text()) } },
            refspec: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            op: StashOp::from_wire(c.get(2).int()),
            stash_id: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            message: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
            include_untracked: { let v = c.get(5); if v.is_null() { None } else { Some(v.boolean()) } },
            include_ignored: { let v = c.get(6); if v.is_null() { None } else { Some(v.boolean()) } },
            expanded: { let v = c.get(7); if v.is_null() { None } else { Some(v.boolean()) } },
            preserve_index: { let v = c.get(8); if v.is_null() { None } else { Some(v.boolean()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            meta: RequestMeta::from_cbor(c.get(1)),
            op: BranchOp::from_wire(c.get(2).int()),
            name: { let v = c.get(3); if v.is_null() { None } else { Some(v.text()) } },
            start_ref: { let v = c.get(4); if v.is_null() { None } else { Some(v.text()) } },
            switch_after_create: { let v = c.get(5); if v.is_null() { None } else { Some(v.boolean()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
            workspace_git_status: { let v = c.get(2); if v.is_null() { None } else { Some(WorkspaceGitStatus::from_cbor(v)) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            name: c.get(1).text(),
            created_at: c.get(2).text(),
            created_by: c.get(3).text(),
            members: c.get(4).int(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
            snapshots: { let v = c.get(2); if v.is_null() { None } else { Some(v.array().iter().map(|x| SnapshotInfo::from_cbor(x)).collect()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            name: c.get(1).text(),
            members: c.get(2).int(),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
            tags: { let v = c.get(2); if v.is_null() { None } else { Some(v.array().iter().map(|x| TagInfo::from_cbor(x)).collect()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
            bundles: { let v = c.get(2); if v.is_null() { None } else { Some(v.array().iter().map(|x| StashBundle::from_cbor(x)).collect()) } },
        }
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
    pub fn from_cbor(c: &Cbor) -> Self {
        Self {
            response: ResponseEnvelope::from_cbor(c.get(1)),
            repos: { let v = c.get(2); if v.is_null() { None } else { Some(v.array().iter().map(|x| BranchRepoSummary::from_cbor(x)).collect()) } },
        }
    }
}
