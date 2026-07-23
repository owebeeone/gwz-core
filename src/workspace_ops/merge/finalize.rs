use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact::{self, CreatedByArtifact, MarkerArtifact, MarkerRootArtifact};
use crate::git::{GitBackend, GitCandidateFile, GitScopedCommitResult};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::{
    publish_workspace_exclude_candidate, workspace_exclude_candidate, workspace_exclude_path,
};

use super::marker::{VerifiedMergeParticipant, marker_merge_from_verified};
use super::{
    MergeOperationRecord, MergeStore, MergeTargetKind, OperationDrift, OperationDriftKind,
    OperationState, ParticipantState, PublicationCandidate, PublicationCandidateHash,
    PublicationProgress, PublicationStep,
};

pub(super) fn finalize<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<bool> {
    if record.state == OperationState::Completed {
        super::archive_merge_record(store, root, &record.merge_id, emitter)?;
        return Ok(true);
    }
    if record.state != OperationState::Finalizing {
        return Err(recovery(format!(
            "merge '{}' is not ready for finalization",
            record.merge_id
        )));
    }
    if record.publication.is_none() {
        record.publication = Some(PublicationProgress {
            step: PublicationStep::NotStarted,
            candidate_lock_sha256: None,
            candidate_marker_path: None,
            root_merge_commit: None,
            composition_commit: None,
            composition_tree: None,
            candidate_hashes: Vec::new(),
            candidate: None,
            evidence_rolled_back: false,
        });
        super::persist_merge_record(store, root, record, emitter)?;
    }

    set_step(
        store,
        root,
        record,
        PublicationStep::ValidatingResults,
        emitter,
    )?;
    let Some(verified) = verified_participants(backend, root, record)? else {
        return Ok(false);
    };
    if !has_changed_participant(record) {
        set_step(store, root, record, PublicationStep::Complete, emitter)?;
        complete_and_archive(store, root, record, emitter)?;
        return Ok(true);
    }

    set_step(
        store,
        root,
        record,
        PublicationStep::PreparingCandidate,
        emitter,
    )?;
    if progress(record)?.candidate.is_none() {
        let prepared = match prepare_candidate(backend, root, record, context, &verified) {
            Ok(prepared) => prepared,
            Err(error) if error.code == ErrorCode::MergeDrift => {
                return block_root(store, root, record, emitter, &error.message);
            }
            Err(error) => return Err(error),
        };
        let publication = progress_mut(record)?;
        publication.candidate_lock_sha256 = Some(sha256(prepared.lock_yaml.as_bytes()));
        publication.candidate_marker_path = Some(format!(
            "{}/{}.yaml",
            artifact::MARKER_DIR,
            prepared.marker_id
        ));
        publication.candidate = Some(prepared);
        super::persist_merge_record(store, root, record, emitter)?;
    }
    validate_candidate(record)?;

    set_step(
        store,
        root,
        record,
        PublicationStep::CommittingEvidence,
        emitter,
    )?;
    let Some(verified) = verified_participants(backend, root, record)? else {
        return Ok(false);
    };
    marker_merge_from_verified(record, &verified)?;
    if !ensure_composition_commit(backend, store, root, record, emitter)? {
        return Ok(false);
    }

    set_step(
        store,
        root,
        record,
        PublicationStep::PublishingCandidate,
        emitter,
    )?;
    let Some(verified) = verified_participants(backend, root, record)? else {
        return Ok(false);
    };
    marker_merge_from_verified(record, &verified)?;
    if !publish_candidate(backend, store, root, record, emitter)? {
        return Ok(false);
    }

    set_step(
        store,
        root,
        record,
        PublicationStep::VerifyingPublication,
        emitter,
    )?;
    let Some(verified) = verified_participants(backend, root, record)? else {
        return Ok(false);
    };
    marker_merge_from_verified(record, &verified)?;
    if !verify_publication(backend, store, root, record, emitter)? {
        return Ok(false);
    }

    set_step(store, root, record, PublicationStep::Complete, emitter)?;
    complete_and_archive(store, root, record, emitter)?;
    Ok(true)
}

fn verified_participants<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Option<Vec<VerifiedMergeParticipant>>> {
    let snapshot = super::status::snapshot_status(backend, root, record.clone())?;
    if snapshot
        .participants
        .values()
        .any(|participant| !participant.drift.is_empty())
    {
        return Ok(None);
    }
    record
        .selected_targets
        .iter()
        .map(|target_id| {
            let durable = record
                .participants
                .get(target_id)
                .ok_or_else(|| unreadable(format!("merge participant '{target_id}' is missing")))?;
            let observed = snapshot
                .participants
                .get(target_id)
                .ok_or_else(|| unreadable(format!("merge observation '{target_id}' is missing")))?;
            let resulting_commit = observed.live_commit.clone().ok_or_else(|| {
                recovery(format!(
                    "verified participant '{target_id}' has no live commit"
                ))
            })?;
            Ok(VerifiedMergeParticipant {
                target_id: target_id.clone(),
                target_branch: durable.target_branch.clone(),
                resulting_commit,
            })
        })
        .collect::<ModelResult<Vec<_>>>()
        .map(Some)
}

fn prepare_candidate<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    context: &OperationContext,
    verified: &[VerifiedMergeParticipant],
) -> ModelResult<PublicationCandidate> {
    require_baseline_artifacts(root, record)?;
    let manifest = artifact::read_manifest(root)?;
    let mut lock = artifact::read_lock(root)?;
    let baseline_lock_yaml =
        fs::read_to_string(root.join(artifact::LOCK_PATH)).map_err(|error| {
            ModelError::new(
                ErrorCode::IoError,
                format!("failed to read merge baseline lock: {error}"),
            )
        })?;
    if manifest.workspace.id != record.workspace_id || lock.workspace_id != record.workspace_id {
        return Err(metadata("workspace identity changed before finalization"));
    }
    for target_id in &record.selected_targets {
        let participant = record
            .participants
            .get(target_id)
            .ok_or_else(|| unreadable(format!("participant '{target_id}' is missing")))?;
        if participant.target_kind != MergeTargetKind::Member {
            return Err(ModelError::new(
                ErrorCode::RootMergeNotYetSupported,
                "root participant finalization is deferred to M2c",
            ));
        }
        let member = manifest
            .members
            .iter()
            .find(|member| member.id == *target_id && member.active)
            .ok_or_else(|| metadata(format!("active member '{target_id}' is missing")))?;
        let locked = lock
            .members
            .get_mut(target_id)
            .ok_or_else(|| metadata(format!("lock member '{target_id}' is missing")))?;
        if locked.path != participant.path
            || locked.path != member.path
            || locked.source_id.as_deref() != Some(member.source_id.as_str())
            || locked.source_kind != member.source_kind
        {
            return Err(metadata(format!(
                "member '{target_id}' identity changed before finalization"
            )));
        }
        let result = participant.resulting_commit.clone().ok_or_else(|| {
            unreadable(format!("participant '{target_id}' has no resulting commit"))
        })?;
        locked.commit = Some(result);
        locked.branch = Some(participant.target_branch.clone());
        locked.detached = Some(false);
        locked.dirty = Some(false);
        locked.materialized = Some(true);
    }
    let lock_yaml = lock.to_yaml()?;
    let marker_id = crate::workspace_ops::handle_commit::new_uuid_v7()?;
    let root_head = backend.head(root)?;
    if root_head.is_detached
        || root_head.branch.is_none()
        || root_head.commit != record.baseline.root_head
        || record
            .baseline
            .root_branch
            .as_ref()
            .is_some_and(|branch| root_head.branch.as_deref() != Some(branch.as_str()))
    {
        return Err(root_drift(
            "workspace root changed before candidate creation",
        ));
    }
    let actor_id = context
        .attribution
        .as_ref()
        .and_then(|attribution| attribution.actor.as_ref())
        .map(|actor| actor.actor_id.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let merge = marker_merge_from_verified(record, verified)?;
    let mut committed_targets = record
        .selected_targets
        .iter()
        .filter(|target_id| {
            record
                .participants
                .get(*target_id)
                .is_some_and(|participant| {
                    participant.resulting_commit.as_deref()
                        != Some(participant.before_commit.as_str())
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    committed_targets.push("@root".to_owned());
    let marker = MarkerArtifact {
        schema: artifact::MARKER_SCHEMA.to_owned(),
        gwz_commit_id: marker_id.clone(),
        workspace_id: record.workspace_id.clone(),
        origin_url_hash: None,
        created_at: record.created_at.clone(),
        created_by: CreatedByArtifact {
            actor_id: actor_id.clone(),
        },
        root: MarkerRootArtifact {
            path: ".".to_owned(),
            before_commit: record.baseline.root_head.clone(),
            branch: root_head.branch.clone(),
        },
        selected_targets: record.selected_targets.clone(),
        committed_targets,
        members: lock.members.clone(),
        merge: Some(merge),
    };
    let marker_yaml = marker.to_yaml()?;
    let (baseline_boundary, boundary_text) =
        workspace_exclude_candidate(backend, root, &manifest, &lock)?;
    Ok(PublicationCandidate {
        marker_id,
        root_branch: record
            .baseline
            .root_branch
            .clone()
            .or(root_head.branch)
            .expect("attached root branch was checked"),
        actor_id,
        baseline_lock_yaml,
        lock_yaml,
        marker_sha256: sha256(marker_yaml.as_bytes()),
        marker_yaml,
        baseline_boundary_text: baseline_boundary.clone(),
        baseline_boundary_sha256: sha256(baseline_boundary.as_bytes()),
        boundary_sha256: sha256(boundary_text.as_bytes()),
        boundary_text,
        extensions: BTreeMap::new(),
    })
}

fn ensure_composition_commit<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<bool> {
    let files = candidate_files(record)?;
    let message = composition_message(record);
    let expected = record.baseline.root_head.as_deref();
    let candidate = candidate(record)?;
    let head = backend.head(root)?;
    if head.is_detached || head.branch.as_deref() != Some(candidate.root_branch.as_str()) {
        return block_root(
            store,
            root,
            record,
            emitter,
            "workspace root branch changed",
        );
    }
    let result = if let Some(commit) = progress(record)?.composition_commit.as_deref() {
        match backend.verify_gwz_paths_commit(root, commit, expected, &files, &message) {
            Ok(result) => result,
            Err(_) => {
                return block_root(
                    store,
                    root,
                    record,
                    emitter,
                    "recorded root evidence commit no longer matches the publication candidate",
                );
            }
        }
    } else if head.commit.as_deref() == expected {
        backend.commit_gwz_paths_checked(root, expected, &files, &message)?
    } else if let Some(commit) = head.commit.as_deref() {
        match backend.verify_gwz_paths_commit(root, commit, expected, &files, &message) {
            Ok(result) => result,
            Err(_) => {
                return block_root(
                    store,
                    root,
                    record,
                    emitter,
                    "workspace root moved before evidence publication",
                );
            }
        }
    } else {
        return block_root(
            store,
            root,
            record,
            emitter,
            "workspace root became unborn before evidence publication",
        );
    };
    record_composition(record, &result);
    clear_root_drift(record);
    super::persist_merge_record(store, root, record, emitter)?;
    Ok(true)
}

fn publish_candidate<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<bool> {
    if !candidate_destinations_safe(root, record)? {
        return block_root(
            store,
            root,
            record,
            emitter,
            "candidate destination changed after preparation",
        );
    }
    let candidate = candidate(record)?.clone();
    let marker_path = artifact::marker_path(root, &candidate.marker_id);
    artifact::write_atomic(&marker_path, &candidate.marker_yaml)?;
    artifact::write_atomic(&root.join(artifact::LOCK_PATH), &candidate.lock_yaml)?;
    publish_workspace_exclude_candidate(root, &candidate.boundary_text)?;
    backend.stage_paths(
        root,
        &[
            artifact::LOCK_PATH,
            progress(record)?
                .candidate_marker_path
                .as_deref()
                .ok_or_else(|| unreadable("candidate marker path is missing"))?,
        ],
    )?;
    clear_root_drift(record);
    super::persist_merge_record(store, root, record, emitter)?;
    Ok(true)
}

fn verify_publication<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<bool> {
    let candidate = candidate(record)?;
    let lock_ok = file_sha256(&root.join(artifact::LOCK_PATH)).as_deref()
        == progress(record)?.candidate_lock_sha256.as_deref();
    let marker_ok = file_sha256(&artifact::marker_path(root, &candidate.marker_id)).as_deref()
        == Some(candidate.marker_sha256.as_str());
    let boundary_ok = file_sha256(&workspace_exclude_path(root)).as_deref()
        == Some(candidate.boundary_sha256.as_str());
    let commit = progress(record)?
        .composition_commit
        .as_deref()
        .ok_or_else(|| unreadable("composition commit is missing"))?;
    let result = backend.verify_gwz_paths_commit(
        root,
        commit,
        record.baseline.root_head.as_deref(),
        &candidate_files(record)?,
        &composition_message(record),
    );
    if !lock_ok || !marker_ok || !boundary_ok || result.is_err() {
        return block_root(
            store,
            root,
            record,
            emitter,
            "published merge candidate failed verification",
        );
    }
    clear_root_drift(record);
    super::persist_merge_record(store, root, record, emitter)?;
    Ok(true)
}

fn candidate_destinations_safe(root: &Path, record: &MergeOperationRecord) -> ModelResult<bool> {
    let candidate = candidate(record)?;
    let lock = file_sha256(&root.join(artifact::LOCK_PATH));
    let marker = file_sha256(&artifact::marker_path(root, &candidate.marker_id));
    let boundary = file_sha256(&workspace_exclude_path(root));
    let baseline_lock = lock.as_deref() == Some(record.baseline.lock_sha256.as_str());
    let candidate_lock = lock.as_deref() == progress(record)?.candidate_lock_sha256.as_deref();
    let marker_absent = marker.is_none();
    let candidate_marker = marker.as_deref() == Some(candidate.marker_sha256.as_str());
    let baseline_boundary = boundary.as_deref()
        == Some(candidate.baseline_boundary_sha256.as_str())
        || (boundary.is_none() && candidate.baseline_boundary_text.is_empty());
    let candidate_boundary = boundary.as_deref() == Some(candidate.boundary_sha256.as_str());
    Ok(
        (baseline_lock && baseline_boundary && (marker_absent || candidate_marker))
            || (candidate_lock && candidate_marker && (baseline_boundary || candidate_boundary)),
    )
}

fn validate_candidate(record: &MergeOperationRecord) -> ModelResult<()> {
    let candidate = candidate(record)?;
    let publication = progress(record)?;
    let lock_sha256 = sha256(candidate.lock_yaml.as_bytes());
    let marker_path = format!("{}/{}.yaml", artifact::MARKER_DIR, candidate.marker_id);
    if publication.candidate_lock_sha256.as_deref() != Some(lock_sha256.as_str())
        || publication.candidate_marker_path.as_deref() != Some(marker_path.as_str())
        || sha256(candidate.baseline_lock_yaml.as_bytes()) != record.baseline.lock_sha256
        || candidate.baseline_boundary_sha256 != sha256(candidate.baseline_boundary_text.as_bytes())
        || candidate.marker_sha256 != sha256(candidate.marker_yaml.as_bytes())
        || candidate.boundary_sha256 != sha256(candidate.boundary_text.as_bytes())
        || record
            .baseline
            .root_branch
            .as_ref()
            .is_some_and(|branch| branch != &candidate.root_branch)
    {
        return Err(unreadable("persisted merge candidate hashes do not match"));
    }
    let baseline_lock = artifact::LockArtifact::from_yaml(&candidate.baseline_lock_yaml)?;
    let lock = artifact::LockArtifact::from_yaml(&candidate.lock_yaml)?;
    let marker = MarkerArtifact::from_yaml(&candidate.marker_yaml)?;
    if baseline_lock.workspace_id != record.workspace_id
        || lock.workspace_id != record.workspace_id
        || marker.workspace_id != record.workspace_id
        || marker.gwz_commit_id != candidate.marker_id
        || marker.created_by.actor_id != candidate.actor_id
        || marker.root.branch.as_deref() != Some(candidate.root_branch.as_str())
        || marker.members != lock.members
    {
        return Err(unreadable(
            "persisted merge candidate identities do not match",
        ));
    }
    Ok(())
}

fn candidate_files(record: &MergeOperationRecord) -> ModelResult<Vec<GitCandidateFile>> {
    let candidate = candidate(record)?;
    Ok(vec![
        GitCandidateFile {
            path: artifact::LOCK_PATH.to_owned(),
            bytes: candidate.lock_yaml.as_bytes().to_vec(),
        },
        GitCandidateFile {
            path: progress(record)?
                .candidate_marker_path
                .clone()
                .ok_or_else(|| unreadable("candidate marker path is missing"))?,
            bytes: candidate.marker_yaml.as_bytes().to_vec(),
        },
    ])
}

fn record_composition(record: &mut MergeOperationRecord, result: &GitScopedCommitResult) {
    let publication = record
        .publication
        .as_mut()
        .expect("publication exists during finalization");
    publication.composition_commit = Some(result.commit.clone());
    publication.composition_tree = Some(result.tree.clone());
    publication.candidate_hashes = result
        .candidate_hashes
        .iter()
        .map(|hash| PublicationCandidateHash {
            path: hash.path.clone(),
            sha256: hash.sha256.clone(),
        })
        .collect();
}

fn set_step<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    next: PublicationStep,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let publication = progress_mut(record)?;
    if publication.step < next {
        publication.step = publication.step.transition(next)?;
        super::persist_merge_record(store, root, record, emitter)?;
    }
    Ok(())
}

fn complete_and_archive<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    super::persist_operation_transition(store, root, record, OperationState::Completed, emitter)?;
    super::archive_merge_record(store, root, &record.merge_id, emitter)
}

fn block_root<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
    message: &str,
) -> ModelResult<bool> {
    clear_root_drift(record);
    record.operation_drift.push(OperationDrift {
        kind: OperationDriftKind::RootCandidateStateChanged,
        message: message.to_owned(),
    });
    super::persist_merge_record(store, root, record, emitter)?;
    Ok(false)
}

fn clear_root_drift(record: &mut MergeOperationRecord) {
    record
        .operation_drift
        .retain(|drift| drift.kind != OperationDriftKind::RootCandidateStateChanged);
}

fn require_baseline_artifacts(root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
    if file_sha256(&root.join(artifact::LOCK_PATH)).as_deref()
        != Some(record.baseline.lock_sha256.as_str())
        || file_sha256(&root.join(WORKSPACE_MANIFEST)).as_deref()
            != Some(record.baseline.manifest_sha256.as_str())
    {
        return Err(root_drift(
            "workspace manifest or lock changed before candidate creation",
        ));
    }
    Ok(())
}

fn has_changed_participant(record: &MergeOperationRecord) -> bool {
    record.participants.values().any(|participant| {
        matches!(
            participant.state,
            ParticipantState::FastForwarded
                | ParticipantState::Merged
                | ParticipantState::Continued
        ) && participant.resulting_commit.as_deref() != Some(participant.before_commit.as_str())
    })
}

fn progress(record: &MergeOperationRecord) -> ModelResult<&PublicationProgress> {
    record
        .publication
        .as_ref()
        .ok_or_else(|| unreadable("publication progress is missing"))
}

fn progress_mut(record: &mut MergeOperationRecord) -> ModelResult<&mut PublicationProgress> {
    record
        .publication
        .as_mut()
        .ok_or_else(|| unreadable("publication progress is missing"))
}

fn candidate(record: &MergeOperationRecord) -> ModelResult<&PublicationCandidate> {
    progress(record)?
        .candidate
        .as_ref()
        .ok_or_else(|| unreadable("publication candidate is missing"))
}

fn composition_message(record: &MergeOperationRecord) -> String {
    format!(
        "gwz merge: {}\n\nGWZ-Merge-ID: {}\nGWZ-Operation-ID: {}",
        record.source_ref, record.merge_id, record.operation_id
    )
}

fn file_sha256(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|bytes| sha256(&bytes))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn unreadable(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
}

fn recovery(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}

fn metadata(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::ManifestInvalid, message)
}

fn root_drift(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeDrift, message).with_member("@root", ".")
}
