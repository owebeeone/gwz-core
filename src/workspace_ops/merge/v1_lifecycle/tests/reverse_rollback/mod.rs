mod entry;
mod faults;
mod phases;
mod real_git;
mod recovery;
mod root_artifacts;

use crate::git::{Git2Backend, GitBackend};
use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;
use crate::workspace_ops::merge::{MergeTargetKind, OperationState, ParticipantState};
use crate::workspace_ops::tests::{TempDir, commit_file};

struct ParticipantFixture {
    root: TempDir,
    backend: Git2Backend,
    member: std::path::PathBuf,
    before: String,
    result: String,
    model: MergeOperationRecordV1,
}

fn integrated_fixture(name: &str) -> ParticipantFixture {
    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    let member = root.path.join("members/a");
    backend.create_repo(&member).unwrap();
    let before = commit_file(&member, "README.md", "before\n", "before", &[]).unwrap();
    let result = commit_file(
        &member,
        "README.md",
        "result\n",
        "result",
        &[before.parse::<git2::Oid>().unwrap()],
    )
    .unwrap();
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::RollingBack;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.path = "members/a".into();
    row.target_kind = MergeTargetKind::Member;
    row.target_branch = "main".into();
    row.before_commit = before.clone();
    row.source_commit = result.clone();
    row.state = ParticipantState::FastForwarded;
    row.resulting_commit = Some(result.clone());
    ParticipantFixture {
        root,
        backend,
        member,
        before,
        result,
        model,
    }
}

struct EvidenceFixture {
    root: TempDir,
    backend: Git2Backend,
    model: MergeOperationRecordV1,
}

fn staged_evidence_fixture(
    name: &str,
    change_boundary: bool,
    change_lock: bool,
) -> EvidenceFixture {
    use crate::artifact::LOCK_PATH;
    use crate::workspace_ops::merge::model::v1::{
        AcceptedLockV1, AcceptedMetadataBaseV1, AcceptedMetadataSourceV1, AcceptedRootBaseV1,
        AcceptedWorkspaceV1, RootArtifactHashesV1, RootPublicationInputV1,
    };
    use crate::workspace_ops::merge::{
        PublicationCandidate, PublicationCandidateHash, PublicationProgress, PublicationStep,
    };
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;

    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
    std::fs::create_dir_all(root.path.join("gwz.conf/markers")).unwrap();
    let baseline_lock = "baseline lock\n";
    let baseline = commit_file(&root.path, LOCK_PATH, baseline_lock, "baseline", &[]).unwrap();
    let boundary_path = crate::workspace_ops::workspace_exclude_path(&root.path);
    let baseline_boundary = std::fs::read_to_string(&boundary_path).unwrap();
    let candidate_boundary = if change_boundary {
        format!("{baseline_boundary}candidate boundary\n")
    } else {
        baseline_boundary.clone()
    };
    let marker_path = "gwz.conf/markers/rollback.yaml";
    let candidate_lock = if change_lock {
        "candidate lock\n"
    } else {
        baseline_lock
    };
    let marker = "candidate marker\n";
    let digest = |value: &str| format!("{:x}", Sha256::digest(value.as_bytes()));

    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::RollingBack;
    model.accepted_workspace = Some(AcceptedWorkspaceV1 {
        operation_baseline_lock_sha256: digest(baseline_lock),
        metadata_base: AcceptedMetadataBaseV1 {
            source: AcceptedMetadataSourceV1::OperationBaseline,
            manifest_exact_yaml: String::new(),
            manifest_sha256: digest(""),
            lock_exact_yaml: baseline_lock.into(),
            lock_sha256: digest(baseline_lock),
        },
        lock: AcceptedLockV1 {
            exact_yaml: candidate_lock.into(),
            sha256: digest(candidate_lock),
        },
        member_audit: BTreeMap::new(),
        root: RootPublicationInputV1 {
            base: AcceptedRootBaseV1::BornAttached {
                commit: baseline.clone(),
                symbolic_branch: "main".into(),
            },
            publication_branch: Some("main".into()),
            baseline_artifact_hashes: RootArtifactHashesV1 {
                lock_worktree_sha256: digest(baseline_lock),
                manifest_worktree_sha256: digest(""),
                lock_commit_sha256: None,
                manifest_commit_sha256: None,
            },
        },
    });
    model.publication = Some(PublicationProgress {
        step: PublicationStep::CommittingEvidence,
        candidate_lock_sha256: Some(digest(candidate_lock)),
        candidate_marker_path: Some(marker_path.into()),
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: Some(PublicationCandidate {
            marker_id: "rollback".into(),
            root_branch: "main".into(),
            actor_id: "agent_test".into(),
            baseline_lock_yaml: baseline_lock.into(),
            lock_yaml: candidate_lock.into(),
            marker_yaml: marker.into(),
            baseline_boundary_sha256: digest(&baseline_boundary),
            baseline_boundary_text: baseline_boundary,
            boundary_sha256: digest(&candidate_boundary),
            boundary_text: candidate_boundary.clone(),
            marker_sha256: digest(marker),
            extensions: BTreeMap::new(),
        }),
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    });
    let files = crate::workspace_ops::merge::acceptance::v1_candidate_files(&model).unwrap();
    let message = crate::workspace_ops::merge::acceptance::v1_composition_message(&model);
    let evidence = backend
        .commit_gwz_paths_checked(&root.path, Some(&baseline), &files, &message)
        .unwrap();
    let publication = model.publication.as_mut().unwrap();
    publication.composition_commit = Some(evidence.commit);
    publication.composition_tree = Some(evidence.tree);
    publication.candidate_hashes = evidence
        .candidate_hashes
        .into_iter()
        .map(|hash| PublicationCandidateHash {
            path: hash.path,
            sha256: hash.sha256,
        })
        .collect();
    crate::workspace_ops::publish_workspace_exclude_candidate(&root.path, &candidate_boundary)
        .unwrap();
    std::fs::write(root.path.join(LOCK_PATH), candidate_lock).unwrap();
    std::fs::write(root.path.join(marker_path), marker).unwrap();
    backend
        .stage_paths(&root.path, &[LOCK_PATH, marker_path])
        .unwrap();
    EvidenceFixture {
        root,
        backend,
        model,
    }
}
