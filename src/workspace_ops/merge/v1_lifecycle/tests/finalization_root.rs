use std::fs;

use sha2::{Digest, Sha256};

use super::*;
use crate::artifact::{LOCK_PATH, LockArtifact, ManifestArtifact, MarkerArtifact};
use crate::git::{Git2Backend, GitBackend};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::model::v1::{AcceptedRootBaseV1, test_record};
use crate::workspace_ops::merge::{MergeTargetKind, OperationState, ParticipantState};
use crate::workspace_ops::tests::{TempDir, commit_file};

use super::tests::{CrashAfterRuntime, RecordingRuntime, context, fixture, seed_open};

#[test]
fn selected_root_acceptance_uses_exact_result_metadata_and_evidence_parent() {
    let (root, backend, model, root_result) =
        selected_root_fixture("merge-v1-finalization-selected-root", true, false);
    seed_open(&root, &model);
    let context = context();
    let mut runtime = FinalizationRuntime::new(&backend, &context);

    let response = super::super::service::run(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();

    let record = response.current().record();
    assert_eq!(record.state, OperationState::Completed);
    assert!(matches!(
        record.accepted_workspace.as_ref().unwrap().metadata_base.source,
        crate::workspace_ops::merge::model::v1::AcceptedMetadataSourceV1::SelectedRootResult {
            ref commit
        } if commit == &root_result
    ));
    let publication = record.publication.as_ref().unwrap();
    assert_eq!(
        publication.root_merge_commit.as_deref(),
        Some(root_result.as_str())
    );
    let composition = publication.composition_commit.as_ref().unwrap();
    let repo = git2::Repository::open(&root.path).unwrap();
    assert_eq!(
        repo.find_commit(git2::Oid::from_str(composition).unwrap())
            .unwrap()
            .parent_id(0)
            .unwrap()
            .to_string(),
        root_result
    );
    let marker =
        MarkerArtifact::from_yaml(&publication.candidate.as_ref().unwrap().marker_yaml).unwrap();
    assert_eq!(
        marker.root.before_commit.as_deref(),
        Some(root_result.as_str())
    );
}

#[test]
fn degenerate_marker_restart_uses_only_stage_then_completion() {
    for (target, expected) in [
        (
            PublicationPhysicalAction::WriteMarker,
            vec![PhysicalActionKind::Publication(
                PublicationPhysicalAction::StageIndex,
            )],
        ),
        (PublicationPhysicalAction::StageIndex, Vec::new()),
    ] {
        let (root, backend, model, _) = selected_root_fixture(
            &format!("merge-v1-finalization-degenerate-{target:?}"),
            false,
            true,
        );
        seed_open(&root, &model);
        let context = context();
        let mut crashing = CrashAfterRuntime::new(&backend, &context, target);
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::super::service::run(
                &super::super::store::CheckedV1Store::default(),
                &root.path,
                &model.merge_id,
                super::super::authority::V1LifecycleRequest::Continue,
                &mut crashing,
            )
        }));
        assert!(crashed.is_err(), "{target:?}");

        let stored = super::super::store::CheckedV1Store::default()
            .load_open(&root.path, &model.merge_id)
            .unwrap();
        let candidate = stored
            .record()
            .publication
            .as_ref()
            .unwrap()
            .candidate
            .as_ref()
            .unwrap();
        assert_eq!(candidate.lock_yaml, candidate.baseline_lock_yaml);
        assert_eq!(candidate.boundary_text, candidate.baseline_boundary_text);

        let mut resumed = RecordingRuntime::new(&backend, &context);
        let response = super::super::service::run(
            &super::super::store::CheckedV1Store::default(),
            &root.path,
            &model.merge_id,
            super::super::authority::V1LifecycleRequest::Continue,
            &mut resumed,
        )
        .unwrap();
        assert_eq!(response.current().record().state, OperationState::Completed);
        assert_eq!(resumed.actions, expected, "{target:?}");
    }
}

#[test]
fn unborn_attached_root_completes_without_publication() {
    let (root, backend, model, head) = unborn_fixture("merge-v1-finalization-unborn", false);
    seed_open(&root, &model);
    let context = context();
    let mut runtime = RecordingRuntime::new(&backend, &context);

    let response = super::super::service::run(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();

    let record = response.current().record();
    assert_eq!(record.state, OperationState::Completed);
    assert!(matches!(
        record.accepted_workspace.as_ref().unwrap().root.base,
        AcceptedRootBaseV1::UnbornAttached { ref symbolic_branch }
            if Some(symbolic_branch) == head.branch.as_ref()
    ));
    assert!(record.publication.as_ref().unwrap().candidate.is_none());
    assert!(runtime.actions.is_empty());
    assert_eq!(backend.head(&root.path).unwrap(), head);
}

#[test]
fn unborn_publication_uses_the_exact_checked_first_commit_candidate() {
    let (root, backend, model, _) =
        unborn_fixture("merge-v1-finalization-unborn-publication", true);
    seed_open(&root, &model);
    let context = context();
    let mut runtime = RecordingRuntime::new(&backend, &context);

    let response = super::super::service::run(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();

    let record = response.current().record();
    assert_eq!(record.state, OperationState::Completed);
    let publication = record.publication.as_ref().unwrap();
    let composition = publication.composition_commit.as_ref().unwrap();
    assert_eq!(publication.candidate_hashes.len(), 2);
    backend
        .verify_gwz_paths_commit(
            &root.path,
            composition,
            None,
            &crate::workspace_ops::merge::acceptance::v1_candidate_files(record).unwrap(),
            &crate::workspace_ops::merge::acceptance::v1_composition_message(record),
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.path.join(WORKSPACE_MANIFEST)).unwrap(),
        model.baseline.manifest_yaml.unwrap()
    );
    let repo = git2::Repository::open(&root.path).unwrap();
    assert_eq!(
        repo.find_commit(git2::Oid::from_str(composition).unwrap())
            .unwrap()
            .parent_count(),
        0
    );
}

#[cfg(unix)]
#[test]
fn publication_rejects_symlinked_marker_parent_before_ref_or_external_write() {
    use std::os::unix::fs::symlink;

    let (root, backend, model) = fixture("merge-v1-finalization-marker-parent-symlink", true);
    let external = TempDir::new("merge-v1-finalization-marker-parent-external");
    symlink(&external.path, root.path.join("gwz.conf/markers")).unwrap();
    seed_open(&root, &model);
    let head_before = backend.head(&root.path).unwrap();
    let context = context();
    let mut runtime = RecordingRuntime::new(&backend, &context);

    let result = super::super::service::run(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut runtime,
    );
    let Err(error) = result else {
        panic!("symlinked marker parent unexpectedly received authority")
    };

    assert_eq!(error.code, crate::model::ErrorCode::MergeDrift);
    let current = super::super::store::CheckedV1Store::default()
        .load_open(&root.path, &model.merge_id)
        .unwrap();
    assert_eq!(current.record().state, OperationState::Finalizing);
    assert!(current.record().accepted_workspace.is_some());
    let publication = current.record().publication.as_ref().unwrap();
    assert_eq!(
        publication.step,
        crate::workspace_ops::merge::PublicationStep::PreparingCandidate
    );
    assert!(publication.candidate.is_none());
    assert!(runtime.actions.is_empty());
    assert_eq!(backend.head(&root.path).unwrap(), head_before);
    assert_eq!(fs::read_dir(&external.path).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn detached_no_publication_preserves_an_unused_symlinked_marker_parent() {
    use std::os::unix::fs::symlink;

    let (root, backend, mut model) =
        fixture("merge-v1-finalization-detached-unused-marker-parent", false);
    let root_commit = model.baseline.root_head.clone().unwrap();
    backend.checkout_commit(&root.path, &root_commit).unwrap();
    model.baseline.root_branch = None;
    let external = TempDir::new("merge-v1-finalization-unused-marker-parent-external");
    let marker_parent = root.path.join("gwz.conf/markers");
    symlink(&external.path, &marker_parent).unwrap();
    seed_open(&root, &model);
    let context = context();
    let mut runtime = RecordingRuntime::new(&backend, &context);

    let response = super::super::service::run(
        &super::super::store::CheckedV1Store::default(),
        &root.path,
        &model.merge_id,
        super::super::authority::V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();

    assert_eq!(response.current().record().state, OperationState::Completed);
    assert!(
        response
            .current()
            .record()
            .publication
            .as_ref()
            .unwrap()
            .candidate
            .is_none()
    );
    assert!(runtime.actions.is_empty());
    assert!(
        fs::symlink_metadata(&marker_parent)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_dir(&external.path).unwrap().count(), 0);
}

fn selected_root_fixture(
    name: &str,
    member_changed: bool,
    prime_boundary: bool,
) -> (
    TempDir,
    Git2Backend,
    crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    String,
) {
    let (root, backend, mut model) = fixture(name, member_changed);
    let root_before = model.baseline.root_head.clone().unwrap();
    model.baseline.lock_commit_sha256 = model.baseline.lock_yaml.as_deref().map(digest);
    model.baseline.manifest_commit_sha256 = model.baseline.manifest_yaml.as_deref().map(digest);
    let placeholder = "f".repeat(40);
    let mut row = model.participants["mem_a"].clone();
    row.path = ".".into();
    row.target_kind = MergeTargetKind::Root;
    row.target_branch = model.baseline.root_branch.clone().unwrap();
    row.before_commit = root_before.clone();
    row.source_commit = placeholder.clone();
    row.resulting_commit = Some(placeholder.clone());
    row.state = ParticipantState::FastForwarded;
    if prime_boundary {
        model.selected_targets.clear();
        model.participants.clear();
    }
    model.selected_targets.push("@root".into());
    model.participants.insert("@root".into(), row);
    let root_result = commit_file(
        &root.path,
        "root-change.txt",
        "selected root\n",
        "selected root",
        &[git2::Oid::from_str(&root_before).unwrap()],
    )
    .unwrap();
    let row = model.participants.get_mut("@root").unwrap();
    row.source_commit = root_result.clone();
    row.resulting_commit = Some(root_result.clone());
    if prime_boundary {
        let manifest =
            ManifestArtifact::from_yaml(model.baseline.manifest_yaml.as_ref().unwrap()).unwrap();
        let lock = LockArtifact::from_yaml(model.baseline.lock_yaml.as_ref().unwrap()).unwrap();
        crate::workspace_ops::sync_workspace_boundary(&backend, &root.path, &manifest, &lock)
            .unwrap();
    } else {
        fs::write(root.path.join(".git/info/exclude"), "/.gwz/\n/members/a/\n").unwrap();
    }
    (root, backend, model, root_result)
}

fn unborn_fixture(
    name: &str,
    changed: bool,
) -> (
    TempDir,
    Git2Backend,
    crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    crate::git::GitHeadState,
) {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
    fs::create_dir_all(root.path.join("gwz.conf")).unwrap();
    let mut model = test_record();
    fs::write(
        root.path.join(WORKSPACE_MANIFEST),
        model.baseline.manifest_yaml.as_ref().unwrap(),
    )
    .unwrap();
    fs::write(
        root.path.join(LOCK_PATH),
        model.baseline.lock_yaml.as_ref().unwrap(),
    )
    .unwrap();
    backend
        .stage_paths(&root.path, &[WORKSPACE_MANIFEST, LOCK_PATH])
        .unwrap();
    let head = backend.head(&root.path).unwrap();
    model.baseline.root_head = None;
    model.baseline.root_branch = head.branch.clone();
    let member = root.path.join("members/a");
    backend.create_repo(&member).unwrap();
    let before = commit_file(&member, "README.md", "before\n", "before", &[]).unwrap();
    let after = if changed {
        commit_file(
            &member,
            "README.md",
            "after\n",
            "after",
            &[git2::Oid::from_str(&before).unwrap()],
        )
        .unwrap()
    } else {
        before.clone()
    };
    let row = model.participants.get_mut("mem_a").unwrap();
    row.before_commit = before;
    row.source_commit = after.clone();
    row.resulting_commit = Some(after);
    row.state = if changed {
        ParticipantState::FastForwarded
    } else {
        ParticipantState::UpToDate
    };
    (root, backend, model, head)
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
