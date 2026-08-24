//! Adaptation dispositions for the four live M4 residue rows (C-1).
//!
//! `GwzM5-8R4bG-Evidence.md` §12.7 names four UNBOUND rows that the frozen
//! contract's class cite does not settle on its own: the three `Finalizing`
//! mid-publication-prefix shapes `F-BASELINE`, `F-MARKER`, `F-LOCK`, and
//! `J-NO-PUBLICATION-UNBORN`. Every one of them is inside `Finalizing`, the
//! only operation state the A1 whitelist adapts, so "not a migration rule"
//! has to be *shown* rather than asserted by class.
//!
//! The dispositions asserted here are read off `GwzM5-8I2CompatibilityContract.md`:
//!
//! - `:117-123` whitelists seven shapes, the publication-era one being the
//!   "exact boundary/index publication prefix", and requires that "every
//!   selected result, baseline byte/hash, candidate/composition relation,
//!   participant HEAD/ref/index/worktree, and root observation is exact";
//! - `:123-125` excludes, by name, "marker/lock-only prefixes … born root …
//!   and terminal rows";
//! - `:159` "Zero whitelist matches is not an error", with the bytes left
//!   unchanged;
//! - `:167-169` "an unreadable/contradictory row … rejects before staging".
//!
//! The mid-prefix rows split on that boundary and the split is the point: the
//! baseline prefix produces a well-formed descriptor that simply matches no
//! rule (valid-unlisted), while the marker-only and lock-only prefixes are not
//! an exact index-aligned observation at all and refuse typed. Each case is
//! driven through the real `decode -> adapt -> upgrade` path, and the
//! published boundary prefix rides along as the positive control that keeps
//! the discrimination non-vacuous.

use serde_yaml::Value;

use super::*;

/// Fails the *next* durable write that records `PublishingCandidate` with
/// evidence, but only after it lands. That leaves the one durable shape no
/// injected publication mutation can leave — step `PublishingCandidate` with
/// the live root still at the pre-publication baseline prefix (`F-BASELINE`) —
/// exactly as a crash between the step write and the first artifact write
/// would. Same shape as `characterization_publication_v0`'s post-write store.
#[derive(Default)]
struct PublishingBaselinePostWriteStore {
    fired: Cell<bool>,
}

impl MergeStore for PublishingBaselinePostWriteStore {
    fn discover_open(&self, root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        FileMergeStore.discover_open(root)
    }

    fn load(&self, root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
        FileMergeStore.load(root, merge_id)
    }

    fn write_open(&self, root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
        FileMergeStore.write_open(root, record)?;
        let publishing = record.publication.as_ref().is_some_and(|progress| {
            progress.step == PublicationStep::PublishingCandidate
                && progress.composition_commit.is_some()
        });
        if !self.fired.get() && publishing {
            self.fired.set(true);
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "injected post-write publishing-candidate failure",
            ));
        }
        Ok(())
    }

    fn archive(&self, root: &Path, merge_id: &str) -> ModelResult<()> {
        FileMergeStore.archive(root, merge_id)
    }
}

struct Fixture {
    temp: TempDir,
    backend: crate::git::Git2Backend,
    _remote: RemoteFixture,
}

/// A one-member workspace whose member has a source commit to fast-forward to,
/// so finalization produces a publication candidate.
fn changed_member_fixture(label: &str) -> Fixture {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let _remote = init_one_member_workspace(temp.path(), &backend, label);
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    Fixture {
        temp,
        backend,
        _remote,
    }
}

/// A one-member workspace whose member is already up to date, so finalization
/// takes the no-publication path, with the root born or unborn as asked.
fn unchanged_member_fixture(label: &str, born: bool) -> Fixture {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let _remote = init_one_member_workspace(temp.path(), &backend, label);
    if born {
        backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
        commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    }
    backend
        .branch_create(&temp.path().join("remote"), "feature/source", "HEAD")
        .unwrap();
    Fixture {
        temp,
        backend,
        _remote,
    }
}

fn adapt<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<crate::workspace_ops::merge::OpenV0Adaptation> {
    let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
        serde_yaml::to_string(record).unwrap().as_bytes(),
    )
    .unwrap();
    crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
        backend,
        root,
        &decoded,
        "r3-test-writer",
    )
}

/// `(rule_id, next_action)` when the adapter accepts the row, `None` when it
/// answers `ValidUnlisted`. Refusals stay `unwrap_err`-shaped at the call site.
fn adapted_rule<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> Option<(String, String)> {
    match adapt(backend, root, record).unwrap() {
        crate::workspace_ops::merge::OpenV0Adaptation::ValidUnlisted => None,
        crate::workspace_ops::merge::OpenV0Adaptation::Eligible {
            rule_id,
            next_action,
            ..
        } => Some((rule_id, next_action)),
    }
}

fn assert_live_prefix(root: &Path, record: &MergeOperationRecord, prefix: &str, label: &str) {
    let candidate = record
        .publication
        .as_ref()
        .and_then(|progress| progress.candidate.as_ref())
        .unwrap();
    let marker = crate::artifact::marker_path(root, &candidate.marker_id);
    assert_eq!(marker.is_file(), prefix != "baseline", "{label} marker");
    assert_eq!(
        fs::read_to_string(root.join(crate::artifact::LOCK_PATH)).unwrap(),
        if matches!(prefix, "baseline" | "marker") {
            candidate.baseline_lock_yaml.as_str()
        } else {
            candidate.lock_yaml.as_str()
        },
        "{label} lock"
    );
    assert_eq!(
        fs::read_to_string(crate::workspace_ops::workspace_exclude_path(root)).unwrap(),
        if prefix == "boundary" {
            candidate.boundary_text.as_str()
        } else {
            candidate.baseline_boundary_text.as_str()
        },
        "{label} boundary"
    );
}

fn assert_publishing_candidate_record(record: &MergeOperationRecord, label: &str) {
    assert_eq!(record.state, OperationState::Finalizing, "{label}");
    let publication = record.publication.as_ref().unwrap();
    assert_eq!(
        publication.step,
        PublicationStep::PublishingCandidate,
        "{label}"
    );
    assert!(publication.candidate.is_some(), "{label}");
    assert!(publication.composition_commit.is_some(), "{label}");
    assert!(publication.composition_tree.is_some(), "{label}");
    assert!(!publication.candidate_hashes.is_empty(), "{label}");
}

/// `F-BASELINE`, `F-MARKER`, `F-LOCK` — the three `Finalizing` mid-prefix M4
/// shapes — are not A1 migration rules, and the two ways the adapter says so
/// are pinned per shape. The published boundary prefix is the positive
/// control: one durable record shape, five live prefixes, exactly one adapted.
#[test]
fn v0_mid_publication_prefixes_are_not_a1_migration_rules() {
    use crate::workspace_ops::merge::{
        CandidatePublicationMutation, fail_next_candidate_publication_after,
    };

    let boundary_rule =
        super::compatibility_v0::i2_whitelist_rule("candidate-published-before-recording");
    let boundary_descriptor = &boundary_rule["descriptor"];
    let prefix_reason = super::compatibility_v0::i2_rejection_reason("PublicationPrefixMismatch");
    assert!(!prefix_reason.is_empty());

    // F-BASELINE: the durable step advance landed; nothing is published yet.
    let Fixture { temp, backend, .. } = changed_member_fixture("v0-residue-f-baseline");
    let store = PublishingBaselinePostWriteStore::default();
    let clock = FixedClock::new(TimestampMs(1_700_000_000_000));
    let mut ids = SequentialIdProvider::new();
    let error = handle_merge_with_dependencies(
        MergeDependencies {
            backend: &backend,
            store: &store,
            clock: &clock,
            ids: &mut ids,
            events: &crate::operation::NullSink,
        },
        temp.path(),
        request(false),
        "op_v0_residue_f_baseline",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(store.fired.get());
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    assert_publishing_candidate_record(&record, "F-BASELINE");
    assert_live_prefix(temp.path(), &record, "baseline", "F-BASELINE");

    // The durable half of the descriptor is byte-identical to the whitelisted
    // boundary rule's: only the live root observation separates them. That is
    // precisely the coincidence C-1 asks to be pinned, so it is asserted in
    // both directions.
    let descriptor =
        crate::workspace_ops::merge::verified_v0_descriptor(&backend, temp.path(), &record)
            .unwrap();
    assert_eq!(
        descriptor.value()["publication"],
        boundary_descriptor["publication"]
    );
    assert_eq!(
        descriptor.value()["observation"]["root"],
        Value::String("recorded_evidence".to_owned())
    );
    assert_eq!(
        boundary_descriptor["observation"]["root"],
        Value::String("prefix_boundary".to_owned())
    );
    assert_ne!(descriptor.value(), boundary_descriptor);
    assert_eq!(
        super::compatibility_v0::i2_whitelist_matches(descriptor.value()),
        Vec::<String>::new()
    );
    assert_eq!(adapted_rule(&backend, temp.path(), &record), None);
    super::atomic_upgrade_v0::assert_valid_unlisted(&backend, temp.path(), &record);

    // The forward-interrupted prefixes. Marker-only and lock-only are the two
    // the contract excludes by name; the unstaged boundary prefix rides with
    // them because the index has not been published either.
    for (label, mutation, prefix) in [
        ("F-MARKER", CandidatePublicationMutation::Marker, "marker"),
        ("F-LOCK", CandidatePublicationMutation::Lock, "lock"),
        (
            "F-BOUNDARY-UNSTAGED",
            CandidatePublicationMutation::Boundary,
            "boundary",
        ),
    ] {
        let Fixture { temp, backend, .. } = changed_member_fixture(&format!("v0-residue-{label}"));
        fail_next_candidate_publication_after(mutation);
        let error = handle_merge(
            &backend,
            temp.path(),
            request(false),
            format!("op_v0_residue_{label}"),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired, "{label}");
        let record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
        assert_publishing_candidate_record(&record, label);
        assert_live_prefix(temp.path(), &record, prefix, label);

        // No descriptor exists to match: the observation is not one exact
        // index-aligned publication prefix, so the row refuses before staging.
        assert_eq!(
            crate::workspace_ops::merge::verified_v0_descriptor(&backend, temp.path(), &record)
                .unwrap_err()
                .code,
            ErrorCode::PublicationPrefixMismatch,
            "{label}"
        );
        let error = adapt(&backend, temp.path(), &record).unwrap_err();
        assert_eq!(error.code, ErrorCode::PublicationPrefixMismatch, "{label}");
        assert!(error.message.contains(&prefix_reason), "{label}");
        let message = super::atomic_upgrade_v0::assert_typed_refusal(
            &backend,
            temp.path(),
            &record,
            ErrorCode::PublicationPrefixMismatch,
        );
        assert!(message.contains(&prefix_reason), "{label}");
    }

    // Positive control: the same durable record with the complete published
    // prefix, index included, is the whitelisted rule.
    let Fixture { temp, backend, .. } = changed_member_fixture("v0-residue-f-boundary");
    let store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_v0_residue_f_boundary",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    assert_publishing_candidate_record(&record, "F-BOUNDARY");
    assert_live_prefix(temp.path(), &record, "boundary", "F-BOUNDARY");
    let descriptor =
        crate::workspace_ops::merge::verified_v0_descriptor(&backend, temp.path(), &record)
            .unwrap();
    assert_eq!(descriptor.value(), boundary_descriptor);
    assert_eq!(
        adapted_rule(&backend, temp.path(), &record),
        Some((
            "candidate-published-before-recording".to_owned(),
            "publish_candidate".to_owned()
        ))
    );
}

/// `J-NO-PUBLICATION-UNBORN` at the pre-terminal `Finalizing` window: the
/// unborn twin *is* the whitelisted no-publication rule, per case and by
/// descriptor equality, and its born twin is excluded by the contract's named
/// "born root" class. This is the row whose class membership §12.7 records as
/// ambiguous; the ambiguity is that the two twins take opposite branches.
#[test]
fn v0_no_publication_finalizing_twins_split_on_the_born_root_exclusion() {
    let rule =
        super::compatibility_v0::i2_whitelist_rule("no-publication-complete-before-terminal");

    for born in [false, true] {
        let kind = if born { "born" } else { "unborn" };
        let Fixture { temp, backend, .. } =
            unchanged_member_fixture(&format!("v0-residue-nopub-{kind}"), born);
        let store = FaultingMergeStore::new(FinalizationFault::AfterNoPublicationComplete);
        let error = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            request(false),
            "op_v0_residue_nopub",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired, "{kind}");
        let record = store.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(record.state, OperationState::Finalizing, "{kind}");
        let publication = record.publication.as_ref().unwrap();
        assert_eq!(publication.step, PublicationStep::Complete, "{kind}");
        assert!(publication.candidate.is_none(), "{kind}");
        assert_eq!(record.baseline.root_head.is_some(), born, "{kind}");

        if born {
            // Contract `:123`: "born root … are not A1 migration rules". The
            // record never reaches the descriptor: it is classified out first,
            // so zero matches, bytes unchanged.
            assert_eq!(
                crate::workspace_ops::merge::verified_v0_descriptor(&backend, temp.path(), &record)
                    .unwrap_err()
                    .code,
                ErrorCode::AcceptanceInputDrift,
                "{kind}"
            );
            assert_eq!(adapted_rule(&backend, temp.path(), &record), None, "{kind}");
            super::atomic_upgrade_v0::assert_valid_unlisted(&backend, temp.path(), &record);
            continue;
        }

        let descriptor =
            crate::workspace_ops::merge::verified_v0_descriptor(&backend, temp.path(), &record)
                .unwrap();
        assert_eq!(descriptor.value(), &rule["descriptor"], "{kind}");
        assert_eq!(
            super::compatibility_v0::i2_whitelist_matches(descriptor.value()),
            vec!["no-publication-complete-before-terminal".to_owned()],
            "{kind}"
        );
        assert_eq!(
            adapted_rule(&backend, temp.path(), &record),
            Some((
                "no-publication-complete-before-terminal".to_owned(),
                "complete_no_publication".to_owned()
            )),
            "{kind}"
        );
    }
}

/// `J-NO-PUBLICATION-UNBORN` in R0 §4 row J's own terminal shape — operation
/// `Completed`, publication `Complete`, no candidate — for both root twins.
/// Contract `:161-163`: "Completed and aborted v0 records remain v0 and use
/// byte-preserving archival."
#[test]
fn v0_no_publication_terminal_twins_stay_v0_for_born_and_unborn_roots() {
    for born in [false, true] {
        let kind = if born { "born" } else { "unborn" };
        let Fixture { temp, backend, .. } =
            unchanged_member_fixture(&format!("v0-residue-nopub-terminal-{kind}"), born);
        let root_before = backend.head(temp.path()).unwrap();
        let store = FaultingMergeStore::new(FinalizationFault::BeforeArchive);
        invoke_with_store(
            &backend,
            &store,
            temp.path(),
            request(false),
            "op_v0_residue_nopub_terminal",
        )
        .unwrap_err();
        let record = store.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(record.state, OperationState::Completed, "{kind}");
        let publication = record.publication.as_ref().unwrap();
        assert_eq!(publication.step, PublicationStep::Complete, "{kind}");
        assert!(publication.candidate.is_none(), "{kind}");
        assert!(publication.composition_commit.is_none(), "{kind}");
        assert!(publication.candidate_hashes.is_empty(), "{kind}");
        assert_eq!(record.baseline.root_head, root_before.commit, "{kind}");
        assert_eq!(record.baseline.root_branch, root_before.branch, "{kind}");

        assert_eq!(
            crate::workspace_ops::merge::verified_v0_descriptor(&backend, temp.path(), &record)
                .unwrap_err()
                .code,
            ErrorCode::MergeRecordUnreadable,
            "{kind}"
        );
        assert_eq!(adapted_rule(&backend, temp.path(), &record), None, "{kind}");
        super::atomic_upgrade_v0::assert_valid_unlisted(&backend, temp.path(), &record);
    }
}
