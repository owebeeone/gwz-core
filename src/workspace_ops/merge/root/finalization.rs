use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact::{self, LockArtifact, ManifestArtifact};
use crate::git::{GitBackend, GitRepositoryState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;

use super::super::model::v1::MergeOperationRecordV1;
use super::super::{MergeStatusRecordView, MergeTargetKind, OperationState, participant_semantics};
use super::super::publication::{candidate_files_view, classify_candidate_publication_view, observe_root_evidence_view};
use super::super::acceptance::{
    AcceptedRootBase, accepted_root_checkout_with_observation, selected_root_participant,
};
use super::super::publication::RootEvidenceObservation;

pub(in crate::workspace_ops::merge) struct CandidateMetadata {
    pub(in crate::workspace_ops::merge) manifest: ManifestArtifact,
    pub(in crate::workspace_ops::merge) lock: LockArtifact,
    pub(in crate::workspace_ops::merge) baseline_lock_yaml: String,
    pub(in crate::workspace_ops::merge) evidence_parent: Option<String>,
    pub(in crate::workspace_ops::merge) root_branch: String,
}

pub(in crate::workspace_ops::merge) fn candidate_metadata<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<CandidateMetadata> {
    candidate_metadata_inner(backend, root, record).map_err(root_context)
}

fn candidate_metadata_inner<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<CandidateMetadata> {
    let root_participant = selected_root_participant(record)?;
    let unselected_root_head = if root_participant.is_none() {
        Some(backend.head(root)?)
    } else {
        None
    };
    if record.baseline.root_head.is_none()
        && record.baseline.root_branch.is_none()
        && unselected_root_head
            .as_ref()
            .is_some_and(|head| head.is_detached || head.commit.is_some() || head.branch.is_none())
    {
        return Err(root_drift(
            "workspace root changed before candidate metadata was read",
        ));
    }
    let accepted_root =
        accepted_root_checkout_with_observation(record, unselected_root_head.as_ref())?;
    let (baseline_manifest_bytes, baseline_lock_bytes, evidence_parent, root_branch) =
        if let Some(participant) = root_participant {
            (
                committed_file(
                    backend,
                    root,
                    &participant.before_commit,
                    WORKSPACE_MANIFEST,
                )?,
                committed_file(
                    backend,
                    root,
                    &participant.before_commit,
                    artifact::LOCK_PATH,
                )?,
                accepted_root.evidence_parent().map(str::to_owned),
                accepted_root
                    .publication_branch()
                    .expect("selected root is attached")
                    .to_owned(),
            )
        } else {
            let root_head = unselected_root_head.expect("unselected root was observed");
            let root_is_exact = match &accepted_root.base {
                AcceptedRootBase::BornAttached {
                    commit,
                    symbolic_branch,
                } => {
                    !root_head.is_detached
                        && root_head.commit.as_deref() == Some(commit.as_str())
                        && root_head.branch.as_deref() == Some(symbolic_branch.as_str())
                }
                AcceptedRootBase::BornDetached { .. } => false,
                AcceptedRootBase::UnbornAttached { symbolic_branch } => {
                    !root_head.is_detached
                        && root_head.commit.is_none()
                        && root_head.branch.as_deref() == Some(symbolic_branch.as_str())
                }
            };
            if !root_is_exact {
                return Err(root_drift(
                    "workspace root changed before candidate metadata was read",
                ));
            }
            (
                fs::read(root.join(WORKSPACE_MANIFEST)).map_err(io_error)?,
                fs::read(root.join(artifact::LOCK_PATH)).map_err(io_error)?,
                accepted_root.evidence_parent().map(str::to_owned),
                accepted_root
                    .publication_branch()
                    .map(str::to_owned)
                    .ok_or_else(|| root_drift("workspace root branch is missing"))?,
            )
        };
    verify_digest(
        WORKSPACE_MANIFEST,
        &baseline_manifest_bytes,
        record.baseline.manifest_commit_sha256.as_deref(),
    )?;
    verify_digest(
        artifact::LOCK_PATH,
        &baseline_lock_bytes,
        record.baseline.lock_commit_sha256.as_deref(),
    )?;
    let baseline_manifest_yaml = text(WORKSPACE_MANIFEST, baseline_manifest_bytes)?;
    let baseline_lock_yaml = text(artifact::LOCK_PATH, baseline_lock_bytes)?;
    let baseline_manifest = ManifestArtifact::from_yaml(&baseline_manifest_yaml)?;
    let baseline_lock = LockArtifact::from_yaml(&baseline_lock_yaml)?;
    if baseline_manifest.workspace.id != record.workspace_id
        || baseline_lock.workspace_id != record.workspace_id
    {
        return Err(metadata(
            "baseline root metadata identifies a different workspace",
        ));
    }
    let manifest = artifact::read_manifest(root)?;
    let prepublication_lock_yaml =
        fs::read_to_string(root.join(artifact::LOCK_PATH)).map_err(io_error)?;
    let lock = LockArtifact::from_yaml(&prepublication_lock_yaml)?;
    if manifest.workspace.id != record.workspace_id || lock.workspace_id != record.workspace_id {
        return Err(metadata(
            "merged root metadata identifies a different workspace",
        ));
    }
    Ok(CandidateMetadata {
        manifest,
        lock,
        baseline_lock_yaml: prepublication_lock_yaml,
        evidence_parent,
        root_branch,
    })
}

pub(in crate::workspace_ops::merge) fn evidence_parent(
    record: &MergeOperationRecordV1,
) -> ModelResult<Option<&str>> {
    evidence_parent_view(super::super::status::MergeStatusRecordView::from_v1(record))
}

pub(in crate::workspace_ops::merge) fn evidence_parent_view(
    view: super::super::status::MergeStatusRecordView<'_>,
) -> ModelResult<Option<&str>> {
    Ok(match view.selected_root_participant()? {
        Some(participant) => Some(
            participant
                .resulting_commit
                .as_deref()
                .ok_or_else(|| unreadable("root participant has no resulting commit"))?,
        ),
        None => view.baseline().root_head.as_deref(),
    })
}

pub(in crate::workspace_ops::merge) fn root_merge_commit(
    record: &MergeOperationRecordV1,
) -> ModelResult<Option<&str>> {
    selected_root_participant(record)?
        .map(|participant| {
            participant
                .resulting_commit
                .as_deref()
                .ok_or_else(|| unreadable("root participant has no resulting commit"))
        })
        .transpose()
}

pub(in crate::workspace_ops::merge) fn root_finalization_is_exact<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecordV1,
) -> ModelResult<bool> {
    root_finalization_is_exact_view(
        backend,
        root,
        super::super::status::MergeStatusRecordView::from_v1(record),
    )
}

pub(in crate::workspace_ops::merge) fn root_finalization_is_exact_view<B: GitBackend>(
    backend: &B,
    root: &Path,
    view: super::super::status::MergeStatusRecordView<'_>,
) -> ModelResult<bool> {
    if view
        .publication()
        .and_then(|publication| publication.candidate.as_ref())
        .is_none()
    {
        return Ok(false);
    }
    if view.selected_root_participant()?.is_none() {
        return Ok(false);
    }
    if !matches!(
        super::super::publication::observe_root_evidence_view(backend, root, view)?,
        Some(RootEvidenceObservation::Composition(_))
    ) {
        return Ok(false);
    }
    let Some(prefix) = super::super::publication::classify_candidate_publication_view(root, view)?
    else {
        return Ok(false);
    };
    if !super::super::publication::publication_prefix_allowed_view(view, prefix)?
        || backend.repository_state(root)? != GitRepositoryState::Clean
    {
        return Ok(false);
    }
    let allowed = super::super::publication::candidate_files_view(view)?
        .into_iter()
        .map(|file| file.path)
        .collect::<Vec<_>>();
    let status = backend.status(root)?;
    Ok(status.unresolved == 0
        && status
            .files
            .iter()
            .all(|file| allowed.iter().any(|path| path == &file.path)))
}

fn committed_file<B: GitBackend>(
    backend: &B,
    root: &Path,
    commit: &str,
    relative_path: &str,
) -> ModelResult<Vec<u8>> {
    backend
        .read_file_at_commit(root, commit, relative_path)?
        .ok_or_else(|| {
            root_drift(format!(
                "root before commit does not contain '{relative_path}'"
            ))
        })
}

fn verify_digest(relative_path: &str, bytes: &[u8], expected: Option<&str>) -> ModelResult<()> {
    if expected.is_some_and(|expected| format!("{:x}", Sha256::digest(bytes)) != expected) {
        return Err(root_drift(format!(
            "root before-commit '{relative_path}' does not match the recorded baseline"
        )));
    }
    Ok(())
}

fn text(relative_path: &str, bytes: Vec<u8>) -> ModelResult<String> {
    String::from_utf8(bytes).map_err(|error| {
        ModelError::new(
            ErrorCode::ManifestInvalid,
            format!("root before-commit '{relative_path}' is not UTF-8: {error}"),
        )
    })
}

fn io_error(error: std::io::Error) -> ModelError {
    ModelError::new(
        ErrorCode::IoError,
        format!("failed to read workspace metadata: {error}"),
    )
}

fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}

fn metadata(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::ManifestInvalid, message)
}

fn root_drift(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeDrift, message).with_member("@root", ".")
}

fn root_context(error: ModelError) -> ModelError {
    if error.member_id.is_some() {
        error
    } else {
        error.with_member("@root", ".")
    }
}

/// Whether the workspace shows an interrupted root-evidence rollback exactly.
///
/// **M5d.** Relocated from `merge/root/abort.rs`, whose v0 abort surface is
/// deleted. This is a status OBSERVATION, not a rollback step: it is what
/// `status/snapshot.rs` asks before it lets `RollingBack` normalise, and it
/// reads through the version-agnostic `MergeStatusRecordView`.
pub(in crate::workspace_ops::merge) fn interrupted_evidence_rollback_is_exact_view<
    B: GitBackend,
>(
    backend: &B,
    root: &Path,
    view: MergeStatusRecordView<'_>,
) -> ModelResult<bool> {
    let Some(publication) = view.publication() else {
        return Ok(false);
    };
    let Some(participant) = view.participants().get("@root") else {
        return Ok(false);
    };
    if view.state() != OperationState::RollingBack
        || publication.candidate.is_none()
        || publication.composition_commit.is_none()
        || publication.evidence_rolled_back
        || participant.target_kind != MergeTargetKind::Root
        || participant.path != "."
        || !view
            .selected_targets()
            .iter()
            .any(|target| target == "@root")
        || !participant_semantics::result::is_successful_result(participant.state)
        || !matches!(
            observe_root_evidence_view(backend, root, view)?,
            Some(RootEvidenceObservation::Baseline)
        )
        || classify_candidate_publication_view(root, view)?.is_none()
        || backend.repository_state(root)? != GitRepositoryState::Clean
    {
        return Ok(false);
    }
    let allowed = candidate_files_view(view)?
        .into_iter()
        .map(|file| file.path)
        .collect::<Vec<_>>();
    let status = backend.status(root)?;
    Ok(status.unresolved == 0
        && status.files.iter().all(|file| {
            allowed.iter().any(|path| path == &file.path)
                && file
                    .original_path
                    .as_ref()
                    .is_none_or(|original| allowed.iter().any(|path| path == original))
        }))
}
