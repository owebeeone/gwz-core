use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact::{self, LockArtifact, ManifestArtifact};
use crate::git::{GitBackend, GitRepositoryState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;

use super::super::publication::{
    RootEvidenceObservation, candidate_files, classify_candidate_publication,
    observe_root_evidence, publication_prefix_allowed,
};
use super::super::{MergeOperationRecord, MergeParticipantRecord, MergeTargetKind};

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
    record: &MergeOperationRecord,
) -> ModelResult<CandidateMetadata> {
    candidate_metadata_inner(backend, root, record).map_err(root_context)
}

fn candidate_metadata_inner<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<CandidateMetadata> {
    let root_participant = root_participant(record)?;
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
                participant.resulting_commit.clone(),
                participant.target_branch.clone(),
            )
        } else {
            let root_head = backend.head(root)?;
            if root_head.is_detached
                || root_head.commit != record.baseline.root_head
                || record
                    .baseline
                    .root_branch
                    .as_deref()
                    .is_some_and(|branch| root_head.branch.as_deref() != Some(branch))
            {
                return Err(root_drift(
                    "workspace root changed before candidate metadata was read",
                ));
            }
            (
                fs::read(root.join(WORKSPACE_MANIFEST)).map_err(io_error)?,
                fs::read(root.join(artifact::LOCK_PATH)).map_err(io_error)?,
                record.baseline.root_head.clone(),
                record
                    .baseline
                    .root_branch
                    .clone()
                    .or(root_head.branch)
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
    record: &MergeOperationRecord,
) -> ModelResult<Option<&str>> {
    Ok(match root_participant(record)? {
        Some(participant) => Some(
            participant
                .resulting_commit
                .as_deref()
                .ok_or_else(|| unreadable("root participant has no resulting commit"))?,
        ),
        None => record.baseline.root_head.as_deref(),
    })
}

pub(in crate::workspace_ops::merge) fn root_merge_commit(
    record: &MergeOperationRecord,
) -> ModelResult<Option<&str>> {
    root_participant(record)?
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
    record: &MergeOperationRecord,
) -> ModelResult<bool> {
    if record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .is_none()
    {
        return Ok(false);
    }
    if root_participant(record)?.is_none() {
        return Ok(false);
    }
    if !matches!(
        observe_root_evidence(backend, root, record)?,
        Some(RootEvidenceObservation::Composition(_))
    ) {
        return Ok(false);
    }
    let Some(prefix) = classify_candidate_publication(root, record)? else {
        return Ok(false);
    };
    if !publication_prefix_allowed(record, prefix)?
        || backend.repository_state(root)? != GitRepositoryState::Clean
    {
        return Ok(false);
    }
    let allowed = candidate_files(record)?
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

fn root_participant(record: &MergeOperationRecord) -> ModelResult<Option<&MergeParticipantRecord>> {
    let participant = record.participants.get("@root");
    let selected = record
        .selected_targets
        .iter()
        .any(|target| target == "@root");
    match (selected, participant) {
        (false, None) => Ok(None),
        (true, Some(participant))
            if participant.target_kind == MergeTargetKind::Root
                && participant.path == "."
                && super::super::participant_semantics::result::is_successful_result(
                    participant.state,
                ) =>
        {
            Ok(Some(participant))
        }
        _ => Err(unreadable(
            "selected root participant identity or successful state is inconsistent",
        )),
    }
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
