//! `gwz-history-check`: bounded, read-only, in-memory history verification
//! (lane H).
//!
//! [`check_history`] decides whether every protected root of a deletion
//! target is reachable, with a complete locally available object graph, from
//! the surviving witnesses' retained roots (gwz-dev
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §5, design §5.1).
//! It reads through an injected `gwz_repo_contract::ObjectReader`, memoizes
//! visits within one invocation, accounts for its own bookkeeping against
//! [`Limits`], polls its own [`Cancellation`] port between bounded units,
//! and persists nothing. A read failure, cancellation or exceeded limit is
//! [`HistoryOutcome::Unknown`], never `Verified`. Nothing here can delete.
//!
//! # What counts as preservation
//!
//! The verifier walks the witness object store from the witness's own
//! **eligible retained roots** (see [`is_eligible_witness_root`]) and
//! records, for every object it reaches, whether that object's entire
//! subgraph is locally available. A protected root is covered only when its
//! **exact object id** was reached that way and its subgraph is complete.
//! Consequently none of these covers a root, by construction:
//!
//! - a squash or rebase with the same patch or tree but a different commit
//!   id — the id is the evidence, not the content;
//! - a witness ref with the same *name* pointing at a different id;
//! - the object merely being present in the witness store with no eligible
//!   retained root reaching it (a `git gc` away from gone);
//! - cached origin/remote-tracking state in the *target*, which this
//!   library never reads: it only ever reads the witness side;
//! - a prior push to a network remote — nothing here fetches, so history
//!   that exists only on a server is not preserved locally.
//!
//! # Outcome precedence
//!
//! `Unknown` > `Unpreserved` > `Verified`. Anything the verifier could not
//! establish — an unreadable or oversized object, unsupported evidence, a
//! cancelled walk, a cap reached — is `Unknown` and stops the check before
//! any conclusion, because `Unknown` is not waivable downstream while
//! `Unpreserved` is. `ReadError::Missing` is the one read failure that is a
//! *conclusion*: the object is definitively not in the witness, so the root
//! that needs it is `Unpreserved` and the item names the missing id.
//!
//! # Roots this verifier does not interpret
//!
//! A protected root with `RootSource::Other` is evidence this library does
//! not interpret (an uninterpretable nested repository, an unsupported
//! record format); it yields `Unknown` with `UnknownKind::UnsupportedEvidence`
//! before any read. Callers that can name a root — a ref, `HEAD`, a reflog
//! entry, a stash entry, an annotated tag, and the Git objects a GWZ stash
//! coordination record references — must present it with that named source
//! so it can be verified.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use gwz_repo_contract::{
    ObjectId, ObjectReader, ProtectedRoot, ProtectedRoots, ReadError, ReadLimits, RepoKey,
    RootSource, UnknownKind, UnknownReason,
};

#[cfg(test)]
mod tests;

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

/// One surviving family repository eligible to preserve history, paired
/// with the target repository by identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Witness {
    pub repository: RepoKey,
    /// Human-readable location for diagnostics only.
    pub label: String,
}

/// Explicit resource bounds. Exceeding either returns `Unknown` before any
/// disposing row is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_roots: u64,
    pub max_bookkeeping_bytes: u64,
    /// Per-object bounds handed to the reader on every read.
    pub reads: ReadLimits,
}

impl Default for Limits {
    /// The architecture's initial cap: 100,000 roots and 256 MiB of
    /// verifier bookkeeping.
    fn default() -> Self {
        Self {
            max_roots: 100_000,
            max_bookkeeping_bytes: 256 * 1024 * 1024,
            reads: ReadLimits::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub roots_checked: u64,
    /// Objects successfully read from the witness during this invocation.
    /// Memoization makes this the number of *distinct* objects reached.
    pub objects_visited: u64,
    pub witnesses_used: Vec<RepoKey>,
    /// Peak bookkeeping bytes accounted during the walk.
    pub bookkeeping_bytes: u64,
    /// Which protected root is covered by which witness root, in the order
    /// the roots were presented.
    pub covered: Vec<RootCoverage>,
}

/// One protected root and the witness evidence that covers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootCoverage {
    /// The target's protected root.
    pub root: ProtectedRoot,
    /// The surviving repository that covers it, when the injected reader
    /// serves exactly one declared witness. `None` when several witnesses
    /// share one object access: this library then cannot attribute further
    /// than [`Coverage::witnesses_used`], and `witness_root` is still exact.
    pub witness: Option<RepoKey>,
    /// The eligible retained witness root the protected object hangs from.
    pub witness_root: ProtectedRoot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnpreservedItem {
    pub root: ProtectedRoot,
    /// The first object found missing from every witness, when the root
    /// itself was present somewhere.
    pub missing: Option<ObjectId>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryOutcome {
    Verified(Coverage),
    Unpreserved(Vec<UnpreservedItem>),
    Unknown(Vec<UnknownReason>),
}

impl HistoryOutcome {
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified(_))
    }
}

/// Whether a retained root of a *witness* may be used as a preservation
/// root (design §5.1, §6.2).
///
/// Eligible: ordinary persistent refs — including the retained
/// `refs/gwz/local-imports/<transfer-id>` refs, which are ordinary Git refs
/// that hold their objects against garbage collection — plus `HEAD`
/// (attached or detached) and annotated tag roots, which are named by
/// `refs/tags/...`.
///
/// Not eligible: temporary operation refs. That is every other `refs/gwz/`
/// namespace (`refs/gwz/merge/...` is the existing preservation/cleanup
/// namespace, not a durable witness), the pseudo-refs of unfinished native
/// operations, bisect and rewritten refs, and a witness's own reflog or
/// stash entries — all of these are operation state that ordinary Git
/// expiry or the next command may drop, so they cannot certify that
/// history survives a deletion.
pub fn is_eligible_witness_root(source: &RootSource) -> bool {
    match source {
        RootSource::Head | RootSource::AnnotatedTag { .. } => true,
        RootSource::Ref { name } => !is_temporary_operation_ref(name),
        RootSource::Reflog { .. } | RootSource::Stash { .. } | RootSource::Other { .. } => false,
    }
}

/// Import refs are durable; every other GWZ namespace and the native
/// operation pseudo-refs are not.
fn is_temporary_operation_ref(name: &str) -> bool {
    const OPERATION_PSEUDO_REFS: [&str; 8] = [
        "ORIG_HEAD",
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "REBASE_HEAD",
        "BISECT_HEAD",
        "FETCH_HEAD",
        "AUTO_MERGE",
    ];
    const OPERATION_NAMESPACES: [&str; 3] = ["refs/bisect/", "refs/rewritten/", "refs/gwz/"];

    if name.starts_with("refs/gwz/local-imports/") {
        return false;
    }
    OPERATION_PSEUDO_REFS.contains(&name)
        || OPERATION_NAMESPACES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// Check that every root in `protected` is preserved by `witnesses` through
/// `reader`.
///
/// `reader` is the witness-side object access: its `retained_roots` are the
/// candidate preservation roots and its `read_object` serves the witness
/// object store. `witnesses` names the surviving repositories that access
/// represents, already paired to this target by member/source identity by
/// the caller (root is paired explicitly as [`RepoKey::Root`]); different
/// targets are checked by separate calls with their own witnesses. An empty
/// `witnesses` slice preserves nothing.
pub fn check_history(
    protected: &ProtectedRoots,
    witnesses: &[Witness],
    reader: &dyn ObjectReader,
    limits: Limits,
    cancellation: &dyn Cancellation,
) -> HistoryOutcome {
    if cancellation.is_cancelled() {
        return HistoryOutcome::Unknown(vec![cancelled()]);
    }
    if protected.roots.len() as u64 > limits.max_roots {
        return HistoryOutcome::Unknown(vec![UnknownReason::new(
            UnknownKind::LimitExceeded,
            format!(
                "{} protected roots exceed the {} root limit",
                protected.roots.len(),
                limits.max_roots
            ),
        )]);
    }
    let unsupported: Vec<UnknownReason> = protected
        .roots
        .iter()
        .filter_map(|root| match &root.source {
            RootSource::Other { detail } => Some(UnknownReason::new(
                UnknownKind::UnsupportedEvidence,
                format!(
                    "protected root {} is evidence this verifier does not interpret: {detail}",
                    root.oid
                ),
            )),
            _ => None,
        })
        .collect();
    if !unsupported.is_empty() {
        return HistoryOutcome::Unknown(unsupported);
    }
    if protected.roots.is_empty() {
        return HistoryOutcome::Verified(Coverage::default());
    }
    if witnesses.is_empty() {
        return HistoryOutcome::Unpreserved(
            protected
                .roots
                .iter()
                .map(|root| UnpreservedItem {
                    root: root.clone(),
                    missing: None,
                    detail: format!(
                        "no surviving witness repository was offered for {}",
                        root.oid
                    ),
                })
                .collect(),
        );
    }

    let retained = match reader.retained_roots() {
        Ok(retained) => retained,
        Err(error) => return HistoryOutcome::Unknown(vec![read_reason(&error)]),
    };
    let eligible: Vec<ProtectedRoot> = retained
        .roots
        .into_iter()
        .filter(|root| is_eligible_witness_root(&root.source))
        .collect();
    if (protected.roots.len() + eligible.len()) as u64 > limits.max_roots {
        return HistoryOutcome::Unknown(vec![UnknownReason::new(
            UnknownKind::LimitExceeded,
            format!(
                "{} protected and {} witness roots exceed the {} root limit",
                protected.roots.len(),
                eligible.len(),
                limits.max_roots
            ),
        )]);
    }

    let mut verifier = Verifier::new(reader, cancellation, limits);
    if let Err(reason) = verifier.run(protected, &eligible) {
        return HistoryOutcome::Unknown(vec![reason]);
    }
    verifier.conclude(protected, witnesses, &eligible)
}

// ---------------------------------------------------------------- internals

/// Bytes charged for one entry of a map keyed by an object id, beyond the
/// key and value themselves: an allowance for the container's per-entry
/// node overhead. These are bounds on the verifier's own bookkeeping, not a
/// promise about total process memory.
const MAP_ENTRY_OVERHEAD: u64 = 48;

fn oid_bytes(oid: &ObjectId) -> u64 {
    (size_of::<ObjectId>() + oid.as_bytes().len()) as u64
}

fn root_bytes(root: &ProtectedRoot) -> u64 {
    let name = match &root.source {
        RootSource::Ref { name } | RootSource::AnnotatedTag { name } => name.len(),
        RootSource::Reflog { reference, .. } => reference.len(),
        RootSource::Other { detail } => detail.len(),
        RootSource::Head | RootSource::Stash { .. } => 0,
    };
    (size_of::<ProtectedRoot>() + name) as u64 + oid_bytes(&root.oid)
}

fn cancelled() -> UnknownReason {
    // `UnknownKind` has no `Cancelled` variant yet; a cancelled walk is
    // reported as the work bound it is, with an explicit detail. See the
    // lane C proposal in the landing report.
    UnknownReason::new(
        UnknownKind::LimitExceeded,
        "history verification was cancelled before it completed",
    )
}

fn read_reason(error: &ReadError) -> UnknownReason {
    let kind = match error {
        ReadError::LimitExceeded { .. } => UnknownKind::LimitExceeded,
        ReadError::Unimplemented { .. } => UnknownKind::Unimplemented,
        _ => UnknownKind::Unreadable,
    };
    UnknownReason::new(kind, error.to_string())
}

/// Whether an object's own subgraph is entirely available in the witness.
/// This is a property of the object, not of the root that reached it, so it
/// is memoized once per invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeState {
    InProgress,
    Complete,
    Incomplete,
}

struct Frame {
    oid: ObjectId,
    edges: Vec<ObjectId>,
    next: usize,
    complete: bool,
    missing: Option<ObjectId>,
    charged: u64,
}

/// The verifier's own bookkeeping bound. `used` rises and falls with the
/// live structures; `peak` is what the coverage reports.
struct Budget {
    used: u64,
    peak: u64,
    limit: u64,
}

impl Budget {
    fn new(limit: u64) -> Self {
        Self {
            used: 0,
            peak: 0,
            limit,
        }
    }

    fn charge(&mut self, bytes: u64) -> Result<(), UnknownReason> {
        self.used = self.used.saturating_add(bytes);
        self.peak = self.peak.max(self.used);
        if self.used > self.limit {
            return Err(UnknownReason::new(
                UnknownKind::LimitExceeded,
                format!(
                    "history verification bookkeeping reached {} bytes, above the {} byte limit",
                    self.used, self.limit
                ),
            ));
        }
        Ok(())
    }

    fn release(&mut self, bytes: u64) {
        self.used = self.used.saturating_sub(bytes);
    }
}

struct Verifier<'a> {
    reader: &'a dyn ObjectReader,
    cancellation: &'a dyn Cancellation,
    limits: Limits,
    budget: Budget,
    /// Memoized subgraph completeness, one entry per distinct object read.
    state: BTreeMap<ObjectId, NodeState>,
    /// For incomplete objects only: the first missing object beneath them.
    first_missing: BTreeMap<ObjectId, ObjectId>,
    /// The protected object ids, so origins are recorded only for those.
    targets: BTreeSet<ObjectId>,
    /// Protected object id -> index of the eligible witness root that first
    /// reached it.
    origin: BTreeMap<ObjectId, usize>,
    objects_visited: u64,
    current_root: usize,
}

impl<'a> Verifier<'a> {
    fn new(
        reader: &'a dyn ObjectReader,
        cancellation: &'a dyn Cancellation,
        limits: Limits,
    ) -> Self {
        Self {
            reader,
            cancellation,
            limits,
            budget: Budget::new(limits.max_bookkeeping_bytes),
            state: BTreeMap::new(),
            first_missing: BTreeMap::new(),
            targets: BTreeSet::new(),
            origin: BTreeMap::new(),
            objects_visited: 0,
            current_root: 0,
        }
    }

    fn run(
        &mut self,
        protected: &ProtectedRoots,
        eligible: &[ProtectedRoot],
    ) -> Result<(), UnknownReason> {
        for root in &protected.roots {
            self.budget
                .charge(oid_bytes(&root.oid) + MAP_ENTRY_OVERHEAD)?;
            self.targets.insert(root.oid.clone());
        }
        for root in eligible {
            self.budget.charge(root_bytes(root))?;
        }
        for (index, root) in eligible.iter().enumerate() {
            self.current_root = index;
            self.walk(&root.oid)?;
        }
        Ok(())
    }

    /// Depth-first over the witness graph from one eligible root, computing
    /// each object's subgraph completeness bottom-up. Objects already
    /// decided in this invocation are never read again.
    fn walk(&mut self, start: &ObjectId) -> Result<(), UnknownReason> {
        if self.state.contains_key(start) {
            return Ok(());
        }
        let Some(frame) = self.open(start)? else {
            return Ok(());
        };
        let mut stack = vec![frame];
        while let Some(top) = stack.len().checked_sub(1) {
            let next = stack[top].next;
            if next < stack[top].edges.len() {
                let child = stack[top].edges[next].clone();
                stack[top].next += 1;
                match self.state.get(&child).copied() {
                    // A cycle cannot occur in a Git object graph; if one is
                    // presented, the in-progress edge adds no new evidence.
                    Some(NodeState::Complete | NodeState::InProgress) => {}
                    Some(NodeState::Incomplete) => {
                        let missing = self.first_missing.get(&child).cloned();
                        Self::taint(&mut stack[top], missing.or(Some(child)));
                    }
                    None => match self.open(&child)? {
                        Some(frame) => stack.push(frame),
                        None => Self::taint(&mut stack[top], Some(child)),
                    },
                }
                continue;
            }
            let frame = stack.pop().expect("the stack is not empty here");
            self.budget.release(frame.charged);
            let state = if frame.complete {
                NodeState::Complete
            } else {
                NodeState::Incomplete
            };
            self.state.insert(frame.oid.clone(), state);
            if state == NodeState::Incomplete
                && let Some(missing) = frame.missing.clone()
            {
                self.budget
                    .charge(oid_bytes(&missing) + oid_bytes(&frame.oid) + MAP_ENTRY_OVERHEAD)?;
                self.first_missing.insert(frame.oid.clone(), missing);
            }
            if let Some(parent) = stack.last_mut()
                && state == NodeState::Incomplete
            {
                Self::taint(parent, frame.missing);
            }
        }
        Ok(())
    }

    fn taint(frame: &mut Frame, missing: Option<ObjectId>) {
        frame.complete = false;
        if frame.missing.is_none() {
            frame.missing = missing;
        }
    }

    /// Read one not-yet-decided object. `Ok(None)` means the witness does
    /// not have it: a conclusion, recorded as an incomplete node. Any other
    /// read failure is unknown and stops the whole check.
    fn open(&mut self, oid: &ObjectId) -> Result<Option<Frame>, UnknownReason> {
        if self.cancellation.is_cancelled() {
            return Err(cancelled());
        }
        self.budget
            .charge(oid_bytes(oid) + size_of::<NodeState>() as u64 + MAP_ENTRY_OVERHEAD)?;
        match self.reader.read_object(oid, &self.limits.reads) {
            Ok(record) => {
                self.objects_visited += 1;
                self.state.insert(oid.clone(), NodeState::InProgress);
                if self.targets.contains(oid) && !self.origin.contains_key(oid) {
                    self.budget
                        .charge(oid_bytes(oid) + size_of::<usize>() as u64 + MAP_ENTRY_OVERHEAD)?;
                    self.origin.insert(oid.clone(), self.current_root);
                }
                let charged =
                    (size_of::<Frame>() as u64) + record.edges.iter().map(oid_bytes).sum::<u64>();
                self.budget.charge(charged)?;
                Ok(Some(Frame {
                    oid: oid.clone(),
                    edges: record.edges,
                    next: 0,
                    complete: true,
                    missing: None,
                    charged,
                }))
            }
            Err(ReadError::Missing { .. }) => {
                self.state.insert(oid.clone(), NodeState::Incomplete);
                self.budget
                    .charge(2 * oid_bytes(oid) + MAP_ENTRY_OVERHEAD)?;
                self.first_missing.insert(oid.clone(), oid.clone());
                Ok(None)
            }
            Err(error) => Err(read_reason(&error)),
        }
    }

    fn conclude(
        self,
        protected: &ProtectedRoots,
        witnesses: &[Witness],
        eligible: &[ProtectedRoot],
    ) -> HistoryOutcome {
        // One declared witness behind the injected reader can be named
        // exactly; several sharing it cannot be told apart here.
        let attribution = match witnesses {
            [only] => Some(only.repository.clone()),
            _ => None,
        };
        let mut covered: Vec<RootCoverage> = Vec::new();
        let mut unpreserved: Vec<UnpreservedItem> = Vec::new();
        for root in &protected.roots {
            match self.state.get(&root.oid) {
                Some(NodeState::Complete) => {
                    let witness_root = self
                        .origin
                        .get(&root.oid)
                        .and_then(|index| eligible.get(*index))
                        .cloned();
                    match witness_root {
                        Some(witness_root) => covered.push(RootCoverage {
                            root: root.clone(),
                            witness: attribution.clone(),
                            witness_root,
                        }),
                        // Unreachable: a completed object was opened, and
                        // opening a protected id records its origin. Refuse
                        // rather than claim coverage without evidence.
                        None => unpreserved.push(UnpreservedItem {
                            root: root.clone(),
                            missing: None,
                            detail: format!(
                                "coverage of {} could not be attributed to a witness root",
                                root.oid
                            ),
                        }),
                    }
                }
                Some(NodeState::Incomplete | NodeState::InProgress) => {
                    unpreserved.push(UnpreservedItem {
                        root: root.clone(),
                        missing: self.first_missing.get(&root.oid).cloned(),
                        detail: format!(
                            "the witness graph below {} is incomplete; the history is not self-contained there",
                            root.oid
                        ),
                    });
                }
                None => unpreserved.push(UnpreservedItem {
                    root: root.clone(),
                    missing: None,
                    detail: format!("no eligible retained witness root reaches {}", root.oid),
                }),
            }
        }
        if !unpreserved.is_empty() {
            return HistoryOutcome::Unpreserved(unpreserved);
        }
        HistoryOutcome::Verified(Coverage {
            roots_checked: protected.roots.len() as u64,
            objects_visited: self.objects_visited,
            witnesses_used: witnesses
                .iter()
                .map(|witness| witness.repository.clone())
                .collect(),
            bookkeeping_bytes: self.budget.peak,
            covered,
        })
    }
}
