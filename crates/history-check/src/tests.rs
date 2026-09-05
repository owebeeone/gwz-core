//! Tier A tests for the bounded history verifier (architecture §5 test list).
//!
//! Everything is in memory: `gwz_repo_contract::contract_tests`'
//! `InMemoryObjectReader` graphs plus two tiny local doubles for the read
//! failures and the cancellation the shared fake cannot express. No test
//! touches a filesystem, a process or a real repository.

use std::cell::Cell;
use std::collections::BTreeSet;

use gwz_repo_contract::contract_tests::{InMemoryObjectReader, oid};
use gwz_repo_contract::{
    ObjectFormat, ObjectId, ObjectRecord, ProtectedRoot, ProtectedRoots, ReadError, ReadLimits,
    RepoKey, RootSource, UnknownKind,
};

use super::{
    Cancellation, Coverage, HistoryOutcome, Limits, NeverCancelled, RootCoverage, UnpreservedItem,
    Witness, check_history, is_eligible_witness_root,
};

const SHA1: ObjectFormat = ObjectFormat::Sha1;

// ---------------------------------------------------------------- fixtures

fn ref_source(name: &str) -> RootSource {
    RootSource::Ref {
        name: name.to_owned(),
    }
}

fn root(source: RootSource, id: ObjectId) -> ProtectedRoot {
    ProtectedRoot { source, oid: id }
}

fn roots(items: Vec<ProtectedRoot>) -> ProtectedRoots {
    ProtectedRoots { roots: items }
}

fn member(id: &str) -> Witness {
    Witness {
        repository: RepoKey::Member { id: id.to_owned() },
        label: format!("/workspaces/{id}"),
    }
}

fn root_witness() -> Witness {
    Witness {
        repository: RepoKey::Root,
        label: "/workspaces/root".to_owned(),
    }
}

/// `commit(base)` with `tree(base + 1)` holding `blob(base + 2)`.
fn commit(
    reader: &mut InMemoryObjectReader,
    format: ObjectFormat,
    base: u8,
    parents: Vec<ObjectId>,
) -> ObjectId {
    let blob = oid(format, base + 2);
    let tree = oid(format, base + 1);
    let head = oid(format, base);
    reader
        .blob(blob.clone(), 5)
        .tree(tree.clone(), vec![blob])
        .commit(head.clone(), tree, parents);
    head
}

/// A reader whose every object read fails, used for the unreadable and
/// unreadable-roots cases the in-memory graph cannot express.
struct BrokenReader {
    roots: Result<ProtectedRoots, ReadError>,
    error: ReadError,
    reads: Cell<u64>,
}

impl BrokenReader {
    fn reading(roots: ProtectedRoots, error: ReadError) -> Self {
        Self {
            roots: Ok(roots),
            error,
            reads: Cell::new(0),
        }
    }

    fn unreadable_roots() -> Self {
        Self {
            roots: Err(ReadError::ReadFailed {
                detail: "refs are unreadable".to_owned(),
            }),
            error: ReadError::ReadFailed {
                detail: "unused".to_owned(),
            },
            reads: Cell::new(0),
        }
    }
}

impl gwz_repo_contract::ObjectReader for BrokenReader {
    fn retained_roots(&self) -> Result<ProtectedRoots, ReadError> {
        self.roots.clone()
    }

    fn read_object(
        &self,
        _oid: &ObjectId,
        _limits: &ReadLimits,
    ) -> Result<ObjectRecord, ReadError> {
        self.reads.set(self.reads.get() + 1);
        Err(self.error.clone())
    }
}

/// Cancels once it has been polled more than `at` times.
struct CancelAfter {
    polls: Cell<u32>,
    at: u32,
}

impl CancelAfter {
    fn new(at: u32) -> Self {
        Self {
            polls: Cell::new(0),
            at,
        }
    }
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        let polls = self.polls.get() + 1;
        self.polls.set(polls);
        polls > self.at
    }
}

// -------------------------------------------------------------- invariants

/// Every outcome is checked against the invariants the design states: a
/// root is never both covered and uncovered, no root is named twice, only
/// the roots that were asked about are named, and `Verified` names all of
/// them. Roots are compared whole (source and oid), because two roots may
/// legitimately share one object id.
fn invariants(protected: &ProtectedRoots, outcome: &HistoryOutcome) {
    let (covered, uncovered) = named(outcome);
    for item in covered.iter().chain(uncovered.iter()) {
        assert!(
            protected.roots.contains(item),
            "{item:?} was never asked about"
        );
        let times = covered.iter().filter(|other| *other == item).count()
            + uncovered.iter().filter(|other| *other == item).count();
        assert_eq!(times, 1, "{item:?} is named more than once in {outcome:?}");
    }
    match outcome {
        HistoryOutcome::Verified(coverage) => {
            assert_eq!(
                covered.len(),
                protected.roots.len(),
                "Verified must name every protected root"
            );
            for asked in &protected.roots {
                assert!(covered.contains(asked), "{asked:?} is not named as covered");
            }
            assert_eq!(coverage.roots_checked, protected.roots.len() as u64);
        }
        HistoryOutcome::Unpreserved(items) => {
            assert!(!items.is_empty(), "Unpreserved names at least one root");
        }
        HistoryOutcome::Unknown(reasons) => {
            assert!(!reasons.is_empty(), "Unknown carries a reason");
        }
    }
}

fn named(outcome: &HistoryOutcome) -> (Vec<ProtectedRoot>, Vec<ProtectedRoot>) {
    match outcome {
        HistoryOutcome::Verified(coverage) => (
            coverage
                .covered
                .iter()
                .map(|entry| entry.root.clone())
                .collect(),
            Vec::new(),
        ),
        HistoryOutcome::Unpreserved(items) => (
            Vec::new(),
            items.iter().map(|item| item.root.clone()).collect(),
        ),
        HistoryOutcome::Unknown(_) => (Vec::new(), Vec::new()),
    }
}

/// The verifier is read-only: the fake records reads, and its roots are
/// unchanged after the call.
fn read_only(reader: &InMemoryObjectReader, before: &ProtectedRoots) {
    use gwz_repo_contract::ObjectReader;
    assert_eq!(
        &reader
            .retained_roots()
            .expect("witness roots stay readable"),
        before,
        "the verifier wrote to the witness"
    );
}

fn check(
    protected: &ProtectedRoots,
    witnesses: &[Witness],
    reader: &InMemoryObjectReader,
) -> HistoryOutcome {
    use gwz_repo_contract::ObjectReader;
    let before = reader.retained_roots().expect("witness roots are readable");
    let outcome = check_history(
        protected,
        witnesses,
        reader,
        Limits::default(),
        &NeverCancelled,
    );
    invariants(protected, &outcome);
    read_only(reader, &before);
    outcome
}

fn expect_verified(outcome: HistoryOutcome) -> Coverage {
    match outcome {
        HistoryOutcome::Verified(coverage) => coverage,
        other => panic!("expected Verified, got {other:?}"),
    }
}

fn expect_unpreserved(outcome: HistoryOutcome) -> Vec<UnpreservedItem> {
    match outcome {
        HistoryOutcome::Unpreserved(items) => items,
        other => panic!("expected Unpreserved, got {other:?}"),
    }
}

fn expect_unknown(outcome: HistoryOutcome) -> Vec<gwz_repo_contract::UnknownReason> {
    match outcome {
        HistoryOutcome::Unknown(reasons) => reasons,
        other => panic!("expected Unknown, got {other:?}"),
    }
}

// ------------------------------------------------------------------ tests

#[test]
fn a_clean_unique_commit_is_verified_against_a_witness_branch() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());
    let before = roots(vec![root(ref_source("refs/heads/main"), head.clone())]);

    let protected = roots(vec![
        root(ref_source("refs/heads/main"), head.clone()),
        root(RootSource::Head, head.clone()),
    ]);
    let outcome = check(&protected, &[member("lane-c")], &reader);
    let coverage = expect_verified(outcome);

    assert_eq!(coverage.roots_checked, 2);
    assert_eq!(
        coverage.objects_visited, 3,
        "commit, tree and blob are read"
    );
    assert_eq!(
        coverage.witnesses_used,
        vec![RepoKey::Member {
            id: "lane-c".to_owned()
        }]
    );
    assert!(coverage.bookkeeping_bytes > 0, "bookkeeping is accounted");
    let cover: &RootCoverage = &coverage.covered[0];
    assert_eq!(
        cover.witness,
        Some(RepoKey::Member {
            id: "lane-c".to_owned()
        })
    );
    assert_eq!(
        cover.witness_root.source,
        ref_source("refs/heads/main"),
        "coverage names the exact witness root"
    );
    assert_eq!(cover.witness_root.oid, head);
    read_only(&reader, &before);
}

#[test]
fn a_root_only_history_is_paired_to_the_root_repository_explicitly() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x20, Vec::new());
    reader.root(RootSource::Head, head.clone());

    let protected = roots(vec![root(RootSource::Head, head)]);
    let coverage = expect_verified(check(&protected, &[root_witness()], &reader));

    assert_eq!(coverage.covered[0].witness, Some(RepoKey::Root));
    assert_eq!(coverage.covered[0].witness_root.source, RootSource::Head);
    assert_eq!(coverage.witnesses_used, vec![RepoKey::Root]);
}

#[test]
fn a_secondary_branch_absent_from_the_witness_is_unpreserved() {
    let mut reader = InMemoryObjectReader::new();
    let main = commit(&mut reader, SHA1, 0x30, Vec::new());
    // The side branch exists only in the target: nothing in the witness.
    let side = oid(SHA1, 0x40);
    reader.root(ref_source("refs/heads/main"), main.clone());

    let protected = roots(vec![
        root(ref_source("refs/heads/main"), main),
        root(ref_source("refs/heads/side"), side.clone()),
    ]);
    let items = expect_unpreserved(check(&protected, &[member("hub")], &reader));

    assert_eq!(items.len(), 1, "only the uncovered branch is named");
    assert_eq!(items[0].root.source, ref_source("refs/heads/side"));
    assert_eq!(items[0].root.oid, side);
    assert_eq!(items[0].missing, None, "the root itself was never reached");
    assert!(
        items[0].detail.contains(&side.to_hex()),
        "the detail names the exact oid: {}",
        items[0].detail
    );
}

#[test]
fn a_reflog_only_commit_is_unpreserved_until_a_witness_ref_reaches_it() {
    let mut reader = InMemoryObjectReader::new();
    let base = commit(&mut reader, SHA1, 0x50, Vec::new());
    let amended = commit(&mut reader, SHA1, 0x60, vec![base.clone()]);
    reader.root(ref_source("refs/heads/main"), base.clone());

    let protected = roots(vec![
        root(ref_source("refs/heads/main"), base.clone()),
        root(
            RootSource::Reflog {
                reference: "refs/heads/main".to_owned(),
                index: 1,
            },
            amended.clone(),
        ),
    ]);
    let items = expect_unpreserved(check(&protected, &[member("hub")], &reader));
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].root.source,
        RootSource::Reflog {
            reference: "refs/heads/main".to_owned(),
            index: 1
        }
    );

    // The same reflog root is covered once a witness ref retains it.
    let mut retaining = reader.clone();
    retaining.root(ref_source("refs/gwz/local-imports/t1"), amended.clone());
    let coverage = expect_verified(check(&protected, &[member("hub")], &retaining));
    let reflog_cover = coverage
        .covered
        .iter()
        .find(|entry| entry.root.oid == amended)
        .expect("the reflog root is covered");
    assert_eq!(
        reflog_cover.witness_root.source,
        ref_source("refs/gwz/local-imports/t1")
    );
}

#[test]
fn an_annotated_tag_object_must_itself_be_covered() {
    let mut reader = InMemoryObjectReader::new();
    let tagged = commit(&mut reader, SHA1, 0x70, Vec::new());
    let tag = oid(SHA1, 0x80);
    reader.tag(tag.clone(), tagged.clone());
    // The witness retains the commit, but not the tag object.
    reader.root(ref_source("refs/heads/main"), tagged.clone());

    let protected = roots(vec![root(
        RootSource::AnnotatedTag {
            name: "v1".to_owned(),
        },
        tag.clone(),
    )]);
    let items = expect_unpreserved(check(&protected, &[member("hub")], &reader));
    assert_eq!(items[0].root.oid, tag);

    let mut with_tag_ref = reader.clone();
    with_tag_ref.root(ref_source("refs/tags/v1"), tag.clone());
    let coverage = expect_verified(check(&protected, &[member("hub")], &with_tag_ref));
    assert_eq!(
        coverage.covered[0].witness_root.source,
        ref_source("refs/tags/v1")
    );
    assert_eq!(
        coverage.objects_visited, 4,
        "the tag object and its target graph are read"
    );
}

#[test]
fn a_stash_stack_is_covered_only_when_every_entry_is_reachable() {
    let mut reader = InMemoryObjectReader::new();
    let base = commit(&mut reader, SHA1, 0x10, Vec::new());
    let newest = commit(&mut reader, SHA1, 0x20, vec![base.clone()]);
    let older = commit(&mut reader, SHA1, 0x30, vec![base.clone()]);
    let record = commit(&mut reader, SHA1, 0x40, vec![base.clone()]);
    // One import ref retains the newest stash and the coordination record's
    // referenced object, but nothing retains the older entry.
    let bundle = oid(SHA1, 0x50);
    reader.commit(
        bundle.clone(),
        oid(SHA1, 0x11),
        vec![newest.clone(), record.clone()],
    );
    reader.root(ref_source("refs/gwz/local-imports/t7"), bundle);

    let protected = roots(vec![
        root(RootSource::Stash { index: 0 }, newest),
        root(RootSource::Stash { index: 1 }, older.clone()),
        root(ref_source("refs/gwz/stash/record-1"), record),
    ]);
    let items = expect_unpreserved(check(&protected, &[member("hub")], &reader));

    assert_eq!(items.len(), 1, "only the older stash entry is uncovered");
    assert_eq!(items[0].root.source, RootSource::Stash { index: 1 });
    assert_eq!(items[0].root.oid, older);
}

#[test]
fn a_squash_with_the_same_patch_and_a_matching_ref_name_do_not_cover() {
    let mut reader = InMemoryObjectReader::new();
    let tree = oid(SHA1, 0x11);
    let blob = oid(SHA1, 0x12);
    reader.blob(blob.clone(), 5).tree(tree.clone(), vec![blob]);
    // The witness has a squash: same tree, different commit id, and the
    // very same ref name as the target's branch.
    let squash = oid(SHA1, 0x20);
    reader.commit(squash.clone(), tree.clone(), Vec::new());
    reader.root(ref_source("refs/heads/work"), squash);
    // The target's own commit id is not in the witness at all.
    let exact = oid(SHA1, 0x30);

    let protected = roots(vec![root(ref_source("refs/heads/work"), exact.clone())]);
    let items = expect_unpreserved(check(&protected, &[member("hub")], &reader));

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].root.oid, exact, "the exact oid is the evidence");
    assert_eq!(items[0].missing, None);
}

#[test]
fn object_presence_without_a_retained_witness_root_does_not_cover() {
    let mut reader = InMemoryObjectReader::new();
    let orphan = commit(&mut reader, SHA1, 0x10, Vec::new());
    let retained = commit(&mut reader, SHA1, 0x20, Vec::new());
    // The orphan's objects are in the witness store but no root retains them.
    reader.root(ref_source("refs/heads/main"), retained);

    let protected = roots(vec![root(ref_source("refs/heads/topic"), orphan.clone())]);
    let items = expect_unpreserved(check(&protected, &[member("hub")], &reader));

    assert_eq!(items[0].root.oid, orphan);
    assert!(
        items[0].detail.contains("no eligible"),
        "presence alone is not coverage: {}",
        items[0].detail
    );
}

#[test]
fn a_missing_object_in_the_witness_graph_leaves_the_root_unpreserved() {
    let mut reader = InMemoryObjectReader::new();
    let tree = oid(SHA1, 0x11);
    let absent_blob = oid(SHA1, 0x12);
    let head = oid(SHA1, 0x10);
    // The tree names a blob the witness does not have.
    reader
        .tree(tree.clone(), vec![absent_blob.clone()])
        .commit(head.clone(), tree, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());

    let protected = roots(vec![root(ref_source("refs/heads/main"), head.clone())]);
    let items = expect_unpreserved(check(&protected, &[member("hub")], &reader));

    assert_eq!(items[0].root.oid, head);
    assert_eq!(
        items[0].missing,
        Some(absent_blob),
        "the first missing object is named"
    );
}

#[test]
fn a_retained_import_ref_is_a_witness_root_and_a_temporary_merge_ref_is_not() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/gwz/merge/op-4/source"), head.clone());

    let protected = roots(vec![root(ref_source("refs/heads/main"), head.clone())]);
    let items = expect_unpreserved(check(&protected, &[member("hub")], &reader));
    assert_eq!(items[0].root.oid, head, "a temporary ref is not a witness");

    let mut imported = InMemoryObjectReader::new();
    let head = commit(&mut imported, SHA1, 0x10, Vec::new());
    imported.root(ref_source("refs/gwz/local-imports/t3"), head);
    let coverage = expect_verified(check(&protected, &[member("hub")], &imported));
    assert_eq!(
        coverage.covered[0].witness_root.source,
        ref_source("refs/gwz/local-imports/t3")
    );

    assert!(is_eligible_witness_root(&ref_source(
        "refs/gwz/local-imports/t3"
    )));
    assert!(!is_eligible_witness_root(&ref_source(
        "refs/gwz/merge/op-4/source"
    )));
    assert!(!is_eligible_witness_root(&ref_source("MERGE_HEAD")));
    assert!(!is_eligible_witness_root(&RootSource::Stash { index: 0 }));
    assert!(!is_eligible_witness_root(&RootSource::Reflog {
        reference: "refs/heads/main".to_owned(),
        index: 2
    }));
    assert!(is_eligible_witness_root(&RootSource::Head));
    assert!(is_eligible_witness_root(&ref_source("refs/heads/main")));
}

#[test]
fn a_bare_hub_covers_without_a_head_of_its_own() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/lane/agent-17"), head.clone());

    let protected = roots(vec![
        root(RootSource::Head, head.clone()),
        root(ref_source("refs/heads/lane/from-a"), head),
    ]);
    let coverage = expect_verified(check(&protected, &[member("hub")], &reader));
    assert_eq!(coverage.covered.len(), 2);
    assert!(
        coverage
            .covered
            .iter()
            .all(|entry| entry.witness_root.source == ref_source("refs/heads/lane/agent-17"))
    );
}

#[test]
fn unknown_nested_repository_evidence_is_unknown_before_any_read() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());

    let protected = roots(vec![
        root(ref_source("refs/heads/main"), head),
        root(
            RootSource::Other {
                detail: "nested repository at vendor/dep is not interpretable".to_owned(),
            },
            oid(SHA1, 0x90),
        ),
    ]);
    let reasons = expect_unknown(check(&protected, &[member("hub")], &reader));

    assert_eq!(reasons[0].kind, UnknownKind::UnsupportedEvidence);
    assert!(reasons[0].detail.contains("vendor/dep"));
    assert!(
        reader.reads().is_empty(),
        "unsupported evidence reads nothing"
    );
}

#[test]
fn the_root_cap_counts_the_witness_roots_too() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());
    reader.root(ref_source("refs/heads/other"), head.clone());

    let protected = roots(vec![root(ref_source("refs/heads/main"), head)]);
    let limits = Limits {
        max_roots: 2,
        ..Limits::default()
    };
    let outcome = check_history(
        &protected,
        &[member("hub")],
        &reader,
        limits,
        &NeverCancelled,
    );
    invariants(&protected, &outcome);

    let reasons = expect_unknown(outcome);
    assert_eq!(reasons[0].kind, UnknownKind::LimitExceeded);
    assert!(
        reasons[0].detail.contains("witness roots"),
        "the reason names both counts: {}",
        reasons[0].detail
    );
    assert!(
        reader.reads().is_empty(),
        "the cap is checked before reading"
    );
}

#[test]
fn a_reader_that_does_not_read_is_unknown_never_unpreserved() {
    let head = oid(SHA1, 0x10);
    let reader = BrokenReader::reading(
        roots(vec![root(ref_source("refs/heads/main"), head.clone())]),
        ReadError::Unimplemented {
            operation: "read_object",
        },
    );

    let protected = roots(vec![root(ref_source("refs/heads/main"), head)]);
    let outcome = check_history(
        &protected,
        &[member("hub")],
        &reader,
        Limits::default(),
        &NeverCancelled,
    );
    invariants(&protected, &outcome);

    let reasons = expect_unknown(outcome);
    assert_eq!(reasons[0].kind, UnknownKind::Unimplemented);
}

#[test]
fn different_targets_are_checked_against_their_own_witnesses() {
    // Two surviving repositories, each retaining a different history.
    let mut hub = InMemoryObjectReader::new();
    let hub_head = commit(&mut hub, SHA1, 0x10, Vec::new());
    hub.root(ref_source("refs/heads/main"), hub_head.clone());

    let mut workspace_root = InMemoryObjectReader::new();
    let root_head = commit(&mut workspace_root, SHA1, 0x40, Vec::new());
    workspace_root.root(ref_source("refs/heads/main"), root_head.clone());

    let app = roots(vec![root(ref_source("refs/heads/main"), hub_head)]);
    let shell = roots(vec![root(ref_source("refs/heads/main"), root_head)]);

    let app_coverage = expect_verified(check(&app, &[member("hub")], &hub));
    assert_eq!(
        app_coverage.covered[0].witness,
        Some(RepoKey::Member {
            id: "hub".to_owned()
        })
    );
    let shell_coverage = expect_verified(check(&shell, &[root_witness()], &workspace_root));
    assert_eq!(shell_coverage.covered[0].witness, Some(RepoKey::Root));

    // Nothing carries over between calls: each target needs its own witness.
    expect_unpreserved(check(&app, &[root_witness()], &workspace_root));
    expect_unpreserved(check(&shell, &[member("hub")], &hub));
}

#[test]
fn more_protected_roots_than_the_cap_is_unknown_before_any_read() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());

    let protected = roots(vec![
        root(ref_source("refs/heads/main"), head.clone()),
        root(ref_source("refs/heads/other"), head),
    ]);
    let limits = Limits {
        max_roots: 1,
        ..Limits::default()
    };
    let outcome = check_history(
        &protected,
        &[member("hub")],
        &reader,
        limits,
        &NeverCancelled,
    );
    invariants(&protected, &outcome);

    let reasons = expect_unknown(outcome);
    assert_eq!(reasons[0].kind, UnknownKind::LimitExceeded);
    assert!(
        reader.reads().is_empty(),
        "the cap is checked before reading"
    );
}

#[test]
fn exceeding_the_bookkeeping_cap_is_unknown_never_verified() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());

    let protected = roots(vec![root(ref_source("refs/heads/main"), head)]);
    let limits = Limits {
        max_bookkeeping_bytes: 64,
        ..Limits::default()
    };
    let outcome = check_history(
        &protected,
        &[member("hub")],
        &reader,
        limits,
        &NeverCancelled,
    );
    invariants(&protected, &outcome);

    let reasons = expect_unknown(outcome);
    assert_eq!(reasons[0].kind, UnknownKind::LimitExceeded);
    assert!(
        reasons[0].detail.contains("64"),
        "the limit is reported: {}",
        reasons[0].detail
    );
}

#[test]
fn cancellation_mid_walk_is_unknown_and_stops_reading() {
    let mut reader = InMemoryObjectReader::new();
    let first = commit(&mut reader, SHA1, 0x10, Vec::new());
    let second = commit(&mut reader, SHA1, 0x20, vec![first.clone()]);
    let third = commit(&mut reader, SHA1, 0x30, vec![second.clone()]);
    reader.root(ref_source("refs/heads/main"), third.clone());

    let protected = roots(vec![root(ref_source("refs/heads/main"), third)]);
    let outcome = check_history(
        &protected,
        &[member("hub")],
        &reader,
        Limits::default(),
        &CancelAfter::new(3),
    );
    invariants(&protected, &outcome);

    let reasons = expect_unknown(outcome);
    assert_eq!(
        reasons[0].kind,
        UnknownKind::Cancelled,
        "a cancelled walk is its own kind (H1), not an exhausted bound"
    );
    assert!(
        reasons[0].detail.contains("cancel"),
        "the reason says it was cancelled: {}",
        reasons[0].detail
    );
    let reads = reader.reads().len();
    assert!(reads > 0, "the walk had started");
    assert!(reads < 9, "the walk stopped early, read {reads} objects");
}

#[test]
fn cancellation_before_any_work_is_unknown_without_reading() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());
    let protected = roots(vec![root(ref_source("refs/heads/main"), head)]);

    let outcome = check_history(
        &protected,
        &[member("hub")],
        &reader,
        Limits::default(),
        &CancelAfter::new(0),
    );
    invariants(&protected, &outcome);
    let reasons = expect_unknown(outcome);
    assert_eq!(reasons[0].kind, UnknownKind::Cancelled);
    assert!(reader.reads().is_empty());
}

/// H3 (LCM1.0c follow-up 2): a Git object a GWZ coordination record
/// references is a *named* protected root, so it is verified against the
/// witnesses like any other root -- covered when its exact id is reachable
/// whole, unpreserved when it is not -- and never `Unknown` as
/// uninterpreted evidence. As a witness root it is operation state and is
/// not eligible.
#[test]
fn a_coordination_record_object_is_a_named_root_that_is_verified_or_unpreserved() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());
    let stash_source = RootSource::CoordinationRecord {
        record: "stash gwz_stash_0007".to_owned(),
        object: "worktree".to_owned(),
    };

    let held = roots(vec![root(stash_source.clone(), head.clone())]);
    let coverage = expect_verified(check(&held, &[member("hub")], &reader));
    assert_eq!(coverage.covered.len(), 1);
    assert_eq!(coverage.covered[0].root.source, stash_source);
    assert_eq!(
        coverage.covered[0].witness_root.source,
        ref_source("refs/heads/main")
    );

    let gone = roots(vec![root(stash_source.clone(), oid(SHA1, 0x90))]);
    let items = expect_unpreserved(check(&gone, &[member("hub")], &reader));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].root.source, stash_source);

    assert!(
        !is_eligible_witness_root(&stash_source),
        "a record's objects are operation state, not a durable witness root"
    );
}

#[test]
fn a_shared_subgraph_is_read_once() {
    let mut reader = InMemoryObjectReader::new();
    // A diamond: merge -> {left, right} -> base, all sharing one tree.
    let tree = oid(SHA1, 0x11);
    let blob = oid(SHA1, 0x12);
    reader.blob(blob.clone(), 5).tree(tree.clone(), vec![blob]);
    let base = oid(SHA1, 0x20);
    let left = oid(SHA1, 0x30);
    let right = oid(SHA1, 0x40);
    let merge = oid(SHA1, 0x50);
    reader
        .commit(base.clone(), tree.clone(), Vec::new())
        .commit(left.clone(), tree.clone(), vec![base.clone()])
        .commit(right.clone(), tree.clone(), vec![base.clone()])
        .commit(
            merge.clone(),
            tree.clone(),
            vec![left.clone(), right.clone()],
        );
    reader.root(ref_source("refs/heads/main"), merge.clone());
    reader.root(ref_source("refs/heads/left"), left.clone());

    let protected = roots(vec![
        root(ref_source("refs/heads/main"), merge),
        root(ref_source("refs/heads/left"), left),
        root(ref_source("refs/heads/base"), base),
    ]);
    let coverage = expect_verified(check(&protected, &[member("hub")], &reader));

    let reads = reader.reads();
    let distinct: BTreeSet<ObjectId> = reads.iter().cloned().collect();
    assert_eq!(
        reads.len(),
        distinct.len(),
        "every object is read once: {reads:?}"
    );
    assert_eq!(distinct.len(), 6, "tree, blob and four commits");
    assert_eq!(coverage.objects_visited, 6);
}

#[test]
fn no_surviving_witness_leaves_every_root_unpreserved() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());

    let protected = roots(vec![root(ref_source("refs/heads/main"), head)]);
    let items = expect_unpreserved(check(&protected, &[], &reader));

    assert_eq!(items.len(), 1);
    assert!(
        items[0].detail.contains("witness"),
        "the detail says there is no witness: {}",
        items[0].detail
    );
    assert!(reader.reads().is_empty(), "no witness means no read");
}

#[test]
fn a_witness_with_no_eligible_retained_root_preserves_nothing() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(RootSource::Stash { index: 0 }, head.clone());

    let protected = roots(vec![root(ref_source("refs/heads/main"), head)]);
    let items = expect_unpreserved(check(&protected, &[member("hub")], &reader));
    assert_eq!(items.len(), 1);
}

#[test]
fn an_oversized_object_is_unknown_never_verified() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());

    let protected = roots(vec![root(ref_source("refs/heads/main"), head)]);
    let limits = Limits {
        reads: ReadLimits::new(10),
        ..Limits::default()
    };
    let outcome = check_history(
        &protected,
        &[member("hub")],
        &reader,
        limits,
        &NeverCancelled,
    );
    invariants(&protected, &outcome);

    let reasons = expect_unknown(outcome);
    assert_eq!(reasons[0].kind, UnknownKind::LimitExceeded);
    assert!(reasons[0].detail.contains("exceeds the read limit"));
}

#[test]
fn an_unreadable_object_is_unknown_never_unpreserved() {
    let head = oid(SHA1, 0x10);
    let reader = BrokenReader::reading(
        roots(vec![root(ref_source("refs/heads/main"), head.clone())]),
        ReadError::Corrupt {
            oid: head.clone(),
            detail: "truncated loose object".to_owned(),
        },
    );

    let protected = roots(vec![root(ref_source("refs/heads/main"), head)]);
    let outcome = check_history(
        &protected,
        &[member("hub")],
        &reader,
        Limits::default(),
        &NeverCancelled,
    );
    invariants(&protected, &outcome);

    let reasons = expect_unknown(outcome);
    assert_eq!(reasons[0].kind, UnknownKind::Unreadable);
    assert!(reasons[0].detail.contains("truncated loose object"));
    assert_eq!(reader.reads.get(), 1);
}

#[test]
fn unreadable_witness_roots_are_unknown() {
    let reader = BrokenReader::unreadable_roots();
    let protected = roots(vec![root(ref_source("refs/heads/main"), oid(SHA1, 0x10))]);

    let outcome = check_history(
        &protected,
        &[member("hub")],
        &reader,
        Limits::default(),
        &NeverCancelled,
    );
    invariants(&protected, &outcome);

    let reasons = expect_unknown(outcome);
    assert_eq!(reasons[0].kind, UnknownKind::Unreadable);
    assert_eq!(reader.reads.get(), 0, "no object read was attempted");
}

#[test]
fn an_empty_protected_inventory_is_verified_without_reading() {
    let reader = InMemoryObjectReader::new();
    let protected = ProtectedRoots::default();

    let coverage = expect_verified(check(&protected, &[member("hub")], &reader));
    assert_eq!(coverage.roots_checked, 0);
    assert!(coverage.covered.is_empty());
    assert!(coverage.witnesses_used.is_empty());
    assert!(reader.reads().is_empty());
}

#[test]
fn several_witnesses_sharing_one_reader_are_not_attributed_to_one_repository() {
    let mut reader = InMemoryObjectReader::new();
    let head = commit(&mut reader, SHA1, 0x10, Vec::new());
    reader.root(ref_source("refs/heads/main"), head.clone());

    let protected = roots(vec![root(ref_source("refs/heads/main"), head)]);
    let witnesses = [root_witness(), member("lane-a")];
    let coverage = expect_verified(check(&protected, &witnesses, &reader));

    assert_eq!(coverage.covered[0].witness, None);
    assert_eq!(
        coverage.witnesses_used,
        vec![
            RepoKey::Root,
            RepoKey::Member {
                id: "lane-a".to_owned()
            }
        ]
    );
}

#[test]
fn both_object_formats_verify_the_same_shape() {
    for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
        let mut reader = InMemoryObjectReader::new();
        let head = commit(&mut reader, format, 0x10, Vec::new());
        reader.root(ref_source("refs/heads/main"), head.clone());

        let protected = roots(vec![root(ref_source("refs/heads/main"), head.clone())]);
        let coverage = expect_verified(check(&protected, &[member("hub")], &reader));
        assert_eq!(coverage.covered[0].root.oid.format(), format);

        let absent = roots(vec![root(ref_source("refs/heads/gone"), oid(format, 0xEE))]);
        let items = expect_unpreserved(check(&absent, &[member("hub")], &reader));
        assert_eq!(items[0].root.oid.format(), format);
    }
}

#[test]
fn the_default_limits_are_the_architecture_caps() {
    let limits = Limits::default();
    assert_eq!(limits.max_roots, 100_000);
    assert_eq!(limits.max_bookkeeping_bytes, 256 * 1024 * 1024);
    assert_eq!(limits.reads, ReadLimits::default());
    assert!(!HistoryOutcome::Unknown(Vec::new()).is_verified());
    assert!(!NeverCancelled.is_cancelled());
}
