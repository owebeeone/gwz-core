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
    pub(crate) backend: GitTestRepository,
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
            value
                .backend
                .test_force_checkout(&value.root.path.join(&row.path), &row.before_commit)
                .unwrap();
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
    let backend = make_repository();
    backend
        .test_init_repo(&root.path, &TestRepoSpec::default())
        .unwrap();
    let member = root.path.join("members/a");
    backend
        .test_init_repo(&member, &TestRepoSpec::default())
        .unwrap();
    let base = fixture_commit_file(&backend, &member, "README.md", "base\n", "base", &[]);
    backend
        .test_set_ref(
            &member,
            "refs/heads/feature",
            Some(&TestRefTarget::Direct(base.clone())),
        )
        .unwrap();
    backend
        .test_set_head(&member, &TestHead::Attached("refs/heads/feature".into()))
        .unwrap();
    let source = fixture_commit_file(
        &backend,
        &member,
        "README.md",
        "source\n",
        "source",
        std::slice::from_ref(&base),
    );
    backend
        .test_set_head(&member, &TestHead::Attached("refs/heads/main".into()))
        .unwrap();
    let before = fixture_commit_file(
        &backend,
        &member,
        "README.md",
        "target\n",
        "target",
        std::slice::from_ref(&base),
    );
    let snapshot = backend
        .test_seed_merge_conflict(&member, &before, &source)
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
    row.conflict_paths = snapshot
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
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
    let backend = make_repository();
    backend
        .test_init_repo(&root.path, &TestRepoSpec::default())
        .unwrap();
    let exclude_path = crate::workspace_ops::workspace_exclude_path(&root.path);
    let mut exclude = make_filesystem().read(&exclude_path).unwrap();
    exclude.extend_from_slice(b"/.gwz/\n");
    write_for_test(&exclude_path, &exclude).unwrap();
    make_filesystem()
        .create_directories(&root.path.join("gwz.conf"))
        .unwrap();
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    let manifest = model.baseline.manifest_yaml.clone().unwrap();
    let lock = model.baseline.lock_yaml.clone().unwrap();
    let manifest_commit = fixture_commit_file(
        &backend,
        &root.path,
        WORKSPACE_MANIFEST,
        &manifest,
        "baseline manifest",
        &[],
    );
    let before = fixture_commit_file(
        &backend,
        &root.path,
        LOCK_PATH,
        &lock,
        "baseline lock",
        std::slice::from_ref(&manifest_commit),
    );
    let result_manifest = format!("{manifest}# selected-root result\n");
    let result_lock = format!("{lock}# selected-root result\n");
    let result_manifest_commit = fixture_commit_file(
        &backend,
        &root.path,
        WORKSPACE_MANIFEST,
        &result_manifest,
        "result manifest",
        std::slice::from_ref(&before),
    );
    let result = fixture_commit_file(
        &backend,
        &root.path,
        LOCK_PATH,
        &result_lock,
        "result lock",
        std::slice::from_ref(&result_manifest_commit),
    );

    model.state = OperationState::RollingBack;
    model.baseline.root_head = Some(before.clone());
    model.baseline.root_branch = Some("main".into());
    model.baseline.manifest_commit_sha256 = Some(digest(&manifest));
    model.baseline.lock_commit_sha256 = Some(digest(&lock));
    model.selected_targets = vec!["@root".into()];
    backend.test_reset_mixed(&root.path, &before).unwrap();
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
    make_filesystem().create_directories(&merge_root).unwrap();
    write_for_test(
        &merge_root.join(format!("{}.yaml", model.merge_id)),
        serde_yaml::to_string(model).unwrap().as_bytes(),
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
