//! The A1 activation's own suite — the enabled paths, executed.
//!
//! Safety review `GwzM5-8A1Activation-ReviewSafety.md` §2 is this package's
//! binding spec. These tests execute what the activation turned on: the
//! contract-§2 writer floor producing a v1 record, `--no-ff` running to a
//! two-parent integration through production dispatch, the R1/R2 coupled pair
//! moving as one gate, and the two conditions the review attached to the
//! change ([P1-1] and [P2-1]).

use super::*;

use crate::workspace_ops::merge::{
    AdaptationPrecheck, RecordVersion, RequestedSemantics, classify_open_record,
    select_record_version,
};

/// A `--no-ff` start request, otherwise identical to the ordinary one.
fn no_ff_request() -> crate::MergeRequest {
    crate::MergeRequest {
        mode: Some(crate::MergeMode::NoFf),
        ..request(false)
    }
}

/// The exact `(schema, record_schema_version)` pair on disk for `merge_id`,
/// wherever the record currently lives.
fn envelope_on_disk(root: &Path, merge_id: &str) -> (String, u64) {
    let open = root.join(format!(".gwz/merge/{merge_id}.yaml"));
    let done = root.join(format!(".gwz/merge/done/{merge_id}.yaml"));
    let path = if open.is_file() { open } else { done };
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("record at '{}' is readable: {error}", path.display()));
    let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    (
        value["schema"].as_str().unwrap().to_owned(),
        value["record_schema_version"].as_u64().unwrap(),
    )
}

/// **R4, executed.** The contract-§2 writer floor chooses the version at
/// creation and the created record carries that version's envelope. Pre-A1
/// `start/record.rs` hard-coded `gwz.merge-operation/v0` / `0` for every
/// start; the activated no-ff surface writes `gwz.merge-operation/v1` / `1`.
#[test]
fn the_production_writer_floor_writes_a_v1_record_for_no_ff() {
    let temp = TempDir::new("a1-writer-floor-v1");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "a1-writer-floor-v1-source");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );

    let response = handle_merge(&backend, temp.path(), no_ff_request(), "op_a1_floor").unwrap();
    let merge_id = response.merge_id.as_deref().unwrap();

    assert_eq!(
        envelope_on_disk(temp.path(), merge_id),
        ("gwz.merge-operation/v1".to_owned(), 1),
        "the writer floor's record carries the v1 envelope"
    );
    assert_eq!(
        select_record_version(RequestedSemantics::NoFf).unwrap(),
        RecordVersion::V1
    );
}

/// **R1 + the v1 writer, executed end to end.** A fast-forwardable member
/// merged with `--no-ff` gets a real two-parent integration commit whose
/// parents are exactly the member's prior HEAD and the source commit — the
/// behaviour the pre-A1 typed refusal made unreachable.
#[test]
fn no_ff_start_publishes_a_two_parent_integration_commit() {
    let temp = TempDir::new("a1-no-ff-two-parent");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "a1-no-ff-two-parent-source");
    let member = temp.path().join("remote");
    let (before, source) = feature_commit(&backend, &member, "README.md", "source\n");

    let response = handle_merge(&backend, temp.path(), no_ff_request(), "op_a1_no_ff").unwrap();

    let head = backend.head(&member).unwrap().commit.unwrap();
    assert_ne!(
        head, source,
        "no-ff must not fast-forward onto the source commit"
    );
    let repository = git2::Repository::open(&member).unwrap();
    let commit = repository
        .find_commit(git2::Oid::from_str(&head).unwrap())
        .unwrap();
    let parents: Vec<String> = commit.parent_ids().map(|id| id.to_string()).collect();
    assert_eq!(parents, vec![before, source], "{response:?}");
}

/// **The R1/R2 coupled pair, pinned.** `validate.rs`'s NoFf refusal and
/// `runtime/dispatch.rs`'s `mode != Some(NoFf)` message-validation exclusion
/// were two halves of one gate. Landing R1 alone would let a NoFf start carry
/// an unvalidated custom message into record creation, because the v1 forward
/// path consumes `row.commit_message` from the record and performs no
/// request-message validation of its own.
///
/// This is the inversion of M5b's designed marker
/// `custom_messages_validate_while_no_ff_remains_reserved`: NoFf is no longer
/// reserved, and every invalid custom-message body that an ordinary start
/// rejects a NoFf start must reject identically.
#[test]
fn the_coupled_pair_validates_custom_messages_on_no_ff_starts() {
    let temp = TempDir::new("a1-coupled-pair");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "a1-coupled-pair-source");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );

    for body in ["", " \t\n", "\u{2003}\r\n", "subject\0body"] {
        let mut ordinary = request(false);
        ordinary.message = Some(body.to_owned());
        let mut no_ff = no_ff_request();
        no_ff.message = Some(body.to_owned());

        let ordinary_error =
            handle_merge(&backend, temp.path(), ordinary, "op_a1_message_v0").unwrap_err();
        let no_ff_error =
            handle_merge(&backend, temp.path(), no_ff, "op_a1_message_v1").unwrap_err();

        assert_eq!(ordinary_error.code, ErrorCode::MergeValidationFailed);
        assert_eq!(
            no_ff_error.code, ordinary_error.code,
            "the coupled pair rejects {body:?} on both modes"
        );
        assert_ne!(
            no_ff_error.code,
            ErrorCode::MergePhaseUnsupported,
            "T-1 inverted: no-ff is no longer a reserved phase"
        );
        assert!(
            !no_ff_error.message.contains("not yet activated"),
            "{}",
            no_ff_error.message
        );
    }
}

/// **[P1-1], executed after activation.** The C-1 dispositions make
/// `adapt_open` refuse the F-MARKER and F-LOCK crash prefixes typed
/// (`PublicationPrefixMismatch`), and those are exactly the prefixes the v0
/// lifecycle resumes to `Completed` today. If the activation's dispatch
/// surfaced that refusal as the resume outcome, currently-recoverable states
/// would become permanent wedges.
///
/// The fix shape the finding names is dispatch routing, not the adapter: the
/// typed refusal is the migration's answer, never the command's, so the v0
/// lifecycle stays in command of rows it can already recover. This test walks
/// both refused prefixes through the post-activation dispatch and requires
/// them to complete, and it requires the record to still be v0 at resume —
/// proving the run really did traverse the adaptation preflight's refusal arm
/// rather than skipping the preflight because the row had migrated.
#[test]
fn post_activation_resume_completes_the_refused_v0_crash_prefixes() {
    use crate::workspace_ops::merge::{
        CandidatePublicationMutation, fail_next_candidate_publication_after,
    };

    for mutation in [
        CandidatePublicationMutation::Marker,
        CandidatePublicationMutation::Lock,
    ] {
        let temp = TempDir::new(&format!("a1-p1-1-resume-{mutation:?}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture =
            init_one_member_workspace(temp.path(), &backend, &format!("a1-p1-1-{mutation:?}"));
        feature_commit(
            &backend,
            &temp.path().join("remote"),
            "README.md",
            "source\n",
        );

        fail_next_candidate_publication_after(mutation);
        let error = handle_merge(
            &backend,
            temp.path(),
            request(false),
            format!("op_a1_p1_1_{mutation:?}"),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);

        // The crash left a Finalizing normal-mode v0 row: exactly the class
        // the [P2-1] pre-check admits to the preflight, so this resume does
        // reach `adapt_open` and does meet its typed refusal.
        let open = classify_open_record(temp.path()).unwrap().unwrap();
        assert_eq!(open.version, RecordVersion::V0);
        assert_eq!(open.adaptation, AdaptationPrecheck::MayAdapt);
        let merge_id = open.merge_id.clone();

        let completed = handle_merge(
            &backend,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, Some(merge_id)),
            format!("op_a1_p1_1_resume_{mutation:?}"),
        )
        .unwrap();

        assert_eq!(
            completed.state,
            crate::MergeOperationState::Completed,
            "the {mutation:?} prefix must still resume to Completed after activation"
        );
        assert!(!completed.open);
        assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
    }
}

/// **[P2-1], pinned.** The adapter's order is envelope -> legacy-mode check ->
/// `validate_v0_structure` -> `classify_open_v0`, so the structural
/// validator's typed-refusal surface runs BEFORE the cheap state
/// pre-classification that would answer `ValidUnlisted` anyway. C-2's two
/// open v0 progress shapes carry zero fixtures, so whether a legal
/// NotStarted / Preparing-empty crash row survives that validator is
/// unmeasured.
///
/// Condition (i): the dispatch gates adaptation on the pre-classification, so
/// only `Finalizing` normal-mode rows can reach the preflight at all. This
/// pins the pre-check over the open v0 state space — every non-`Finalizing`
/// state, and `Finalizing` in a non-normal mode, answers `Skip`.
#[test]
fn the_adaptation_precheck_admits_only_finalizing_normal_mode_v0_rows() {
    let temp = TempDir::new("a1-p2-1-precheck");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "a1-p2-1-precheck-source");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    // Halt the start inside candidate publication so an open Finalizing v0
    // record survives for the walk.
    crate::workspace_ops::merge::fail_next_candidate_publication_after(
        crate::workspace_ops::merge::CandidatePublicationMutation::Marker,
    );
    handle_merge(&backend, temp.path(), request(false), "op_a1_p2_1").unwrap_err();

    let store = FileMergeStore;
    let mut record = store.discover_open(temp.path()).unwrap().unwrap();

    // B-NOT-STARTED and B-PREPARING-EMPTY are `Executing`/`Preserving`-class
    // progress shapes; neither, nor any other non-Finalizing state, may enter
    // the preflight.
    for state in [
        OperationState::Executing,
        OperationState::AwaitingResolution,
        OperationState::Halted,
        OperationState::Preserving,
        OperationState::RollingBack,
        OperationState::RecoveryRequired,
    ] {
        record.state = state;
        store.write_open(temp.path(), &record).unwrap();
        assert_eq!(
            classify_open_record(temp.path())
                .unwrap()
                .unwrap()
                .adaptation,
            AdaptationPrecheck::Skip,
            "{state:?} must never reach validate_v0_structure through the new path"
        );
    }

    record.state = OperationState::Finalizing;
    for mode in [
        crate::workspace_ops::merge::MergeExecutionMode::FfOnly,
        crate::workspace_ops::merge::MergeExecutionMode::NoFf,
    ] {
        record.mode = mode;
        store.write_open(temp.path(), &record).unwrap();
        assert_eq!(
            classify_open_record(temp.path())
                .unwrap()
                .unwrap()
                .adaptation,
            AdaptationPrecheck::Skip,
            "{mode:?} is outside the whitelist's normal-mode class"
        );
    }

    record.mode = crate::workspace_ops::merge::MergeExecutionMode::Normal;
    store.write_open(temp.path(), &record).unwrap();
    assert_eq!(
        classify_open_record(temp.path())
            .unwrap()
            .unwrap()
            .adaptation,
        AdaptationPrecheck::MayAdapt,
        "the one admitted class is Finalizing + normal mode"
    );
}

/// The merge-record id the O9 arms below are started under.
///
/// Both record stagers build their temporary beside the record with
/// `Path::with_extension`, off the same process-global sequence
/// (`store/mod.rs`'s `TEMP_SEQUENCE`), and their names differ by exactly the
/// eight bytes `.upgrade`:
///
/// - store    `{id}.yaml.{pid}.{seq}.tmp`          — `id + pid + seq + 11`
/// - upgrade  `{id}.yaml.{pid}.{seq}.upgrade.tmp`  — `id + pid + seq + 19`
///
/// At `id = 236 - pid` the upgrade's name is `255 + seq_digits` bytes — past a
/// 255-byte component limit for every sequence value that exists — while the
/// store's is `247 + seq_digits`, inside it for any sequence below 100 000 000.
/// The record itself is `241 - pid` bytes and is written normally.
///
/// The id is supplied to the *start*, not patched in afterwards: the v1
/// validator requires each participant commit message to end in the
/// `GWZ-Merge-ID:` trailer (`model/v1/validate/common.rs`), so a record whose
/// id was rewritten under it would be refused by the adapter's own validator
/// and the filesystem would never be reached.
#[cfg(unix)]
fn upgrade_only_overlong_merge_id() -> String {
    let length = 236 - std::process::id().to_string().len();
    let id = format!("merge_{}", "e".repeat(length - "merge_".len()));
    assert_eq!(id.len(), length);
    id
}

/// Starts one merge under an exact merge-record id, otherwise
/// [`invoke_with_store`].
fn invoke_with_store_and_merge_id(
    backend: &crate::git::Git2Backend,
    store: &FaultingMergeStore,
    root: &Path,
    merge_id: &str,
    operation_id: &str,
) -> ModelResult<crate::MergeResponse> {
    struct FixedMergeId {
        merge_id: String,
        next: u64,
    }

    impl crate::runtime::ids::IdProvider for FixedMergeId {
        fn next_id(&mut self, prefix: &str) -> crate::runtime::ids::GeneratedId {
            self.next += 1;
            if prefix == "merge" {
                return crate::runtime::ids::GeneratedId::new(self.merge_id.clone());
            }
            crate::runtime::ids::GeneratedId::new(format!("{prefix}_{:04}", self.next))
        }
    }

    let clock = FixedClock::new(TimestampMs(1_700_000_000_000));
    let mut ids = FixedMergeId {
        merge_id: merge_id.to_owned(),
        next: 0,
    };
    handle_merge_with_dependencies(
        MergeDependencies {
            backend,
            store,
            clock: &clock,
            ids: &mut ids,
            events: &crate::operation::NullSink,
        },
        root,
        request(false),
        operation_id,
    )
}

/// **O9 / Safety [P3-R2-2], executed — the eligible-row upgrade-failure
/// fallback, and with it the restoration L14 records as owed.**
///
/// `adapt_before_mutating` maps every non-`Upgraded` answer, `Err(_)`
/// included, to `Ok(false)` and leaves the v0 lifecycle in command
/// (`runtime/dispatch.rs`). [P1-1]'s rows exercise that mapping only through
/// the adapter's *typed refusals*; no test drove it on a row the whitelist
/// actually admits, whose atomic upgrade then fails. That is the arm the
/// activation record's §14 names as owed once the phase-persistence pin moved
/// to `source_version == V1`
/// (`finalization::resumed_finalization_persists_each_phase_before_a_nested_mutation_fault`).
///
/// The production caller hardcodes `AtomicUpgradeFault::None` and must keep
/// doing so, so the failure has to come from the filesystem. Why it comes from
/// the *name* and not from a permission is worth recording, because a cleared
/// write bit is the obvious first try and cannot work: the upgrade's only own
/// filesystem step is staging its temporary into `.gwz/merge`, and the v0
/// lifecycle's first act on this row is staging its own temporary into the same
/// directory through the same primitive, so any directory-level fault — mode
/// bits, ACL, immutable flag — fails both legs. Nor can such a fault be lifted
/// in between: the composed entry crosses no checked-artifact fault boundary on
/// this path, and `EventSink` delivers `OperationStarted` before the preflight
/// and nothing again until `OperationFinished`.
///
/// The eight bytes `.upgrade` in the staging name do separate them
/// ([`upgrade_only_overlong_merge_id`]). A record id sized into that window
/// makes the upgrade's own `open(2)` refuse the name while every other name in
/// the operation stays legal — a real filesystem refusal, no production seam,
/// `AtomicUpgradeFault::None` untouched. If the window were ever missed the row
/// would migrate and the `V0` assertion below would trip.
///
/// The control arm is the same durable shape under an ordinary id, and it
/// migrates. The two together say the row was genuinely eligible and that the
/// filesystem refusal — not ineligibility — is what returned it to v0.
#[cfg(unix)]
#[test]
fn an_eligible_row_completes_under_v0_when_its_atomic_upgrade_fails() {
    /// Every filesystem this suite runs on caps one path component here.
    const NAME_MAX: usize = 255;

    let overlong = upgrade_only_overlong_merge_id();
    let pid = std::process::id().to_string().len();
    // The window, machine-held rather than described: the upgrade's shortest
    // possible staging name is already over the cap and the store's longest
    // reachable one is still under it.
    assert!(overlong.len() + pid + 1 + 19 > NAME_MAX);
    assert!(overlong.len() + pid + 8 + 11 <= NAME_MAX);
    assert!(overlong.len() + ".yaml".len() <= NAME_MAX);

    for (arm, merge_id) in [
        ("upgrade-refused", overlong),
        ("control", "merge_o9_control".to_owned()),
    ] {
        let temp = TempDir::new(&format!("a1-o9-{arm}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture = init_one_member_workspace(temp.path(), &backend, &format!("a1-o9-{arm}"));
        feature_commit(
            &backend,
            &temp.path().join("remote"),
            "README.md",
            "source\n",
        );

        // The `finalizing-before-publication-record` durable shape, from the
        // same injected-store window `characterization_v0` registers it under.
        let store = FaultingMergeStore::new(FinalizationFault::AfterEnteringFinalizing);
        let error =
            invoke_with_store_and_merge_id(&backend, &store, temp.path(), &merge_id, "op_a1_o9")
                .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired, "{arm}");

        let record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(record.merge_id, merge_id, "{arm}");
        assert_eq!(record.state, OperationState::Finalizing, "{arm}");
        assert!(record.publication.is_none(), "{arm}");

        // The row really is one the whitelist admits, under this id: the
        // merge id is not part of the normalized descriptor, so the two arms
        // match the same rule.
        let descriptor =
            crate::workspace_ops::merge::verified_v0_descriptor(&backend, temp.path(), &record)
                .unwrap();
        assert_eq!(
            super::compatibility_v0::i2_whitelist_matches(descriptor.value()),
            vec!["finalizing-before-publication-record".to_owned()],
            "{arm}"
        );
        let open = classify_open_record(temp.path()).unwrap().unwrap();
        assert_eq!(open.version, RecordVersion::V0, "{arm}");
        assert_eq!(open.adaptation, AdaptationPrecheck::MayAdapt, "{arm}");

        let completed = handle_merge(
            &backend,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
            format!("op_a1_o9_{arm}"),
        )
        .unwrap();

        assert_eq!(
            completed.state,
            crate::MergeOperationState::Completed,
            "{arm}"
        );
        assert!(!completed.open, "{arm}");
        assert!(
            FileMergeStore.discover_open(temp.path()).unwrap().is_none(),
            "{arm}"
        );
        assert_eq!(
            completed
                .record
                .as_ref()
                .map(|record| record.source_version),
            Some(if arm == "control" {
                crate::MergeRecordVersion::V1
            } else {
                // The composed fallback: the atomic upgrade was refused by the
                // filesystem, `Ok(false)` kept the v0 lifecycle in command, and
                // that lifecycle finished the operation itself.
                crate::MergeRecordVersion::V0
            }),
            "{arm}"
        );
        // The durable outcome, not just the projection: the refused arm's
        // archived body is still the v0 envelope the migration would have
        // replaced.
        assert_eq!(
            envelope_on_disk(temp.path(), &merge_id),
            if arm == "control" {
                ("gwz.merge-operation/v1".to_owned(), 1)
            } else {
                ("gwz.merge-operation/v0".to_owned(), 0)
            },
            "{arm}"
        );
    }
}

/// **O9's fault, isolated.** The composed test reads the fallback off the
/// operation's outcome, where an upgrade that failed *earlier* — a typed
/// compatibility refusal instead of the filesystem one — would keep every
/// assertion green while covering something else. This reads the refusal off
/// the upgrade itself: the row is prepared, and the filesystem is what stops
/// it.
#[cfg(unix)]
#[test]
fn the_overlong_staging_name_refuses_the_atomic_upgrade_at_the_filesystem() {
    let merge_id = upgrade_only_overlong_merge_id();
    let temp = TempDir::new("a1-o9-isolated");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "a1-o9-isolated");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let store = FaultingMergeStore::new(FinalizationFault::AfterEnteringFinalizing);
    invoke_with_store_and_merge_id(&backend, &store, temp.path(), &merge_id, "op_a1_o9_iso")
        .unwrap_err();

    let path = temp.path().join(format!(".gwz/merge/{merge_id}.yaml"));
    let source = fs::read(&path).unwrap();
    let error = crate::workspace_ops::merge::upgrade_open_v0(
        &backend,
        temp.path(),
        &merge_id,
        crate::VERSION,
        crate::workspace_ops::merge::AtomicUpgradeFault::None,
    )
    .unwrap_err();

    // An I/O refusal, not a compatibility verdict: the row passed structural
    // validation and matched its rule, and the staged bytes were prepared,
    // before the filesystem refused the staging name.
    assert_eq!(error.code, ErrorCode::IoError);
    assert_eq!(fs::read(&path).unwrap(), source, "nothing was published");
}

/// A foreign object where the catalog's own directory belongs: activation
/// refuses on this workspace from here on.
fn obstruct_the_catalog(root: &Path) {
    fs::create_dir_all(root.join(".gwz")).unwrap();
    fs::write(root.join(".gwz/catalog-final"), b"foreign").unwrap();
}

/// The three rows' shared setup: a one-member workspace with a source commit.
fn workspace(label: &str) -> (TempDir, crate::git::Git2Backend, RemoteFixture) {
    let temp = TempDir::new(label);
    let backend = crate::git::Git2Backend::new();
    let fixture = init_one_member_workspace(temp.path(), &backend, label);
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    (temp, backend, fixture)
}

/// An ordinary merge interrupted at `Finalizing` — a `MayAdapt` v0 row.
fn interrupted_ordinary_merge(
    backend: &crate::git::Git2Backend,
    root: &Path,
    merge_id: &str,
    label: &str,
) {
    let store = FaultingMergeStore::new(FinalizationFault::AfterEnteringFinalizing);
    let error = invoke_with_store_and_merge_id(backend, &store, root, merge_id, label).unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired, "{label}");
    let open = classify_open_record(root).unwrap().unwrap();
    assert_eq!(open.version, RecordVersion::V0, "{label}");
    assert_eq!(open.adaptation, AdaptationPrecheck::MayAdapt, "{label}");
}

/// Leaves a genuine OPEN v1 record and returns its id, by carrying an
/// interrupted ordinary merge across with the production A1 migration itself —
/// the adapter's own call, with the production `AtomicUpgradeFault::None`.
fn open_v1_record_from_an_adapted_crash(
    backend: &crate::git::Git2Backend,
    root: &Path,
    label: &str,
) -> String {
    use crate::workspace_ops::merge::{AtomicUpgradeFault, AtomicUpgradeOutcome, upgrade_open_v0};

    let merge_id = format!("merge_{label}");
    interrupted_ordinary_merge(backend, root, &merge_id, &format!("op_{label}"));
    let outcome = upgrade_open_v0(
        backend,
        root,
        &merge_id,
        crate::VERSION,
        AtomicUpgradeFault::None,
    )
    .unwrap();
    assert!(
        matches!(outcome, AtomicUpgradeOutcome::Upgraded { .. }),
        "{label}: {outcome:?}"
    );
    assert_eq!(
        classify_open_record(root).unwrap().unwrap().version,
        RecordVersion::V1,
        "{label}"
    );
    merge_id
}

/// **E4.1 review [P1-1], cured and driven.** The A1 adapter used to upgrade a
/// `MayAdapt` v0 row durably to v1 before the v1 lifecycle spoke, so an
/// unavailable catalog left it v1, `Skip`, and refusing forever. It now proves
/// the destination viable BEFORE it writes: the catalog is one more
/// non-`Upgraded` answer, v0 completes the record, the envelope stays v0.
#[test]
fn an_interrupted_ordinary_merge_completes_under_v0_when_the_catalog_is_unavailable() {
    let (temp, backend, _fixture) = workspace("a1-e41-wedge");
    let merge_id = "merge_e41_wedge".to_owned();
    interrupted_ordinary_merge(&backend, temp.path(), &merge_id, "op_e41_wedge");

    obstruct_the_catalog(temp.path());
    let completed = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
        "op_e41_wedge_resume".to_owned(),
    )
    .unwrap();

    assert_eq!(completed.state, crate::MergeOperationState::Completed);
    assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
    assert_eq!(
        completed
            .record
            .as_ref()
            .map(|record| record.source_version),
        Some(crate::MergeRecordVersion::V0),
        "the declined upgrade must leave the v0 lifecycle in command"
    );
    assert_eq!(
        envelope_on_disk(temp.path(), &merge_id),
        ("gwz.merge-operation/v0".to_owned(), 0),
        "no durable upgrade may precede a refusal the v1 lifecycle would raise"
    );
}

/// **E4.1 review R3, driven — and R2's abort claim with it.** Resuming a genuine
/// v1 record whose catalog cannot be activated refuses typed, writes nothing,
/// and leaves the record byte-identical; `--abort` on that same record then
/// clears it without ever asking for a catalog. That recoverability is the
/// whole safety argument for refusing at all, and it is also R4's exit for the
/// disclosed post-upgrade race.
#[test]
fn a_v1_resume_refuses_without_mutation_and_abort_still_clears_the_record() {
    let (temp, backend, _fixture) = workspace("a1-e41-v1-resume");
    let merge_id = open_v1_record_from_an_adapted_crash(&backend, temp.path(), "e41_v1_resume");
    let record_path = temp.path().join(format!(".gwz/merge/{merge_id}.yaml"));
    let before = fs::read(&record_path).unwrap();
    obstruct_the_catalog(temp.path());

    let refused = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
        "op_e41_v1_resume".to_owned(),
    )
    .unwrap_err();
    assert!(
        refused.message.contains("merge artifact catalog"),
        "the refusal does not name the catalog: {refused:?}"
    );
    assert_eq!(fs::read(&record_path).unwrap(), before, "record mutated");

    let aborted = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id)),
        "op_e41_v1_abort".to_owned(),
    )
    .unwrap();
    assert!(!aborted.open, "{aborted:?}");
    assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
    assert_eq!(
        fs::read(temp.path().join(".gwz/catalog-final")).unwrap(),
        b"foreign",
        "abort must not have touched the catalog it never needed"
    );
}
