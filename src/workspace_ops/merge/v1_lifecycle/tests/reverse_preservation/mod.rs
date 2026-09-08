mod durable_cursor;
mod entry;
mod factory_contract;
mod faults;
mod invariants;
mod phases;
mod real_git;
mod recovery;
mod root_ambiguity_matrix;
mod root_durability;
mod root_fault_matrix;
mod root_successor_matrix;

use crate::filesystem::{FileSystem, make_filesystem};
use crate::git::{
    GitBackend, GitTestRepository, TestCommitSpec, TestHead, TestRefTarget, TestRepoSpec,
    make_repository,
};
use crate::operation::{ActionKind, OperationContext};
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, PreservationPublicationHandoffV1, PublicationIndexFormV1,
    PublicationPrefixV1,
};
use crate::workspace_ops::merge::{
    MergeTargetKind, OperationState, ParticipantState, PublicationCandidateHash,
    PublicationProgress, PublicationStep,
};
use crate::workspace_ops::tests::TempDir;
use sha2::{Digest, Sha256};

mod fs {
    use super::*;
    use std::io;

    pub(super) fn create_dir_all(path: impl AsRef<std::path::Path>) -> io::Result<()> {
        make_filesystem().create_directories(path.as_ref())
    }

    pub(super) fn write(
        path: impl AsRef<std::path::Path>,
        bytes: impl AsRef<[u8]>,
    ) -> io::Result<()> {
        let filesystem = make_filesystem();
        let path = path.as_ref();
        match filesystem.remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let file = filesystem.create_file(path)?;
        filesystem.write_all(&file, bytes.as_ref())
    }

    pub(super) fn read(path: impl AsRef<std::path::Path>) -> io::Result<Vec<u8>> {
        make_filesystem().read(path.as_ref())
    }

    pub(super) fn read_to_string(path: impl AsRef<std::path::Path>) -> io::Result<String> {
        String::from_utf8(read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub(super) fn remove_file(path: impl AsRef<std::path::Path>) -> io::Result<()> {
        make_filesystem().remove_file(path.as_ref())
    }

    pub(super) fn remove_dir(path: impl AsRef<std::path::Path>) -> io::Result<()> {
        make_filesystem().remove_directory(path.as_ref())
    }

    pub(super) fn exists(path: impl AsRef<std::path::Path>) -> bool {
        make_filesystem().metadata(path.as_ref()).is_ok()
    }
}

use std::io;
struct FixtureFs<'a>(&'a dyn FileSystem);
impl FixtureFs<'_> {
    fn create_dir_all(&self, path: impl AsRef<std::path::Path>) -> io::Result<()> {
        self.0.create_directories(path.as_ref())
    }

    fn write(&self, path: impl AsRef<std::path::Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
        let filesystem = self.0;
        let path = path.as_ref();
        match filesystem.remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let file = filesystem.create_file(path)?;
        filesystem.write_all(&file, bytes.as_ref())
    }

    fn read(&self, path: impl AsRef<std::path::Path>) -> io::Result<Vec<u8>> {
        self.0.read(path.as_ref())
    }

    fn read_to_string(&self, path: impl AsRef<std::path::Path>) -> io::Result<String> {
        String::from_utf8(self.read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) struct PreservationFixture<
    B = GitTestRepository,
> {
    services: crate::operation_context::OperationServices,
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) root: TempDir,
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) backend: B,
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) member: std::path::PathBuf,
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) before: String,
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) result: String,
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) protected: String,
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) model: MergeOperationRecordV1,
}

impl<B> PreservationFixture<B> {
    fn current(&self) -> crate::workspace_ops::merge::v1_lifecycle::checked::StoredV1Record {
        crate::workspace_ops::merge::v1_lifecycle::checked::StoredV1Record::from_open_bytes_in(
            &self.services,
            &self.root.path,
            &self
                .root
                .path
                .join(".gwz/merge")
                .join(format!("{}.yaml", self.model.merge_id)),
            serde_yaml::to_string(&self.model).unwrap().as_bytes(),
        )
        .unwrap()
    }

    fn seed_open(&self) {
        let fs = FixtureFs(self.services.filesystem());
        let merge_root = self.root.path.join(".gwz/merge");
        fs.create_dir_all(&merge_root).unwrap();
        fs.write(
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
    integrated_fixture_using(name, make_repository())
}

fn integrated_fixture_using<B: GitBackend + crate::git::MergeAuthorityBackend + Clone>(
    name: &str,
    backend: B,
) -> PreservationFixture<B> {
    integrated_fixture_in(name, backend.clone(), backend.operation_services())
}

fn integrated_fixture_in<B: GitBackend>(
    _name: &str,
    backend: B,
    services: crate::operation_context::OperationServices,
) -> PreservationFixture<B> {
    let fs = FixtureFs(services.filesystem());
    let workspace = services.filesystem().test_workspace().unwrap();
    let root = TempDir {
        path: workspace.path().to_path_buf(),
        _workspace: workspace,
    };
    backend
        .test_init_repo(&root.path, &TestRepoSpec::default())
        .unwrap();
    fs.create_dir_all(root.path.join(crate::stash::STASH_BUNDLE_DIR))
        .unwrap();
    let member = root.path.join("members/a");
    backend
        .test_init_repo(&member, &TestRepoSpec::default())
        .unwrap();
    let before = fixture_commit_file_in(
        fs.0,
        &backend,
        &member,
        "README.md",
        "before\n",
        "before",
        &[],
    )
    .unwrap();
    let result = fixture_commit_file_in(
        fs.0,
        &backend,
        &member,
        "README.md",
        "merged\n",
        "merge result",
        std::slice::from_ref(&before),
    )
    .unwrap();
    let protected = fixture_commit_file_in(
        fs.0,
        &backend,
        &member,
        "feature.txt",
        "post-merge commit\n",
        "post merge",
        std::slice::from_ref(&result),
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
        services,
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

fn add_integrated_member<B: GitBackend>(
    fixture: &mut PreservationFixture<B>,
    member_id: &str,
    relative_path: &str,
) -> (std::path::PathBuf, String, String, String) {
    let path = fixture.root.path.join(relative_path);
    let fs = FixtureFs(fixture.services.filesystem());
    fixture
        .backend
        .test_init_repo(&path, &TestRepoSpec::default())
        .unwrap();
    let before = fixture_commit_file_in(
        fs.0,
        &fixture.backend,
        &path,
        "README.md",
        "before b\n",
        "before b",
        &[],
    )
    .unwrap();
    let result = fixture_commit_file_in(
        fs.0,
        &fixture.backend,
        &path,
        "README.md",
        "merged b\n",
        "merge result b",
        std::slice::from_ref(&before),
    )
    .unwrap();
    let protected = fixture_commit_file_in(
        fs.0,
        &fixture.backend,
        &path,
        "protected-b.txt",
        "post-merge b\n",
        "post merge b",
        std::slice::from_ref(&result),
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

pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) struct RootPreservationFixture<
    B = GitTestRepository,
> {
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) base: PreservationFixture<B>,
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) anchor: String,
    pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) protected: String,
}

fn dirty_root_handoff_fixture(name: &str) -> RootPreservationFixture {
    dirty_root_handoff_fixture_with_owner(name, false, false, false)
}

pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) fn dirty_selected_root_handoff_fixture(
    name: &str,
) -> RootPreservationFixture {
    dirty_root_handoff_fixture_with_owner(name, true, false, false)
}

pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) fn dirty_root_degenerate_handoff_fixture(
    name: &str,
    selected_root_owner: bool,
) -> RootPreservationFixture {
    dirty_root_handoff_fixture_with_owner(name, selected_root_owner, false, true)
}

pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) fn dirty_selected_root_handoff_fixture_with_later_member(
    name: &str,
) -> RootPreservationFixture {
    dirty_root_handoff_fixture_with_owner(name, true, true, false)
}

fn dirty_root_handoff_fixture_with_owner(
    name: &str,
    selected_root_owner: bool,
    include_later_member: bool,
    degenerate_candidate: bool,
) -> RootPreservationFixture {
    dirty_root_handoff_fixture_using(
        name,
        selected_root_owner,
        include_later_member,
        degenerate_candidate,
        make_repository(),
    )
}

fn dirty_root_handoff_fixture_using<B: GitBackend + crate::git::MergeAuthorityBackend + Clone>(
    name: &str,
    selected_root_owner: bool,
    include_later_member: bool,
    degenerate_candidate: bool,
    backend: B,
) -> RootPreservationFixture<B> {
    dirty_root_handoff_fixture_in(
        name,
        selected_root_owner,
        include_later_member,
        degenerate_candidate,
        backend.clone(),
        backend.operation_services(),
    )
}

fn dirty_root_handoff_fixture_in<B: GitBackend>(
    name: &str,
    selected_root_owner: bool,
    include_later_member: bool,
    degenerate_candidate: bool,
    backend: B,
    services: crate::operation_context::OperationServices,
) -> RootPreservationFixture<B> {
    let mut base = integrated_fixture_in(name, backend, services.clone());
    let fs = FixtureFs(services.filesystem());
    base.backend
        .set_branch_target_checked(&base.member, "main", &base.protected, &base.result)
        .unwrap();
    if include_later_member {
        add_integrated_member(&mut base, "mem_z", "members/z");
    }
    fs.create_dir_all(base.root.path.join("gwz.conf")).unwrap();
    let manifest = base.model.baseline.manifest_yaml.clone().unwrap();
    if degenerate_candidate {
        let mut lock = crate::artifact::LockArtifact::from_yaml(
            base.model.baseline.lock_yaml.as_deref().unwrap(),
        )
        .unwrap();
        let manifest_artifact = crate::artifact::ManifestArtifact::from_yaml(&manifest).unwrap();
        let member = manifest_artifact
            .members
            .iter()
            .find(|member| member.id == "mem_a")
            .unwrap();
        let row = lock.members.entry("mem_a".into()).or_default();
        row.path = member.path.clone();
        row.source_id = Some(member.source_id.clone());
        row.source_kind = member.source_kind;
        row.commit = Some(if selected_root_owner {
            base.before.clone()
        } else {
            base.result.clone()
        });
        row.branch = Some("main".into());
        row.detached = Some(false);
        row.dirty = Some(false);
        row.materialized = Some(true);
        let lock = lock.to_yaml().unwrap();
        base.model.baseline.lock_sha256 = format!("{:x}", Sha256::digest(lock.as_bytes()));
        base.model.baseline.lock_yaml = Some(lock);
    }
    let lock = base.model.baseline.lock_yaml.clone().unwrap();
    crate::workspace_ops::ensure_workspace_exclude_in(
        fs.0,
        &base.backend,
        &base.root.path,
        &crate::artifact::ManifestArtifact::from_yaml(&manifest).unwrap(),
        &crate::artifact::LockArtifact::from_yaml(&lock).unwrap(),
    )
    .unwrap();
    let first = fixture_commit_file_in(
        fs.0,
        &base.backend,
        &base.root.path,
        crate::workspace::WORKSPACE_MANIFEST,
        &manifest,
        "baseline manifest",
        &[],
    )
    .unwrap();
    let root_baseline = fixture_commit_file_in(
        fs.0,
        &base.backend,
        &base.root.path,
        crate::artifact::LOCK_PATH,
        &lock,
        "baseline lock",
        std::slice::from_ref(&first),
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
        let root_result = fixture_commit_file_in(
            fs.0,
            &base.backend,
            &base.root.path,
            "selected-root.txt",
            "selected root result\n",
            "selected root result",
            std::slice::from_ref(&root_baseline),
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

    if degenerate_candidate && selected_root_owner {
        base.backend
            .set_branch_target_checked(&base.member, "main", &base.result, &base.before)
            .unwrap();
        let row = base.model.participants.get_mut("mem_a").unwrap();
        row.state = ParticipantState::UpToDate;
        row.resulting_commit = Some(row.before_commit.clone());
    }

    let current = base.current();
    let accepted =
        crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::accepted_workspace(&current);
    let mut candidate =
        crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::candidate_payload(&current);
    let boundary_path = crate::workspace_ops::workspace_exclude_path(&base.root.path);
    let baseline_boundary = fs.read_to_string(&boundary_path).unwrap();
    let manifest_artifact = crate::artifact::ManifestArtifact::from_yaml(&manifest).unwrap();
    let mut boundary = baseline_boundary.clone();
    if !degenerate_candidate {
        for member in manifest_artifact
            .members
            .iter()
            .filter(|member| member.active)
        {
            boundary.push('/');
            boundary.push_str(member.path.trim_matches('/'));
            boundary.push_str("/\n");
        }
        boundary.push_str("/.gwz/\n");
    }
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

    base.backend
        .test_set_ref(
            &base.root.path,
            "refs/heads/main",
            Some(&TestRefTarget::Direct(anchor.clone())),
        )
        .unwrap();
    base.backend
        .test_set_head(
            &base.root.path,
            &TestHead::Attached("refs/heads/main".into()),
        )
        .unwrap();
    let publication = base.model.publication.as_ref().unwrap();
    let candidate = publication.candidate.as_ref().unwrap();
    let marker_path = publication.candidate_marker_path.as_ref().unwrap();
    fs.create_dir_all(base.root.path.join(marker_path).parent().unwrap())
        .unwrap();
    fs.write(
        base.root.path.join(marker_path),
        candidate.marker_yaml.as_bytes(),
    )
    .unwrap();
    fs.write(
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
    crate::workspace_ops::publish_workspace_exclude_candidate_in(fs.0, &base.root.path, &boundary)
        .unwrap();
    let protected = fixture_commit_file_in(
        fs.0,
        &base.backend,
        &base.root.path,
        "root-protected.txt",
        "protected root commit\n",
        "protected root",
        std::slice::from_ref(&anchor),
    )
    .unwrap();
    fs.write(
        base.root.path.join("root-protected.txt"),
        "unstaged root work\n",
    )
    .unwrap();
    fs.write(base.root.path.join("root-staged.txt"), "staged root work\n")
        .unwrap();
    base.backend
        .stage_paths(&base.root.path, &["root-staged.txt"])
        .unwrap();
    fs.write(
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

fn install_root_handoff<B: GitBackend>(
    fixture: &mut RootPreservationFixture<B>,
    prefix: PublicationPrefixV1,
    index: PublicationIndexFormV1,
) {
    let publication = fixture.base.model.publication.as_ref().unwrap();
    let marker_path = publication.candidate_marker_path.as_ref().unwrap().clone();
    let candidate = publication.candidate.as_ref().unwrap().clone();

    let mut baseline_boundary = candidate.baseline_boundary_text.clone();
    let manifest = crate::artifact::ManifestArtifact::from_yaml(
        fixture
            .base
            .model
            .baseline
            .manifest_yaml
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    let mut required = vec![".gwz".to_owned(), "gwz.conf/.tmp".to_owned()];
    required.extend(
        manifest
            .members
            .into_iter()
            .filter(|member| member.active)
            .map(|member| member.path),
    );
    for path in required {
        let line = format!("/{}/", path.trim_matches('/'));
        if !baseline_boundary.lines().any(|actual| actual == line) {
            if !baseline_boundary.ends_with('\n') {
                baseline_boundary.push('\n');
            }
            baseline_boundary.push_str(&line);
            baseline_boundary.push('\n');
        }
    }
    let publication = fixture.base.model.publication.as_mut().unwrap();
    let stored = publication.candidate.as_mut().unwrap();
    stored.baseline_boundary_sha256 = format!("{:x}", Sha256::digest(baseline_boundary.as_bytes()));
    stored.baseline_boundary_text = baseline_boundary.clone();

    let marker = fixture.base.root.path.join(&marker_path);
    fs::create_dir_all(marker.parent().unwrap()).unwrap();
    if fs::exists(&marker) {
        fs::remove_file(&marker).unwrap();
    }
    fs::write(
        fixture.base.root.path.join(crate::artifact::LOCK_PATH),
        &candidate.baseline_lock_yaml,
    )
    .unwrap();
    fixture
        .base
        .backend
        .stage_paths(
            &fixture.base.root.path,
            &[marker_path.as_str(), crate::artifact::LOCK_PATH],
        )
        .unwrap();

    if prefix != PublicationPrefixV1::Baseline {
        fs::write(&marker, &candidate.marker_yaml).unwrap();
    }
    if matches!(
        prefix,
        PublicationPrefixV1::Lock | PublicationPrefixV1::Boundary
    ) {
        fs::write(
            fixture.base.root.path.join(crate::artifact::LOCK_PATH),
            &candidate.lock_yaml,
        )
        .unwrap();
    }
    if index == PublicationIndexFormV1::Staged {
        fixture
            .base
            .backend
            .stage_paths(
                &fixture.base.root.path,
                &[marker_path.as_str(), crate::artifact::LOCK_PATH],
            )
            .unwrap();
    }
    crate::workspace_ops::publish_workspace_exclude_candidate(
        &fixture.base.root.path,
        if prefix == PublicationPrefixV1::Boundary {
            &candidate.boundary_text
        } else {
            &baseline_boundary
        },
    )
    .unwrap();
    if prefix == PublicationPrefixV1::Baseline {
        fs::remove_dir(marker.parent().unwrap()).unwrap();
    }
    fixture.base.model.preservation_publication_handoff =
        Some(PreservationPublicationHandoffV1::Candidate { prefix, index });
}

fn install_selected_root_no_candidate_handoff<B>(fixture: &mut RootPreservationFixture<B>) {
    let selected_anchor = fixture.base.model.participants["@root"]
        .resulting_commit
        .clone()
        .unwrap();
    fixture.anchor = selected_anchor;
    fixture.base.model.publication = None;
    fixture.base.model.preservation_publication_handoff =
        Some(PreservationPublicationHandoffV1::NoCandidate);
}

fn evidence_pending_non_root_fixture(name: &str) -> PreservationFixture {
    let mut fixture = dirty_root_handoff_fixture(name);
    install_root_handoff(
        &mut fixture,
        PublicationPrefixV1::Baseline,
        PublicationIndexFormV1::Pre,
    );

    let baseline = fixture.base.model.baseline.root_head.clone().unwrap();
    fixture
        .base
        .backend
        .test_force_checkout(&fixture.base.root.path, &baseline)
        .unwrap();
    let root_untracked = fixture.base.root.path.join("root-untracked.txt");
    if fs::exists(&root_untracked) {
        fs::remove_file(root_untracked).unwrap();
    }

    let publication = fixture.base.model.publication.as_mut().unwrap();
    publication.step = PublicationStep::CommittingEvidence;
    publication.root_merge_commit = None;
    publication.composition_commit = None;
    publication.composition_tree = None;
    publication.candidate_hashes.clear();
    publication.evidence_rolled_back = false;
    publication.root_preservation.clear();
    publication.preservation_prefix = None;
    fixture.base.model.preservation_publication_handoff =
        Some(PreservationPublicationHandoffV1::EvidencePending);

    fs::write(
        fixture.base.member.join("README.md"),
        "unstaged user work\n",
    )
    .unwrap();
    fs::write(fixture.base.member.join("staged.txt"), "staged user work\n").unwrap();
    fixture
        .base
        .backend
        .stage_paths(&fixture.base.member, &["staged.txt"])
        .unwrap();
    fs::write(
        fixture.base.member.join("untracked.txt"),
        "untracked user work\n",
    )
    .unwrap();

    fixture.base
}

fn fixture_commit_file<B: GitBackend>(
    backend: &B,
    path: &std::path::Path,
    relative: &str,
    contents: &str,
    message: &str,
    parents: &[String],
) -> crate::model::ModelResult<String> {
    fixture_commit_file_in(
        &make_filesystem(),
        backend,
        path,
        relative,
        contents,
        message,
        parents,
    )
}

fn fixture_commit_file_in<B: GitBackend>(
    filesystem: &dyn FileSystem,
    backend: &B,
    path: &std::path::Path,
    relative: &str,
    contents: &str,
    message: &str,
    parents: &[String],
) -> crate::model::ModelResult<String> {
    FixtureFs(filesystem)
        .write(path.join(relative), contents)
        .unwrap();
    backend.stage_paths(path, &[relative])?;
    let commit =
        backend.test_create_commit(path, &TestCommitSpec::from_index(message, parents.to_vec()))?;
    backend.test_set_ref(
        path,
        "refs/heads/main",
        Some(&TestRefTarget::Direct(commit.clone())),
    )?;
    backend.test_set_head(path, &TestHead::Attached("refs/heads/main".into()))?;
    Ok(commit)
}
