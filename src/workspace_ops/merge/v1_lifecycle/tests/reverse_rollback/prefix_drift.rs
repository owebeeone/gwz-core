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
                let record_before = std::fs::read(&record_path).unwrap();
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
                assert_eq!(std::fs::read(&record_path).unwrap(), record_before);
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
                let status = std::process::Command::new("git")
                    .args(["reset", "--hard", &fixture.later_before])
                    .current_dir(&fixture.later)
                    .status()
                    .unwrap();
                assert!(status.success());
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
                let record_before = std::fs::read(&record_path).unwrap();
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
                assert_eq!(std::fs::read(&record_path).unwrap(), record_before);
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
    backend: Git2Backend,
    completed: std::path::PathBuf,
    later: std::path::PathBuf,
    later_before: String,
    completed_before: String,
    model: MergeOperationRecordV1,
}

fn prefix_fixture(terminal: ParticipantState, drift: Drift) -> PrefixFixture {
    let root = TempDir::new(&format!("rollback-prefix-{terminal:?}-{drift:?}"));
    let backend = Git2Backend::new();
    let later = root.path.join("members/later");
    let completed = root.path.join("members/completed");
    backend.create_repo(&later).unwrap();
    backend.create_repo(&completed).unwrap();
    let later_before = commit_file(&later, "README.md", "later before\n", "before", &[]).unwrap();
    let later_result = commit_file(
        &later,
        "README.md",
        "later result\n",
        "result",
        &[later_before.parse().unwrap()],
    )
    .unwrap();
    let completed_before =
        commit_file(&completed, "README.md", "completed before\n", "before", &[]).unwrap();
    let completed_result = commit_file(
        &completed,
        "README.md",
        "completed result\n",
        "result",
        &[completed_before.parse().unwrap()],
    )
    .unwrap();
    let status = std::process::Command::new("git")
        .args(["reset", "--hard", &completed_before])
        .current_dir(&completed)
        .status()
        .unwrap();
    assert!(status.success());

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
            std::fs::write(fixture.completed.join("README.md"), "unstaged drift\n").unwrap();
        }
        Drift::Staged => {
            std::fs::write(fixture.completed.join("README.md"), "staged drift\n").unwrap();
            let status = std::process::Command::new("git")
                .args(["add", "README.md"])
                .current_dir(&fixture.completed)
                .status()
                .unwrap();
            assert!(status.success());
        }
        Drift::Untracked => {
            std::fs::write(fixture.completed.join("foreign.txt"), "untracked drift\n").unwrap();
        }
        Drift::SemanticIndex => {
            let status = std::process::Command::new("git")
                .args(["update-index", "--assume-unchanged", "README.md"])
                .current_dir(&fixture.completed)
                .status()
                .unwrap();
            assert!(status.success());
        }
        Drift::Branch => {
            fixture
                .backend
                .branch_create(&fixture.completed, "foreign", &fixture.completed_before)
                .unwrap();
            fixture
                .backend
                .switch_branch(&fixture.completed, "foreign")
                .unwrap();
        }
        Drift::NativeState => {
            let repo = git2::Repository::open(&fixture.completed).unwrap();
            std::fs::write(
                repo.path().join("MERGE_HEAD"),
                format!("{}\n", fixture.completed_before),
            )
            .unwrap();
            std::fs::write(repo.path().join("MERGE_MSG"), "foreign merge\n").unwrap();
        }
    }
}
