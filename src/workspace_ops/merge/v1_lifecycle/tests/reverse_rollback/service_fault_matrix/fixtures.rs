use super::super::*;
use crate::artifact::LOCK_PATH;
use crate::operation::{ActionKind, OperationContext};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::ConflictFileEvidence;
use crate::workspace_ops::merge::model::v1::{RecoveryContextV1, RecoveryOriginStateV1};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Lane {
    AbortConflict,
    ResetIntegrated,
    Evidence,
    SelectedRoot,
}

pub(crate) struct MatrixFixture {
    pub(crate) root: TempDir,
    pub(crate) backend: Git2Backend,
    pub(crate) model: MergeOperationRecordV1,
}

pub(crate) fn fixture(lane: Lane, name: &str) -> MatrixFixture {
    match lane {
        Lane::AbortConflict => conflict_fixture(name),
        Lane::ResetIntegrated => {
            let value = integrated_fixture(name);
            MatrixFixture {
                root: value.root,
                backend: value.backend,
                model: value.model,
            }
        }
        Lane::Evidence => {
            let mut value = staged_evidence_fixture(name, true, true);
            let row = value.model.participants.get_mut("mem_a").unwrap();
            row.state = if row.resulting_commit.as_deref() == Some(row.before_commit.as_str()) {
                ParticipantState::Aborted
            } else {
                ParticipantState::RolledBack
            };
            let status = std::process::Command::new("git")
                .args(["reset", "--hard", &row.before_commit])
                .current_dir(value.root.path.join(&row.path))
                .status()
                .unwrap();
            assert!(status.success());
            MatrixFixture {
                root: value.root,
                backend: value.backend,
                model: value.model,
            }
        }
        Lane::SelectedRoot => selected_root_fixture(name),
    }
}

fn conflict_fixture(name: &str) -> MatrixFixture {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    let member = root.path.join("members/a");
    backend.create_repo(&member).unwrap();
    let base = commit_file(&member, "README.md", "base\n", "base", &[]).unwrap();
    backend.branch_create(&member, "feature", &base).unwrap();
    backend.switch_branch(&member, "feature").unwrap();
    let source = commit_file(
        &member,
        "README.md",
        "source\n",
        "source",
        &[base.parse().unwrap()],
    )
    .unwrap();
    backend.switch_branch(&member, "main").unwrap();
    let before = commit_file(
        &member,
        "README.md",
        "target\n",
        "target",
        &[base.parse().unwrap()],
    )
    .unwrap();
    let result = backend
        .merge_upstream_checked(&member, "main", &before, &source, "merge", None)
        .unwrap();
    assert!(result.commit.is_none());
    let snapshot = backend
        .merge_conflict_snapshot(&member, &before, &source)
        .unwrap();
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::RollingBack;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.path = "members/a".into();
    row.target_kind = MergeTargetKind::Member;
    row.target_branch = "main".into();
    row.before_commit = before;
    row.source_commit = source.clone();
    row.state = ParticipantState::Conflicted;
    row.resulting_commit = None;
    row.expected_merge_head = Some(source);
    row.conflict_paths = result.conflicts;
    row.conflict_snapshot = snapshot
        .files
        .into_iter()
        .map(|file| ConflictFileEvidence {
            path: file.path,
            sha256: file.sha256,
        })
        .collect();
    MatrixFixture {
        root,
        backend,
        model,
    }
}

fn selected_root_fixture(name: &str) -> MatrixFixture {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
    use std::io::Write;
    let mut exclude = std::fs::OpenOptions::new()
        .append(true)
        .open(crate::workspace_ops::workspace_exclude_path(&root.path))
        .unwrap();
    writeln!(exclude, "/.gwz/").unwrap();
    std::fs::create_dir_all(root.path.join("gwz.conf")).unwrap();
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    let manifest = model.baseline.manifest_yaml.clone().unwrap();
    let lock = model.baseline.lock_yaml.clone().unwrap();
    let manifest_commit = commit_file(
        &root.path,
        WORKSPACE_MANIFEST,
        &manifest,
        "baseline manifest",
        &[],
    )
    .unwrap();
    let before = commit_file(
        &root.path,
        LOCK_PATH,
        &lock,
        "baseline lock",
        &[manifest_commit.parse().unwrap()],
    )
    .unwrap();
    let result_manifest = format!("{manifest}# selected-root result\n");
    let result_lock = format!("{lock}# selected-root result\n");
    let result_manifest_commit = commit_file(
        &root.path,
        WORKSPACE_MANIFEST,
        &result_manifest,
        "result manifest",
        &[before.parse().unwrap()],
    )
    .unwrap();
    let result = commit_file(
        &root.path,
        LOCK_PATH,
        &result_lock,
        "result lock",
        &[result_manifest_commit.parse().unwrap()],
    )
    .unwrap();

    model.state = OperationState::RollingBack;
    model.baseline.root_head = Some(before.clone());
    model.baseline.root_branch = Some("main".into());
    model.baseline.manifest_commit_sha256 = Some(digest(&manifest));
    model.baseline.lock_commit_sha256 = Some(digest(&lock));
    model.selected_targets = vec!["@root".into()];
    git2::Repository::open(&root.path)
        .unwrap()
        .find_reference("refs/heads/main")
        .unwrap()
        .set_target(before.parse().unwrap(), "seed post-participant rollback")
        .unwrap();
    let status = std::process::Command::new("git")
        .args(["reset", "--mixed", &before])
        .current_dir(&root.path)
        .status()
        .unwrap();
    assert!(status.success());
    let mut row = model.participants.remove("mem_a").unwrap();
    row.path = ".".into();
    row.target_kind = MergeTargetKind::Root;
    row.target_branch = "main".into();
    row.before_commit = before;
    row.source_commit = result.clone();
    row.state = ParticipantState::RolledBack;
    row.resulting_commit = Some(result);
    model.participants.clear();
    model.participants.insert("@root".into(), row);
    MatrixFixture {
        root,
        backend,
        model,
    }
}

fn digest(bytes: &str) -> String {
    format!("{:x}", Sha256::digest(bytes.as_bytes()))
}

pub(crate) fn seed_open(root: &std::path::Path, model: &MergeOperationRecordV1) {
    let merge_root = root.join(".gwz/merge");
    std::fs::create_dir_all(&merge_root).unwrap();
    std::fs::write(
        merge_root.join(format!("{}.yaml", model.merge_id)),
        serde_yaml::to_string(model).unwrap(),
    )
    .unwrap();
}

pub(crate) fn seed_recovery(root: &std::path::Path, model: &MergeOperationRecordV1) {
    let mut recovery = model.clone();
    recovery.state = OperationState::RecoveryRequired;
    recovery.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::RollingBack,
    });
    seed_open(root, &recovery);
}

pub(crate) fn context(model: &MergeOperationRecordV1) -> OperationContext {
    OperationContext {
        operation_id: model.operation_id.clone(),
        request_id: format!("req_{}", model.merge_id),
        schema_version: "gwz.protocol/v0".into(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}
