//! The archive-equivalence mechanism, tier 1: byte-preservation by digest.
//!
//! `GwzM5-8R4bG-Evidence.md` §12.9(d)'s eight fixtured archive shapes, each
//! proved to be a byte-preserving archival of the terminal open v0 record that
//! preceded it. The instrument is the one the mechanism decision names: SHA256
//! of the archived record bytes against SHA256 of the open v0 record bytes,
//! which "is precisely what 'byte-preserving archival' asserts, it is a digest
//! comparison against the archived bytes".
//!
//! The per-shape record lives in `GwzM5-8I2CompatibilityPredicates.json`'s
//! `archive_corpus`, a **standalone** corpus and deliberately not a third
//! corpus of the migration registry: §12.7 records that "there is no registry
//! vocabulary in which an archive shape could be bound", and §12.9(c) that
//! widening `valid_unlisted_corpus` "would weaken the registry, not extend it".
//! The rows are therefore cited by clause, in the shape §12.9's disposition
//! table uses, and machine-validated by
//! `scripts/checks/check_merge_compatibility_predicates.py`.
//!
//! Clauses, content-anchored per the R2-E citing rule
//! (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` Appendix A):
//!
//! - **§5 "Migration eligibility and atomic boundary"**, "Completed and
//!   aborted v0 records remain v0 and use byte-preserving archival. Archived v0
//!   uses only the archive decoder/projection below." (`:178-184` as of
//!   2026-08-28) — the clause tier 1 discharges, per shape.
//! - **§7 "Archive decoder and GC rules"**, "Archive projection reads only the
//!   exact done-record bytes. It performs no Git, manifest, lock-file,
//!   member-repository, root, snapshot, candidate-destination, or worktree
//!   observation and never rewrites the archive." (`:214-250` as of
//!   2026-08-28) — why the archived bytes are the only authority a projection
//!   may read, and therefore why a digest over them is the whole property.
//!
//! **What tier 1 does NOT claim, and must not be read as claiming.** Tier 2 —
//! projection equivalence for archives an operation *finished under v1* — is
//! `owed` on every one of these rows, and the corpus says so per row rather
//! than in prose. `GwzM5-8R4bG-Evidence.md` §12.8's byte-equivalence verdict
//! therefore stays **PARTIAL** and must still not be cited as green. Two
//! further shapes, `AC-NOPUB-UNBORN` and `AP-PRESERVED`, are
//! DISPOSITIONED-PROJECTION-ONLY and UNFIXTURED: both their tiers are
//! `pending-fixture` with R2-F's fixtures/native-evidence lane named as
//! carrier, and nothing here reports the O8 archive clause met on them. The
//! `GwzM5-8R4bG-Evidence.md` §12.8 PARTIAL statement survives unchanged.
//!
//! **Eight rows, five distinct durable bases — stated so the count is not
//! over-read.** Byte-preservation is a property of the archival act, not of
//! what a fixture does to the archive afterwards, so the three rows whose
//! Table B fixtures are post-archive overlays share a base with the row they
//! were cut from: `AL-UNKNOWN` with `AC-CANDIDATE`, and `AL-OPTIONAL-MISSING`
//! and `AR-C` with `AC-NOPUB-BORN`. Each still gets its own workspace, its own
//! digest pair and its own corpus row — a regression in any one of them fails
//! here on its own name — but a reader counting distinct durable archive
//! fixtures should count five, not eight (E5 review [P2-2], 2026-08-28:
//! `CompletedCandidate` ×2, `CompletedNoPublication` ×3, and three distinct
//! aborted-fault bases; the first landing of this doc said six). The overlays those three rows are
//! named for are byte-pinned separately and already:
//! `characterization_archive_v0::archived_v0_optional_evidence_gaps_remain_readable_and_untouched`,
//! `::archived_v0_unknown_fields_and_raw_bytes_survive_status_and_retention`,
//! and `::archived_v0_missing_optional_evidence_is_not_an_unreadable_contradiction`
//! each assert the archive bytes are unchanged across status and retention.

use super::*;

/// A base terminal scenario, and how its archive is closed.
#[derive(Clone, Copy, Debug)]
enum Base {
    /// One-member workspace whose member has a source commit: finalization
    /// produces a full candidate publication and completes.
    CompletedCandidate,
    /// One-member workspace already up to date, root born: finalization takes
    /// the no-publication path and completes.
    CompletedNoPublication,
    /// The same, with the merge aborted from the named finalization window.
    Aborted(FinalizationFault),
}

/// One archive shape of `GwzM5-8R4bG-Evidence.md` §12.4 Table B that carries a
/// durable v0 fixture, with the `archive_corpus` subcase naming it.
#[derive(Clone, Copy, Debug)]
struct ArchiveRow {
    shape: &'static str,
    subcase: &'static str,
    base: Base,
}

/// The eight fixtured rows of Table B, in table order. `AC-NOPUB-UNBORN` and
/// `AP-PRESERVED` are absent by disposition, not by omission: they have no
/// durable archive fixture at all, their corpus rows are `pending-fixture`
/// with R2-F named, and the checker asserts that the pending pair is exactly
/// those two.
const ARCHIVE_ROWS: [ArchiveRow; 8] = [
    ArchiveRow {
        shape: "AC-CANDIDATE",
        subcase: "av0_b",
        base: Base::CompletedCandidate,
    },
    ArchiveRow {
        shape: "AC-NOPUB-BORN",
        subcase: "av0_c",
        base: Base::CompletedNoPublication,
    },
    ArchiveRow {
        shape: "AA-PREACCEPTANCE",
        subcase: "av0_e",
        base: Base::Aborted(FinalizationFault::AfterEnteringFinalizing),
    },
    ArchiveRow {
        shape: "AA-CANDIDATE-COMPLETE",
        subcase: "av0_f",
        base: Base::Aborted(FinalizationFault::AfterEvidencePersistence),
    },
    ArchiveRow {
        shape: "AA-CANDIDATE-PARTIAL",
        subcase: "av0_g",
        base: Base::Aborted(FinalizationFault::AfterCandidatePersistence),
    },
    ArchiveRow {
        shape: "AL-OPTIONAL-MISSING",
        subcase: "av0_d",
        base: Base::CompletedNoPublication,
    },
    ArchiveRow {
        shape: "AL-UNKNOWN",
        subcase: "unknown_retention",
        base: Base::CompletedCandidate,
    },
    ArchiveRow {
        shape: "AR-C",
        subcase: "ar_c",
        base: Base::CompletedNoPublication,
    },
];

const ARCHIVE_CORPUS_REGISTRY: &str =
    include_str!("../../../../dev-docs/GwzM5-8I2CompatibilityPredicates.json");

fn open_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(format!(".gwz/merge/{merge_id}.yaml"))
}

fn done_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(format!(".gwz/merge/done/{merge_id}.yaml"))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The `archive_corpus` row for `shape`, so this test asserts against the
/// frozen registry rather than a hand-copy of it.
fn archive_corpus_row(shape: &str) -> serde_yaml::Value {
    let registry: serde_yaml::Value = serde_yaml::from_str(ARCHIVE_CORPUS_REGISTRY).unwrap();
    registry
        .as_mapping()
        .unwrap()
        .get(serde_yaml::Value::String("archive_corpus".to_owned()))
        .unwrap()
        .as_sequence()
        .unwrap()
        .iter()
        .find(|row| row["shape"].as_str() == Some(shape))
        .unwrap_or_else(|| panic!("archive corpus is missing {shape:?}"))
        .clone()
}

struct Live {
    temp: TempDir,
    backend: crate::git::Git2Backend,
    _remote: RemoteFixture,
}

/// A workspace whose merge reaches `row`'s terminal state with the archive
/// still refused, so the open record that the archive must preserve is on
/// disk and readable.
fn terminal_before_archive(row: ArchiveRow) -> (Live, FaultingMergeStore, String, crate::MergeOp) {
    let label = format!("archive-tier1-{}", row.subcase);
    let temp = TempDir::new(&label);
    let backend = crate::git::Git2Backend::new();
    let _remote = init_one_member_workspace(temp.path(), &backend, &label);

    match row.base {
        Base::CompletedCandidate => {
            feature_commit(
                &backend,
                &temp.path().join("remote"),
                "README.md",
                "source\n",
            );
        }
        Base::CompletedNoPublication => {
            // R0 §5.2's `AC-NOPUB-BORN` is the born-baseline twin: "completed,
            // no candidate, exact born baseline fields". Its unborn twin is
            // `AC-NOPUB-UNBORN`, which has no fixture at all and is one of the
            // two PENDING-FIXTURE rows.
            backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
            commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
            backend
                .branch_create(&temp.path().join("remote"), "feature/source", "HEAD")
                .unwrap();
        }
        Base::Aborted(_) => {
            feature_commit(
                &backend,
                &temp.path().join("remote"),
                "README.md",
                "source\n",
            );
        }
    }

    let live = Live {
        temp,
        backend,
        _remote,
    };

    if let Base::Aborted(fault) = row.base {
        // Reach the named finalization window first, on its own store, so the
        // abort has the candidate/evidence evidence the shape is named for.
        let window = FaultingMergeStore::new(fault);
        invoke_with_store(
            &live.backend,
            &window,
            live.temp.path(),
            request(false),
            "op_archive_tier1_window",
        )
        .unwrap_err();
        let merge_id = window
            .discover_open(live.temp.path())
            .unwrap()
            .unwrap()
            .merge_id;
        let store = FaultingMergeStore::new(FinalizationFault::BeforeArchive);
        invoke_with_store(
            &live.backend,
            &store,
            live.temp.path(),
            recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
            "op_archive_tier1_abort",
        )
        .unwrap_err();
        return (live, store, merge_id, crate::MergeOp::Abort);
    }

    let store = FaultingMergeStore::new(FinalizationFault::BeforeArchive);
    invoke_with_store(
        &live.backend,
        &store,
        live.temp.path(),
        request(false),
        "op_archive_tier1_start",
    )
    .unwrap_err();
    let merge_id = store
        .discover_open(live.temp.path())
        .unwrap()
        .unwrap()
        .merge_id;
    (live, store, merge_id, crate::MergeOp::Resume)
}

/// The durable identity each Table B row claims, asserted on the archived
/// record itself so a row cannot pass its digest while having drifted into a
/// different archive shape.
fn assert_archive_shape(root: &Path, merge_id: &str, row: ArchiveRow) {
    let record = FileMergeStore.load_archived(root, merge_id).unwrap();
    let shape = row.shape;
    match row.base {
        Base::CompletedCandidate => {
            assert_eq!(record.state, OperationState::Completed, "{shape}");
            let publication = record.publication.unwrap();
            assert!(publication.candidate.is_some(), "{shape}");
            assert!(publication.composition_commit.is_some(), "{shape}");
        }
        Base::CompletedNoPublication => {
            assert_eq!(record.state, OperationState::Completed, "{shape}");
            assert!(record.baseline.root_head.is_some(), "{shape} born baseline");
            assert!(record.publication.unwrap().candidate.is_none(), "{shape}");
        }
        Base::Aborted(FinalizationFault::AfterEnteringFinalizing) => {
            assert_eq!(record.state, OperationState::Aborted, "{shape}");
            assert!(
                record
                    .publication
                    .is_none_or(|publication| publication.candidate.is_none()),
                "{shape}"
            );
        }
        Base::Aborted(FinalizationFault::AfterEvidencePersistence) => {
            assert_eq!(record.state, OperationState::Aborted, "{shape}");
            let publication = record.publication.unwrap();
            assert!(publication.candidate.is_some(), "{shape}");
            assert!(publication.composition_commit.is_some(), "{shape}");
            assert!(publication.evidence_rolled_back, "{shape}");
        }
        Base::Aborted(FinalizationFault::AfterCandidatePersistence) => {
            assert_eq!(record.state, OperationState::Aborted, "{shape}");
            let publication = record.publication.unwrap();
            assert!(publication.candidate.is_some(), "{shape}");
            assert!(publication.composition_commit.is_none(), "{shape}");
        }
        Base::Aborted(other) => panic!("{shape}: no archive shape is named for {other:?}"),
    }
}

/// **Tier 1 of the O8 archive-equivalence mechanism, executed per shape.**
///
/// For each of the eight fixtured Table B archive shapes: the terminal open v0
/// record is left on disk with the archive refused, its bytes are digested,
/// the close is then allowed to proceed, and the archived bytes are digested
/// against it. Equality is the contract's "byte-preserving archival", stated
/// per archive shape rather than per progress row, which is what O8's archive
/// clause has never had.
///
/// Each row additionally asserts its own `archive_corpus` entry — disposition,
/// tier-1 status, and the subcase binding back to this test — so the corpus
/// and its runtime binding cannot drift apart in either direction.
#[test]
fn archived_v0_shapes_are_byte_preserved_from_their_open_records() {
    for row in ARCHIVE_ROWS {
        let shape = row.shape;
        let corpus = archive_corpus_row(shape);
        assert_eq!(
            corpus["disposition"].as_str(),
            Some("byte-preserved-v0-origin"),
            "{shape}"
        );
        assert_eq!(
            corpus["tier1"]["status"].as_str(),
            Some("executed"),
            "{shape}"
        );
        assert_eq!(
            corpus["tier1"]["subcase"].as_str(),
            Some(row.subcase),
            "{shape}"
        );
        // Tier 2 is a different population and is owed, per row. Asserted here
        // so this test can never be read as reporting the whole O8 archive
        // clause met: `GwzM5-8R4bG-Evidence.md` §12.8 stays PARTIAL.
        assert_eq!(corpus["tier2"]["status"].as_str(), Some("owed"), "{shape}");
        assert_eq!(corpus["tier2"]["test"], serde_yaml::Value::Null, "{shape}");

        let (live, store, merge_id, close) = terminal_before_archive(row);
        let root = live.temp.path();
        let open = open_path(root, &merge_id);
        let done = done_path(root, &merge_id);
        assert!(open.is_file(), "{shape}: terminal open record is missing");
        assert!(
            !done.exists(),
            "{shape}: archived before the digest was taken"
        );
        let open_bytes = fs::read(&open).unwrap();
        let open_digest = digest(&open_bytes);

        let closed = invoke_with_store(
            &live.backend,
            &store,
            root,
            recovery_request(close, Some(merge_id.clone())),
            "op_archive_tier1_close",
        )
        .unwrap();
        assert!(!closed.open, "{shape}");
        assert!(
            !open.exists(),
            "{shape}: the open record outlived its archive"
        );

        let archived_bytes = fs::read(&done).unwrap();
        assert_eq!(
            digest(&archived_bytes),
            open_digest,
            "{shape}: archival is not byte-preserving"
        );
        assert_archive_shape(root, &merge_id, row);
    }
}

/// The corpus's own denominators, asserted from the runtime side so the
/// registry and this suite cannot disagree: ten Table B shapes, eight of them
/// tier-1 executed and bound to the test above, and exactly two
/// PENDING-FIXTURE — `AC-NOPUB-UNBORN` and `AP-PRESERVED`, the E0 §6.4 pair,
/// with a carrier named on both tiers and no test claimed on either.
///
/// This is the row-level guarantee that E5.2 does not report the O8 archive
/// clause met where it is not: a pending row carries no binding it has not
/// earned, and `GwzM5-8R4bG-Evidence.md` §12.8's PARTIAL statement survives
/// for it unchanged.
#[test]
fn archive_corpus_denominators_match_the_o8_archive_dispositions() {
    let registry: serde_yaml::Value = serde_yaml::from_str(ARCHIVE_CORPUS_REGISTRY).unwrap();
    let corpus = registry["archive_corpus"].as_sequence().unwrap();
    assert_eq!(corpus.len(), 10);

    let executed = corpus
        .iter()
        .filter(|row| row["tier1"]["status"].as_str() == Some("executed"))
        .collect::<Vec<_>>();
    assert_eq!(executed.len(), 8);
    for row in executed {
        assert_eq!(
            row["tier1"]["test"].as_str(),
            Some(concat!(
                "workspace_ops::tests::g23::archive_equivalence_v0::",
                "archived_v0_shapes_are_byte_preserved_from_their_open_records"
            ))
        );
    }

    let pending = corpus
        .iter()
        .filter(|row| row["disposition"].as_str() == Some("pending-fixture"))
        .map(|row| row["shape"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(pending, vec!["AC-NOPUB-UNBORN", "AP-PRESERVED"]);
    for shape in pending {
        let row = archive_corpus_row(shape);
        for tier in ["tier1", "tier2"] {
            assert_eq!(
                row[tier]["status"].as_str(),
                Some("pending-fixture"),
                "{shape} {tier}"
            );
            assert_eq!(row[tier]["test"], serde_yaml::Value::Null, "{shape} {tier}");
            assert!(
                row[tier]["carrier"]
                    .as_str()
                    .is_some_and(|carrier| carrier.contains("R2-F")),
                "{shape} {tier} must name R2-F as its carrier"
            );
        }
    }

    // Every row is recorded by clause, content-anchored, because the archive
    // family has no registry vocabulary to be bound by instead.
    for row in corpus {
        let clause = row["clause"].as_str().unwrap();
        assert!(clause.contains("GwzM5-8I2CompatibilityContract.md"));
        assert!(clause.contains('§'));
    }
}
