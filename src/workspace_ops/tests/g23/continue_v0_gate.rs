use super::*;
use crate::workspace_ops::merge::{
    MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeBaseline, MergeExecutionMode,
    MergeParticipantRecord, MergeTargetKind, ParticipantState, PendingCommitSpec,
    PendingGitSignature, PendingMergeAction, PendingMergeActionKind, PendingMergeExpectedResult,
};

// The v0 forged-action resume gate (M5b-IF review finding Code F-1).
//
// No supported v0 writer produces `mode: no_ff` (start refuses it before
// record creation) or a durable two-parent commit action over a
// fast-forwardable pair (that durable shape exists only for v1 no-ff
// semantics). The amended I2 contracts promise `UnsupportedLegacyMode` for
// these rows before resume or mutation; these suites pin that promise on the
// production continue path, plus the positive row proving the gate stays
// silent for the legal v0 true-merge shape over a genuinely divergent pair.

fn forged_signature() -> PendingGitSignature {
    PendingGitSignature {
        name: "Forger".to_owned(),
        email: "forger@example.test".to_owned(),
        time_seconds: 1_700_000_000,
        timezone_offset_minutes: 0,
        extensions: BTreeMap::new(),
    }
}

fn two_parent_pending_action(
    participant: &MergeParticipantRecord,
    tree_oid: &str,
) -> PendingMergeAction {
    PendingMergeAction {
        kind: PendingMergeActionKind::TrueMerge,
        target_branch: participant.target_branch.clone(),
        before_commit: participant.before_commit.clone(),
        source_commit: participant.source_commit.clone(),
        commit_message: participant.commit_message.clone(),
        expected_result: Some(PendingMergeExpectedResult::Commit),
        commit_spec: Some(PendingCommitSpec {
            tree_oid: tree_oid.to_owned(),
            author: forged_signature(),
            committer: forged_signature(),
            extensions: BTreeMap::new(),
        }),
        extensions: BTreeMap::new(),
    }
}

fn forged_participant(path: &str, before: &str, source: &str) -> MergeParticipantRecord {
    MergeParticipantRecord {
        path: path.to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: before.to_owned(),
        source_commit: source.to_owned(),
        commit_message: format!("Forged resume action for {path}"),
        state: ParticipantState::Failed,
        resulting_commit: None,
        expected_merge_head: None,
        conflict_paths: Vec::new(),
        conflict_snapshot: Vec::new(),
        error: None,
        pending_action: None,
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: BTreeMap::new(),
    }
}

fn forged_record(
    root: &Path,
    merge_id: &str,
    mode: MergeExecutionMode,
    participants: Vec<(&str, MergeParticipantRecord)>,
) -> MergeOperationRecord {
    let digest = |path: PathBuf| format!("{:x}", Sha256::digest(fs::read(path).unwrap()));
    MergeOperationRecord {
        schema: MERGE_RECORD_SCHEMA.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION,
        writer_version: crate::VERSION.to_owned(),
        workspace_id: "ws_ops".to_owned(),
        merge_id: merge_id.to_owned(),
        operation_id: "op_forge".to_owned(),
        state: OperationState::Halted,
        source_ref: "feature/source".to_owned(),
        mode,
        created_at: "now".to_owned(),
        baseline: MergeBaseline {
            lock_sha256: digest(root.join(crate::artifact::LOCK_PATH)),
            manifest_sha256: digest(root.join(crate::workspace::WORKSPACE_MANIFEST)),
            lock_yaml: None,
            manifest_yaml: None,
            lock_commit_sha256: None,
            manifest_commit_sha256: None,
            root_head: None,
            root_branch: None,
            extensions: BTreeMap::new(),
        },
        selected_targets: participants
            .iter()
            .map(|(id, _)| (*id).to_owned())
            .collect(),
        participants: participants
            .into_iter()
            .map(|(id, participant)| (id.to_owned(), participant))
            .collect(),
        publication: None,
        operation_drift: Vec::new(),
        extensions: BTreeMap::new(),
    }
}

fn open_record_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(format!(".gwz/merge/{merge_id}.yaml"))
}

fn commit_tree_oid(repo: &Path, commit: &str) -> String {
    git2::Repository::open(repo)
        .unwrap()
        .find_commit(git2::Oid::from_str(commit).unwrap())
        .unwrap()
        .tree_id()
        .to_string()
}

fn resume(
    backend: &crate::git::Git2Backend,
    root: &Path,
    merge_id: &str,
    operation_id: &str,
) -> ModelResult<crate::MergeResponse> {
    handle_merge(
        backend,
        root,
        recovery_request(crate::MergeOp::Resume, Some(merge_id.to_owned())),
        operation_id,
    )
}

#[test]
fn v0_resume_rejects_forged_two_parent_action_over_fast_forwardable_pair() {
    let temp = TempDir::new("merge-forged-two-parent-ff");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (before, source) = feature_commit(&backend, &app, "source.txt", "app\n");
    let (lib_before, lib_source) = feature_commit(&backend, &lib, "source.txt", "lib\n");
    let source_tree = commit_tree_oid(&app, &source);

    let mut app_row = forged_participant("app", &before, &source);
    app_row.pending_action = Some(two_parent_pending_action(&app_row, &source_tree));
    let record = forged_record(
        temp.path(),
        "merge_forged_two_parent",
        MergeExecutionMode::Normal,
        vec![
            ("mem_app", app_row),
            (
                "mem_lib",
                forged_participant("lib", &lib_before, &lib_source),
            ),
        ],
    );
    FileMergeStore.write_open(temp.path(), &record).unwrap();
    let record_path = open_record_path(temp.path(), &record.merge_id);
    let bytes_before = fs::read(&record_path).unwrap();

    let error = resume(&backend, temp.path(), &record.merge_id, "op_forged_resume").unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedLegacyMode);
    assert_eq!(error.member_id.as_deref(), Some("mem_app"));
    assert!(
        error.message.contains("two-parent merge commit")
            && error.message.contains("fast-forwardable"),
        "unexpected message: {}",
        error.message
    );
    // The target refs never moved: no two-parent commit, no fast-forward.
    assert_eq!(
        backend.head(&app).unwrap().commit.as_deref(),
        Some(before.as_str())
    );
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(lib_before.as_str())
    );
    // The worktree was never touched: the source-only file did not materialize.
    assert!(!app.join("source.txt").exists());
    // The durable record is byte-identical: the refusal preceded every write.
    assert_eq!(fs::read(&record_path).unwrap(), bytes_before);
}

#[test]
fn v0_resume_rejects_forged_no_ff_mode_row() {
    let temp = TempDir::new("merge-forged-no-ff-mode");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (before, source) = feature_commit(&backend, &app, "source.txt", "app\n");
    let (lib_before, lib_source) = feature_commit(&backend, &lib, "source.txt", "lib\n");

    // Plain retry rows that a normal-mode resume would legally fast-forward;
    // only the forged `mode: no_ff` marks the record as unsupported legacy.
    let record = forged_record(
        temp.path(),
        "merge_forged_no_ff",
        MergeExecutionMode::NoFf,
        vec![
            ("mem_app", forged_participant("app", &before, &source)),
            (
                "mem_lib",
                forged_participant("lib", &lib_before, &lib_source),
            ),
        ],
    );
    FileMergeStore.write_open(temp.path(), &record).unwrap();
    let record_path = open_record_path(temp.path(), &record.merge_id);
    let bytes_before = fs::read(&record_path).unwrap();

    let error = resume(&backend, temp.path(), &record.merge_id, "op_no_ff_resume").unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedLegacyMode);
    assert_eq!(error.member_id, None);
    assert!(
        error.message.contains("no_ff") && error.message.contains(&record.merge_id),
        "unexpected message: {}",
        error.message
    );
    // Nothing moved and nothing was rewritten before the refusal.
    assert_eq!(
        backend.head(&app).unwrap().commit.as_deref(),
        Some(before.as_str())
    );
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(lib_before.as_str())
    );
    assert!(!app.join("source.txt").exists());
    assert_eq!(fs::read(&record_path).unwrap(), bytes_before);
}

#[test]
fn durable_two_parent_action_over_divergent_pair_still_resumes_and_merges() {
    let temp = TempDir::new("merge-durable-divergent-true-merge");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (base, source) = feature_commit(&backend, &app, "source.txt", "app\n");
    let (lib_before, lib_source) = feature_commit(&backend, &lib, "source.txt", "lib\n");
    // Diverge the target: advance main past the recorded merge base so the
    // recorded (before, source) pair classifies as a genuine true merge.
    let before = commit_file(
        &app,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    // The exact clean merge tree the v0 writer would have frozen.
    let repo = git2::Repository::open(&app).unwrap();
    let tree_of = |commit: &str| {
        repo.find_commit(git2::Oid::from_str(commit).unwrap())
            .unwrap()
            .tree()
            .unwrap()
    };
    let mut merge_index = repo
        .merge_trees(&tree_of(&base), &tree_of(&before), &tree_of(&source), None)
        .unwrap();
    assert!(!merge_index.has_conflicts());
    let merge_tree = merge_index.write_tree_to(&repo).unwrap().to_string();

    let mut app_row = forged_participant("app", &before, &source);
    app_row.pending_action = Some(two_parent_pending_action(&app_row, &merge_tree));
    let record = forged_record(
        temp.path(),
        "merge_durable_divergent",
        MergeExecutionMode::Normal,
        vec![
            ("mem_app", app_row),
            (
                "mem_lib",
                forged_participant("lib", &lib_before, &lib_source),
            ),
        ],
    );
    FileMergeStore.write_open(temp.path(), &record).unwrap();

    let response = resume(
        &backend,
        temp.path(),
        &record.merge_id,
        "op_divergent_resume",
    )
    .unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        merge_repo(&response, "mem_app").state,
        crate::MergeParticipantState::Merged
    );
    assert_eq!(
        merge_repo(&response, "mem_lib").state,
        crate::MergeParticipantState::FastForwarded
    );
    let head = backend.head(&app).unwrap().commit.unwrap();
    let commit = repo
        .find_commit(git2::Oid::from_str(&head).unwrap())
        .unwrap();
    assert_eq!(commit.parent_count(), 2);
    assert_eq!(commit.parent_id(0).unwrap().to_string(), before);
    assert_eq!(commit.parent_id(1).unwrap().to_string(), source);
    assert_eq!(commit.tree_id().to_string(), merge_tree);
    assert_eq!(backend.head(&lib).unwrap().commit, Some(lib_source));
}
