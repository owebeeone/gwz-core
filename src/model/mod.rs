use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::runtime::clock::TimestampMs;

pub type ModelResult<T> = Result<T, ModelError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Ok,
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
    UnsupportedRecordVersion,
    UnsupportedLegacyMode,
    ArchivedRecordUnreadable,
    UnexpectedAcceptanceEvidence,
    AcceptanceInputDrift,
    CandidateIntegrityMismatch,
    AmbiguousEvidenceCommit,
    RecordedEvidenceDrift,
    PublicationPrefixMismatch,
    PublishedCandidateMismatch,
    PreservationEvidenceMismatch,
    RollbackEvidenceMismatch,
    UnexpectedPublicationEvidence,
    TerminalEvidenceMismatch,
    RecoveryEvidenceMismatch,
    TerminalRollbackMismatch,
    /// The family-only merge miss: `gwz merge --remote <name>` named no ready
    /// family member -- absent, reserved (`origin`) or creating/disposing --
    /// and merge has no Git-remote fallback (gwz-dev
    /// dev-docs/GwzLocalCloneDesign.md §6/§7; wire `unknown_local` = 62).
    UnknownLocal,
    /// A design §4.0 source-layout hazard refused before reservation -- a
    /// gitfile, an external common directory, alternates, escaping metadata
    /// or configuration, a partial clone, an environment override; v0
    /// refuses, never rewrites (LCM1.1 fix 1, 2026-09-06; wire
    /// `unsupported_source_layout` = 63).
    UnsupportedSourceLayout,
    /// The local clone's tree copy stopped (permission, space, I/O, metadata,
    /// an uncopyable entry); the `creating` row and the partial destination
    /// are retained (design §4, §12; wire `copy_failed` = 64).
    CopyFailed,
    /// The source changed between the snapshot and publication (design §4
    /// step 3's recheck, §12); the destination is not marked ready (wire
    /// `source_drift` = 65).
    SourceDrift,
    /// The destination failed a completion rule before ready -- §4.0
    /// dest-complete, §4.1's at-ready column, lock recapture, marker
    /// regeneration -- or the install was cancelled; the row and directory
    /// are retained for inspection (wire `destination_incomplete` = 66).
    DestinationIncomplete,
    /// A family merge's two workspaces are no longer the same shape: a
    /// member id on one side only, the same id at different recorded paths
    /// or with a different source identity, or a selected `@root` with no
    /// root to pair (design §6). Refused before any fetch; nothing written
    /// (wire `pairing_mismatch` = 67; LCM1.2).
    PairingMismatch,
    /// A family merge's import stopped before the engine was entered -- a
    /// fetch or receiver read failed, or the import was cancelled. The
    /// import refs created before the stop are retained and named; the
    /// engine was not entered; a retry mints a fresh transfer id (wire
    /// `import_incomplete` = 68; LCM1.2).
    ImportIncomplete,
    /// Ordinary `gwz local dispose` found one or more known deletion
    /// hazards that `--force` did not name -- an open merge or unfinished
    /// native operation (`open-merge`), uncommitted, untracked, ignored,
    /// suppressed or stashed work (`dirty`), or history preserved whole in
    /// no surviving family repository (`unpreserved-history`; design §5,
    /// §5.1). The message lists every finding per repository; refused
    /// before `disposing`, nothing removed (wire `unwaived_hazard` = 69;
    /// LCM2.2).
    UnwaivedHazard,
    /// The deletion tree's work or history evidence could not be
    /// established -- an unreadable path or store, an unsupported index
    /// flag, an uninterpretable layout or coordination record, a verifier
    /// limit -- so ordinary `dispose` refuses and no force name waives it
    /// (design §5.1); `--keep` still detaches. Nothing removed (wire
    /// `unknown_evidence` = 70; LCM2.1).
    UnknownEvidence,
    /// The directory removal of an ordinary `dispose` stopped part-way: the
    /// row is `disposing`, what remains is named, there is no replay and a
    /// repeat is refused (design §5.2); manual cleanup, then an explicit
    /// dispose removes the stale row, or `--keep` detaches the remainder
    /// (wire `disposal_incomplete` = 71; LCM2.2).
    DisposalIncomplete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelError {
    pub code: ErrorCode,
    pub message: String,
    pub member_id: Option<String>,
    pub member_path: Option<String>,
    pub record_context: Option<Box<crate::MergeRecordCompatibilityContext>>,
}

impl ModelError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            member_id: None,
            member_path: None,
            record_context: None,
        }
    }

    pub fn with_member(
        mut self,
        member_id: impl Into<String>,
        member_path: impl Into<String>,
    ) -> Self {
        let member_id = member_id.into();
        let member_path = member_path.into();
        let target = if member_id == "@root" && member_path == "." {
            "workspace root '@root' at '.'".to_owned()
        } else {
            format!("member '{member_id}' at '{member_path}'")
        };
        self.message = format!("{target}: {}", self.message);
        self.member_id = Some(member_id);
        self.member_path = Some(member_path);
        self
    }

    pub fn with_record_context(
        mut self,
        record_context: crate::MergeRecordCompatibilityContext,
    ) -> Self {
        self.record_context = Some(Box::new(record_context));
        self
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ModelError {}

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            pub fn parse_str(value: &str) -> ModelResult<Self> {
                parse_id(Self::PREFIX, value).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = ModelError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse_str(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(WorkspaceId, "ws_");
id_type!(SourceId, "src_");
id_type!(MemberId, "mem_");
id_type!(OperationId, "op_");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    Git,
    Archive,
    Package,
    Local,
    Generated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceSpec {
    pub id: WorkspaceId,
    pub sources: Vec<SourceSpec>,
    pub members: Vec<MemberSpec>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSpec {
    pub id: SourceId,
    pub kind: SourceKind,
    pub remotes: Vec<RemoteSpec>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberSpec {
    pub id: MemberId,
    pub path: String,
    pub source_id: SourceId,
    pub source_kind: SourceKind,
    pub active: bool,
    pub desired: Option<DesiredRef>,
    pub remotes: Vec<RemoteSpec>,
}

impl MemberSpec {
    pub fn new(
        id: MemberId,
        path: impl Into<String>,
        source_id: SourceId,
        source_kind: SourceKind,
        active: bool,
        desired: Option<DesiredRef>,
        remotes: Vec<RemoteSpec>,
    ) -> ModelResult<Self> {
        reject_duplicate_remote_names(&remotes)?;
        for remote in &remotes {
            remote.validate()?;
        }
        if let Some(desired) = &desired {
            desired.validate()?;
        }
        Ok(Self {
            id,
            path: path.into(),
            source_id,
            source_kind,
            active,
            desired,
            remotes,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSpec {
    pub name: String,
    pub url: String,
    pub fetch: bool,
    pub push: bool,
}

impl RemoteSpec {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            fetch: true,
            push: true,
        }
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_non_empty("remote.name", &self.name)?;
        require_non_empty("remote.url", &self.url)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesiredRef {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub git_tag: Option<String>,
    pub local_only: Option<bool>,
}

impl DesiredRef {
    pub fn branch(branch: impl Into<String>) -> Self {
        Self {
            branch: Some(branch.into()),
            ..Self::default()
        }
    }

    pub fn git_tag(git_tag: impl Into<String>) -> Self {
        Self {
            git_tag: Some(git_tag.into()),
            ..Self::default()
        }
    }

    pub fn local_only() -> Self {
        Self {
            local_only: Some(true),
            ..Self::default()
        }
    }

    pub fn validate(&self) -> ModelResult<()> {
        if self.local_only == Some(false) {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "desired.local_only must be true when present",
            ));
        }

        let mut targets = 0;
        targets += validate_optional_target("desired.branch", &self.branch)?;
        targets += validate_optional_target("desired.commit", &self.commit)?;
        targets += validate_optional_target("desired.git_tag", &self.git_tag)?;
        if self.local_only == Some(true) {
            targets += 1;
        }

        if targets == 1 {
            Ok(())
        } else {
            Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "desired ref must specify exactly one target",
            ))
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationActor {
    pub actor_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub authority: Option<String>,
}

impl OperationActor {
    pub fn new(actor_id: impl Into<String>) -> Self {
        Self {
            actor_id: actor_id.into(),
            ..Self::default()
        }
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_non_empty("actor.actor_id", &self.actor_id)?;
        validate_optional_text("actor.display_name", &self.display_name)?;
        validate_optional_text("actor.email", &self.email)?;
        validate_optional_text("actor.authority", &self.authority)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitObjectIdentity {
    pub name: String,
    pub email: String,
    pub time_ms: Option<TimestampMs>,
    pub timezone_offset_minutes: Option<i64>,
}

impl GitObjectIdentity {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
            ..Self::default()
        }
    }

    pub fn validate(&self) -> ModelResult<()> {
        validate_git_identity_field("git_identity.name", &self.name)?;
        validate_git_identity_field("git_identity.email", &self.email)?;
        if let Some(offset) = self.timezone_offset_minutes
            && !(-1_440..=1_440).contains(&offset)
        {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "git identity timezone offset is out of range",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationAttribution {
    pub actor: Option<OperationActor>,
    pub git_author: Option<GitObjectIdentity>,
    pub git_committer: Option<GitObjectIdentity>,
    pub credential_ref: Option<String>,
}

impl OperationAttribution {
    pub fn validate(&self) -> ModelResult<()> {
        if let Some(actor) = &self.actor {
            actor.validate()?;
        }
        if let Some(author) = &self.git_author {
            author.validate()?;
        }
        if let Some(committer) = &self.git_committer {
            committer.validate()?;
        }
        validate_optional_text("credential_ref", &self.credential_ref)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    pub all: bool,
    pub member_ids: Vec<MemberId>,
    pub paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartialBehavior {
    Atomic,
    Partial,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DestructiveBehavior {
    Refuse,
    Allow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncBehavior {
    FetchOnly,
    FfOnly,
    Merge,
    Rebase,
    Reset,
    DriverSelected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedMemberBehavior {
    Fail,
    Skip,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationPolicy {
    pub partial: PartialBehavior,
    pub destructive: DestructiveBehavior,
    pub sync: SyncBehavior,
    pub unsupported_member: UnsupportedMemberBehavior,
    pub remote: Option<String>,
    pub concurrency: Option<usize>,
}

impl OperationPolicy {
    pub fn builtin_default() -> Self {
        Self::default()
    }
}

impl Default for OperationPolicy {
    fn default() -> Self {
        Self {
            partial: PartialBehavior::Atomic,
            destructive: DestructiveBehavior::Refuse,
            sync: SyncBehavior::FfOnly,
            unsupported_member: UnsupportedMemberBehavior::Fail,
            remote: None,
            concurrency: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMemberState {
    pub member_id: MemberId,
    pub path: String,
    pub source_id: SourceId,
    pub source_kind: SourceKind,
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub dirty: bool,
    pub materialized: bool,
    pub remotes: Vec<RemoteSpec>,
}

fn parse_id(prefix: &str, value: &str) -> ModelResult<String> {
    let valid = value.starts_with(prefix)
        && value.len() > prefix.len()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    if valid {
        Ok(value.to_owned())
    } else {
        Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("id must start with {prefix} and contain only portable characters"),
        ))
    }
}

fn reject_duplicate_remote_names(remotes: &[RemoteSpec]) -> ModelResult<()> {
    let mut names = BTreeSet::new();
    for remote in remotes {
        if !names.insert(remote.name.as_str()) {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                format!("duplicate remote name '{}'", remote.name),
            ));
        }
    }
    Ok(())
}

fn validate_optional_target(field: &str, value: &Option<String>) -> ModelResult<usize> {
    match value {
        Some(value) => {
            require_non_empty(field, value)?;
            Ok(1)
        }
        None => Ok(0),
    }
}

fn validate_optional_text(field: &str, value: &Option<String>) -> ModelResult<()> {
    match value {
        Some(value) => require_non_empty(field, value),
        None => Ok(()),
    }
}

fn require_non_empty(field: &str, value: &str) -> ModelResult<()> {
    if value.trim().is_empty() {
        Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("{field} must not be empty"),
        ))
    } else {
        Ok(())
    }
}

fn validate_git_identity_field(field: &str, value: &str) -> ModelResult<()> {
    require_non_empty(field, value)?;
    if value
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '<' | '>'))
    {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("{field} contains characters Git signatures cannot represent"),
        ));
    }
    if value
        .trim_matches(is_git_signature_edge_character)
        .is_empty()
    {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("{field} does not contain a Git signature value"),
        ));
    }
    Ok(())
}

fn is_git_signature_edge_character(ch: char) -> bool {
    // libgit2 trims this set from both edges, then rejects an empty result.
    ch.is_ascii() && (ch <= ' ' || matches!(ch, ',' | ':' | ';' | '<' | '>' | '"' | '\\' | '\''))
}

#[cfg(test)]
mod tests;
