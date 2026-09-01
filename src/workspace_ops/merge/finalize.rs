use std::collections::BTreeMap;
use std::path::Path;

use crate::artifact::{self, CreatedByArtifact, MarkerArtifact, MarkerRootArtifact};
use crate::git::{GitBackend, GitScopedCommitResult};
use crate::model::{ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext};
use crate::workspace_ops::{
    publish_workspace_exclude_candidate, workspace_exclude_candidate, workspace_exclude_path,
};

use super::acceptance::{CompleteLockErrorKind, construct_complete_lock};
pub(super) use super::finalize_dispatch::finalize;
use super::finalize_support::{
    block_root, candidate, clear_root_drift, file_sha256, progress, root_drift, sha256, unreadable,
};
use super::marker::{VerifiedMergeParticipant, marker_merge_from_verified};
use super::publication::{candidate_files, classify_candidate_publication, composition_message};
use super::{MergeOperationRecord, MergeStore, PublicationCandidate, PublicationCandidateHash};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CandidatePublicationMutation {
    Marker,
    Lock,
    Boundary,
    Staging,
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_CANDIDATE_PUBLICATION_AFTER:
        std::cell::Cell<Option<CandidatePublicationMutation>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_candidate_publication_after(mutation: CandidatePublicationMutation) {
    FAIL_NEXT_CANDIDATE_PUBLICATION_AFTER.with(|next| next.set(Some(mutation)));
}

#[cfg(test)]
fn fail_candidate_publication_after(mutation: CandidatePublicationMutation) -> ModelResult<()> {
    FAIL_NEXT_CANDIDATE_PUBLICATION_AFTER.with(|next| {
        if next.get() == Some(mutation) {
            next.set(None);
            return Err(super::finalize_support::recovery(format!(
                "injected failure after candidate {mutation:?} publication"
            )));
        }
        Ok(())
    })
}

pub(super) enum CandidatePreparationError {
    Metadata(ModelError),
    Other(ModelError),
}

pub(super) fn prepare_candidate<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    context: &OperationContext,
    verified: &[VerifiedMergeParticipant],
) -> Result<PublicationCandidate, CandidatePreparationError> {
    let root_metadata = super::root::candidate_metadata(backend, root, record)
        .map_err(CandidatePreparationError::Metadata)?;
    let manifest = root_metadata.manifest;
    let lock = root_metadata.lock;
    let baseline_lock_yaml = root_metadata.baseline_lock_yaml;
    let lock =
        construct_complete_lock(record, &manifest, lock).map_err(|error| match error.kind {
            CompleteLockErrorKind::Metadata => CandidatePreparationError::Metadata(error.error),
            CompleteLockErrorKind::Record => CandidatePreparationError::Other(error.error),
        })?;
    let lock_yaml = lock
        .to_yaml()
        .map_err(CandidatePreparationError::Metadata)?;
    let marker_id = crate::workspace_ops::handle_commit::new_uuid_v7()
        .map_err(CandidatePreparationError::Other)?;
    let root_head = backend
        .head(root)
        .map_err(CandidatePreparationError::Other)?;
    if root_head.is_detached
        || root_head.branch.is_none()
        || root_head.commit != root_metadata.evidence_parent
        || root_head.branch.as_deref() != Some(root_metadata.root_branch.as_str())
    {
        return Err(CandidatePreparationError::Other(root_drift(
            "workspace root changed before candidate creation",
        )));
    }
    let actor_id = context
        .attribution
        .as_ref()
        .and_then(|attribution| attribution.actor.as_ref())
        .map(|actor| actor.actor_id.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let merge =
        marker_merge_from_verified(record, verified).map_err(CandidatePreparationError::Other)?;
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
    if !committed_targets.iter().any(|target| target == "@root") {
        committed_targets.push("@root".to_owned());
    }
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
            before_commit: root_metadata.evidence_parent.clone(),
            branch: root_head.branch.clone(),
        },
        selected_targets: record.selected_targets.clone(),
        committed_targets,
        members: lock.members.clone(),
        merge: Some(merge),
    };
    let marker_yaml = marker.to_yaml().map_err(CandidatePreparationError::Other)?;
    let (baseline_boundary, boundary_text) =
        workspace_exclude_candidate(backend, root, &manifest, &lock)
            .map_err(CandidatePreparationError::Other)?;
    Ok(PublicationCandidate {
        marker_id,
        root_branch: root_metadata.root_branch,
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

pub(super) fn ensure_composition_commit<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<bool> {
    let files = candidate_files(record)?;
    let message = composition_message(record);
    let expected = super::root::evidence_parent(record)?;
    let candidate = candidate(record)?;
    let head = backend.head(root)?;
    let had_recorded_composition = progress(record)?.composition_commit.is_some();
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
    if had_recorded_composition && !recorded_composition_matches(record, &result)? {
        return Err(unreadable(
            "recorded root evidence tree or candidate hashes do not match",
        ));
    }
    record_composition(record, &result);
    clear_root_drift(record);
    super::persist_merge_record(store, root, record, emitter)?;
    Ok(true)
}

pub(super) fn publish_candidate<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<bool> {
    if classify_candidate_publication(root, record)?.is_none() {
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
    // CAPABILITY-FREE EXCEPTION, §10 rows `:277`/`:278`/`:279`: an ORDINARY merge is on E0.2 §5.2's capability-free list, so this publication trio stays raw permanently (2026-09-02, GwzM5-8R2E-CapabilityFreeAmendment.md §3).
    artifact::write_atomic(&marker_path, &candidate.marker_yaml)?;
    #[cfg(test)]
    fail_candidate_publication_after(CandidatePublicationMutation::Marker)?;
    artifact::write_atomic(&root.join(artifact::LOCK_PATH), &candidate.lock_yaml)?;
    #[cfg(test)]
    fail_candidate_publication_after(CandidatePublicationMutation::Lock)?;
    publish_workspace_exclude_candidate(root, &candidate.boundary_text)?;
    #[cfg(test)]
    fail_candidate_publication_after(CandidatePublicationMutation::Boundary)?;
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
    #[cfg(test)]
    fail_candidate_publication_after(CandidatePublicationMutation::Staging)?;
    clear_root_drift(record);
    super::persist_merge_record(store, root, record, emitter)?;
    Ok(true)
}

pub(super) fn verify_publication<B: GitBackend, S: MergeStore>(
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
        .clone()
        .ok_or_else(|| unreadable("composition commit is missing"))?;
    let result = backend.verify_gwz_paths_commit(
        root,
        &commit,
        super::root::evidence_parent(record)?,
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
    emitter.artifact_written(format!("git:@root/{commit}"));
    emitter.artifact_written(
        progress(record)?
            .candidate_marker_path
            .as_deref()
            .ok_or_else(|| unreadable("candidate marker path is missing"))?,
    );
    emitter.artifact_written(artifact::LOCK_PATH);
    emitter.artifact_written(".git/info/exclude");
    Ok(true)
}

pub(super) fn validate_candidate(record: &MergeOperationRecord) -> ModelResult<()> {
    let candidate = candidate(record)?;
    let publication = progress(record)?;
    let lock_sha256 = sha256(candidate.lock_yaml.as_bytes());
    let marker_path = format!("{}/{}.yaml", artifact::MARKER_DIR, candidate.marker_id);
    let root_participated = publication.root_merge_commit.is_some();
    if publication.candidate_lock_sha256.as_deref() != Some(lock_sha256.as_str())
        || publication.candidate_marker_path.as_deref() != Some(marker_path.as_str())
        || (!root_participated
            && sha256(candidate.baseline_lock_yaml.as_bytes()) != record.baseline.lock_sha256)
        || candidate.baseline_boundary_sha256 != sha256(candidate.baseline_boundary_text.as_bytes())
        || candidate.marker_sha256 != sha256(candidate.marker_yaml.as_bytes())
        || candidate.boundary_sha256 != sha256(candidate.boundary_text.as_bytes())
        || record
            .baseline
            .root_branch
            .as_ref()
            .is_some_and(|branch| branch != &candidate.root_branch)
        || publication.root_merge_commit.as_deref() != super::root::root_merge_commit(record)?
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

pub(crate) fn validate_candidate_for_i2_fixture(record: &MergeOperationRecord) -> ModelResult<()> {
    validate_candidate(record)
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

fn recorded_composition_matches(
    record: &MergeOperationRecord,
    result: &GitScopedCommitResult,
) -> ModelResult<bool> {
    let publication = progress(record)?;
    Ok(
        publication.composition_commit.as_deref() == Some(result.commit.as_str())
            && publication.composition_tree.as_deref() == Some(result.tree.as_str())
            && publication.candidate_hashes.len() == result.candidate_hashes.len()
            && publication
                .candidate_hashes
                .iter()
                .zip(&result.candidate_hashes)
                .all(|(recorded, observed)| {
                    recorded.path == observed.path && recorded.sha256 == observed.sha256
                }),
    )
}
