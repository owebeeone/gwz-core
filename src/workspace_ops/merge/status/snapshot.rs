use std::collections::BTreeMap;
use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace::WORKSPACE_MANIFEST;

use super::super::publication::{
    RootEvidenceObservation, classify_candidate_publication, observe_root_evidence,
    publication_prefix_allowed,
};
use super::super::{
    MergeOperationRecord, MergeStatusSnapshot, MergeStore, MergeTargetKind, OperationDriftKind,
    OperationState, ParticipantState,
};
use super::*;

pub(crate) fn handle_status<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let Some(record) = store.discover_open(root)? else {
        return super::super::response::idle_status_response(context);
    };
    snapshot_status(backend, root, record)?.to_response(context)
}

pub(crate) fn snapshot_status<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: MergeOperationRecord,
) -> ModelResult<MergeStatusSnapshot> {
    // Validate the entire durable path set before the first repository access;
    // a corrupt unselected row must not become a later filesystem escape.
    for (target_id, participant) in &record.participants {
        validated_participant_path(root, target_id, participant)?;
    }
    let mut participants = BTreeMap::new();
    for target_id in &record.selected_targets {
        let participant = record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        participants.insert(
            target_id.clone(),
            observe_participant(backend, root, target_id, participant)?,
        );
    }

    let root_attempted = record.participants.values().any(|participant| {
        participant.target_kind == MergeTargetKind::Root
            && !matches!(
                participant.state,
                ParticipantState::Planned | ParticipantState::Unattempted
            )
    });
    let mut operation_drift = record.operation_drift.clone();
    if !root_attempted {
        match record
            .publication
            .as_ref()
            .and_then(|publication| publication.candidate.as_ref())
        {
            Some(_) => {
                let prefix = classify_candidate_publication(root, &record)?;
                if prefix.is_none()
                    || !publication_prefix_allowed(
                        &record,
                        prefix.expect("candidate prefix was checked"),
                    )?
                {
                    push_operation_drift(
                        &mut operation_drift,
                        OperationDriftKind::RootCandidateStateChanged,
                        "workspace root candidate artifacts do not match an allowed publication prefix",
                    );
                }
            }
            None => compare_digest(
                root,
                artifact::LOCK_PATH,
                &record.baseline.lock_sha256,
                OperationDriftKind::BaselineLockChanged,
                &mut operation_drift,
            ),
        }
        compare_digest(
            root,
            WORKSPACE_MANIFEST,
            &record.baseline.manifest_sha256,
            OperationDriftKind::BaselineManifestChanged,
            &mut operation_drift,
        );
        if record.state == OperationState::Finalizing
            && let Some(publication) = record.publication.as_ref()
        {
            let root_matches = if publication.candidate.is_some() {
                match observe_root_evidence(backend, root, &record)? {
                    Some(RootEvidenceObservation::Baseline) => {
                        publication.composition_commit.is_none()
                    }
                    Some(RootEvidenceObservation::Composition(result)) => publication
                        .composition_commit
                        .as_deref()
                        .is_none_or(|recorded| recorded == result.commit),
                    None => false,
                }
            } else {
                let root_head = backend.head(root)?;
                !root_head.is_detached
                    && root_head.commit == record.baseline.root_head
                    && record
                        .baseline
                        .root_branch
                        .as_deref()
                        .is_none_or(|branch| root_head.branch.as_deref() == Some(branch))
            };
            if !root_matches {
                push_operation_drift(
                    &mut operation_drift,
                    OperationDriftKind::RootCandidateStateChanged,
                    "workspace root HEAD does not match the recorded merge publication state",
                );
            }
        }
    }
    Ok(MergeStatusSnapshot {
        record,
        participants,
        operation_drift,
    })
}
