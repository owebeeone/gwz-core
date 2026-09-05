//! `gwz-local-disposal`: explicit keep and one-shot disposal (lane D).
//!
//! Disband has no entry point here and never will: it removes pointers and
//! the index and no directory contents, so it is core's own composition over
//! `gwz_family_model::FamilyChange::Disband` and the store session, and it
//! must never route a remaining row through this crate's removal path
//! (design §5.2, last paragraph).
//!
//! [`dispose`] is the only local-clone service that removes directory
//! contents (gwz-dev `dev-docs/GwzLocalCloneImplementationArchitecture.md`
//! §8, design §5). Under the family lock it validates an intact ready
//! target, gathers fresh work evidence through [`DisposalPorts`] and
//! classifies it with `gwz-work-detector`, asks the history port whether
//! every protected root is preserved elsewhere, refuses dirty, unpreserved
//! or unknown evidence unless a named [`HazardWaiver`] covers it, writes
//! `disposing` through the store session, then removes the validated
//! directory once. `--keep` detaches the matching row and pointer and
//! retains every file. On any error it stops and reports what remains; there
//! is no rollback or replay. The hazard vocabulary is owned here; core uses
//! it to reject unknown request hazards before any effect.
//!
//! **The refusal is the product.** The operator's standing default (design
//! §5, 2026-09-05) is that deletion refuses unless history is verifiably
//! preserved elsewhere, so every uncertain answer refuses: `Unknown` from
//! the work detector or the history port is never waivable by any force
//! name, and a path mismatch, an incomplete create and an interrupted
//! deletion are not forceable at all. A named waiver is an operator loss
//! waiver over an intact ready tree, never crash recovery. On every refusal
//! path the removal port is not called.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use gwz_family_model::{
    FamilyChange, ListState, MemberName, MemberPath, MemberRow, MemberState, PathError, Refusal,
    RemovalReason, TargetObservation, classify_target, relate_member_paths, validate_member_path,
};
use gwz_family_store_contract::{FamilySession, MetadataEffect, StoreError};
use gwz_repo_contract::{
    Observation, ProtectedRoots, RepoKey, RepositoryInfo, UnknownKind, UnknownReason,
    WorkObservation,
};
use gwz_work_detector::{GwzEvidence, Hazard, WorkVerdict, classify_observed_work};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

/// The named hazards an explicit `--force` may waive (design §5.2). Only
/// these spellings exist on the wire; core rejects any other name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HazardWaiver {
    OpenMerge,
    Dirty,
    UnpreservedHistory,
}

impl HazardWaiver {
    pub const ALL: [HazardWaiver; 3] = [Self::OpenMerge, Self::Dirty, Self::UnpreservedHistory];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenMerge => "open-merge",
            Self::Dirty => "dirty",
            Self::UnpreservedHistory => "unpreserved-history",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|waiver| waiver.as_str() == name)
    }

    /// Parse a request's hazard list: every name must be known and no name
    /// may repeat. Empty means no force.
    pub fn parse_all(names: &[String]) -> Result<Vec<Self>, UnknownHazard> {
        let mut waivers = Vec::new();
        for name in names {
            let waiver = Self::parse(name).ok_or_else(|| UnknownHazard { name: name.clone() })?;
            if waivers.contains(&waiver) {
                return Err(UnknownHazard {
                    name: format!("{name} (repeated)"),
                });
            }
            waivers.push(waiver);
        }
        Ok(waivers)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownHazard {
    pub name: String,
}

impl fmt::Display for UnknownHazard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown hazard `{}`; known hazards: {}",
            self.name,
            HazardWaiver::ALL
                .iter()
                .map(|waiver| waiver.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl std::error::Error for UnknownHazard {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposePolicy {
    /// Detach the row and pointer; retain every file.
    Keep,
    /// One-shot removal after fresh checks, with explicit waivers.
    Delete { waivers: Vec<HazardWaiver> },
}

impl DisposePolicy {
    /// Decide the policy from the wire pair `(keep, force_hazards)`, so the
    /// three request-shape refusals of design §5.2 live with the vocabulary
    /// that defines them rather than being restated by each driver:
    /// an unknown or empty **hazard name** refuses ([`HazardWaiver::parse_all`];
    /// an empty *list* is simply no force), and `--keep` with any force name
    /// refuses because keep removes no file and so waives nothing.
    pub fn parse(keep: bool, force_hazards: &[String]) -> Result<Self, PolicyError> {
        if keep {
            if !force_hazards.is_empty() {
                return Err(PolicyError::KeepWithForce {
                    names: force_hazards.to_vec(),
                });
            }
            return Ok(Self::Keep);
        }
        Ok(Self::Delete {
            waivers: HazardWaiver::parse_all(force_hazards).map_err(PolicyError::UnknownHazard)?,
        })
    }
}

/// Why a `(keep, force_hazards)` pair is not a policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyError {
    UnknownHazard(UnknownHazard),
    KeepWithForce { names: Vec<String> },
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownHazard(error) => error.fmt(f),
            Self::KeepWithForce { names } => write!(
                f,
                "--keep and --force <hazards> are mutually exclusive; keep removes no file and \
                 waives nothing (got `{}`)",
                names.join(",")
            ),
        }
    }
}

impl std::error::Error for PolicyError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisposeRequest {
    pub name: MemberName,
    pub policy: DisposePolicy,
    /// The registering root.
    ///
    /// **Canonical absolute paths.** This library compares `root`, `cwd`,
    /// the row's recorded path and every observed repository path
    /// *lexically* (it is pure: it opens nothing and resolves no symlink),
    /// exactly as the family-store contract's reference resolution does. The
    /// root/cwd/overlap guards below are therefore only as exact as the
    /// spellings core supplies: core must pass canonicalised absolute paths,
    /// or a symlinked spelling could make an overlapping target look
    /// disjoint.
    pub root: PathBuf,
    /// The invoking process's working directory; disposal refuses a target
    /// that contains it.
    pub cwd: PathBuf,
}

/// Fresh evidence for one repository inside the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryEvidence {
    pub key: RepoKey,
    pub info: RepositoryInfo,
    pub work: gwz_repo_contract::Observation<WorkObservation>,
    pub gwz: GwzEvidence,
    pub history: gwz_repo_contract::Observation<ProtectedRoots>,
}

/// Fresh evidence for the whole target tree, including nested repositories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetEvidence {
    /// What stands at the row's recorded path: absent, undecodable, or
    /// present with its family pointer and allocation marker as core read
    /// them. [`gwz_family_model::classify_target`] turns this and the row
    /// into the [`ListState`] disposal acts on, so "validate name, pointer
    /// and path" (design §5.2 step 1) is one pure decision the model owns.
    /// A moved root, a replaced directory or an interrupted detach all reach
    /// disposal here and refuse as [`DisposeError::PathMismatch`].
    pub target: TargetObservation,
    pub repositories: Vec<RepositoryEvidence>,
    /// Nested repositories or layouts the observer could not interpret.
    pub unknown: Vec<UnknownReason>,
}

/// What the history port is asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryQuery {
    pub target: RepoKey,
    pub protected: ProtectedRoots,
}

/// The history port's answer, mirroring `gwz-history-check` outcomes
/// without depending on that crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryAnswer {
    Preserved,
    Unpreserved { detail: String },
    Unknown { reasons: Vec<UnknownReason> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortError {
    Evidence { detail: String },
    Removal { path: PathBuf, detail: String },
    Unimplemented { operation: &'static str },
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evidence { detail } => write!(f, "evidence: {detail}"),
            Self::Removal { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Unimplemented { operation } => write!(f, "{operation} is not implemented"),
        }
    }
}

impl std::error::Error for PortError {}

/// The narrow ports disposal consumes. Core implements them over the
/// repository inspector, decoded GWZ evidence, `gwz-history-check` and
/// ordinary recursive removal that never follows symlinks out of the tree.
pub trait DisposalPorts {
    fn observe_target(&mut self, target: &Path) -> Result<TargetEvidence, PortError>;
    fn check_history(&mut self, query: &HistoryQuery) -> HistoryAnswer;
    /// Remove `target` once; on error, `remaining` lists what is left.
    fn remove_directory(&mut self, target: &Path) -> Result<(), RemovalFailure>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovalFailure {
    pub error: PortError,
    pub remaining: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposeEffect {
    PointerRemoved,
    RowDetached,
    RowDisposing,
    DirectoryRemoved,
    RowRemoved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisposeReport {
    pub effects: Vec<DisposeEffect>,
}

/// One repository's refusal under one waiver name. Findings are per
/// repository because the deletion tree holds several (design §5.1
/// inspects every one of them), and a report that named only the waiver
/// could not say which lane's history is unpreserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HazardFinding {
    pub waiver: HazardWaiver,
    /// The repository inside the deletion tree the finding belongs to.
    pub repository: RepoKey,
    /// The classifier's hazards, in classification order. Empty for a
    /// history finding, which the work detector never produces.
    pub hazards: Vec<Hazard>,
    /// The history verifier's detail, for
    /// [`HazardWaiver::UnpreservedHistory`] only.
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisposeError {
    /// Root, cwd, path mismatch, non-ready target, keep+force, unknown row.
    Refused(Refusal),
    /// The row's recorded path resolves to the registering root itself or to
    /// a directory containing it. The original tree is never deleted; `gwz
    /// local disband` retires the family instead (design §5, §8.4).
    RootImmutable,
    TargetContainsCwd {
        target: PathBuf,
    },
    /// The validated target is not the recorded member: a moved root, a
    /// replaced or foreign directory, an interrupted detach, undecodable
    /// metadata, or an observation that reached outside the deletion tree.
    /// **Never forceable** (design §5.2 step 3): a waiver authorises losing
    /// *this member's* known work, not deleting something else.
    PathMismatch {
        /// The validated target path.
        expected: PathBuf,
        /// What was found there instead.
        observed: String,
    },
    /// Hazards present and not waived.
    Hazards(Vec<HazardFinding>),
    /// Evidence or history could not be established.
    Unknown(Vec<UnknownReason>),
    Store(StoreError),
    Port(PortError),
    /// Removal stopped; `remaining` is what is left for manual cleanup.
    RemovalStopped {
        remaining: Vec<PathBuf>,
        detail: String,
    },
    /// **Never returned.** [`dispose`] implements the whole design §5.2
    /// sequence; a port that implements nothing surfaces as
    /// [`Port`](Self::Port)`(`[`PortError::Unimplemented`]`)` instead. The
    /// variant is retained only so a consumer written against the LCM1.0c
    /// stub still compiles; drop that arm and this variant goes with it.
    Unimplemented,
}

impl fmt::Display for DisposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::RootImmutable => f.write_str("root is never disposed; use disband"),
            Self::TargetContainsCwd { target } => {
                write!(f, "{} contains the working directory", target.display())
            }
            Self::PathMismatch { expected, observed } => {
                write!(
                    f,
                    "{} is not the recorded target: {observed}",
                    expected.display()
                )
            }
            Self::Hazards(findings) => write!(f, "unwaived hazards: {findings:?}"),
            Self::Unknown(reasons) => write!(f, "unknown evidence: {reasons:?}"),
            Self::Store(error) => write!(f, "{error}"),
            Self::Port(error) => write!(f, "{error}"),
            Self::RemovalStopped { remaining, detail } => {
                write!(
                    f,
                    "removal stopped ({detail}); {} path(s) remain",
                    remaining.len()
                )
            }
            Self::Unimplemented => f.write_str("gwz-local-disposal: dispose is not implemented"),
        }
    }
}

impl std::error::Error for DisposeError {}

/// A failed disposal: the typed cause plus the effects that completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisposeFailure {
    pub error: DisposeError,
    pub effects: Vec<DisposeEffect>,
}

/// Keep, or check and remove, one family member.
///
/// The design §5.2 sequence, in order, with the port each step consults:
///
/// 1. **Validate name, pointer and path** — pure, over the session's own
///    reread view. Refuses an unrecognised name, an unusable recorded path,
///    the root, a target containing the working directory, a target
///    overlapping the root or another member, and a `root` the held lock
///    does not belong to ([`DisposeError::PathMismatch`], the moved-root
///    case). No port is consulted.
/// 2. **`--keep`** — [`FamilySession::remove_pointer`] then `RemoveRow`
///    with [`RemovalReason::Keep`]. No evidence, history or removal call:
///    every file stays, including an incomplete or interrupted tree the
///    ordinary path refuses.
/// 3. **Fresh checks** — one [`DisposalPorts::observe_target`], then per
///    repository `gwz_work_detector::classify_observed_work` (pure) and one
///    [`DisposalPorts::check_history`]. `Unknown` from either dominates and
///    is never waivable; a *known* hazard refuses unless its
///    [`HazardWaiver`] was named. An absent target takes the stale-row exit
///    (step 5) instead, and every other observed state is a `PathMismatch`.
/// 4. **Remove** — `MarkDisposing` through the session, its result checked,
///    then exactly one [`DisposalPorts::remove_directory`] on the validated
///    target. An error stops and reports what remains; nothing is rolled
///    back or replayed.
/// 5. **Detach** — `remove_pointer` then `RemoveRow`, so a pointer the
///    store cannot remove is reported as the pointer, not as the row.
pub fn dispose(
    request: &DisposeRequest,
    session: &mut dyn FamilySession,
    ports: &mut dyn DisposalPorts,
) -> Result<DisposeReport, DisposeFailure> {
    let mut effects = Vec::new();
    match run(request, session, ports, &mut effects) {
        Ok(()) => Ok(DisposeReport { effects }),
        Err(error) => Err(DisposeFailure { error, effects }),
    }
}

/// The validated target of one invocation. A request is an invocation-local
/// value, never a reusable deletion authorization: nothing here is retained.
struct Plan {
    row: MemberRow,
    /// The recorded path resolved against the root, lexically normalised.
    target: PathBuf,
}

fn run(
    request: &DisposeRequest,
    session: &mut dyn FamilySession,
    ports: &mut dyn DisposalPorts,
    effects: &mut Vec<DisposeEffect>,
) -> Result<(), DisposeError> {
    let waivers = match &request.policy {
        DisposePolicy::Keep => None,
        DisposePolicy::Delete { waivers } => {
            let unique: std::collections::BTreeSet<_> = waivers.iter().collect();
            if unique.len() != waivers.len() {
                return Err(DisposeError::Refused(Refusal::InvalidRow {
                    name: request.name.clone(),
                    detail: "repeated hazard waiver".to_owned(),
                }));
            }
            Some(waivers.as_slice())
        }
    };

    // Step 1.
    let plan = validate(request, session)?;

    // Step 2: keep detaches metadata only, whatever state the target is in.
    let Some(waivers) = waivers else {
        return detach(request, session, effects, RemovalReason::Keep);
    };

    // Step 3: fresh evidence, then the work and history checks.
    let evidence = ports
        .observe_target(&plan.target)
        .map_err(DisposeError::Port)?;
    if !evidence.unknown.is_empty() {
        return Err(DisposeError::Unknown(evidence.unknown));
    }
    match classify_target(&plan.row, Some(&evidence.target)) {
        ListState::Ready => {}
        // Step 5's second half: the contents are already gone, so an
        // explicit dispose may remove the stale row. No file is touched, so
        // no work or history check applies.
        ListState::Missing => {
            // An observer that reports the target absent and repositories
            // inside it contradicts itself. This is the one path that
            // removes a row with no work or history check, so an
            // inconsistent observation refuses rather than being reconciled.
            if !evidence.repositories.is_empty() {
                return Err(DisposeError::PathMismatch {
                    expected: plan.target,
                    observed: format!(
                        "the target is absent, yet {} repositor(y|ies) were observed in it",
                        evidence.repositories.len()
                    ),
                });
            }
            return detach(request, session, effects, RemovalReason::Stale);
        }
        state @ (ListState::Incomplete | ListState::InterruptedDisposal) => {
            debug_assert_ne!(plan.row.state, MemberState::Ready, "{state:?}");
            return Err(DisposeError::Refused(Refusal::WrongState {
                name: request.name.clone(),
                expected: MemberState::Ready,
                actual: plan.row.state,
            }));
        }
        state => {
            return Err(DisposeError::PathMismatch {
                expected: plan.target,
                observed: describe(state, &evidence.target),
            });
        }
    }
    inspect(&plan, &evidence, waivers, ports)?;

    // Step 4. The write result is checked before anything is removed: the
    // session revalidates the row's state and allocation under the lock.
    session
        .apply(&FamilyChange::MarkDisposing {
            name: request.name.clone(),
            expected_allocation: plan.row.allocation_id.clone(),
        })
        .map_err(DisposeError::Store)?;
    effects.push(DisposeEffect::RowDisposing);

    ports
        .remove_directory(&plan.target)
        .map_err(|failure| DisposeError::RemovalStopped {
            remaining: failure.remaining,
            detail: failure.error.to_string(),
        })?;
    effects.push(DisposeEffect::DirectoryRemoved);

    // Step 5.
    detach(request, session, effects, RemovalReason::Disposed)
}

/// Design §5.2 step 1, and the only decision made without a port.
fn validate(
    request: &DisposeRequest,
    session: &mut dyn FamilySession,
) -> Result<Plan, DisposeError> {
    let root = resolve(&request.root);
    // Checkpoint §11 (lane D): the lock is held on the root the store found;
    // a request naming a different one is addressing a moved or replaced
    // root, and its `root.join(path)` would resolve somewhere else entirely.
    if root != resolve(session.root()) {
        return Err(DisposeError::PathMismatch {
            expected: request.root.clone(),
            observed: format!(
                "the family lock is held on {}, not this root",
                session.root().display()
            ),
        });
    }
    let view = session
        .reread()
        .map_err(DisposeError::Store)?
        .ok_or_else(|| {
            DisposeError::Store(StoreError::NoFamily {
                workspace: request.root.clone(),
            })
        })?;
    let row = view
        .members
        .get(&request.name)
        .ok_or_else(|| {
            DisposeError::Refused(Refusal::NotFound {
                name: request.name.clone(),
            })
        })?
        .clone();
    let recorded = member_path(&request.name, &row.path)?;
    let target = resolve(&request.root.join(recorded.as_str()));

    // The original tree is never deleted (design §5, §8.4). The pure path
    // rules already refuse `.` and `..` spellings; this catches the ones only
    // the host paths reveal, such as `../<the root's own directory name>`.
    if target == root || root.starts_with(&target) {
        return Err(DisposeError::RootImmutable);
    }
    if target.starts_with(&root) {
        return Err(DisposeError::Refused(Refusal::NestedPath {
            path: row.path.clone(),
            other: gwz_family_model::ROOT_PATH.to_owned(),
        }));
    }
    if resolve(&request.cwd).starts_with(&target) {
        return Err(DisposeError::TargetContainsCwd { target });
    }
    for (other_name, other) in &view.members {
        if other_name == &request.name {
            continue;
        }
        let other_path = member_path(other_name, &other.path)?;
        if relate_member_paths(&recorded, &other_path).overlaps() {
            return Err(DisposeError::Refused(Refusal::NestedPath {
                path: row.path.clone(),
                other: other.path.clone(),
            }));
        }
    }
    Ok(Plan { row, target })
}

/// Design §5.1 and §5.2 step 3: every repository in the deletion tree is
/// inspected, `Unknown` dominates and is never waivable, and a known hazard
/// refuses unless its own name was given.
fn inspect(
    plan: &Plan,
    evidence: &TargetEvidence,
    waivers: &[HazardWaiver],
    ports: &mut dyn DisposalPorts,
) -> Result<(), DisposeError> {
    if evidence.repositories.is_empty() {
        return Err(DisposeError::Unknown(vec![UnknownReason::new(
            UnknownKind::UnsupportedLayout,
            format!(
                "no repository was observed in {}; the target is not a recognised clone",
                plan.target.display()
            ),
        )]));
    }
    let mut unknown = Vec::new();
    let mut findings = Vec::new();
    for repository in &evidence.repositories {
        // Nothing outside the validated target is ever considered, and an
        // observation that reached out of it is the visible evidence of a
        // symlink entry into an external tree (design §5.2 step 4).
        for (label, path) in [
            ("worktree", &repository.info.path),
            ("git directory", &repository.info.git_dir),
            ("object store", &repository.info.common_dir),
        ] {
            if !resolve(path).starts_with(&plan.target) {
                return Err(DisposeError::PathMismatch {
                    expected: plan.target.clone(),
                    observed: format!(
                        "the {label} of `{}` is {}, outside the deletion tree",
                        repository.key,
                        path.display()
                    ),
                });
            }
        }

        let report = classify_observed_work(&repository.work, &repository.gwz);
        if report.verdict == WorkVerdict::Unknown && report.unknown.is_empty() {
            unknown.push(UnknownReason::new(
                UnknownKind::Unimplemented,
                format!("`{}`: an unknown verdict with no reason", repository.key),
            ));
        }
        unknown.extend(report.unknown);
        let mut grouped: BTreeMap<HazardWaiver, Vec<Hazard>> = BTreeMap::new();
        for hazard in report.hazards {
            // The classifier's force name and this crate's waiver vocabulary
            // are one map. A hazard this vocabulary cannot spell is unknown,
            // never silently dropped and never waivable.
            match hazard.kind.force_name().and_then(HazardWaiver::parse) {
                Some(waiver) => grouped.entry(waiver).or_default().push(hazard),
                None => unknown.push(UnknownReason::new(
                    UnknownKind::UnsupportedEvidence,
                    format!(
                        "`{}`: {:?} has no waiver ({})",
                        repository.key, hazard.kind, hazard.detail
                    ),
                )),
            }
        }
        findings.extend(grouped.into_iter().map(|(waiver, hazards)| HazardFinding {
            waiver,
            repository: repository.key.clone(),
            hazards,
            detail: None,
        }));

        match &repository.history {
            Observation::Unknown(reasons) => unknown.extend(reasons.iter().cloned()),
            Observation::Known(protected) => {
                let query = HistoryQuery {
                    target: repository.key.clone(),
                    protected: protected.clone(),
                };
                match ports.check_history(&query) {
                    HistoryAnswer::Preserved => {}
                    HistoryAnswer::Unpreserved { detail } => findings.push(HazardFinding {
                        waiver: HazardWaiver::UnpreservedHistory,
                        repository: repository.key.clone(),
                        hazards: Vec::new(),
                        detail: Some(detail),
                    }),
                    HistoryAnswer::Unknown { reasons } => unknown.extend(reasons),
                }
            }
        }
    }
    // Unknown dominates: a force name waives a *known* loss, never an
    // observation that was never established (design §5.1, §12).
    if !unknown.is_empty() {
        return Err(DisposeError::Unknown(unknown));
    }
    let unwaived: Vec<HazardFinding> = findings
        .into_iter()
        .filter(|finding| !waivers.contains(&finding.waiver))
        .collect();
    if !unwaived.is_empty() {
        return Err(DisposeError::Hazards(unwaived));
    }
    Ok(())
}

/// Remove the matching pointer, then the row. The order is the store
/// contract's only recoverable one, and a pointer the store cannot
/// physically remove stops here — the report names the pointer, not the row
/// (checkpoint §11, lane D).
fn detach(
    request: &DisposeRequest,
    session: &mut dyn FamilySession,
    effects: &mut Vec<DisposeEffect>,
    reason: RemovalReason,
) -> Result<(), DisposeError> {
    let applied = session
        .remove_pointer(&request.name)
        .map_err(DisposeError::Store)?;
    if applied
        .effects
        .iter()
        .any(|effect| matches!(effect, MetadataEffect::PointerRemoved { .. }))
    {
        effects.push(DisposeEffect::PointerRemoved);
    }
    session
        .apply(&FamilyChange::RemoveRow {
            name: request.name.clone(),
            reason,
        })
        .map_err(DisposeError::Store)?;
    effects.push(match reason {
        RemovalReason::Keep => DisposeEffect::RowDetached,
        RemovalReason::Disposed | RemovalReason::Stale => DisposeEffect::RowRemoved,
    });
    Ok(())
}

fn member_path(name: &MemberName, path: &str) -> Result<MemberPath, DisposeError> {
    validate_member_path(path).map_err(|error| match error {
        // A row that names the root or an ancestor of it is refused as the
        // root, not as a malformed row: the answer the operator needs is
        // that the original tree is never deleted.
        PathError::RootItself { .. } | PathError::ContainsRoot { .. } => {
            DisposeError::RootImmutable
        }
        PathError::NotNormalised { path, normalised } => {
            DisposeError::Refused(Refusal::PathNotNormalised {
                name: name.clone(),
                path,
                normalised,
            })
        }
        other => DisposeError::Refused(Refusal::InvalidRow {
            name: name.clone(),
            detail: other.to_string(),
        }),
    })
}

fn describe(state: ListState, observation: &TargetObservation) -> String {
    match (state, observation) {
        (_, TargetObservation::Malformed { detail }) => {
            format!("its family metadata could not be decoded: {detail}")
        }
        (ListState::PointerRemoved, _) => {
            "its family pointer is gone; an interrupted detach left the tree in place".to_owned()
        }
        (_, TargetObservation::Present { pointer, marker }) => format!(
            "its family pointer or allocation marker does not belong to this row \
             (pointer {pointer:?}, marker {marker:?})"
        ),
        (state, observation) => format!("{state:?} for {observation:?}"),
    }
}

/// One lexical resolution of a host path, matching the family-store
/// contract's reference resolution (`contract_tests::resolve`), so this
/// library, the model and the store agree on which directory a spelling
/// names. It opens nothing: symlink equivalence is the store's to close by
/// canonicalising before it hands the paths over (see [`DisposeRequest`]).
fn resolve(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match resolved.components().next_back() {
                Some(Component::Normal(_)) => {
                    resolved.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => resolved.push(".."),
            },
            other => resolved.push(other),
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{DisposalCall, RecordingDisposalPorts};
    use gwz_family_model::FamilyView;
    use gwz_family_model::{
        AllocationId, CloneMode, FamilyId, MarkerObservation, MemberKind, PointerObservation,
    };
    use gwz_family_store_contract::contract_tests::InMemoryFamilyStore;
    use gwz_family_store_contract::{AppliedChange, FamilyLocation, FamilyStore, StoreOperation};
    use gwz_repo_contract::{HeadState, ObjectFormat, ObjectId, WorkEntry, WorkKind};
    use gwz_work_detector::EvidenceState;

    const ROOT: &str = "/fam/root";
    const WS_A: &str = "/fam/ws-A";

    fn name(text: &str) -> MemberName {
        MemberName::parse(text).unwrap()
    }

    fn allocation() -> AllocationId {
        AllocationId::new("alloc-A").unwrap()
    }

    fn row(path: &str) -> MemberRow {
        MemberRow {
            path: path.to_owned(),
            kind: MemberKind::Checkout,
            state: MemberState::Creating,
            allocation_id: allocation(),
            source_path: ".".to_owned(),
            mode: CloneMode::Verbatim,
            last_error: None,
        }
    }

    type Session = <InMemoryFamilyStore as FamilyStore>::Session;

    /// A founded family holding `A` at `path` in `state`, with its pointer
    /// and marker installed. `Creating` leaves the row as allocated.
    fn family(path: &str, state: MemberState) -> (InMemoryFamilyStore, Session) {
        let store = InMemoryFamilyStore::new();
        let mut session = store.try_lock(&FamilyLocation::new(ROOT)).unwrap();
        session
            .found(
                FamilyId::new("fam").unwrap(),
                AllocationId::new("alloc-root").unwrap(),
            )
            .unwrap();
        session
            .apply(&FamilyChange::Allocate {
                name: name("A"),
                row: row(path),
            })
            .unwrap();
        session
            .install_pointer(&name("A"), &PathBuf::from(ROOT).join(path))
            .unwrap();
        if state != MemberState::Creating {
            session
                .apply(&FamilyChange::MarkReady {
                    name: name("A"),
                    expected_allocation: allocation(),
                })
                .unwrap();
        }
        if state == MemberState::Disposing {
            session
                .apply(&FamilyChange::MarkDisposing {
                    name: name("A"),
                    expected_allocation: allocation(),
                })
                .unwrap();
        }
        (store, session)
    }

    /// The ordinary case: `A` is ready at `../ws-A`.
    fn ready() -> (InMemoryFamilyStore, Session) {
        family("../ws-A", MemberState::Ready)
    }

    fn delete(waivers: &[HazardWaiver]) -> DisposeRequest {
        DisposeRequest {
            name: name("A"),
            policy: DisposePolicy::Delete {
                waivers: waivers.to_vec(),
            },
            root: PathBuf::from(ROOT),
            cwd: PathBuf::from(ROOT),
        }
    }

    fn keep() -> DisposeRequest {
        DisposeRequest {
            policy: DisposePolicy::Keep,
            ..delete(&[])
        }
    }

    fn oid() -> ObjectId {
        ObjectId::parse_hex(
            ObjectFormat::Sha1,
            "0123456789abcdef0123456789abcdef01234567",
        )
        .unwrap()
    }

    fn present() -> TargetObservation {
        TargetObservation::Present {
            pointer: PointerObservation::Matches,
            marker: MarkerObservation::Matches,
        }
    }

    fn repository(key: RepoKey, path: &str) -> RepositoryEvidence {
        let path = PathBuf::from(path);
        RepositoryEvidence {
            key,
            info: RepositoryInfo {
                git_dir: path.join(".git"),
                common_dir: path.join(".git"),
                path,
                bare: false,
                object_format: ObjectFormat::Sha1,
                head: HeadState::Attached {
                    branch: "refs/heads/main".to_owned(),
                    target: oid(),
                },
            },
            work: Observation::Known(WorkObservation::default()),
            gwz: GwzEvidence::default(),
            history: Observation::Known(ProtectedRoots::default()),
        }
    }

    /// One clean, known repository at the target, present with matching
    /// metadata: the only shape ordinary deletion accepts.
    fn clean_evidence() -> TargetEvidence {
        TargetEvidence {
            target: present(),
            repositories: vec![repository(RepoKey::Root, WS_A)],
            unknown: Vec::new(),
        }
    }

    fn scripted(evidence: TargetEvidence, history: HistoryAnswer) -> RecordingDisposalPorts {
        let mut ports = RecordingDisposalPorts::new();
        ports.evidence(evidence);
        ports.history(history);
        ports
    }

    /// Every refusal must leave the filesystem untouched.
    fn assert_no_removal(ports: &RecordingDisposalPorts) {
        assert!(
            !ports
                .calls()
                .iter()
                .any(|call| matches!(call, DisposalCall::RemoveDirectory { .. })),
            "a refusal called the remover: {:?}",
            ports.calls()
        );
    }

    /// A session over an index handed back exactly as written, so disposal's
    /// defences against a **decoded** index can be exercised: a hand-edited
    /// or foreign `local-family.yml` can carry a row `validate_transition`
    /// would never have created, and those rows must still refuse. The
    /// contract-faithful `InMemoryFamilyStore` is used everywhere else.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum SessionCall {
        Reread,
        Apply(FamilyChange),
        RemovePointer(MemberName),
    }

    struct ScriptedSession {
        root: PathBuf,
        view: Option<FamilyView>,
        /// Members whose clone pointer stands at their recorded path.
        pointers: Vec<MemberName>,
        calls: Vec<SessionCall>,
        fail_apply: Option<StoreError>,
        fail_remove_pointer: Option<StoreError>,
    }

    impl ScriptedSession {
        /// A family whose rows are taken verbatim, with a pointer standing
        /// for every one of them.
        fn with_rows(rows: &[(&str, MemberRow)]) -> Self {
            let mut view = FamilyView::founded(
                FamilyId::new("fam").unwrap(),
                AllocationId::new("alloc-root").unwrap(),
            );
            let mut pointers = Vec::new();
            for (member, row) in rows {
                view.members.insert(name(member), row.clone());
                pointers.push(name(member));
            }
            Self {
                root: PathBuf::from(ROOT),
                view: Some(view),
                pointers,
                calls: Vec::new(),
                fail_apply: None,
                fail_remove_pointer: None,
            }
        }

        fn ready_at(path: &str) -> Self {
            Self::with_rows(&[(
                "A",
                MemberRow {
                    state: MemberState::Ready,
                    ..row(path)
                },
            )])
        }

        fn applied(&self) -> Vec<FamilyChange> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    SessionCall::Apply(change) => Some(change.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    impl FamilySession for ScriptedSession {
        fn root(&self) -> &Path {
            &self.root
        }

        fn reread(&mut self) -> Result<Option<FamilyView>, StoreError> {
            self.calls.push(SessionCall::Reread);
            Ok(self.view.clone())
        }

        fn found(
            &mut self,
            _family_id: FamilyId,
            _root_allocation: AllocationId,
        ) -> Result<AppliedChange, StoreError> {
            Err(StoreError::Unimplemented {
                operation: StoreOperation::WriteIndex,
            })
        }

        fn apply(&mut self, change: &FamilyChange) -> Result<AppliedChange, StoreError> {
            self.calls.push(SessionCall::Apply(change.clone()));
            if let Some(error) = self.fail_apply.take() {
                return Err(error);
            }
            let view = self.view.clone().ok_or_else(|| StoreError::NoFamily {
                workspace: self.root.clone(),
            })?;
            // The store's own ordering guard: a row may not be removed while
            // this family's pointer still stands at its recorded path.
            if let FamilyChange::RemoveRow { name, .. } = change
                && self.pointers.contains(name)
            {
                return Err(StoreError::PointerStillInstalled {
                    member: name.as_str().to_owned(),
                    workspace: self.root.join(&view.members[name].path),
                });
            }
            let validated = gwz_family_model::validate_transition(&view, change)?;
            self.view = match change {
                FamilyChange::Disband => None,
                _ => Some(validated.next),
            };
            Ok(AppliedChange {
                effects: vec![MetadataEffect::IndexWritten],
                view: self.view.clone(),
            })
        }

        fn install_pointer(
            &mut self,
            _name: &MemberName,
            _destination: &Path,
        ) -> Result<AppliedChange, StoreError> {
            Err(StoreError::Unimplemented {
                operation: StoreOperation::WritePointer,
            })
        }

        fn remove_pointer(&mut self, member: &MemberName) -> Result<AppliedChange, StoreError> {
            self.calls.push(SessionCall::RemovePointer(member.clone()));
            if let Some(error) = self.fail_remove_pointer.take() {
                return Err(error);
            }
            let view = self.view.clone().ok_or_else(|| StoreError::NoFamily {
                workspace: self.root.clone(),
            })?;
            let mut effects = Vec::new();
            if let Some(at) = self.pointers.iter().position(|held| held == member) {
                self.pointers.remove(at);
                effects.push(MetadataEffect::PointerRemoved {
                    workspace: self.root.join(&view.members[member].path),
                });
            }
            Ok(AppliedChange {
                effects,
                view: Some(view),
            })
        }
    }

    fn dirty_work() -> Observation<WorkObservation> {
        Observation::Known(WorkObservation {
            entries: vec![WorkEntry {
                path: b"notes.txt".to_vec(),
                kind: WorkKind::Untracked,
                binary: Some(false),
            }],
            ..WorkObservation::default()
        })
    }

    fn open_merge() -> GwzEvidence {
        GwzEvidence {
            merge: EvidenceState::Open {
                detail: "merge gwz_merge_0001 is open".to_owned(),
            },
            ..GwzEvidence::default()
        }
    }

    #[test]
    fn hazard_vocabulary_is_exact() {
        assert_eq!(
            HazardWaiver::parse("open-merge"),
            Some(HazardWaiver::OpenMerge)
        );
        assert_eq!(HazardWaiver::parse("dirty"), Some(HazardWaiver::Dirty));
        assert_eq!(
            HazardWaiver::parse("unpreserved-history"),
            Some(HazardWaiver::UnpreservedHistory)
        );
        assert_eq!(HazardWaiver::parse("force"), None);
        assert_eq!(HazardWaiver::parse("Dirty"), None);
        let parsed =
            HazardWaiver::parse_all(&["dirty".to_owned(), "open-merge".to_owned()]).unwrap();
        assert_eq!(parsed, vec![HazardWaiver::Dirty, HazardWaiver::OpenMerge]);
        assert_eq!(HazardWaiver::parse_all(&[]).unwrap(), Vec::new());
        let unknown = HazardWaiver::parse_all(&["true".to_owned()]).unwrap_err();
        assert_eq!(unknown.name, "true");
        assert!(unknown.to_string().contains("unpreserved-history"));
        assert!(HazardWaiver::parse_all(&["dirty".to_owned(), "dirty".to_owned()]).is_err());
    }

    /// Design §5.2 step 3: empty and unknown force names and keep+force all
    /// refuse, and every hazard the classifier can name has a waiver.
    #[test]
    fn empty_unknown_and_keep_plus_force_names_refuse() {
        assert_eq!(
            DisposePolicy::parse(true, &[]).unwrap(),
            DisposePolicy::Keep
        );
        assert_eq!(
            DisposePolicy::parse(false, &[]).unwrap(),
            DisposePolicy::Delete {
                waivers: Vec::new()
            },
            "an absent force list is no force, not a refusal"
        );
        for name in ["", " ", "all", "true", "unpreserved history"] {
            let error = DisposePolicy::parse(false, &[name.to_owned()]).unwrap_err();
            assert_eq!(
                error,
                PolicyError::UnknownHazard(UnknownHazard {
                    name: name.to_owned()
                }),
                "`{name}` is not a hazard name"
            );
        }
        let keep_force = DisposePolicy::parse(true, &["dirty".to_owned()]).unwrap_err();
        assert_eq!(
            keep_force,
            PolicyError::KeepWithForce {
                names: vec!["dirty".to_owned()]
            }
        );
        assert!(keep_force.to_string().contains("mutually exclusive"));
        // The classifier's force names and this vocabulary are one map.
        for kind in [
            gwz_work_detector::HazardKind::Work(WorkKind::Untracked),
            gwz_work_detector::HazardKind::Suppressed,
            gwz_work_detector::HazardKind::NativeStash,
            gwz_work_detector::HazardKind::OpenNativeOperation,
            gwz_work_detector::HazardKind::OpenGwzMerge,
            gwz_work_detector::HazardKind::OpenGwzStash,
            gwz_work_detector::HazardKind::OpenGwzRecord,
        ] {
            let force = kind.force_name().expect("a known hazard has a force name");
            assert!(
                HazardWaiver::parse(force).is_some(),
                "{kind:?} names `{force}`, which this crate cannot waive"
            );
        }
        assert_eq!(
            gwz_work_detector::HazardKind::UninterpretableEvidence.force_name(),
            None,
            "uninterpretable evidence is never waivable"
        );
    }

    /// The operator's standing default (design §5): unpreserved history
    /// refuses, and nothing is removed.
    #[test]
    fn unpreserved_history_refuses_before_any_removal() {
        let (_store, mut session) = ready();
        let mut ports = scripted(
            clean_evidence(),
            HistoryAnswer::Unpreserved {
                detail: "refs/heads/lane/agent-17 is in no survivor".to_owned(),
            },
        );
        let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
        assert_eq!(
            failure.error,
            DisposeError::Hazards(vec![HazardFinding {
                waiver: HazardWaiver::UnpreservedHistory,
                repository: RepoKey::Root,
                hazards: Vec::new(),
                detail: Some("refs/heads/lane/agent-17 is in no survivor".to_owned()),
            }])
        );
        assert!(failure.effects.is_empty());
        assert_no_removal(&ports);
        assert_eq!(
            session.reread().unwrap().unwrap().members[&name("A")].state,
            MemberState::Ready,
            "the row is untouched"
        );
    }

    /// A clean tree with unique history still refuses without the name
    /// (design §8.4), and proceeds past the check with it.
    #[test]
    fn unpreserved_history_is_waivable_only_by_its_own_name() {
        for waivers in [
            vec![],
            vec![HazardWaiver::Dirty],
            vec![HazardWaiver::OpenMerge, HazardWaiver::Dirty],
        ] {
            let (_store, mut session) = ready();
            let mut ports = scripted(
                clean_evidence(),
                HistoryAnswer::Unpreserved {
                    detail: "unique".to_owned(),
                },
            );
            let failure = dispose(&delete(&waivers), &mut session, &mut ports).unwrap_err();
            assert!(
                matches!(failure.error, DisposeError::Hazards(ref findings)
                    if findings.iter().all(|f| f.waiver == HazardWaiver::UnpreservedHistory)),
                "{waivers:?} must not waive unpreserved history: {:?}",
                failure.error
            );
            assert_no_removal(&ports);
        }
    }

    /// Design §5.1: unknown work or unknown history refuses ordinary
    /// deletion, and **no** force name waives it.
    #[test]
    fn unknown_work_or_history_refuses_and_no_force_waives_it() {
        let unreadable = UnknownReason::new(UnknownKind::Unreadable, "the index could not be read");
        let cases: Vec<(&str, TargetEvidence, HistoryAnswer)> = vec![
            (
                "unknown work inventory",
                TargetEvidence {
                    repositories: vec![RepositoryEvidence {
                        work: Observation::Unknown(vec![unreadable.clone()]),
                        ..repository(RepoKey::Root, WS_A)
                    }],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "a known inventory with an unestablished path",
                TargetEvidence {
                    repositories: vec![RepositoryEvidence {
                        work: Observation::Known(WorkObservation {
                            unknown: vec![unreadable.clone()],
                            ..WorkObservation::default()
                        }),
                        ..repository(RepoKey::Root, WS_A)
                    }],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "uninterpretable gwz evidence",
                TargetEvidence {
                    repositories: vec![RepositoryEvidence {
                        gwz: GwzEvidence {
                            stash: EvidenceState::Unknown {
                                detail: "unsupported stash record".to_owned(),
                            },
                            ..GwzEvidence::default()
                        },
                        ..repository(RepoKey::Root, WS_A)
                    }],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "unknown history inventory",
                TargetEvidence {
                    repositories: vec![RepositoryEvidence {
                        history: Observation::Unknown(vec![unreadable.clone()]),
                        ..repository(RepoKey::Root, WS_A)
                    }],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "the verifier hit a resource limit",
                clean_evidence(),
                HistoryAnswer::Unknown {
                    reasons: vec![UnknownReason::new(
                        UnknownKind::LimitExceeded,
                        "100000 roots",
                    )],
                },
            ),
        ];
        for (label, evidence, history) in cases {
            for waivers in [vec![], HazardWaiver::ALL.to_vec()] {
                let (_store, mut session) = ready();
                let mut ports = scripted(evidence.clone(), history.clone());
                let failure = dispose(&delete(&waivers), &mut session, &mut ports).unwrap_err();
                assert!(
                    matches!(failure.error, DisposeError::Unknown(ref reasons) if !reasons.is_empty()),
                    "{label} with {waivers:?} must refuse as unknown, got {:?}",
                    failure.error
                );
                assert!(failure.effects.is_empty());
                assert_no_removal(&ports);
            }
        }
    }

    /// Each known hazard refuses without its own name (design §8.4's
    /// `--force open-merge` still refusing dirt and history).
    #[test]
    fn each_known_hazard_refuses_until_its_own_name_is_given() {
        let evidence = TargetEvidence {
            repositories: vec![RepositoryEvidence {
                work: dirty_work(),
                gwz: open_merge(),
                ..repository(RepoKey::Root, WS_A)
            }],
            ..clean_evidence()
        };
        for (waivers, still) in [
            (
                vec![],
                vec![
                    HazardWaiver::OpenMerge,
                    HazardWaiver::Dirty,
                    HazardWaiver::UnpreservedHistory,
                ],
            ),
            (
                vec![HazardWaiver::OpenMerge],
                vec![HazardWaiver::Dirty, HazardWaiver::UnpreservedHistory],
            ),
            (
                vec![HazardWaiver::OpenMerge, HazardWaiver::Dirty],
                vec![HazardWaiver::UnpreservedHistory],
            ),
            (
                vec![HazardWaiver::Dirty, HazardWaiver::UnpreservedHistory],
                vec![HazardWaiver::OpenMerge],
            ),
        ] {
            let (_store, mut session) = ready();
            let mut ports = scripted(
                evidence.clone(),
                HistoryAnswer::Unpreserved {
                    detail: "unique".to_owned(),
                },
            );
            let failure = dispose(&delete(&waivers), &mut session, &mut ports).unwrap_err();
            let DisposeError::Hazards(findings) = &failure.error else {
                panic!(
                    "{waivers:?} must refuse with hazards, got {:?}",
                    failure.error
                );
            };
            let mut named: Vec<HazardWaiver> = findings.iter().map(|f| f.waiver).collect();
            named.sort();
            named.dedup();
            let mut expected = still.clone();
            expected.sort();
            assert_eq!(named, expected, "waived {waivers:?}");
            assert_no_removal(&ports);
        }
    }

    /// Design §5.2 step 1 and §8.4: the root is never deleted, and the
    /// target may never contain the working directory.
    #[test]
    fn the_root_and_the_working_directory_are_protected() {
        // Host paths the pure rules cannot see: `../root` and `../../fam`
        // are legal member-path spellings that resolve back onto the root
        // and onto a directory containing it. Only a decoded index can carry
        // them, so they arrive through a session, not through an allocation.
        for path in ["../root", "../../fam"] {
            let mut session = ScriptedSession::ready_at(path);
            let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
            let failure =
                dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
            assert_eq!(failure.error, DisposeError::RootImmutable, "path {path}");
            assert!(failure.effects.is_empty());
            assert_no_removal(&ports);
        }
        // The same for the spellings the pure rules do refuse.
        for path in [".", "..", "ws/.."] {
            let mut session = ScriptedSession::ready_at(path);
            let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
            let failure =
                dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
            assert_eq!(failure.error, DisposeError::RootImmutable, "path {path}");
        }

        // The invoking process stands inside the target.
        for cwd in [WS_A, "/fam/ws-A/sub/dir", "/fam/root/../ws-A"] {
            let (_store, mut session) = ready();
            let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
            let request = DisposeRequest {
                cwd: PathBuf::from(cwd),
                ..delete(&HazardWaiver::ALL)
            };
            let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
            assert_eq!(
                failure.error,
                DisposeError::TargetContainsCwd {
                    target: PathBuf::from(WS_A)
                },
                "cwd {cwd}"
            );
            assert_no_removal(&ports);
            // `--keep` removes no file, but step 1 still runs before step 2.
            let (_store, mut session) = ready();
            let request = DisposeRequest {
                cwd: PathBuf::from(cwd),
                ..keep()
            };
            let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
            assert!(matches!(
                failure.error,
                DisposeError::TargetContainsCwd { .. }
            ));
            assert_eq!(
                session.reread().unwrap().unwrap().members.len(),
                1,
                "the row survives the refusal"
            );
        }
    }

    /// A recorded path that overlaps the root or another member would delete
    /// a directory the row does not own. `validate_transition` refuses these
    /// at allocation; a decoded index can still carry them.
    #[test]
    fn an_overlapping_or_unusable_recorded_path_refuses() {
        // Nested inside the root.
        let mut session = ScriptedSession::ready_at("../root/sub");
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert_eq!(
            failure.error,
            DisposeError::Refused(Refusal::NestedPath {
                path: "../root/sub".to_owned(),
                other: ".".to_owned(),
            })
        );
        assert_no_removal(&ports);

        // Containing another member's tree.
        let mut session = ScriptedSession::with_rows(&[
            (
                "A",
                MemberRow {
                    state: MemberState::Ready,
                    ..row("../ws-A")
                },
            ),
            (
                "B",
                MemberRow {
                    state: MemberState::Ready,
                    allocation_id: AllocationId::new("alloc-B").unwrap(),
                    ..row("../ws-A/nested")
                },
            ),
        ]);
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert_eq!(
            failure.error,
            DisposeError::Refused(Refusal::NestedPath {
                path: "../ws-A".to_owned(),
                other: "../ws-A/nested".to_owned(),
            })
        );
        assert_no_removal(&ports);

        // Spellings the index must never record, and paths that are not
        // member paths at all.
        for (path, expected) in [
            (
                "../ws-A/../ws-A",
                DisposeError::Refused(Refusal::PathNotNormalised {
                    name: name("A"),
                    path: "../ws-A/../ws-A".to_owned(),
                    normalised: "../ws-A".to_owned(),
                }),
            ),
            (
                "/fam/ws-A",
                DisposeError::Refused(Refusal::InvalidRow {
                    name: name("A"),
                    detail: gwz_family_model::PathError::Absolute {
                        path: "/fam/ws-A".to_owned(),
                    }
                    .to_string(),
                }),
            ),
            (
                "",
                DisposeError::Refused(Refusal::InvalidRow {
                    name: name("A"),
                    detail: gwz_family_model::PathError::Empty.to_string(),
                }),
            ),
        ] {
            let mut session = ScriptedSession::ready_at(path);
            let failure =
                dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
            assert_eq!(failure.error, expected, "path `{path}`");
            assert!(failure.effects.is_empty());
            assert_no_removal(&ports);
        }
    }

    /// Checkpoint §11 lane-D note: a moved root, a replaced target or an
    /// interrupted detach is a `PathMismatch`, and force never excuses it.
    #[test]
    fn a_moved_root_or_replaced_target_refuses_as_a_path_mismatch() {
        // The request's root is not the root whose lock this session holds.
        let (_store, mut session) = ready();
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let request = DisposeRequest {
            root: PathBuf::from("/fam/moved-root"),
            cwd: PathBuf::from("/fam/moved-root"),
            ..delete(&HazardWaiver::ALL)
        };
        let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
        assert!(
            matches!(failure.error, DisposeError::PathMismatch { .. }),
            "a moved root must be a PathMismatch, got {:?}",
            failure.error
        );
        assert_no_removal(&ports);

        // What stands at the recorded path is not this member.
        let replaced = [
            TargetObservation::Present {
                pointer: PointerObservation::OtherFamily,
                marker: MarkerObservation::Matches,
            },
            TargetObservation::Present {
                pointer: PointerObservation::Matches,
                marker: MarkerObservation::Mismatch,
            },
            TargetObservation::Present {
                pointer: PointerObservation::IsIndex,
                marker: MarkerObservation::Absent,
            },
            TargetObservation::Present {
                pointer: PointerObservation::Absent,
                marker: MarkerObservation::Matches,
            },
            TargetObservation::Malformed {
                detail: "the family pointer is not YAML".to_owned(),
            },
        ];
        for target in replaced {
            let (_store, mut session) = ready();
            let mut ports = scripted(
                TargetEvidence {
                    target: target.clone(),
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            );
            let failure =
                dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
            assert!(
                matches!(failure.error, DisposeError::PathMismatch { .. }),
                "{target:?} must refuse as a PathMismatch, got {:?}",
                failure.error
            );
            assert!(failure.effects.is_empty());
            assert_no_removal(&ports);
        }
    }

    /// Design §5.2/§5.3: an incomplete create and an interrupted deletion
    /// are retained and are not forceable, but `--keep` detaches them.
    #[test]
    fn incomplete_and_interrupted_targets_refuse_deletion_but_accept_keep() {
        for state in [MemberState::Creating, MemberState::Disposing] {
            let (_store, mut session) = family("../ws-A", state);
            let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
            let failure =
                dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
            assert_eq!(
                failure.error,
                DisposeError::Refused(Refusal::WrongState {
                    name: name("A"),
                    expected: MemberState::Ready,
                    actual: state,
                }),
                "{state:?} is not forceable through this path"
            );
            assert!(failure.effects.is_empty());
            assert_no_removal(&ports);

            let report = dispose(&keep(), &mut session, &mut ports).expect("keep detaches");
            assert_eq!(
                report.effects,
                vec![DisposeEffect::PointerRemoved, DisposeEffect::RowDetached]
            );
            assert!(
                session.reread().unwrap().unwrap().members.is_empty(),
                "the row is detached"
            );
            assert_no_removal(&ports);
        }
    }

    /// Design §5.1: an uninterpretable layout, and a tree in which no
    /// repository was observed at all, refuse as unknown.
    #[test]
    fn uninterpretable_or_unrecognised_trees_refuse_as_unknown() {
        let (_store, mut session) = ready();
        let mut ports = scripted(
            TargetEvidence {
                unknown: vec![UnknownReason::new(
                    UnknownKind::UnsupportedLayout,
                    "nested repository with external alternates",
                )],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        );
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert!(matches!(failure.error, DisposeError::Unknown(_)));
        assert_no_removal(&ports);

        let (_store, mut session) = ready();
        let mut ports = scripted(
            TargetEvidence {
                repositories: Vec::new(),
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        );
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert!(
            matches!(failure.error, DisposeError::Unknown(_)),
            "a tree with no observed repository is not recognised: {:?}",
            failure.error
        );
        assert_no_removal(&ports);
    }

    /// Design §5.2 step 4: nothing outside the validated target is ever
    /// considered, so an observation that reached out of the tree — the
    /// visible evidence of a symlink entry into an external tree — refuses.
    #[test]
    fn evidence_from_outside_the_deletion_tree_refuses() {
        let outside = [
            ("/fam/ws-B", None),
            ("/fam/ws-A-sibling", None),
            (WS_A, Some(PathBuf::from("/fam/ws-B/.git"))),
        ];
        for (path, common_dir) in outside {
            let mut evidence = repository(
                RepoKey::Member {
                    id: "m1".to_owned(),
                },
                path,
            );
            if let Some(common_dir) = common_dir {
                evidence.info.common_dir = common_dir;
            }
            let (_store, mut session) = ready();
            let mut ports = scripted(
                TargetEvidence {
                    repositories: vec![repository(RepoKey::Root, WS_A), evidence],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            );
            let failure =
                dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
            assert!(
                matches!(failure.error, DisposeError::PathMismatch { .. }),
                "{path} must refuse, got {:?}",
                failure.error
            );
            assert_no_removal(&ports);
        }
    }

    /// Design §5.2 step 4: `disposing` is written first and its result is
    /// checked, so a failed write stops before anything is removed.
    #[test]
    fn a_failed_disposing_write_stops_before_the_remover_is_called() {
        let mut session = ScriptedSession::ready_at("../ws-A");
        session.fail_apply = Some(StoreError::Io {
            operation: StoreOperation::WriteIndex,
            path: PathBuf::from("/fam/root/.gwz/local-family.yml"),
            detail: "no space left on device".to_owned(),
        });
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert!(
            matches!(failure.error, DisposeError::Store(StoreError::Io { .. })),
            "{:?}",
            failure.error
        );
        assert!(failure.effects.is_empty(), "nothing completed");
        assert_no_removal(&ports);
        assert_eq!(
            session.applied(),
            vec![FamilyChange::MarkDisposing {
                name: name("A"),
                expected_allocation: allocation(),
            }],
            "the only write attempted was `disposing`, and it came first"
        );
        assert_eq!(
            session.reread().unwrap().unwrap().members[&name("A")].state,
            MemberState::Ready,
            "the failed write left the row alone"
        );
    }

    /// An unrecognised name refuses before any observation.
    #[test]
    fn an_unknown_member_refuses_before_any_port_call() {
        let (_store, mut session) = ready();
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let request = DisposeRequest {
            name: name("Z"),
            ..delete(&[])
        };
        let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
        assert_eq!(
            failure.error,
            DisposeError::Refused(Refusal::NotFound { name: name("Z") })
        );
        assert!(ports.calls().is_empty(), "no port was consulted");
    }

    /// Design §8.4: `--keep` detaches C's metadata and leaves its entire
    /// tree, open merge and history on disk. It consults no port at all.
    #[test]
    fn keep_detaches_a_ready_member_without_consulting_any_port() {
        let (store, mut session) = ready();
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let report = dispose(&keep(), &mut session, &mut ports).expect("keep detaches");
        assert_eq!(
            report.effects,
            vec![DisposeEffect::PointerRemoved, DisposeEffect::RowDetached],
            "the pointer goes strictly before the row"
        );
        assert!(
            ports.calls().is_empty(),
            "no evidence, history or removal call: every file stays"
        );
        assert!(session.reread().unwrap().unwrap().members.is_empty());
        assert!(store.pointers().is_empty());
        // A detached tree is no longer a member, so a repeat has no row.
        let failure = dispose(&keep(), &mut session, &mut ports).unwrap_err();
        assert_eq!(
            failure.error,
            DisposeError::Refused(Refusal::NotFound { name: name("A") })
        );
    }

    /// Design §12: a clean intact lane whose protected history lives in a
    /// survivor is deleted once, with no archive and no second removal.
    #[test]
    fn a_clean_preserved_intact_lane_is_deleted_once() {
        let (store, mut session) = ready();
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let report = dispose(&delete(&[]), &mut session, &mut ports).expect("deletion proceeds");
        assert_eq!(
            report.effects,
            vec![
                DisposeEffect::RowDisposing,
                DisposeEffect::DirectoryRemoved,
                DisposeEffect::PointerRemoved,
                DisposeEffect::RowRemoved,
            ],
            "disposing is written first; the pointer goes before the row"
        );
        assert_eq!(
            ports.calls(),
            [
                DisposalCall::ObserveTarget {
                    target: PathBuf::from(WS_A)
                },
                DisposalCall::CheckHistory {
                    query: HistoryQuery {
                        target: RepoKey::Root,
                        protected: ProtectedRoots::default(),
                    }
                },
                DisposalCall::RemoveDirectory {
                    target: PathBuf::from(WS_A)
                },
            ],
            "one observation, one history query, exactly one removal of the validated target"
        );
        assert!(session.reread().unwrap().unwrap().members.is_empty());
        assert!(store.pointers().is_empty(), "no pointer is stranded");
    }

    /// Design §8.4: the explicit destructive alternative. Every named
    /// hazard, and only the named ones, is waived.
    #[test]
    fn every_named_hazard_is_waived_over_an_intact_ready_tree() {
        let (_store, mut session) = ready();
        let mut ports = scripted(
            TargetEvidence {
                repositories: vec![RepositoryEvidence {
                    work: dirty_work(),
                    gwz: open_merge(),
                    ..repository(RepoKey::Root, WS_A)
                }],
                ..clean_evidence()
            },
            HistoryAnswer::Unpreserved {
                detail: "lane/agent-17 is unique".to_owned(),
            },
        );
        let report = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports)
            .expect("all three names waive all three hazards");
        assert!(report.effects.contains(&DisposeEffect::DirectoryRemoved));
        assert!(session.reread().unwrap().unwrap().members.is_empty());
    }

    /// Design §5.1: every repository in the deletion tree is inspected, so
    /// the history port is asked once per repository, keyed by identity.
    #[test]
    fn the_history_port_is_asked_once_per_repository_in_the_tree() {
        let keys = [
            RepoKey::Root,
            RepoKey::Member {
                id: "taut".to_owned(),
            },
            RepoKey::Member {
                id: "nested".to_owned(),
            },
        ];
        let (_store, mut session) = ready();
        let mut ports = scripted(
            TargetEvidence {
                repositories: vec![
                    repository(keys[0].clone(), WS_A),
                    repository(keys[1].clone(), "/fam/ws-A/taut"),
                    repository(keys[2].clone(), "/fam/ws-A/taut/nested"),
                ],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        );
        // The middle repository is the only one whose history is unique.
        ports.history_sequence([
            HistoryAnswer::Preserved,
            HistoryAnswer::Unpreserved {
                detail: "taut has unique commits".to_owned(),
            },
            HistoryAnswer::Preserved,
        ]);
        let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
        assert_eq!(
            failure.error,
            DisposeError::Hazards(vec![HazardFinding {
                waiver: HazardWaiver::UnpreservedHistory,
                repository: keys[1].clone(),
                hazards: Vec::new(),
                detail: Some("taut has unique commits".to_owned()),
            }]),
            "the refusal names the repository that is not preserved"
        );
        let queried: Vec<RepoKey> = ports
            .calls()
            .iter()
            .filter_map(|call| match call {
                DisposalCall::CheckHistory { query } => Some(query.target.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(queried, keys, "one query per repository, in tree order");
        assert_no_removal(&ports);
    }

    /// Design §5.2/§5.3: a removal error stops, reports the remainder, and
    /// rolls nothing back. A later explicit dispose reports the interrupted
    /// state; once the contents are gone it may remove the stale row.
    #[test]
    fn a_removal_error_stops_and_leaves_the_remainder_for_manual_cleanup() {
        let (_store, mut session) = ready();
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let remaining = vec![
            PathBuf::from("/fam/ws-A/locked"),
            PathBuf::from("/fam/ws-A/locked/db"),
        ];
        ports.fail_removal(RemovalFailure {
            error: PortError::Removal {
                path: PathBuf::from("/fam/ws-A/locked/db"),
                detail: "resource busy".to_owned(),
            },
            remaining: remaining.clone(),
        });
        let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
        let DisposeError::RemovalStopped {
            remaining: reported,
            detail,
        } = &failure.error
        else {
            panic!("expected RemovalStopped, got {:?}", failure.error);
        };
        assert_eq!(reported, &remaining);
        assert!(detail.contains("resource busy"), "{detail}");
        assert_eq!(
            failure.effects,
            vec![DisposeEffect::RowDisposing],
            "the row was marked, nothing else completed"
        );
        assert_eq!(
            session.reread().unwrap().unwrap().members[&name("A")].state,
            MemberState::Disposing,
            "no rollback: the interrupted state stands for a later command to report"
        );

        // Repeating it is not a replay: the interrupted row is not forceable.
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert_eq!(
            failure.error,
            DisposeError::Refused(Refusal::WrongState {
                name: name("A"),
                expected: MemberState::Ready,
                actual: MemberState::Disposing,
            })
        );
        assert_no_removal(&ports);

        // After manual cleanup the contents are gone, and an explicit
        // dispose may remove the stale row (design §5.2 step 5).
        let mut ports = scripted(
            TargetEvidence {
                target: TargetObservation::Missing,
                repositories: Vec::new(),
                unknown: Vec::new(),
            },
            HistoryAnswer::Preserved,
        );
        let report = dispose(&delete(&[]), &mut session, &mut ports).expect("the stale row goes");
        assert_eq!(
            report.effects,
            vec![DisposeEffect::PointerRemoved, DisposeEffect::RowRemoved]
        );
        assert!(session.reread().unwrap().unwrap().members.is_empty());
        assert_no_removal(&ports);
    }

    /// A stale row is removed only after validation, and its removal asks
    /// neither the work detector nor the history verifier: no file is
    /// touched, so there is nothing to lose.
    #[test]
    fn a_stale_row_for_an_absent_target_needs_no_checks_and_no_force() {
        let (store, mut session) = ready();
        let mut ports = RecordingDisposalPorts::new();
        ports.evidence(TargetEvidence {
            target: TargetObservation::Missing,
            repositories: Vec::new(),
            unknown: Vec::new(),
        });
        // No history answer is scripted: an unscripted call would be
        // `Unknown` and would refuse, so a green run proves none was made.
        let report = dispose(&delete(&[]), &mut session, &mut ports).expect("the stale row goes");
        assert_eq!(
            report.effects,
            vec![DisposeEffect::PointerRemoved, DisposeEffect::RowRemoved]
        );
        assert_eq!(
            ports.calls(),
            [DisposalCall::ObserveTarget {
                target: PathBuf::from(WS_A)
            }],
            "the target was observed; nothing else was asked and nothing was removed"
        );
        assert!(store.pointers().is_empty());
        assert!(session.reread().unwrap().unwrap().members.is_empty());
    }

    /// The stale exit is the only one that removes a row with no work or
    /// history check, so a self-contradicting observation refuses there.
    #[test]
    fn an_absent_target_holding_repositories_refuses() {
        let (store, mut session) = ready();
        let mut ports = scripted(
            TargetEvidence {
                target: TargetObservation::Missing,
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        );
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert!(
            matches!(failure.error, DisposeError::PathMismatch { .. }),
            "{:?}",
            failure.error
        );
        assert!(failure.effects.is_empty());
        assert_no_removal(&ports);
        assert_eq!(store.pointers().len(), 1, "the row and pointer stand");
    }

    /// Checkpoint §11 (lane D): a pointer the store cannot physically remove
    /// blocks row removal, and the report names the pointer, not the row.
    #[test]
    fn a_pointer_the_store_cannot_remove_blocks_the_row() {
        let pointer_failure = || StoreError::Io {
            operation: StoreOperation::RemovePointer,
            path: PathBuf::from("/fam/ws-A/.gwz/family-root"),
            detail: "permission denied".to_owned(),
        };
        // Through `--keep`.
        let mut session = ScriptedSession::ready_at("../ws-A");
        session.fail_remove_pointer = Some(pointer_failure());
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let failure = dispose(&keep(), &mut session, &mut ports).unwrap_err();
        assert_eq!(failure.error, DisposeError::Store(pointer_failure()));
        assert!(failure.effects.is_empty());
        assert!(
            session.applied().is_empty(),
            "the row removal was never attempted"
        );
        assert!(
            session
                .reread()
                .unwrap()
                .unwrap()
                .members
                .contains_key(&name("A")),
            "the row stands while its pointer does"
        );
        assert!(ports.calls().is_empty(), "keep consults no port");

        // And after a successful deletion: the row stays `disposing` for a
        // later command to report, and nothing is rolled back.
        let mut session = ScriptedSession::ready_at("../ws-A");
        session.fail_remove_pointer = Some(pointer_failure());
        let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
        assert_eq!(failure.error, DisposeError::Store(pointer_failure()));
        assert_eq!(
            failure.effects,
            vec![DisposeEffect::RowDisposing, DisposeEffect::DirectoryRemoved]
        );
        assert_eq!(
            session.applied(),
            vec![FamilyChange::MarkDisposing {
                name: name("A"),
                expected_allocation: allocation(),
            }],
            "RemoveRow was never attempted, so the row is not reported instead"
        );
    }

    /// An evidence port that cannot answer refuses; it never reads as clean.
    #[test]
    fn an_evidence_port_failure_refuses() {
        let (_store, mut session) = ready();
        let mut ports = RecordingDisposalPorts::new();
        ports.fail_evidence(PortError::Evidence {
            detail: "the deletion tree could not be walked".to_owned(),
        });
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert!(
            matches!(
                failure.error,
                DisposeError::Port(PortError::Evidence { .. })
            ),
            "{:?}",
            failure.error
        );
        assert!(failure.effects.is_empty());
        assert_no_removal(&ports);
    }

    /// The whole refusal surface in one sweep: on every one of them the
    /// removal port recorded zero calls and no row was removed.
    #[test]
    fn no_refusal_path_ever_reaches_the_remover() {
        let unreadable = UnknownReason::new(UnknownKind::Unreadable, "unreadable");
        let cases: Vec<(&str, DisposeRequest, TargetEvidence, HistoryAnswer)> = vec![
            (
                "unknown member",
                DisposeRequest {
                    name: name("Z"),
                    ..delete(&HazardWaiver::ALL)
                },
                clean_evidence(),
                HistoryAnswer::Preserved,
            ),
            (
                "repeated waiver",
                delete(&[HazardWaiver::Dirty, HazardWaiver::Dirty]),
                clean_evidence(),
                HistoryAnswer::Preserved,
            ),
            (
                "cwd inside the target",
                DisposeRequest {
                    cwd: PathBuf::from("/fam/ws-A/src"),
                    ..delete(&HazardWaiver::ALL)
                },
                clean_evidence(),
                HistoryAnswer::Preserved,
            ),
            (
                "a root the lock does not hold",
                DisposeRequest {
                    root: PathBuf::from("/fam/elsewhere"),
                    cwd: PathBuf::from("/fam/elsewhere"),
                    ..delete(&HazardWaiver::ALL)
                },
                clean_evidence(),
                HistoryAnswer::Preserved,
            ),
            (
                "a replaced target",
                delete(&HazardWaiver::ALL),
                TargetEvidence {
                    target: TargetObservation::Present {
                        pointer: PointerObservation::OtherFamily,
                        marker: MarkerObservation::Mismatch,
                    },
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "an uninterpretable nested layout",
                delete(&HazardWaiver::ALL),
                TargetEvidence {
                    unknown: vec![unreadable.clone()],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "no repository in the tree",
                delete(&HazardWaiver::ALL),
                TargetEvidence {
                    repositories: Vec::new(),
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "a repository outside the tree",
                delete(&HazardWaiver::ALL),
                TargetEvidence {
                    repositories: vec![repository(RepoKey::Root, "/fam/ws-B")],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "unknown work",
                delete(&HazardWaiver::ALL),
                TargetEvidence {
                    repositories: vec![RepositoryEvidence {
                        work: Observation::Unknown(vec![unreadable.clone()]),
                        ..repository(RepoKey::Root, WS_A)
                    }],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "unknown history",
                delete(&HazardWaiver::ALL),
                clean_evidence(),
                HistoryAnswer::Unknown {
                    reasons: vec![unreadable.clone()],
                },
            ),
            (
                "unwaived dirt",
                delete(&[HazardWaiver::OpenMerge, HazardWaiver::UnpreservedHistory]),
                TargetEvidence {
                    repositories: vec![RepositoryEvidence {
                        work: dirty_work(),
                        ..repository(RepoKey::Root, WS_A)
                    }],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "unwaived open merge",
                delete(&[HazardWaiver::Dirty, HazardWaiver::UnpreservedHistory]),
                TargetEvidence {
                    repositories: vec![RepositoryEvidence {
                        gwz: open_merge(),
                        ..repository(RepoKey::Root, WS_A)
                    }],
                    ..clean_evidence()
                },
                HistoryAnswer::Preserved,
            ),
            (
                "unwaived unpreserved history",
                delete(&[HazardWaiver::Dirty, HazardWaiver::OpenMerge]),
                clean_evidence(),
                HistoryAnswer::Unpreserved {
                    detail: "unique".to_owned(),
                },
            ),
        ];
        for (label, request, evidence, history) in cases {
            let (store, mut session) = ready();
            let mut ports = scripted(evidence, history);
            let failure = dispose(&request, &mut session, &mut ports)
                .err()
                .unwrap_or_else(|| panic!("{label} must refuse"));
            assert!(
                failure.effects.is_empty(),
                "{label} completed {:?} before refusing",
                failure.effects
            );
            assert_no_removal(&ports);
            assert_eq!(
                store.pointers().len(),
                1,
                "{label}: the clone pointer still stands"
            );
            assert_eq!(
                session.reread().unwrap().unwrap().members.len(),
                1,
                "{label}: the row still stands"
            );
        }
    }

    /// A repeated waiver is a malformed request, refused before any effect.
    #[test]
    fn a_repeated_waiver_refuses_before_any_effect() {
        let (_store, mut session) = ready();
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let request = delete(&[HazardWaiver::Dirty, HazardWaiver::Dirty]);
        let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
        assert!(matches!(
            failure.error,
            DisposeError::Refused(Refusal::InvalidRow { .. })
        ));
        assert!(ports.calls().is_empty());
    }
}
