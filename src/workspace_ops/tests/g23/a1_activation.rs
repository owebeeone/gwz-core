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
