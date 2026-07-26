use super::{
    super::{
        MergeOperationRecord, MergeStore, MergeTargetKind, OperationState,
        publication::{
            CandidatePublicationPrefix, RootEvidenceObservation, candidate_files,
            classify_candidate_publication, composition_message, publication_prefix_allowed,
        },
    },
    runtime::AbortRuntime,
};
use crate::artifact;
use crate::git::GitCandidateFile;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventEmitter;
use std::{fs, path::Path};

pub(super) struct EvidenceRollback {
    branch: String,
    composition_commit: String,
    baseline_commit: Option<String>,
    marker_id: String,
    baseline_lock_yaml: String,
    baseline_boundary_text: String,
    candidate_files: Vec<GitCandidateFile>,
    composition_message: String,
    pub(super) root_participant_evidence_present: bool,
}

pub(super) fn preflight_evidence<A: AbortRuntime>(
    runtime: &A,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Option<EvidenceRollback>> {
    let Some(publication) = record.publication.as_ref() else {
        return Ok(None);
    };
    let Some(candidate) = publication.candidate.as_ref() else {
        if publication.candidate_lock_sha256.is_some()
            || publication.candidate_marker_path.is_some()
            || publication.root_merge_commit.is_some()
            || publication.composition_commit.is_some()
            || publication.composition_tree.is_some()
            || !publication.candidate_hashes.is_empty()
            || publication.evidence_rolled_back
        {
            return Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "merge publication has evidence fields but no durable candidate",
            ));
        }
        return Ok(None);
    };
    let root_participant = record.participants.get("@root").filter(|participant| {
        participant.target_kind == MergeTargetKind::Root && participant.path == "."
    });
    if publication.evidence_rolled_back
        && let Some(participant) = root_participant
    {
        let head = runtime.head(root)?;
        if !head.is_detached
            && head.branch.as_deref() == Some(participant.target_branch.as_str())
            && head.commit.as_deref() == Some(participant.before_commit.as_str())
        {
            return Ok(None);
        }
    }
    let prefix = classify_candidate_publication(root, record)?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root candidate artifacts changed after evidence creation",
        )
        .with_member("@root", ".")
    })?;
    if !matches!(
        record.state,
        OperationState::Preserving | OperationState::RollingBack
    ) && !publication_prefix_allowed(record, prefix)?
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root candidate artifacts do not match the recorded publication step",
        )
        .with_member("@root", "."));
    }
    let observation = runtime.observe_root_evidence(root, record)?;
    let (composition_commit, root_participant_evidence_present) = match observation {
        Some(RootEvidenceObservation::Composition(result)) => {
            if root_participant.is_some() && !runtime.root_finalization_is_exact(root, record)? {
                return Err(ModelError::new(
                    ErrorCode::MergeDrift,
                    "workspace root contains post-merge work that must be preserved or removed before abort",
                )
                .with_member("@root", "."));
            }
            (result.commit, root_participant.is_some())
        }
        Some(RootEvidenceObservation::Baseline)
            if publication.composition_commit.is_none()
                && prefix == CandidatePublicationPrefix::Baseline =>
        {
            return Ok(None);
        }
        Some(RootEvidenceObservation::Baseline) => {
            let interrupted_root_rollback = root_participant.is_some()
                && record.state == OperationState::RollingBack
                && !publication.evidence_rolled_back
                && runtime.root_evidence_rollback_is_exact(root, record)?;
            (
                publication.composition_commit.clone().ok_or_else(|| {
                    ModelError::new(
                        ErrorCode::MergeDrift,
                        "published candidate has no recorded root evidence commit",
                    )
                    .with_member("@root", ".")
                })?,
                interrupted_root_rollback,
            )
        }
        None => {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                "workspace root moved after merge evidence creation",
            )
            .with_member("@root", "."));
        }
    };
    Ok(Some(EvidenceRollback {
        branch: candidate.root_branch.clone(),
        composition_commit,
        baseline_commit: super::super::root::evidence_parent(record)?.map(str::to_owned),
        marker_id: candidate.marker_id.clone(),
        baseline_lock_yaml: candidate.baseline_lock_yaml.clone(),
        baseline_boundary_text: candidate.baseline_boundary_text.clone(),
        candidate_files: candidate_files(record)?,
        composition_message: composition_message(record),
        root_participant_evidence_present,
    }))
}

pub(super) fn rollback_evidence<A: AbortRuntime, S: MergeStore>(
    runtime: &A,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    evidence: &EvidenceRollback,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let head = runtime.head(root)?;
    if head.commit.as_deref() == Some(evidence.composition_commit.as_str()) {
        runtime.rollback_evidence_commit(
            root,
            &evidence.branch,
            &evidence.composition_commit,
            evidence.baseline_commit.as_deref(),
            &evidence.candidate_files,
            &evidence.composition_message,
        )?;
    }
    super::super::super::publish_workspace_exclude_candidate(
        root,
        &evidence.baseline_boundary_text,
    )?;
    #[cfg(test)]
    maybe_fail_evidence_rollback_after(EvidenceRollbackMutation::Boundary)?;
    artifact::write_atomic(
        &root.join(artifact::LOCK_PATH),
        &evidence.baseline_lock_yaml,
    )?;
    #[cfg(test)]
    maybe_fail_evidence_rollback_after(EvidenceRollbackMutation::Lock)?;
    let marker_path = artifact::marker_path(root, &evidence.marker_id);
    match fs::remove_file(&marker_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ModelError::new(
                ErrorCode::IoError,
                format!("failed to remove merge marker during abort: {error}"),
            ));
        }
    }
    #[cfg(test)]
    maybe_fail_evidence_rollback_after(EvidenceRollbackMutation::Marker)?;
    let marker_relative = format!("{}/{}.yaml", artifact::MARKER_DIR, evidence.marker_id);
    runtime.stage_paths(root, &[artifact::LOCK_PATH, &marker_relative])?;
    #[cfg(test)]
    maybe_fail_evidence_rollback_after(EvidenceRollbackMutation::Staging)?;
    if let Some(publication) = record.publication.as_mut() {
        publication.evidence_rolled_back = true;
    }
    super::super::persist_merge_record(store, root, record, emitter)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceRollbackMutation {
    Boundary,
    Lock,
    Marker,
    Staging,
}

#[cfg(test)]
thread_local! {
    static EVIDENCE_ROLLBACK_FAILURE:
        std::cell::Cell<Option<EvidenceRollbackMutation>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_evidence_rollback_after(mutation: EvidenceRollbackMutation) {
    EVIDENCE_ROLLBACK_FAILURE.with(|failure| {
        assert!(
            failure.replace(Some(mutation)).is_none(),
            "an evidence rollback failure is already installed"
        );
    });
}

#[cfg(test)]
fn maybe_fail_evidence_rollback_after(mutation: EvidenceRollbackMutation) -> ModelResult<()> {
    EVIDENCE_ROLLBACK_FAILURE.with(|failure| {
        if failure.get() == Some(mutation) {
            failure.set(None);
            Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("injected failure after evidence {mutation:?} restoration"),
            ))
        } else {
            Ok(())
        }
    })
}

pub(super) fn verify_evidence_baseline<A: AbortRuntime>(
    runtime: &A,
    root: &Path,
    evidence: &EvidenceRollback,
) -> ModelResult<()> {
    let head = runtime.head(root)?;
    let marker_absent = !artifact::marker_path(root, &evidence.marker_id).exists();
    let lock_matches = fs::read(root.join(artifact::LOCK_PATH)).ok().as_deref()
        == Some(evidence.baseline_lock_yaml.as_bytes());
    let boundary_matches = fs::read(super::super::super::workspace_exclude_path(root))
        .ok()
        .as_deref()
        == Some(evidence.baseline_boundary_text.as_bytes());
    if head.is_detached
        || head.branch.as_deref() != Some(evidence.branch.as_str())
        || head.commit != evidence.baseline_commit
        || !marker_absent
        || !lock_matches
        || !boundary_matches
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root changed during merge evidence rollback",
        )
        .with_member("@root", "."));
    }
    Ok(())
}
