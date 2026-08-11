mod entry;
mod faults;
mod invariants;
mod phases;
mod real_git;
mod recovery;
mod root_fault_matrix;
mod root_successor_matrix;

use std::fs;

use crate::git::{Git2Backend, GitBackend};
use crate::operation::{ActionKind, OperationContext};
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, PreservationPublicationHandoffV1, PublicationIndexFormV1,
    PublicationPrefixV1,
};
use crate::workspace_ops::merge::{
    MergeTargetKind, OperationState, ParticipantState, PublicationCandidateHash,
    PublicationProgress, PublicationStep,
};
use crate::workspace_ops::tests::{TempDir, commit_file};
use sha2::{Digest, Sha256};

struct PreservationFixture {
    root: TempDir,
    backend: Git2Backend,
    member: std::path::PathBuf,
    before: String,
    result: String,
    protected: String,
    model: MergeOperationRecordV1,
}

impl PreservationFixture {
    fn current(&self) -> crate::workspace_ops::merge::v1_lifecycle::checked::StoredV1Record {
        crate::workspace_ops::merge::v1_lifecycle::checked::StoredV1Record::for_test(
            &self.root.path,
            self.model.clone(),
        )
        .unwrap()
    }

    fn seed_open(&self) {
        let merge_root = self.root.path.join(".gwz/merge");
        fs::create_dir_all(&merge_root).unwrap();
        fs::write(
            merge_root.join(format!("{}.yaml", self.model.merge_id)),
            serde_yaml::to_string(&self.model).unwrap(),
        )
        .unwrap();
    }

    fn context(&self) -> OperationContext {
        OperationContext {
            operation_id: self.model.merge_id.clone(),
            request_id: format!("req_{}", self.model.merge_id),
            schema_version: "gwz.protocol/v0".into(),
            action: ActionKind::Merge,
            dry_run: false,
            attribution: None,
        }
    }
}

fn integrated_fixture(name: &str) -> PreservationFixture {
    let root = TempDir::new(name);
    fs::create_dir_all(root.path.join(crate::stash::STASH_BUNDLE_DIR)).unwrap();
    let backend = Git2Backend::new();
    let member = root.path.join("members/a");
    backend.create_repo(&member).unwrap();
    let before = commit_file(&member, "README.md", "before\n", "before", &[]).unwrap();
    let result = commit_file(
        &member,
        "README.md",
        "merged\n",
        "merge result",
        &[before.parse().unwrap()],
    )
    .unwrap();
    let protected = commit_file(
        &member,
        "feature.txt",
        "post-merge commit\n",
        "post merge",
        &[result.parse().unwrap()],
    )
    .unwrap();

    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::Preserving;
    model.preservation_publication_handoff = Some(PreservationPublicationHandoffV1::NoCandidate);
    model.pending_preservation = None;
    model.pending_rollback = None;
    model.selected_targets = vec!["mem_a".into()];
    let row = model.participants.get_mut("mem_a").unwrap();
    row.path = "members/a".into();
    row.target_kind = MergeTargetKind::Member;
    row.target_branch = "main".into();
    row.before_commit = before.clone();
    row.source_commit = result.clone();
    row.state = ParticipantState::FastForwarded;
    row.resulting_commit = Some(result.clone());
    row.expected_merge_head = None;
    row.conflict_paths.clear();
    row.conflict_snapshot.clear();
    row.error = None;
    row.pending_action = None;
    row.preservation.clear();

    PreservationFixture {
        root,
        backend,
        member,
        before,
        result,
        protected,
        model,
    }
}

fn dirty_integrated_fixture(name: &str) -> PreservationFixture {
    let fixture = integrated_fixture(name);
    fs::write(fixture.member.join("README.md"), "unstaged user work\n").unwrap();
    fs::write(fixture.member.join("staged.txt"), "staged user work\n").unwrap();
    fixture
        .backend
        .stage_paths(&fixture.member, &["staged.txt"])
        .unwrap();
    fs::write(
        fixture.member.join("untracked.txt"),
        "untracked user work\n",
    )
    .unwrap();
    fixture
}

fn add_integrated_member(
    fixture: &mut PreservationFixture,
    member_id: &str,
    relative_path: &str,
) -> (std::path::PathBuf, String, String, String) {
    let path = fixture.root.path.join(relative_path);
    fixture.backend.create_repo(&path).unwrap();
    let before = commit_file(&path, "README.md", "before b\n", "before b", &[]).unwrap();
    let result = commit_file(
        &path,
        "README.md",
        "merged b\n",
        "merge result b",
        &[before.parse().unwrap()],
    )
    .unwrap();
    let protected = commit_file(
        &path,
        "protected-b.txt",
        "post-merge b\n",
        "post merge b",
        &[result.parse().unwrap()],
    )
    .unwrap();
    let mut row = fixture.model.participants["mem_a"].clone();
    row.path = relative_path.into();
    row.before_commit = before.clone();
    row.source_commit = result.clone();
    row.state = ParticipantState::FastForwarded;
    row.resulting_commit = Some(result.clone());
    row.preservation.clear();
    fixture.model.selected_targets.push(member_id.into());
    fixture.model.participants.insert(member_id.into(), row);

    let mut manifest = crate::artifact::ManifestArtifact::from_yaml(
        fixture.model.baseline.manifest_yaml.as_deref().unwrap(),
    )
    .unwrap();
    let mut manifest_member = manifest.members[0].clone();
    manifest_member.id = member_id.into();
    manifest_member.path = relative_path.into();
    manifest_member.source_id = format!("src_{member_id}");
    manifest.members.push(manifest_member);
    let manifest = manifest.to_yaml().unwrap();
    fixture.model.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest.as_bytes()));
    fixture.model.baseline.manifest_yaml = Some(manifest);

    (path, before, result, protected)
}

struct RootPreservationFixture {
    base: PreservationFixture,
    anchor: String,
    protected: String,
}

fn dirty_root_handoff_fixture(name: &str) -> RootPreservationFixture {
    dirty_root_handoff_fixture_with_owner(name, false)
}

fn dirty_selected_root_handoff_fixture(name: &str) -> RootPreservationFixture {
    dirty_root_handoff_fixture_with_owner(name, true)
}

fn dirty_root_handoff_fixture_with_owner(
    name: &str,
    selected_root_owner: bool,
) -> RootPreservationFixture {
    let mut base = integrated_fixture(name);
    base.backend
        .set_branch_target_checked(&base.member, "main", &base.protected, &base.result)
        .unwrap();
    base.backend.create_repo(&base.root.path).unwrap();
    fs::create_dir_all(base.root.path.join("gwz.conf")).unwrap();
    let manifest = base.model.baseline.manifest_yaml.clone().unwrap();
    let lock = base.model.baseline.lock_yaml.clone().unwrap();
    let first = commit_file(
        &base.root.path,
        crate::workspace::WORKSPACE_MANIFEST,
        &manifest,
        "baseline manifest",
        &[],
    )
    .unwrap();
    let root_baseline = commit_file(
        &base.root.path,
        crate::artifact::LOCK_PATH,
        &lock,
        "baseline lock",
        &[first.parse().unwrap()],
    )
    .unwrap();
    base.model.baseline.root_head = Some(root_baseline.clone());
    base.model.baseline.root_branch = Some("main".into());
    base.model.state = OperationState::Finalizing;
    base.model.preservation_publication_handoff = None;
    let mut publication_parent = root_baseline.clone();

    if selected_root_owner {
        base.model.baseline.manifest_commit_sha256 =
            Some(format!("{:x}", Sha256::digest(manifest.as_bytes())));
        base.model.baseline.lock_commit_sha256 =
            Some(format!("{:x}", Sha256::digest(lock.as_bytes())));
        let root_result = commit_file(
            &base.root.path,
            "selected-root.txt",
            "selected root result\n",
            "selected root result",
            &[root_baseline.parse().unwrap()],
        )
        .unwrap();
        let mut row = base.model.participants["mem_a"].clone();
        row.path = ".".into();
        row.target_kind = MergeTargetKind::Root;
        row.target_branch = "main".into();
        row.before_commit = root_baseline.clone();
        row.source_commit = root_result.clone();
        row.state = ParticipantState::FastForwarded;
        row.resulting_commit = Some(root_result.clone());
        row.expected_merge_head = None;
        row.conflict_paths.clear();
        row.conflict_snapshot.clear();
        row.error = None;
        row.pending_action = None;
        row.preservation.clear();
        publication_parent = root_result;
        base.model.selected_targets.push("@root".into());
        base.model.participants.insert("@root".into(), row);
    }

    let current = base.current();
    let accepted =
        crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::accepted_workspace(&current);
    let mut candidate =
        crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::candidate_payload(&current);
    let boundary_path = crate::workspace_ops::workspace_exclude_path(&base.root.path);
    let baseline_boundary = fs::read_to_string(&boundary_path).unwrap();
    let boundary = format!("{baseline_boundary}/members/a/\n/.gwz/\n");
    candidate.candidate.baseline_boundary_sha256 =
        format!("{:x}", Sha256::digest(baseline_boundary.as_bytes()));
    candidate.candidate.baseline_boundary_text = baseline_boundary;
    candidate.candidate.boundary_text = boundary.clone();
    candidate.candidate.boundary_sha256 = format!("{:x}", Sha256::digest(boundary.as_bytes()));
    base.model.accepted_workspace = Some(accepted);
    base.model.publication = Some(PublicationProgress {
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
    let files = crate::workspace_ops::merge::acceptance::v1_candidate_files(&base.model).unwrap();
    let message = crate::workspace_ops::merge::acceptance::v1_composition_message(&base.model);
    let evidence = base
        .backend
        .commit_gwz_paths_checked(&base.root.path, Some(&publication_parent), &files, &message)
        .unwrap();
    let anchor = evidence.commit.clone();
    let root_merge_commit = base
        .model
        .participants
        .get("@root")
        .and_then(|row| row.resulting_commit.clone());
    let publication = base.model.publication.as_mut().unwrap();
    publication.step = PublicationStep::PublishingCandidate;
    publication.root_merge_commit = root_merge_commit;
    publication.composition_commit = Some(evidence.commit.clone());
    publication.composition_tree = Some(evidence.tree);
    publication.candidate_hashes = evidence
        .candidate_hashes
        .into_iter()
        .map(|hash| PublicationCandidateHash {
            path: hash.path,
            sha256: hash.sha256,
        })
        .collect();

    let repository = git2::Repository::open(&base.root.path).unwrap();
    repository
        .find_reference("refs/heads/main")
        .unwrap()
        .set_target(anchor.parse().unwrap(), "install composition fixture")
        .unwrap();
    repository.set_head("refs/heads/main").unwrap();
    let publication = base.model.publication.as_ref().unwrap();
    let candidate = publication.candidate.as_ref().unwrap();
    let marker_path = publication.candidate_marker_path.as_ref().unwrap();
    fs::create_dir_all(base.root.path.join(marker_path).parent().unwrap()).unwrap();
    fs::write(
        base.root.path.join(marker_path),
        candidate.marker_yaml.as_bytes(),
    )
    .unwrap();
    fs::write(
        base.root.path.join(crate::artifact::LOCK_PATH),
        candidate.lock_yaml.as_bytes(),
    )
    .unwrap();
    base.backend
        .stage_paths(
            &base.root.path,
            &[marker_path.as_str(), crate::artifact::LOCK_PATH],
        )
        .unwrap();
    crate::workspace_ops::publish_workspace_exclude_candidate(&base.root.path, &boundary).unwrap();
    let protected = commit_file(
        &base.root.path,
        "root-protected.txt",
        "protected root commit\n",
        "protected root",
        &[anchor.parse().unwrap()],
    )
    .unwrap();
    fs::write(
        base.root.path.join("root-protected.txt"),
        "unstaged root work\n",
    )
    .unwrap();
    fs::write(base.root.path.join("root-staged.txt"), "staged root work\n").unwrap();
    base.backend
        .stage_paths(&base.root.path, &["root-staged.txt"])
        .unwrap();
    fs::write(
        base.root.path.join("root-untracked.txt"),
        "untracked root work\n",
    )
    .unwrap();

    base.model.state = OperationState::Preserving;
    base.model.preservation_publication_handoff =
        Some(PreservationPublicationHandoffV1::Candidate {
            prefix: PublicationPrefixV1::Boundary,
            index: PublicationIndexFormV1::Staged,
        });
    RootPreservationFixture {
        base,
        anchor,
        protected,
    }
}
