mod entry;
mod entry_service;
mod entry_service_drift;
mod faults;
mod phases;
mod prefix_drift;
mod real_git;
mod recovery;
mod root_artifacts;
mod service_ambiguity_matrix;
mod service_durability;
mod service_fault_matrix;

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
    let root = TempDir::new_git(name);
    // Pin conversion off at creation (safe: created empty, never cloned) —
    // these suites compare worktree bytes with blob bytes on Windows runners.
    crate::workspace_ops::tests::pin_fixture_autocrlf(&root.path);
    let backend = Git2Backend::new();
    let member = root.path.join("members/a");
    backend.create_repo(&member).unwrap();
    crate::workspace_ops::tests::pin_fixture_autocrlf(&member);
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
    use crate::workspace_ops::merge::{
        PublicationCandidateHash, PublicationProgress, PublicationStep,
    };
    use sha2::{Digest, Sha256};

    let root = TempDir::new(name);
    let backend = Git2Backend::new();
    backend.create_repo(&root.path).unwrap();
    crate::workspace_ops::tests::pin_fixture_autocrlf(&root.path);
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    if change_lock {
        use std::io::Write;
        let mut exclude = std::fs::OpenOptions::new()
            .append(true)
            .open(crate::workspace_ops::workspace_exclude_path(&root.path))
            .unwrap();
        writeln!(exclude, "/members/a/").unwrap();
        let member = root.path.join("members/a");
        backend.create_repo(&member).unwrap();
        crate::workspace_ops::tests::pin_fixture_autocrlf(&member);
        let member_before = commit_file(&member, "README.md", "before\n", "before", &[]).unwrap();
        let member_result = commit_file(
            &member,
            "README.md",
            "result\n",
            "result",
            &[member_before.parse().unwrap()],
        )
        .unwrap();
        let row = model.participants.get_mut("mem_a").unwrap();
        row.path = "members/a".into();
        row.target_kind = MergeTargetKind::Member;
        row.target_branch = "main".into();
        row.before_commit = member_before;
        row.source_commit = member_result.clone();
        row.state = ParticipantState::FastForwarded;
        row.resulting_commit = Some(member_result);
    }
    crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::align_baseline_lock(&mut model);
    if !change_lock {
        let row = model.participants.get_mut("mem_a").unwrap();
        row.state = ParticipantState::UpToDate;
        row.resulting_commit = Some(row.before_commit.clone());
    }
    let baseline_manifest = model.baseline.manifest_yaml.clone().unwrap();
    let baseline_lock = model.baseline.lock_yaml.clone().unwrap();
    std::fs::create_dir_all(root.path.join("gwz.conf/markers")).unwrap();
    let manifest_commit = commit_file(
        &root.path,
        crate::workspace::WORKSPACE_MANIFEST,
        &baseline_manifest,
        "manifest",
        &[],
    )
    .unwrap();
    let baseline = commit_file(
        &root.path,
        LOCK_PATH,
        &baseline_lock,
        "baseline",
        &[manifest_commit.parse().unwrap()],
    )
    .unwrap();
    let boundary_path = crate::workspace_ops::workspace_exclude_path(&root.path);
    let baseline_boundary = std::fs::read_to_string(&boundary_path).unwrap();
    let candidate_boundary = if change_boundary {
        format!("{baseline_boundary}candidate boundary\n")
    } else {
        baseline_boundary.clone()
    };
    let digest = |value: &str| format!("{:x}", Sha256::digest(value.as_bytes()));

    model.state = OperationState::RollingBack;
    model.baseline.root_head = Some(baseline.clone());
    model.baseline.root_branch = Some("main".into());
    let current = crate::workspace_ops::merge::v1_lifecycle::checked::StoredV1Record::for_test(
        &root.path,
        model.clone(),
    )
    .unwrap();
    let accepted =
        crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::accepted_workspace(&current);
    let mut candidate =
        crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::candidate_payload(&current);
    candidate.candidate.baseline_boundary_text = baseline_boundary;
    candidate.candidate.baseline_boundary_sha256 =
        digest(&candidate.candidate.baseline_boundary_text);
    candidate.candidate.boundary_text = candidate_boundary.clone();
    candidate.candidate.boundary_sha256 = digest(&candidate_boundary);
    model.accepted_workspace = Some(accepted);
    model.publication = Some(PublicationProgress {
        step: PublicationStep::CommittingEvidence,
        candidate_lock_sha256: Some(candidate.lock_sha256),
        candidate_marker_path: Some(candidate.marker_path),
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: Some(candidate.candidate),
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
    let publication = model.publication.as_ref().unwrap();
    let candidate = publication.candidate.as_ref().unwrap();
    let marker_path = publication.candidate_marker_path.as_ref().unwrap();
    std::fs::write(root.path.join(LOCK_PATH), &candidate.lock_yaml).unwrap();
    std::fs::write(root.path.join(marker_path), &candidate.marker_yaml).unwrap();
    backend
        .stage_paths(&root.path, &[LOCK_PATH, marker_path.as_str()])
        .unwrap();
    EvidenceFixture {
        root,
        backend,
        model,
    }
}
