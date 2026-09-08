use super::*;
use crate::workspace_ops::merge::v1_lifecycle::authority::V1LifecycleRequest;
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::run_test as run;
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[derive(Clone, Copy, Debug)]
enum Drift {
    Unstaged,
    Staged,
    Untracked,
    SemanticIndex,
    Branch,
    NativeState,
}

#[test]
fn completed_participant_drift_blocks_every_later_owner_and_exhaustion() {
    for terminal in [ParticipantState::Aborted, ParticipantState::RolledBack] {
        for request in admitted_requests() {
            for drift in [
                Drift::Unstaged,
                Drift::Staged,
                Drift::Untracked,
                Drift::SemanticIndex,
                Drift::Branch,
                Drift::NativeState,
            ] {
                let fixture = prefix_fixture(terminal, drift);
                let record_path = fixture
                    .root
                    .path
                    .join(".gwz/merge")
                    .join(format!("{}.yaml", fixture.model.merge_id));
                super::service_fault_matrix::seed_open(&fixture.root.path, &fixture.model);
                let record_before = make_filesystem().read(&record_path).unwrap();
                install_drift(&fixture, drift);
                let later_before = fixture.backend.head(&fixture.later).unwrap();
                let context = super::service_fault_matrix::context(&fixture.model);
                let mut runtime = ReverseRuntime::new(&fixture.backend, &context);

                let error = match run(
                    &CheckedV1Store::default(),
                    &fixture.root.path,
                    &fixture.model.merge_id,
                    request,
                    &mut runtime,
                ) {
                    Err(error) => error,
                    Ok(_) => panic!("{terminal:?}/{request:?}/{drift:?}: drift was accepted"),
                };

                assert!(
                    matches!(
                        error.code,
                        crate::model::ErrorCode::MergeRecoveryRequired
                            | crate::model::ErrorCode::RecoveryEvidenceMismatch
                            | crate::model::ErrorCode::PreservationEvidenceMismatch
                    ),
                    "{terminal:?}/{request:?}/{drift:?}: {error:?}"
                );
                assert_eq!(make_filesystem().read(&record_path).unwrap(), record_before);
                assert_eq!(fixture.backend.head(&fixture.later).unwrap(), later_before);
                let stored = CheckedV1Store::default()
                    .load_open(&fixture.root.path, &fixture.model.merge_id)
                    .unwrap();
                assert_eq!(
                    stored.record().state,
                    OperationState::RollingBack,
                    "{terminal:?}/{request:?}/{drift:?}"
                );
            }
        }
    }
}

#[test]
fn completed_participant_drift_blocks_terminal_exhaustion_for_every_request() {
    for terminal in [ParticipantState::Aborted, ParticipantState::RolledBack] {
        for request in admitted_requests() {
            for drift in [
                Drift::Unstaged,
                Drift::Staged,
                Drift::Untracked,
                Drift::SemanticIndex,
                Drift::Branch,
                Drift::NativeState,
            ] {
                let mut fixture = prefix_fixture(terminal, drift);
                fixture
                    .backend
                    .test_force_checkout(&fixture.later, &fixture.later_before)
                    .unwrap();
                let later = fixture.model.participants.get_mut("mem_a").unwrap();
                later.state = terminal;
                later.resulting_commit = match terminal {
                    ParticipantState::Aborted => None,
                    ParticipantState::RolledBack => Some(later.source_commit.clone()),
                    _ => unreachable!(),
                };
                let record_path = fixture
                    .root
                    .path
                    .join(".gwz/merge")
                    .join(format!("{}.yaml", fixture.model.merge_id));
                super::service_fault_matrix::seed_open(&fixture.root.path, &fixture.model);
                let record_before = make_filesystem().read(&record_path).unwrap();
                install_drift(&fixture, drift);
                let context = super::service_fault_matrix::context(&fixture.model);
                let mut runtime = ReverseRuntime::new(&fixture.backend, &context);

                let error = match run(
                    &CheckedV1Store::default(),
                    &fixture.root.path,
                    &fixture.model.merge_id,
                    request,
                    &mut runtime,
                ) {
                    Err(error) => error,
                    Ok(_) => panic!(
                        "{terminal:?}/{request:?}/{drift:?}: terminal exhaustion accepted drift"
                    ),
                };

                assert!(
                    matches!(
                        error.code,
                        crate::model::ErrorCode::MergeRecoveryRequired
                            | crate::model::ErrorCode::RecoveryEvidenceMismatch
                            | crate::model::ErrorCode::PreservationEvidenceMismatch
                    ),
                    "{terminal:?}/{request:?}/{drift:?}: {error:?}"
                );
                assert_eq!(make_filesystem().read(&record_path).unwrap(), record_before);
                assert_eq!(
                    CheckedV1Store::default()
                        .load_open(&fixture.root.path, &fixture.model.merge_id)
                        .unwrap()
                        .record()
                        .state,
                    OperationState::RollingBack,
                    "{terminal:?}/{request:?}/{drift:?}"
                );
            }
        }
    }
}

fn admitted_requests() -> [V1LifecycleRequest; 5] {
    [
        V1LifecycleRequest::ResumeStart,
        V1LifecycleRequest::Continue,
        V1LifecycleRequest::Abort,
        V1LifecycleRequest::Preserve,
        V1LifecycleRequest::Archive,
    ]
}

struct PrefixFixture {
    root: TempDir,
    backend: GitTestRepository,
    completed: std::path::PathBuf,
    later: std::path::PathBuf,
    later_before: String,
    completed_before: String,
    model: MergeOperationRecordV1,
}

fn prefix_fixture(terminal: ParticipantState, drift: Drift) -> PrefixFixture {
    let root = TempDir::new(&format!("rollback-prefix-{terminal:?}-{drift:?}"));
    let backend = make_repository();
    backend
        .test_init_repo(&root.path, &TestRepoSpec::default())
        .unwrap();
    let later = root.path.join("members/later");
    let completed = root.path.join("members/completed");
    backend.create_repo(&later).unwrap();
    backend.create_repo(&completed).unwrap();
    let later_before = fixture_commit_file(
        &backend,
        &later,
        "README.md",
        "later before\n",
        "before",
        &[],
    );
    let later_result = fixture_commit_file(
        &backend,
        &later,
        "README.md",
        "later result\n",
        "result",
        std::slice::from_ref(&later_before),
    );
    let completed_before = fixture_commit_file(
        &backend,
        &completed,
        "README.md",
        "completed before\n",
        "before",
        &[],
    );
    let completed_result = fixture_commit_file(
        &backend,
        &completed,
        "README.md",
        "completed result\n",
        "result",
        std::slice::from_ref(&completed_before),
    );
    backend
        .test_force_checkout(&completed, &completed_before)
        .unwrap();

    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::RollingBack;
    let later_row = model.participants.get_mut("mem_a").unwrap();
    later_row.path = "members/later".into();
    later_row.target_branch = "main".into();
    later_row.before_commit = later_before.clone();
    later_row.source_commit = later_result.clone();
    later_row.state = ParticipantState::FastForwarded;
    later_row.resulting_commit = Some(later_result);
    let mut completed_row = later_row.clone();
    completed_row.path = "members/completed".into();
    completed_row.before_commit = completed_before.clone();
    completed_row.source_commit = completed_result.clone();
    completed_row.state = terminal;
    completed_row.resulting_commit = match terminal {
        ParticipantState::Aborted => None,
        ParticipantState::RolledBack => Some(completed_result),
        _ => unreachable!(),
    };
    completed_row.expected_merge_head = None;
    completed_row.conflict_paths.clear();
    completed_row.conflict_snapshot.clear();
    completed_row.error = None;
    completed_row.pending_action = None;
    model.selected_targets = vec!["mem_a".into(), "mem_z".into()];
    model.participants.insert("mem_z".into(), completed_row);
    let mut manifest = crate::artifact::ManifestArtifact::from_yaml(
        model.baseline.manifest_yaml.as_deref().unwrap(),
    )
    .unwrap();
    manifest.members[0].path = "members/later".into();
    let mut member = manifest.members[0].clone();
    member.id = "mem_z".into();
    member.path = "members/completed".into();
    member.source_id = "src_z".into();
    manifest.members.push(member);
    let manifest_yaml = manifest.to_yaml().unwrap();
    use sha2::{Digest, Sha256};
    model.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest_yaml.as_bytes()));
    model.baseline.manifest_yaml = Some(manifest_yaml);
    crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::align_baseline_lock(&mut model);
    let mut lock =
        crate::artifact::LockArtifact::from_yaml(model.baseline.lock_yaml.as_deref().unwrap())
            .unwrap();
    let mut later_lock = lock.members.remove("mem_a").unwrap();
    later_lock.path = "members/later".into();
    later_lock.commit = Some(model.participants["mem_a"].before_commit.clone());
    let mut completed_lock = later_lock.clone();
    completed_lock.path = "members/completed".into();
    completed_lock.source_id = Some("src_z".into());
    completed_lock.commit = Some(completed_before.clone());
    lock.members.insert("mem_a".into(), later_lock);
    lock.members.insert("mem_z".into(), completed_lock);
    let lock_yaml = lock.to_yaml().unwrap();
    model.baseline.lock_sha256 = format!("{:x}", Sha256::digest(lock_yaml.as_bytes()));
    model.baseline.lock_yaml = Some(lock_yaml);

    PrefixFixture {
        root,
        backend,
        completed,
        later,
        later_before,
        completed_before,
        model,
    }
}

fn install_drift(fixture: &PrefixFixture, drift: Drift) {
    match drift {
        Drift::Unstaged => {
            write_for_test(&fixture.completed.join("README.md"), b"unstaged drift\n").unwrap();
        }
        Drift::Staged => {
            write_for_test(&fixture.completed.join("README.md"), b"staged drift\n").unwrap();
            fixture
                .backend
                .stage_paths(&fixture.completed, &["README.md"])
                .unwrap();
        }
        Drift::Untracked => {
            write_for_test(&fixture.completed.join("foreign.txt"), b"untracked drift\n").unwrap();
        }
        Drift::SemanticIndex => {
            let mut entries = fixture.backend.test_read_index(&fixture.completed).unwrap();
            entries
                .iter_mut()
                .find(|entry| entry.path == b"README.md")
                .unwrap()
                .assume_valid = true;
            fixture
                .backend
                .test_replace_index(&fixture.completed, &entries)
                .unwrap();
        }
        Drift::Branch => {
            fixture
                .backend
                .test_set_ref(
                    &fixture.completed,
                    "refs/heads/foreign",
                    Some(&TestRefTarget::Direct(fixture.completed_before.clone())),
                )
                .unwrap();
            fixture
                .backend
                .test_set_head(
                    &fixture.completed,
                    &TestHead::Attached("refs/heads/foreign".into()),
                )
                .unwrap();
        }
        Drift::NativeState => {
            fixture
                .backend
                .test_set_repository_state(
                    &fixture.completed,
                    crate::git::GitRepositoryState::Merge,
                    Some(&fixture.completed_before),
                )
                .unwrap();
        }
    }
}
