use super::*;
use crate::model::ErrorCode;
use crate::operation::{ActionKind, OperationContext};
use crate::workspace_ops::merge::model::v1::AcceptedMetadataSourceV1;
use crate::workspace_ops::merge::model::v1::AcceptedRootBaseV1;
use crate::workspace_ops::merge::v1_lifecycle::authority::V1LifecycleRequest;
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::run_test as run;
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;
use crate::workspace_ops::merge::{PublicationCandidateHash, PublicationProgress, PublicationStep};
use sha2::{Digest, Sha256};

#[test]
fn selected_root_service_entry_rejects_semantic_drift_without_mutation() {
    let fixture = service_fixture("v1-rollback-service-root-semantic-index");
    seed_open(&fixture);
    let status = std::process::Command::new("git")
        .args(["update-index", "--skip-worktree", "selected-root.txt"])
        .current_dir(&fixture.root.path)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::write(
        fixture.root.path.join("selected-root.txt"),
        "hidden selected-root drift\n",
    )
    .unwrap();
    assert_entry_rejected_without_mutation(&fixture, "semantic index drift", "@root");
}

#[test]
fn selected_root_service_entry_proves_result_artifacts_without_mutation() {
    for case in [ResultArtifactCase::Missing, ResultArtifactCase::NonUtf8] {
        let mut fixture = service_fixture(&format!("v1-rollback-service-result-{case:?}"));
        install_invalid_result_commit(&mut fixture, case);
        seed_open(&fixture);
        assert_entry_rejected_without_mutation(&fixture, &format!("{case:?}"), "@root");
    }

    let mut mismatch = service_fixture("v1-rollback-service-result-metadata-mismatch");
    mismatch
        .model
        .accepted_workspace
        .as_mut()
        .unwrap()
        .metadata_base
        .manifest_exact_yaml
        .push_str("# foreign accepted metadata\n");
    let accepted = mismatch.model.accepted_workspace.as_mut().unwrap();
    accepted.metadata_base.manifest_sha256 = format!(
        "{:x}",
        Sha256::digest(accepted.metadata_base.manifest_exact_yaml.as_bytes())
    );
    rebuild_publication(&mut mismatch);
    seed_open(&mismatch);
    assert_entry_rejected_without_mutation(&mismatch, "accepted metadata mismatch", "@root");
}

pub(super) fn assert_entry_rejected_without_mutation(
    fixture: &ServiceFixture,
    label: &str,
    expected_member: &str,
) {
    let before = NoMutationSnapshot::capture(fixture);
    let context = context(&fixture.model);
    let mut runtime = ReverseRuntime::new(&fixture.backend, &context);
    let error = match run(
        &CheckedV1Store::default(),
        &fixture.root.path,
        &fixture.model.merge_id,
        V1LifecycleRequest::Abort,
        &mut runtime,
    ) {
        Ok(_) => panic!("{label}: rollback entry unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error.code,
            ErrorCode::PreservationEvidenceMismatch
                | ErrorCode::MergeRecoveryRequired
                | ErrorCode::MergeRecordUnreadable
        ),
        "{label}: {error:?}"
    );
    assert_eq!(
        error.member_id.as_deref(),
        Some(expected_member),
        "{label}: {error:?}"
    );
    NoMutationSnapshot::capture(fixture).assert_matches(&before, label);
}

#[derive(Debug, Eq, PartialEq)]
struct NoMutationSnapshot {
    record: Vec<u8>,
    root_head: crate::git::GitHeadState,
    root_branch: Option<String>,
    root_repository_state: crate::git::GitRepositoryState,
    root_index: Vec<u8>,
    root_files: Vec<(String, Vec<u8>)>,
    members: Vec<MemberSnapshot>,
    root_stashes: Vec<crate::git::GitStashEntry>,
    bundle: Option<Vec<u8>>,
}

#[derive(Debug, Eq, PartialEq)]
struct MemberSnapshot {
    member_id: String,
    head: crate::git::GitHeadState,
    branch: Option<String>,
    repository_state: crate::git::GitRepositoryState,
    index: Vec<u8>,
    files: Vec<(String, Vec<u8>)>,
    stashes: Vec<crate::git::GitStashEntry>,
}

impl NoMutationSnapshot {
    fn capture(fixture: &ServiceFixture) -> Self {
        let members = fixture
            .model
            .participants
            .iter()
            .filter(|(_, row)| row.target_kind != MergeTargetKind::Root)
            .map(|(member_id, row)| {
                let member = fixture.root.path.join(&row.path);
                MemberSnapshot {
                    member_id: member_id.clone(),
                    head: fixture.backend.head(&member).unwrap(),
                    branch: fixture
                        .backend
                        .read_ref(&member, "refs/heads/main")
                        .unwrap(),
                    repository_state: fixture.backend.repository_state(&member).unwrap(),
                    index: std::fs::read(member.join(".git/index")).unwrap(),
                    files: files(&member),
                    stashes: fixture.backend.stash_list(&member).unwrap(),
                }
            })
            .collect();
        Self {
            record: std::fs::read(
                fixture
                    .root
                    .path
                    .join(format!(".gwz/merge/{}.yaml", fixture.model.merge_id)),
            )
            .unwrap(),
            root_head: fixture.backend.head(&fixture.root.path).unwrap(),
            root_branch: fixture
                .backend
                .read_ref(&fixture.root.path, "refs/heads/main")
                .unwrap(),
            root_repository_state: fixture
                .backend
                .repository_state(&fixture.root.path)
                .unwrap(),
            root_index: std::fs::read(fixture.root.path.join(".git/index")).unwrap(),
            root_files: files(&fixture.root.path),
            members,
            root_stashes: fixture.backend.stash_list(&fixture.root.path).unwrap(),
            bundle: std::fs::read(crate::stash::bundle_path(
                &fixture.root.path,
                &format!("stash_{}", fixture.model.merge_id),
            ))
            .ok(),
        }
    }

    fn assert_matches(&self, before: &Self, label: &str) {
        assert_eq!(self.record, before.record, "{label}: durable record");
        assert_eq!(self.root_head, before.root_head, "{label}: root HEAD");
        assert_eq!(self.root_branch, before.root_branch, "{label}: root ref");
        assert_eq!(
            self.root_repository_state, before.root_repository_state,
            "{label}: root native state"
        );
        assert_eq!(self.root_index, before.root_index, "{label}: root index");
        assert_eq!(self.root_files, before.root_files, "{label}: root files");
        assert_eq!(self.members, before.members, "{label}: member state");
        assert_eq!(
            self.root_stashes, before.root_stashes,
            "{label}: root stashes"
        );
        assert_eq!(self.bundle, before.bundle, "{label}: preservation bundle");
    }
}

fn files(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    collect_files(root, root, &mut out);
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

fn collect_files(
    root: &std::path::Path,
    current: &std::path::Path,
    out: &mut Vec<(String, Vec<u8>)>,
) {
    for entry in std::fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap();
        // Control state this snapshot deliberately does not weigh: it is proved
        // by the fields beside `root_files`, not by the file set. `.git` is
        // covered by `root_head`/`root_index`/`root_repository_state`,
        // `.gwz/merge` by `record`, and `.gwz/locks` by the operation prologue
        // that creates it. R2-E E4.1 adds `.gwz/catalog-final` to exactly that
        // class: the SAME prologue (`V1MutationLease::acquire`) that creates
        // the lock now also activates the catalog, and the catalog is
        // exact-managed control state with a closed interior grammar that
        // converges on every drive. What these rows prove — that a refused
        // operation moves no merge, member or native Git state — is untouched.
        if relative.components().any(|part| part.as_os_str() == ".git")
            || relative.starts_with(".gwz/merge")
            || relative.starts_with(".gwz/locks")
            || relative.starts_with(".gwz/catalog-final")
        {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            collect_files(root, &path, out);
        } else if metadata.file_type().is_symlink() {
            out.push((
                relative.to_string_lossy().into_owned(),
                std::fs::read_link(path)
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec(),
            ));
        } else {
            out.push((
                relative.to_string_lossy().into_owned(),
                std::fs::read(path).unwrap(),
            ));
        }
    }
}

pub(super) struct ServiceFixture {
    pub(super) root: TempDir,
    pub(super) backend: Git2Backend,
    pub(super) model: MergeOperationRecordV1,
}

pub(super) fn service_fixture(name: &str) -> ServiceFixture {
    finish_service_fixture(
        crate::workspace_ops::merge::v1_lifecycle::reverse::preservation::tests::dirty_selected_root_handoff_fixture(name),
    )
}

pub(super) fn service_fixture_with_later_member(name: &str) -> ServiceFixture {
    finish_service_fixture(
        crate::workspace_ops::merge::v1_lifecycle::reverse::preservation::tests::dirty_selected_root_handoff_fixture_with_later_member(name),
    )
}

fn finish_service_fixture(
    mut fixture: crate::workspace_ops::merge::v1_lifecycle::reverse::preservation::tests::RootPreservationFixture,
) -> ServiceFixture {
    let root_repo = git2::Repository::open(&fixture.base.root.path).unwrap();
    let root_anchor = root_repo
        .find_object(fixture.anchor.parse().unwrap(), None)
        .unwrap();
    root_repo
        .reset(&root_anchor, git2::ResetType::Hard, None)
        .unwrap();
    for member_id in fixture.base.model.selected_targets.clone() {
        let row = &fixture.base.model.participants[&member_id];
        if row.target_kind == MergeTargetKind::Root {
            continue;
        }
        let member_repo = git2::Repository::open(fixture.base.root.path.join(&row.path)).unwrap();
        let result = row.resulting_commit.as_deref().unwrap();
        let member_result = member_repo
            .find_object(result.parse().unwrap(), None)
            .unwrap();
        member_repo
            .reset(&member_result, git2::ResetType::Hard, None)
            .unwrap();
    }
    for path in [
        "root-untracked.txt",
        "root-staged.txt",
        "root-protected.txt",
    ] {
        let _ = std::fs::remove_file(fixture.base.root.path.join(path));
    }
    let _ = std::fs::remove_file(fixture.base.member.join("untracked.txt"));
    fixture.base.model.state = OperationState::Finalizing;
    fixture.base.model.pending_preservation = None;
    fixture.base.model.pending_rollback = None;
    fixture.base.model.preservation_publication_handoff = None;
    crate::workspace_ops::merge::model::v1::validate_v1_journal(&fixture.base.model).unwrap();
    ServiceFixture {
        root: fixture.base.root,
        backend: fixture.base.backend,
        model: fixture.base.model,
    }
}

#[derive(Clone, Copy, Debug)]
enum ResultArtifactCase {
    Missing,
    NonUtf8,
}

fn install_invalid_result_commit(fixture: &mut ServiceFixture, case: ResultArtifactCase) {
    let repo = git2::Repository::open(&fixture.root.path).unwrap();
    let result = fixture.model.participants["@root"]
        .resulting_commit
        .as_deref()
        .unwrap();
    let parent = repo.find_commit(result.parse().unwrap()).unwrap();
    let root_tree = parent.tree().unwrap();
    let manifest = std::path::Path::new(crate::workspace::WORKSPACE_MANIFEST);
    let directory_name = manifest.parent().unwrap().file_name().unwrap();
    let leaf = manifest.file_name().unwrap();
    let directory_tree = repo
        .find_tree(
            root_tree
                .get_name(directory_name.to_str().unwrap())
                .unwrap()
                .id(),
        )
        .unwrap();
    let mut directory_builder = repo.treebuilder(Some(&directory_tree)).unwrap();
    match case {
        ResultArtifactCase::Missing => {
            directory_builder.remove(leaf).unwrap();
        }
        ResultArtifactCase::NonUtf8 => {
            let blob = repo.blob(&[0xff, 0xfe, 0xfd]).unwrap();
            directory_builder.insert(leaf, blob, 0o100644).unwrap();
        }
    }
    let directory_oid = directory_builder.write().unwrap();
    let mut root_builder = repo.treebuilder(Some(&root_tree)).unwrap();
    root_builder
        .insert(directory_name, directory_oid, 0o040000)
        .unwrap();
    let tree = repo.find_tree(root_builder.write().unwrap()).unwrap();
    let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
    let commit = repo
        .commit(
            None,
            &signature,
            &signature,
            "invalid selected-root result",
            &tree,
            &[&parent],
        )
        .unwrap()
        .to_string();
    repo.find_reference("refs/heads/main")
        .unwrap()
        .set_target(commit.parse().unwrap(), "install invalid result fixture")
        .unwrap();
    let row = fixture.model.participants.get_mut("@root").unwrap();
    row.resulting_commit = Some(commit.clone());
    row.source_commit = commit.clone();
    match &mut fixture.model.accepted_workspace.as_mut().unwrap().root.base {
        AcceptedRootBaseV1::BornAttached {
            commit: accepted, ..
        } => *accepted = commit.clone(),
        _ => panic!("fixture must retain an attached selected-root result"),
    }
    match &mut fixture
        .model
        .accepted_workspace
        .as_mut()
        .unwrap()
        .metadata_base
        .source
    {
        AcceptedMetadataSourceV1::SelectedRootResult { commit: source } => {
            *source = commit;
        }
        _ => panic!("fixture must retain selected-root metadata provenance"),
    }
    rebuild_publication(fixture);
}

fn rebuild_publication(fixture: &mut ServiceFixture) {
    let previous = fixture.model.publication.take().unwrap();
    let previous_candidate = previous.candidate.unwrap();
    let current = crate::workspace_ops::merge::v1_lifecycle::checked::StoredV1Record::for_test(
        &fixture.root.path,
        fixture.model.clone(),
    )
    .unwrap();
    let mut payload =
        crate::workspace_ops::merge::v1_lifecycle::tests::fixtures::candidate_payload(&current);
    payload.candidate.baseline_boundary_text = previous_candidate.baseline_boundary_text;
    payload.candidate.baseline_boundary_sha256 = previous_candidate.baseline_boundary_sha256;
    payload.candidate.boundary_text = previous_candidate.boundary_text;
    payload.candidate.boundary_sha256 = previous_candidate.boundary_sha256;
    fixture.model.publication = Some(PublicationProgress {
        step: PublicationStep::CommittingEvidence,
        candidate_lock_sha256: Some(payload.lock_sha256),
        candidate_marker_path: Some(payload.marker_path),
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: Some(payload.candidate),
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    });
    let result = fixture.model.participants["@root"]
        .resulting_commit
        .clone()
        .unwrap();
    let repo = git2::Repository::open(&fixture.root.path).unwrap();
    let object = repo.find_object(result.parse().unwrap(), None).unwrap();
    repo.reset(&object, git2::ResetType::Hard, None).unwrap();
    let files =
        crate::workspace_ops::merge::acceptance::v1_candidate_files(&fixture.model).unwrap();
    let message = crate::workspace_ops::merge::acceptance::v1_composition_message(&fixture.model);
    let evidence = fixture
        .backend
        .commit_gwz_paths_checked(&fixture.root.path, Some(&result), &files, &message)
        .unwrap();
    let publication = fixture.model.publication.as_mut().unwrap();
    publication.step = PublicationStep::PublishingCandidate;
    publication.root_merge_commit = Some(result);
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
    repo.find_reference("refs/heads/main")
        .unwrap()
        .set_target(
            evidence.commit.parse().unwrap(),
            "install rebuilt publication",
        )
        .unwrap();
    let publication = fixture.model.publication.as_ref().unwrap();
    let candidate = publication.candidate.as_ref().unwrap();
    let marker = publication.candidate_marker_path.as_ref().unwrap();
    std::fs::create_dir_all(fixture.root.path.join(marker).parent().unwrap()).unwrap();
    std::fs::write(fixture.root.path.join(marker), &candidate.marker_yaml).unwrap();
    std::fs::write(
        fixture.root.path.join(crate::artifact::LOCK_PATH),
        &candidate.lock_yaml,
    )
    .unwrap();
    fixture
        .backend
        .stage_paths(
            &fixture.root.path,
            &[marker.as_str(), crate::artifact::LOCK_PATH],
        )
        .unwrap();
    crate::workspace_ops::publish_workspace_exclude_candidate(
        &fixture.root.path,
        &candidate.boundary_text,
    )
    .unwrap();
    std::fs::write(
        fixture.root.path.join(crate::workspace::WORKSPACE_MANIFEST),
        fixture
            .model
            .accepted_workspace
            .as_ref()
            .unwrap()
            .metadata_base
            .manifest_exact_yaml
            .as_bytes(),
    )
    .unwrap();
    fixture
        .backend
        .stage_paths(&fixture.root.path, &[crate::workspace::WORKSPACE_MANIFEST])
        .unwrap();
    crate::workspace_ops::merge::model::v1::validate_v1_journal(&fixture.model).unwrap();
}

pub(super) fn seed_open(fixture: &ServiceFixture) {
    let directory = fixture.root.path.join(".gwz/merge");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(format!("{}.yaml", fixture.model.merge_id)),
        serde_yaml::to_string(&fixture.model).unwrap(),
    )
    .unwrap();
}

fn context(model: &MergeOperationRecordV1) -> OperationContext {
    OperationContext {
        operation_id: model.operation_id.clone(),
        request_id: format!("req_{}", model.merge_id),
        schema_version: "gwz.protocol/v0".into(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}
