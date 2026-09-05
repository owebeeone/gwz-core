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
//! Nothing in this crate deletes a ref: [`LocalTransport`] has no removal
//! method, so "no automatic pruning in v0" (design §6.2) is structural, not
//! a discipline.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use gwz_repo_contract::{ObjectId, RepoKey};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;

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
    ///
    /// The name reaches the port unchanged, both when resolving and as the
    /// source side of the fetch refspec. Qualifying a short name is the
    /// adapter's business, not this crate's.
    Ref(String),
}

impl SourceSelector {
    /// The source side of the fetch refspec for this selector.
    fn refspec_source(&self) -> &str {
        match self {
            Self::Head => "HEAD",
            Self::Ref(name) => name,
        }
    }
}

/// One repository of a workspace, as that workspace's lock records it.
///
/// Pairing is by [`RepoKey`] — the dest lock member id / `source_id`, never
/// by path (design §6). `relative_path` is compared across the two
/// workspaces for the same id so that a member moved on one side refuses
/// instead of silently pairing two different trees. The root repository has
/// no member-lock entry and is excluded from both the set comparison and
/// the moved-path check: it pairs separately, root with root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    pub key: RepoKey,
    /// The path this workspace's lock records for the repository, relative
    /// to its workspace root. Ignored for [`RepoKey::Root`].
    pub relative_path: String,
    /// Where the repository is on disk; what the transport receives.
    pub path: PathBuf,
}

impl Participant {
    pub fn new(key: RepoKey, relative_path: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            key,
            relative_path: relative_path.into(),
            path: path.into(),
        }
    }

    /// The root repository, which has no recorded member path.
    pub fn root(path: impl Into<PathBuf>) -> Self {
        Self::new(RepoKey::Root, ".", path)
    }
}

/// One receiver/source pairing by identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub key: RepoKey,
    pub receiver: PathBuf,
    pub source: PathBuf,
}

/// The same member id recorded at different paths in the two workspaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovedMember {
    pub key: RepoKey,
    pub receiver_path: String,
    pub source_path: String,
}

/// One participant whose selected source ref did not resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProblem {
    pub key: RepoKey,
    pub detail: String,
}

/// Everything [`prepare_import`] needs: both workspaces' repository sets,
/// the verb's selection, one fresh transfer id and the source selector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRequest {
    pub transfer: TransferId,
    /// Every repository of the receiving workspace, in lock order, plus its
    /// root when the verb can select it.
    pub receivers: Vec<Participant>,
    /// Every repository of the source workspace, plus its root.
    pub sources: Vec<Participant>,
    /// The receivers this verb selected, by identity. Never empty.
    pub selected: Vec<RepoKey>,
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

impl ImportedSource {
    pub fn oid_for(&self, key: &RepoKey) -> Option<&ObjectId> {
        self.vector
            .iter()
            .find(|commit| &commit.key == key)
            .map(|commit| &commit.oid)
    }
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
    /// The two workspaces' member sets do not correspond. Aggregated over
    /// every selected participant; structurally carries no effect because
    /// it is decided before the first transport call.
    PairingIncomplete {
        /// Member ids present on exactly one side (missing or extra), and
        /// a selected `@root` with no root on the other side.
        missing: Vec<RepoKey>,
        /// Ids both locks record, at different paths.
        moved: Vec<MovedMember>,
    },
    /// The selector does not resolve in one or more sources. Aggregated,
    /// and decided before the first transfer.
    SourceMissing {
        missing: Vec<SourceProblem>,
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

fn join_keys(keys: &[RepoKey]) -> String {
    keys.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest { detail } => write!(f, "invalid import request: {detail}"),
            Self::PairingIncomplete { missing, moved } => {
                f.write_str("import pairing is incomplete")?;
                if !missing.is_empty() {
                    write!(f, "; unpaired: {}", join_keys(missing))?;
                }
                for entry in moved {
                    write!(
                        f,
                        "; {} is at {} here and {} there",
                        entry.key, entry.receiver_path, entry.source_path
                    )?;
                }
                Ok(())
            }
            Self::SourceMissing { missing } => {
                f.write_str("source missing")?;
                for problem in missing {
                    write!(f, "; {}: {}", problem.key, problem.detail)?;
                }
                Ok(())
            }
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

/// Compare two recorded lock paths without adopting a path policy: `\` is a
/// separator, `.` and empty components carry no meaning, and a trailing
/// separator is not part of the name.
fn lock_path_key(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn index_by_key(participants: &[Participant]) -> Result<BTreeMap<&RepoKey, &Participant>, RepoKey> {
    let mut index = BTreeMap::new();
    for participant in participants {
        if index.insert(&participant.key, participant).is_some() {
            return Err(participant.key.clone());
        }
    }
    Ok(index)
}

/// Pair every selected receiver with its source by identity, aggregating
/// every set mismatch (design §6). Pure: it makes no transport call, so a
/// refusal here structurally cannot have touched a repository.
///
/// A selected `@root` pairs with the source workspace's root repository and
/// takes no part in the member-set comparison.
pub fn pair_participants(request: &ImportRequest) -> Result<Vec<Pairing>, ImportError> {
    if request.selected.is_empty() {
        return Err(ImportError::InvalidRequest {
            detail: "an import needs at least one selected participant".to_owned(),
        });
    }
    let receivers =
        index_by_key(&request.receivers).map_err(|key| ImportError::InvalidRequest {
            detail: format!("the receiving workspace lists {key} twice"),
        })?;
    let sources = index_by_key(&request.sources).map_err(|key| ImportError::InvalidRequest {
        detail: format!("the source workspace lists {key} twice"),
    })?;
    let mut seen: Vec<&RepoKey> = Vec::with_capacity(request.selected.len());
    for key in &request.selected {
        if seen.contains(&key) {
            return Err(ImportError::InvalidRequest {
                detail: format!("{key} is selected twice"),
            });
        }
        seen.push(key);
        if !receivers.contains_key(key) {
            return Err(ImportError::InvalidRequest {
                detail: format!("{key} is selected but is not a repository of this workspace"),
            });
        }
    }

    // The member sets must correspond as sets, whatever this verb selected:
    // a member the other workspace does not have (or has under another id)
    // means the two workspaces are no longer the same shape, and design §6
    // refuses the whole operation rather than importing the overlap.
    let mut missing: Vec<RepoKey> = Vec::new();
    let mut moved: Vec<MovedMember> = Vec::new();
    for (key, receiver) in &receivers {
        if **key == RepoKey::Root {
            continue;
        }
        match sources.get(*key) {
            None => missing.push((*key).clone()),
            Some(source) => {
                if lock_path_key(&receiver.relative_path) != lock_path_key(&source.relative_path) {
                    moved.push(MovedMember {
                        key: (*key).clone(),
                        receiver_path: receiver.relative_path.clone(),
                        source_path: source.relative_path.clone(),
                    });
                }
            }
        }
    }
    for key in sources.keys() {
        if **key != RepoKey::Root && !receivers.contains_key(*key) {
            missing.push((*key).clone());
        }
    }
    // The root has no member-lock entry, so it is never part of the set
    // comparison above; it is required only when it was selected.
    if request.selected.contains(&RepoKey::Root) && !sources.contains_key(&RepoKey::Root) {
        missing.push(RepoKey::Root);
    }
    if !missing.is_empty() || !moved.is_empty() {
        missing.sort();
        missing.dedup();
        moved.sort_by(|a, b| a.key.cmp(&b.key));
        return Err(ImportError::PairingIncomplete { missing, moved });
    }

    // Lock order, filtered to the selection: deterministic, and the order
    // every later phase visits participants in.
    Ok(request
        .receivers
        .iter()
        .filter(|participant| request.selected.contains(&participant.key))
        .map(|participant| Pairing {
            key: participant.key.clone(),
            receiver: participant.path.clone(),
            source: sources[&participant.key].path.clone(),
        })
        .collect())
}

fn cancelled(cancellation: &dyn Cancellation, effects: &[ImportEffect]) -> Option<ImportError> {
    cancellation.is_cancelled().then(|| ImportError::Cancelled {
        effects: effects.to_vec(),
    })
}

/// Pair, capture, collision-check, fetch and verify (design §6.2).
///
/// Phases, in order, so that every refusal says exactly how far it got:
/// pairing (no transport at all), capture of the source vector (reads),
/// the collision check in *every* receiver (reads), the fetches (writes),
/// then verification of every received id against the captured vector.
/// No engine sees a partly sourced import, and no failure prunes a ref.
pub fn prepare_import(
    request: &ImportRequest,
    transport: &mut dyn LocalTransport,
    cancellation: &dyn Cancellation,
) -> Result<ImportedSource, ImportError> {
    let pairings = pair_participants(request)?;
    let import_ref = request.transfer.import_ref();
    let mut effects: Vec<ImportEffect> = Vec::new();
    if let Some(error) = cancelled(cancellation, &effects) {
        return Err(error);
    }

    // Capture: resolve every selected source ref once, and report every
    // source that does not resolve rather than the first.
    let mut captured: Vec<ImportedCommit> = Vec::with_capacity(pairings.len());
    let mut unresolved: Vec<SourceProblem> = Vec::new();
    for pairing in &pairings {
        match transport.resolve_source(&pairing.source, &request.selector) {
            Ok(oid) => captured.push(ImportedCommit {
                key: pairing.key.clone(),
                oid,
            }),
            Err(error) => unresolved.push(SourceProblem {
                key: pairing.key.clone(),
                detail: error.to_string(),
            }),
        }
    }
    if !unresolved.is_empty() {
        return Err(ImportError::SourceMissing {
            missing: unresolved,
        });
    }

    // The import name must be free in every receiver before any receiver is
    // fetched into: a collision found halfway would otherwise leave a
    // half-imported family behind a name that means something else.
    for pairing in &pairings {
        match transport.ref_exists(&pairing.receiver, &import_ref) {
            Ok(false) => {}
            Ok(true) => {
                return Err(ImportError::RefCollision {
                    key: pairing.key.clone(),
                    import_ref,
                });
            }
            Err(error) => {
                return Err(ImportError::TransferFailed {
                    key: pairing.key.clone(),
                    detail: format!("import ref collision check failed: {error}"),
                    effects,
                });
            }
        }
    }

    let refspec = format!("{}:{import_ref}", request.selector.refspec_source());
    for (pairing, commit) in pairings.iter().zip(&captured) {
        if let Some(error) = cancelled(cancellation, &effects) {
            return Err(error);
        }
        match transport.fetch_anonymous(
            &pairing.receiver,
            &pairing.source,
            std::slice::from_ref(&refspec),
        ) {
            Ok(()) => effects.push(ImportEffect::RefCreated {
                key: pairing.key.clone(),
                import_ref: import_ref.clone(),
                oid: commit.oid.clone(),
            }),
            Err(error) => {
                // A failed fetch may still have created the ref. Look, and
                // report it if it is there; an unreadable receiver leaves
                // the effect unclaimed rather than guessed.
                if let Ok(Some(oid)) = transport.read_ref(&pairing.receiver, &import_ref) {
                    effects.push(ImportEffect::RefCreated {
                        key: pairing.key.clone(),
                        import_ref: import_ref.clone(),
                        oid,
                    });
                }
                return Err(ImportError::TransferFailed {
                    key: pairing.key.clone(),
                    detail: error.to_string(),
                    effects,
                });
            }
        }
    }

    // Verify the whole vector before the engine sees anything: the name
    // resolved at capture time may have moved, and a receiver may have
    // taken a different object than the one asked for.
    for (pairing, commit) in pairings.iter().zip(&captured) {
        let received = match transport.read_ref(&pairing.receiver, &import_ref) {
            Ok(received) => received,
            Err(error) => {
                return Err(ImportError::TransferFailed {
                    key: pairing.key.clone(),
                    detail: format!("import ref verification failed: {error}"),
                    effects,
                });
            }
        };
        if received.as_ref() != Some(&commit.oid) {
            correct_effect(&mut effects, &pairing.key, received.as_ref());
            return Err(ImportError::VectorMismatch {
                key: pairing.key.clone(),
                expected: commit.oid.clone(),
                received,
                effects,
            });
        }
    }

    Ok(ImportedSource {
        import_ref,
        vector: captured,
    })
}

/// Effects are recorded at fetch time with the id that was asked for.
/// Verification is what proves it, so a mismatch corrects (or drops) the
/// entry rather than reporting a ref that holds something else.
fn correct_effect(effects: &mut Vec<ImportEffect>, key: &RepoKey, received: Option<&ObjectId>) {
    let position = effects.iter().position(|effect| match effect {
        ImportEffect::RefCreated { key: existing, .. } => existing == key,
    });
    let Some(position) = position else { return };
    match received {
        Some(oid) => {
            let ImportEffect::RefCreated { oid: recorded, .. } = &mut effects[position];
            *recorded = oid.clone();
        }
        None => {
            effects.remove(position);
        }
    }
}

/// The source repository's `HEAD`, as the caller observed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushHead {
    Attached {
        branch: String,
    },
    /// No attached branch: the same-branch rule (design §6.1) has no
    /// source, so the push refuses unless the caller supplied an explicit
    /// refspec.
    Detached,
}

/// The receiving repository, as the caller observed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiverState {
    /// A bare repository. libgit2's local transport accepts no other kind
    /// of push receiver (design §11 item 16, still open).
    Bare,
    /// A working checkout, with the branch its `HEAD` is on when attached.
    Checkout { checked_out: Option<String> },
}

/// Why a push was refused before any transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushRefusal {
    /// `HEAD` is detached and no explicit refspec says what to publish.
    DetachedHead,
    /// The receiver has the target branch checked out (design §6.1).
    CheckedOutBranch {
        branch: String,
    },
    /// The receiver is not bare. Recorded platform limit: libgit2's local
    /// transport refuses every push into a non-bare repository, so a
    /// checkout member integrates from its own side instead. Design §11
    /// item 16 is open; this refusal states the limit, it does not settle
    /// it.
    NonBareReceiver {
        target: String,
    },
    /// The source has no such branch or ref.
    MissingSourceRef {
        name: String,
    },
    InvalidRefspec {
        refspec: String,
        detail: String,
    },
}

impl fmt::Display for PushRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DetachedHead => {
                f.write_str("HEAD is detached; name a refspec to publish from here")
            }
            Self::CheckedOutBranch { branch } => {
                write!(f, "{branch} is checked out at the receiver")
            }
            Self::NonBareReceiver { target } => write!(
                f,
                "the receiver is not a bare repository, so the local transport cannot \
                 publish {target} into it"
            ),
            Self::MissingSourceRef { name } => write!(f, "this repository has no {name}"),
            Self::InvalidRefspec { refspec, detail } => {
                write!(f, "refspec `{refspec}` is not usable: {detail}")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushItem {
    pub key: RepoKey,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub head: PushHead,
    pub receiver: ReceiverState,
    /// An explicit refspec keeps its explicit mapping and normal
    /// validation; absence means the same-branch rule of design §6.1.
    pub refspec: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushPlan {
    pub items: Vec<PushItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushOutcome {
    Pushed {
        refspec: String,
    },
    /// Refused before transfer; this repository made no transport call.
    Refused {
        refusal: PushRefusal,
    },
    /// The transfer was attempted and did not complete.
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
            .all(|result| matches!(result.outcome, PushOutcome::Pushed { .. }))
    }

    pub fn outcome_for(&self, key: &RepoKey) -> Option<&PushOutcome> {
        self.results
            .iter()
            .find(|result| &result.key == key)
            .map(|result| &result.outcome)
    }
}

/// Split `src:dst`, tolerating a leading `+` on the source side.
fn split_refspec(refspec: &str) -> Result<(&str, &str), PushRefusal> {
    let invalid = |detail: &str| PushRefusal::InvalidRefspec {
        refspec: refspec.to_owned(),
        detail: detail.to_owned(),
    };
    let (source, destination) = refspec.split_once(':').ok_or_else(|| invalid("no `:`"))?;
    let source = source.trim_start_matches('+');
    if source.is_empty() || destination.is_empty() {
        return Err(invalid("both sides must name a ref"));
    }
    Ok((source, destination))
}

/// The branch a receiver-side ref names, if it is a branch at all.
fn destination_branch(destination: &str) -> Option<&str> {
    destination.strip_prefix("refs/heads/")
}

fn plan_refspec(item: &PushItem) -> Result<String, PushRefusal> {
    match (&item.refspec, &item.head) {
        (Some(refspec), _) => {
            split_refspec(refspec)?;
            Ok(refspec.clone())
        }
        (None, PushHead::Attached { branch }) => {
            Ok(format!("refs/heads/{branch}:refs/heads/{branch}"))
        }
        (None, PushHead::Detached) => Err(PushRefusal::DetachedHead),
    }
}

/// Publish each planned repository, keeping per-repository partial results
/// (design §6.2). Every refusal is decided from the observed state before
/// any transport call for that repository, and no named remote is used.
pub fn push_local(plan: &PushPlan, transport: &mut dyn LocalTransport) -> PushReport {
    let results = plan
        .items
        .iter()
        .map(|item| PushResult {
            key: item.key.clone(),
            outcome: push_one(item, transport),
        })
        .collect();
    PushReport { results }
}

fn push_one(item: &PushItem, transport: &mut dyn LocalTransport) -> PushOutcome {
    let refspec = match plan_refspec(item) {
        Ok(refspec) => refspec,
        Err(refusal) => return PushOutcome::Refused { refusal },
    };
    let (source_ref, destination_ref) = match split_refspec(&refspec) {
        Ok(parts) => parts,
        Err(refusal) => return PushOutcome::Refused { refusal },
    };
    if let ReceiverState::Checkout { checked_out } = &item.receiver {
        let refusal = match (checked_out.as_deref(), destination_branch(destination_ref)) {
            (Some(checked_out), Some(target)) if checked_out == target => {
                PushRefusal::CheckedOutBranch {
                    branch: target.to_owned(),
                }
            }
            _ => PushRefusal::NonBareReceiver {
                target: destination_ref.to_owned(),
            },
        };
        return PushOutcome::Refused { refusal };
    }
    match transport.ref_exists(&item.source, source_ref) {
        Ok(true) => {}
        Ok(false) => {
            return PushOutcome::Refused {
                refusal: PushRefusal::MissingSourceRef {
                    name: source_ref.to_owned(),
                },
            };
        }
        Err(error) => {
            return PushOutcome::Failed {
                detail: error.to_string(),
            };
        }
    }
    match transport.push_anonymous(&item.source, &item.destination, &refspec) {
        Ok(()) => PushOutcome::Pushed { refspec },
        Err(error) => PushOutcome::Failed {
            detail: error.to_string(),
        },
    }
}
