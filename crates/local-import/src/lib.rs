//! `gwz-local-import`: family exchange through retained import refs (lane X).
//!
//! [`prepare_import`] pairs every selected receiver with its source
//! repository by identity, captures the source object ids, fetches them
//! through the [`LocalTransport`] port into one fresh, collision-checked
//! import ref (`refs/gwz/local-imports/<transfer-id>`) in every receiver,
//! and verifies every received object id before the caller enters the
//! merge or pull engine. [`push_local`] publishes explicit refspecs into a
//! family member with per-repository partial results. Both follow gwz-dev
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §7 and design
//! §6.1/§6.2: all pairing and source validation happens before the first
//! transfer; a partial import leaves the refs it created and reports them;
//! import refs are retained indefinitely; no named remote is persisted.
//!
//! The port is owned here. Core implements it over `GitBackend`'s anonymous
//! local fetch/push; drivers never touch it.
//!
//! LCM1.0c checkpoint state: types and the port are frozen;
//! `prepare_import` refuses `ImportError::Unimplemented` and `push_local`
//! refuses every item, both before any transport call.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use gwz_repo_contract::{ObjectId, RepoKey};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

/// Namespace of retained import refs, separate from `refs/gwz/merge/...`.
pub const IMPORT_REF_NAMESPACE: &str = "refs/gwz/local-imports/";

/// A fresh, invocation-unique transfer id minted by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferId(String);

impl TransferId {
    pub fn new(id: impl Into<String>) -> Result<Self, ImportError> {
        let id = id.into();
        if id.is_empty() || id.contains('/') || id.contains(char::is_whitespace) {
            return Err(ImportError::InvalidRequest {
                detail: format!("invalid transfer id `{id}`"),
            });
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The common import ref name every paired receiver uses.
    pub fn import_ref(&self) -> String {
        format!("{IMPORT_REF_NAMESPACE}{}", self.0)
    }
}

/// Which source commit to import from each paired source repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceSelector {
    /// The source's `HEAD` commit, resolved independently per pairing.
    Head,
    /// A ref name resolved inside the source, independently per pairing.
    Ref(String),
}

/// One receiver/source pairing by identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub key: RepoKey,
    pub receiver: PathBuf,
    pub source: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRequest {
    pub transfer: TransferId,
    /// Every selected receiver, including the root when selected. Complete
    /// before the first transfer or the request refuses.
    pub pairings: Vec<Pairing>,
    pub selector: SourceSelector,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedCommit {
    pub key: RepoKey,
    pub oid: ObjectId,
}

/// A complete, verified import vector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedSource {
    pub import_ref: String,
    pub vector: Vec<ImportedCommit>,
}

/// A retained effect of a failed import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportEffect {
    RefCreated {
        key: RepoKey,
        import_ref: String,
        oid: ObjectId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    InvalidRequest {
        detail: String,
    },
    /// A selected receiver has no source pairing (or vice versa).
    PairingIncomplete {
        missing: Vec<RepoKey>,
    },
    /// The selector does not resolve in a source.
    SourceMissing {
        key: RepoKey,
        detail: String,
    },
    /// The import ref already exists in a receiver; retry with a fresh id.
    RefCollision {
        key: RepoKey,
        import_ref: String,
    },
    TransferFailed {
        key: RepoKey,
        detail: String,
        effects: Vec<ImportEffect>,
    },
    /// A received object id differs from the captured one.
    VectorMismatch {
        key: RepoKey,
        expected: ObjectId,
        received: Option<ObjectId>,
        effects: Vec<ImportEffect>,
    },
    Cancelled {
        effects: Vec<ImportEffect>,
    },
    Unimplemented,
}

impl ImportError {
    /// Refs created before the failure; the caller reports them and never
    /// prunes them.
    pub fn effects(&self) -> &[ImportEffect] {
        match self {
            Self::TransferFailed { effects, .. }
            | Self::VectorMismatch { effects, .. }
            | Self::Cancelled { effects } => effects,
            _ => &[],
        }
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest { detail } => write!(f, "invalid import request: {detail}"),
            Self::PairingIncomplete { missing } => {
                write!(f, "import pairing is incomplete for {missing:?}")
            }
            Self::SourceMissing { key, detail } => write!(f, "{key}: source missing: {detail}"),
            Self::RefCollision { key, import_ref } => {
                write!(f, "{key}: import ref {import_ref} already exists")
            }
            Self::TransferFailed { key, detail, .. } => {
                write!(f, "{key}: transfer failed: {detail}")
            }
            Self::VectorMismatch {
                key,
                expected,
                received,
                ..
            } => write!(
                f,
                "{key}: received {} but captured {expected}",
                received
                    .as_ref()
                    .map_or("nothing".to_owned(), ToString::to_string)
            ),
            Self::Cancelled { .. } => f.write_str("import cancelled"),
            Self::Unimplemented => {
                f.write_str("gwz-local-import: prepare_import is not implemented")
            }
        }
    }
}

impl std::error::Error for ImportError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// The peer is not an existing local repository path.
    NotLocal {
        detail: String,
    },
    Repository {
        path: PathBuf,
        detail: String,
    },
    /// The receiving side rejected a ref update.
    Rejected {
        refspec: String,
        detail: String,
    },
    Failed {
        detail: String,
    },
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLocal { detail } => write!(f, "not a local repository: {detail}"),
            Self::Repository { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Rejected { refspec, detail } => write!(f, "{refspec} rejected: {detail}"),
            Self::Failed { detail } => f.write_str(detail),
        }
    }
}

impl std::error::Error for TransportError {}

/// Cooperative cancellation port owned by this crate.
pub trait Cancellation {
    fn is_cancelled(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NeverCancelled;

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// The narrow local transport port. Every method takes local repository
/// paths only; the implementation persists no remote and uses no
/// credential or network helper.
pub trait LocalTransport {
    /// Resolve the selector inside `source` to its object id.
    fn resolve_source(
        &mut self,
        source: &Path,
        selector: &SourceSelector,
    ) -> Result<ObjectId, TransportError>;

    /// Whether `import_ref` already exists in `receiver` (collision check).
    fn ref_exists(&mut self, repository: &Path, name: &str) -> Result<bool, TransportError>;

    /// Anonymous fetch from `source` into `receiver` with explicit refspecs.
    fn fetch_anonymous(
        &mut self,
        receiver: &Path,
        source: &Path,
        refspecs: &[String],
    ) -> Result<(), TransportError>;

    /// Anonymous push from `source` into `destination` with one explicit
    /// refspec; a per-ref rejection is an error.
    fn push_anonymous(
        &mut self,
        source: &Path,
        destination: &Path,
        refspec: &str,
    ) -> Result<(), TransportError>;

    /// Read the object id a ref points at after a transfer (received-OID
    /// verification).
    fn read_ref(
        &mut self,
        repository: &Path,
        name: &str,
    ) -> Result<Option<ObjectId>, TransportError>;
}

/// Pair, capture, fetch and verify. Every pairing and source is validated
/// before the first transfer; a partial import reports its retained refs.
pub fn prepare_import(
    request: &ImportRequest,
    _transport: &mut dyn LocalTransport,
    _cancellation: &dyn Cancellation,
) -> Result<ImportedSource, ImportError> {
    if request.pairings.is_empty() {
        return Err(ImportError::InvalidRequest {
            detail: "an import needs at least one pairing".to_owned(),
        });
    }
    Err(ImportError::Unimplemented)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushItem {
    pub key: RepoKey,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub refspec: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushPlan {
    pub items: Vec<PushItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushOutcome {
    Pushed,
    /// Refused before transfer (checked-out branch, missing branch, or an
    /// unimplemented push).
    Refused {
        reason: String,
    },
    Failed {
        detail: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushResult {
    pub key: RepoKey,
    pub outcome: PushOutcome,
}

/// Per-repository results; ordinary partial-result semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushReport {
    pub results: Vec<PushResult>,
}

impl PushReport {
    pub fn all_pushed(&self) -> bool {
        self.results
            .iter()
            .all(|result| result.outcome == PushOutcome::Pushed)
    }
}

/// Push every planned item, keeping explicit-refspec, partial-result and
/// checked-out-branch rules.
pub fn push_local(plan: &PushPlan, _transport: &mut dyn LocalTransport) -> PushReport {
    PushReport {
        results: plan
            .items
            .iter()
            .map(|item| PushResult {
                key: item.key.clone(),
                outcome: PushOutcome::Refused {
                    reason: "gwz-local-import: push_local is not implemented".to_owned(),
                },
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::RecordingTransport;

    fn request() -> ImportRequest {
        ImportRequest {
            transfer: TransferId::new("t1").unwrap(),
            pairings: vec![Pairing {
                key: RepoKey::Root,
                receiver: PathBuf::from("/recv"),
                source: PathBuf::from("/src"),
            }],
            selector: SourceSelector::Head,
        }
    }

    #[test]
    fn import_ref_names_live_in_the_retained_namespace() {
        let id = TransferId::new("01J").unwrap();
        assert_eq!(id.import_ref(), "refs/gwz/local-imports/01J");
        assert!(TransferId::new("").is_err());
        assert!(TransferId::new("a/b").is_err());
        assert!(!IMPORT_REF_NAMESPACE.starts_with("refs/gwz/merge"));
    }

    #[test]
    fn checkpoint_import_refuses_before_any_transport_call() {
        let mut transport = RecordingTransport::new();
        let error = prepare_import(&request(), &mut transport, &NeverCancelled).unwrap_err();
        assert_eq!(error, ImportError::Unimplemented);
        assert!(error.effects().is_empty());
        assert!(
            transport.calls().is_empty(),
            "no transport call before refusal"
        );

        let empty = ImportRequest {
            pairings: Vec::new(),
            ..request()
        };
        assert!(matches!(
            prepare_import(&empty, &mut transport, &NeverCancelled).unwrap_err(),
            ImportError::InvalidRequest { .. }
        ));
        assert!(transport.calls().is_empty());
    }

    #[test]
    fn checkpoint_push_refuses_every_item_without_transfer() {
        let mut transport = RecordingTransport::new();
        let plan = PushPlan {
            items: vec![PushItem {
                key: RepoKey::Member {
                    id: "mem_app".to_owned(),
                },
                source: PathBuf::from("/src"),
                destination: PathBuf::from("/hub"),
                refspec: "refs/heads/x:refs/heads/x".to_owned(),
            }],
        };
        let report = push_local(&plan, &mut transport);
        assert!(!report.all_pushed());
        assert!(matches!(
            report.results[0].outcome,
            PushOutcome::Refused { .. }
        ));
        assert!(transport.calls().is_empty());
    }
}
