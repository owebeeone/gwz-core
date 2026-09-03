//! An open v0 record is not a merge — and is still an open operation.
//!
//! **`GwzM5-8M5d-Charter.md` §2.** 0.14 has no v0 merge lifecycle. A record
//! left open by a pre-0.14 binary is classified from its ENVELOPE, never
//! decoded, and every merge verb and every gated command answers exactly one
//! sentence:
//!
//! ```text
//! this is a pre-0.14 merge; use gwz 0.13.0 (the last release before 0.14) to continue or abort
//! ```
//!
//! The open-operation remedy that a v1 record still prints — "use merge
//! status, merge continue, or merge abort" — is **suppressed** here, because
//! under 0.14 all three of those verbs refuse (revision 4, L-P3-3).
//!
//! These cases replace the open-v0 lifecycle suites the close deletes. They
//! are deliberately built by writing bytes, not by starting a merge: nothing
//! in this binary can create a v0 record.

use super::*;

const SENTENCE: &str =
    "this is a pre-0.14 merge; use gwz 0.13.0 (the last release before 0.14) to continue or abort";

/// A body that is deliberately NOT a decodable v0 record: only the envelope is
/// well formed. If any site still decodes the body, it fails here rather than
/// quietly succeeding, which is the property §2 actually asks for.
const UNDECODABLE_V0_BODY: &str = concat!(
    "schema: gwz.merge-operation/v0\n",
    "record_schema_version: 0\n",
    "state: executing\n",
    "this_is_not_a_v0_record: true\n",
);

fn plant_open_v0(root: &Path) -> PathBuf {
    let directory = root.join(".gwz/merge");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("merge_legacy.yaml");
    fs::write(&path, UNDECODABLE_V0_BODY).unwrap();
    path
}

fn assert_pre_014(error: &ModelError) {
    assert_eq!(error.code, ErrorCode::OpenOperation, "{}", error.message);
    assert_eq!(error.message, SENTENCE);
    assert!(
        !error.message.contains("merge continue"),
        "the v1 remedy must be suppressed: {}",
        error.message
    );
    assert!(
        !error.message.contains("compatible newer GWZ"),
        "0.14 must not tell the user to upgrade past a record it recognises: {}",
        error.message
    );
}

fn workspace_with_open_v0() -> (TempDir, crate::git::Git2Backend) {
    let temp = TempDir::new("merge-pre-014-refusal");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-pre-014");
    plant_open_v0(temp.path());
    (temp, backend)
}

/// Every merge verb, and the workspace is byte-identical afterwards.
#[test]
fn every_merge_verb_answers_the_pre_014_sentence_and_writes_nothing() {
    let (temp, backend) = workspace_with_open_v0();
    let before = workspace_file_snapshot(temp.path());

    for (op, merge_id) in [
        (crate::MergeOp::Resume, None),
        (crate::MergeOp::Resume, Some("merge_legacy".to_owned())),
        (crate::MergeOp::Abort, None),
        (crate::MergeOp::Abort, Some("merge_legacy".to_owned())),
        (crate::MergeOp::Status, None),
        (crate::MergeOp::Status, Some("merge_legacy".to_owned())),
        (crate::MergeOp::Gc, None),
    ] {
        let error = handle_merge(
            &backend,
            temp.path(),
            recovery_request(op, merge_id),
            "op_pre_014",
        )
        .unwrap_err();
        assert_pre_014(&error);
    }
    assert_eq!(workspace_file_snapshot(temp.path()), before);
}

/// A start is the same answer, dry-run included: it must not read as idle and
/// must not print the v1 start gate's remedy.
#[test]
fn a_start_answers_the_pre_014_sentence_under_both_dry_run_and_real() {
    let (temp, backend) = workspace_with_open_v0();
    let before = workspace_file_snapshot(temp.path());

    for dry_run in [true, false] {
        let error = handle_merge(&backend, temp.path(), request(dry_run), "op_pre_014_start")
            .unwrap_err();
        assert_pre_014(&error);
    }
    assert_eq!(workspace_file_snapshot(temp.path()), before);
}

/// The workspace mutation guard and the pre-dispatch gate. `gwz commit` is the
/// blocked verb the charter names; `gwz stage` is the CONDITIONAL row, and it
/// must not fall through to the ordinary stage arm as if the workspace were
/// idle.
#[test]
fn gated_commands_answer_the_pre_014_sentence() {
    let (temp, _backend) = workspace_with_open_v0();
    let before = workspace_file_snapshot(temp.path());

    for command in [
        crate::operation::OpenMergeCommand::Commit,
        crate::operation::OpenMergeCommand::Push,
        crate::operation::OpenMergeCommand::Pull,
        crate::operation::OpenMergeCommand::Snapshot,
        crate::operation::OpenMergeCommand::Capture,
        crate::operation::OpenMergeCommand::Materialize,
        crate::operation::OpenMergeCommand::BranchMutate,
        crate::operation::OpenMergeCommand::TagMutate,
        crate::operation::OpenMergeCommand::StashMutate,
        crate::operation::OpenMergeCommand::RepoMutate,
        crate::operation::OpenMergeCommand::Forall,
        crate::operation::OpenMergeCommand::InitUpdate,
        crate::operation::OpenMergeCommand::MergeStart,
        crate::operation::OpenMergeCommand::StageConflictResolution,
    ] {
        let error = crate::workspace_ops::merge::enforce_workspace_open_merge_gate(
            temp.path(),
            None,
            command,
        )
        .unwrap_err();
        assert_pre_014(&error);
    }
    assert_eq!(workspace_file_snapshot(temp.path()), before);
}

/// `gwz stage` itself, through its own handler, so the CONDITIONAL row is
/// proved at the command and not only at the gate helper.
#[test]
fn stage_answers_the_pre_014_sentence_and_stages_nothing() {
    let (temp, backend) = workspace_with_open_v0();
    fs::write(temp.path().join("remote/README.md"), "edited\n").unwrap();
    let before = workspace_file_snapshot(temp.path());

    let error = crate::workspace_ops::handle_stage(
        &backend,
        temp.path(),
        crate::StageRequest {
            meta: request_meta(),
            cwd: temp.path().to_string_lossy().into_owned(),
            pathspecs: vec![".".to_owned()],
            all: Some(true),
        },
        "op_pre_014_stage",
    )
    .unwrap_err();
    assert_pre_014(&error);
    assert_eq!(workspace_file_snapshot(temp.path()), before);
}

/// Read verbs stay allowed, exactly as they are under an open v1 merge. §2
/// makes the occupancy refuse the verbs that would act on it; it does not
/// close the workspace to inspection.
#[test]
fn read_verbs_are_still_allowed_over_a_pre_014_occupancy() {
    let (temp, _backend) = workspace_with_open_v0();
    for command in [
        crate::operation::OpenMergeCommand::Status,
        crate::operation::OpenMergeCommand::Ls,
        crate::operation::OpenMergeCommand::Diff,
        crate::operation::OpenMergeCommand::BranchList,
        crate::operation::OpenMergeCommand::SnapshotList,
        crate::operation::OpenMergeCommand::StashList,
        crate::operation::OpenMergeCommand::TagList,
    ] {
        crate::workspace_ops::merge::enforce_workspace_open_merge_gate(
            temp.path(),
            None,
            command,
        )
        .unwrap();
    }
}
