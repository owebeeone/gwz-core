use serde::{Deserialize, Serialize};
use std::fmt;

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
    /// A requested URL scheme (`--url-scheme ssh|https`) cannot be derived for a
    /// known-host URL, for example a nonstandard port (wire `url_scheme_unavailable`
    /// = 72; gwz-dev dev-docs/GwzUrlSchemePlan.md §2.5).
    UrlSchemeUnavailable,
    /// Python native-session admission refused a different live physical pool
    /// policy before consuming the caller request ID. This is a bridge error,
    /// not a terminal GWZ operation result.
    TransportCapacityConflict,
    /// Native Python Client exhausted its bounded operation/ledger admission.
    TransportSessionFull,
    /// A session-issued public operation ID whose retained record was released
    /// or reached its retention deadline.
    OperationExpired,
    /// A native Python operation cancelled before core registration.
    Cancelled,
    /// A helper lookup exceeded its fixed interaction or retained admission
    /// allowance before offering credentials (TR1.6 OQ5(a), wire 75).
    CredentialHelperTimeout,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelError {
    pub code: ErrorCode,
    pub message: String,
    pub member_id: Option<String>,
    pub member_path: Option<String>,
    pub record_context: Option<Box<crate::MergeRecordCompatibilityContext>>,
    /// Typed operation metadata retained when failure precedes an envelope.
    pub response_meta: Option<Box<crate::ResponseMeta>>,
}

// Response metadata contains only equivalence-comparable values (no floats).
impl Eq for ModelError {}

impl ModelError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code,
            message,
            member_id: None,
            member_path: None,
            record_context: None,
            response_meta: None,
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
