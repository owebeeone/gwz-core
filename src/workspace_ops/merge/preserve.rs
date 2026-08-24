use std::path::{Path, PathBuf};

use crate::git::{GitBackend, GitRepositoryState, GitStatus};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, OperationContext};
use crate::stash::{
    STASH_BUNDLE_SCHEMA, StashBundle, StashBundleMember, StashDirtySummary, StashParticipation,
    StashPushLifecycle, StashRestoreState,
};

use super::{
    MergeOperationRecord, MergeParticipantRecord, MergeStore, MergeTargetKind, OperationState,
    PreservationEvidence,
};

mod artifacts;
mod checked_bundle;
mod plan;

use artifacts::*;
use plan::*;

#[cfg(test)]
pub(in crate::workspace_ops::merge) use artifacts::V1_PRESERVATION_IMAGE_CAPTURES;
pub(in crate::workspace_ops::merge) use artifacts::{
    v1_preservation_image, v1_root_preservation_spec,
};
pub(in crate::workspace_ops::merge) use checked_bundle::{
    V1BundleObservation, v1_bundle_cursor_is_exact, v1_bundle_observation, v1_write_bundle_checked,
};
pub(in crate::workspace_ops::merge) use plan::{
    V1PreservationOwnerPlan, v1_owner_evidence, v1_preservation_owners,
};

pub(super) fn classify_index_aligned_root_publication_for_i2<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Option<super::publication::CandidatePublicationPrefix>> {
    artifacts::classify_index_aligned_root_publication(backend, root, record)
}

struct PreservationPlan {
    target_id: String,
    path: PathBuf,
    relative_path: String,
    target_branch: String,
    anchor: String,
    live_commit: String,
    backup_ref: String,
    status: GitStatus,
    preserve_commit: bool,
    preserve_worktree: bool,
    root_publication_prefix: Option<super::publication::CandidatePublicationPrefix>,
    evidence_owner: EvidenceOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EvidenceOwner {
    Participant,
    PublicationRoot,
}

pub(super) fn preserve_then_abort<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    request: &crate::MergeRequest,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<crate::MergeResponse> {
    let Some(mut record) = store.discover_open(root)? else {
        return super::abort::abort_locked(
            backend,
            store,
            root,
            request.merge_id.as_deref(),
            context,
            emitter,
        );
    };
    super::validate::validate_open_merge_id(request.merge_id.as_deref(), &record.merge_id)?;
    if matches!(
        record.state,
        OperationState::Completed | OperationState::Aborted
    ) {
        return super::abort::abort_locked(
            backend,
            store,
            root,
            request.merge_id.as_deref(),
            context,
            emitter,
        );
    }

    recover_root_publication(backend, root, &record)?;
    let plans = preflight(backend, root, &record)?;
    if record.state != OperationState::Preserving {
        super::persist_operation_transition(
            store,
            root,
            &mut record,
            OperationState::Preserving,
            emitter,
        )?;
    }

    for plan in &plans {
        if plan.preserve_commit {
            let target = existing_evidence(&record, plan)?
                .and_then(|evidence| evidence.backup_commit.clone())
                .unwrap_or_else(|| plan.live_commit.clone());
            let result = member_result(
                backend.create_backup_ref(&plan.path, &plan.backup_ref, &target),
                plan,
            )?;
            update_evidence(&mut record, plan, |evidence| {
                evidence.backup_ref = Some(result.name);
                evidence.backup_commit = Some(result.target);
            })?;
            super::persist_merge_record(store, root, &record, emitter)?;
        }
        if plan.preserve_worktree {
            let result = if let Some(prefix) = plan.root_publication_prefix {
                persist_root_preservation_prefix(store, root, &mut record, prefix, emitter)?;
                prepare_root_for_stash(backend, root, &record, plan)?;
                let result =
                    backend.stash_for_merge_preservation(&plan.path, &record.merge_id, true);
                let restored = restore_root_publication(backend, root, &record, plan, prefix);
                restored?;
                member_result(result, plan)?
            } else {
                member_result(
                    backend.stash_for_merge_preservation(&plan.path, &record.merge_id, true),
                    plan,
                )?
            };
            let stash_id = format!("stash_{}", record.merge_id);
            update_evidence(&mut record, plan, |evidence| {
                evidence.stash_id = Some(stash_id.clone());
                evidence.stash_object_id = Some(result.object_id.clone());
            })?;
            super::persist_merge_record(store, root, &record, emitter)?;
            persist_stash_bundle(root, &record, plan, &result.message, &result.object_id)?;
        }
    }

    verify_artifacts(backend, root, &record, &plans)?;
    for plan in &plans {
        if !plan.preserve_commit {
            continue;
        }
        let evidence = existing_evidence(&record, plan)?
            .ok_or_else(|| unreadable(plan, "preservation evidence disappeared"))?;
        let current = member_result(backend.head(&plan.path), plan)?
            .commit
            .ok_or_else(|| drift(plan, "preserved branch no longer has a commit"))?;
        if current == plan.anchor {
            continue;
        }
        let expected = evidence
            .backup_commit
            .as_deref()
            .ok_or_else(|| unreadable(plan, "backup evidence has no recorded commit"))?;
        if current != expected {
            return Err(drift(
                plan,
                "repository changed after preservation artifacts were recorded",
            ));
        }
        member_result(
            backend.set_branch_target_checked(
                &plan.path,
                &plan.target_branch,
                expected,
                &plan.anchor,
            ),
            plan,
        )?;
        if let Some(prefix) = plan.root_publication_prefix {
            restore_root_publication(backend, root, &record, plan, prefix)?;
        }
    }

    super::abort::abort_locked(
        backend,
        store,
        root,
        request.merge_id.as_deref(),
        context,
        emitter,
    )
}

fn persist_root_preservation_prefix<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    prefix: super::publication::CandidatePublicationPrefix,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
    let publication = record.publication.as_mut().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root preservation has no publication record",
        )
        .with_member("@root", ".")
    })?;
    let encoded = publication_prefix_name(prefix);
    match publication.preservation_prefix.as_deref() {
        Some(existing) if existing != encoded => {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                "root publication prefix changed during preservation",
            )
            .with_member("@root", "."));
        }
        Some(_) => return Ok(()),
        None => publication.preservation_prefix = Some(encoded.to_owned()),
    }
    super::persist_merge_record(store, root, record, emitter)
}

fn recover_root_publication<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<()> {
    if record.state != OperationState::Preserving {
        return Ok(());
    }
    let Some(encoded) = record
        .publication
        .as_ref()
        .and_then(|publication| publication.preservation_prefix.as_deref())
    else {
        return Ok(());
    };
    let prefix = parse_publication_prefix(encoded)?;
    let observed = classify_index_aligned_root_publication(backend, root, record)?;
    if observed == Some(prefix) {
        return Ok(());
    }
    let candidate = record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "root preservation candidate disappeared",
            )
            .with_member("@root", ".")
        })?;
    let normalized = if prefix == super::publication::CandidatePublicationPrefix::Boundary
        || candidate.baseline_boundary_sha256 == candidate.boundary_sha256
    {
        super::publication::CandidatePublicationPrefix::Boundary
    } else {
        super::publication::CandidatePublicationPrefix::Lock
    };
    if observed != Some(normalized) {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!(
                "root publication changed after preservation was interrupted (expected {prefix:?} or normalized {normalized:?}, observed {observed:?}); repair the recorded publication prefix before retrying"
            ),
        )
        .with_member("@root", "."));
    }
    restore_root_publication_from_record(backend, root, record, prefix)
}

fn publication_prefix_name(prefix: super::publication::CandidatePublicationPrefix) -> &'static str {
    use super::publication::CandidatePublicationPrefix;
    match prefix {
        CandidatePublicationPrefix::Baseline => "baseline",
        CandidatePublicationPrefix::Marker => "marker",
        CandidatePublicationPrefix::Lock => "lock",
        CandidatePublicationPrefix::Boundary => "boundary",
    }
}

fn parse_publication_prefix(
    value: &str,
) -> ModelResult<super::publication::CandidatePublicationPrefix> {
    use super::publication::CandidatePublicationPrefix;
    match value {
        "baseline" => Ok(CandidatePublicationPrefix::Baseline),
        "marker" => Ok(CandidatePublicationPrefix::Marker),
        "lock" => Ok(CandidatePublicationPrefix::Lock),
        "boundary" => Ok(CandidatePublicationPrefix::Boundary),
        _ => Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root preservation publication prefix is invalid",
        )
        .with_member("@root", ".")),
    }
}

fn preflight<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Vec<PreservationPlan>> {
    verify_root_publication(root, record)?;
    let snapshot = super::status::snapshot_status(backend, root, record.clone())?;
    if let Some(drift) = snapshot.operation_drift.iter().find(|drift| {
        !matches!(
            drift.kind,
            super::OperationDriftKind::RootCandidateMetadataInvalid
                | super::OperationDriftKind::RootCandidateStateChanged
        )
    }) {
        return Err(ModelError::new(ErrorCode::MergeDrift, &drift.message));
    }

    let mut plans = Vec::new();
    for target_id in &record.selected_targets {
        let participant = record.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        if participant.preservation.len() > 1 {
            return Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "participant has multiple preservation evidence rows",
            )
            .with_member(target_id, &participant.path));
        }
        let observation = snapshot.participants.get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge status is missing participant '{target_id}'"),
            )
        })?;
        if participant.pending_action.is_some()
            && observation.pending_action.as_ref().is_some_and(|pending| {
                pending.state == super::PendingActionObservationState::ExpectedConflict
            })
        {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "preserve-abort cannot verify the original conflict worktree after an interrupted merge; preserve partial resolution manually before aborting",
            )
            .with_member(target_id, &participant.path));
        }
        if !super::participant_semantics::result::is_integrated_result(participant.state) {
            if super::participant_semantics::result::is_conflicted_result(participant.state) {
                verify_pristine_conflict(backend, root, target_id, participant)?;
            }
            if !observation.abort_eligibility.eligible {
                let message = observation.drift.first().map_or(
                    "participant is not eligible for coordinated abort",
                    |item| item.message.as_str(),
                );
                return Err(ModelError::new(ErrorCode::MergeDrift, message)
                    .with_member(target_id, &participant.path));
            }
            continue;
        }
        plans.push(plan_participant(
            backend,
            root,
            record,
            target_id,
            participant,
        )?);
    }
    if record
        .publication
        .as_ref()
        .and_then(|publication| publication.composition_commit.as_ref())
        .is_some()
        && !plans.iter().any(|plan| plan.target_id == "@root")
    {
        plans.push(plan_publication_root(backend, root, record)?);
    }
    preflight_artifacts(backend, record, &mut plans)?;
    Ok(plans)
}

fn verify_pristine_conflict<B: GitBackend>(
    backend: &B,
    root: &Path,
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> ModelResult<()> {
    let expected_merge_head = participant
        .expected_merge_head
        .as_deref()
        .unwrap_or(&participant.source_commit);
    if participant.conflict_snapshot.is_empty() {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "preserve-abort cannot verify the original conflict worktree; preserve partial resolution manually before aborting",
        )
        .with_member(target_id, &participant.path));
    }
    let path = super::status::validated_participant_path(root, target_id, participant)?;
    let observed = backend
        .merge_conflict_snapshot(&path, &participant.before_commit, expected_merge_head)
        .map_err(|error| {
            ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!(
                    "preserve-abort refuses modified conflict resolution state: {}",
                    error.message
                ),
            )
            .with_member(target_id, &participant.path)
        })?;
    let observed = observed
        .files
        .into_iter()
        .map(|file| (file.path, file.sha256))
        .collect::<Vec<_>>();
    let expected = participant
        .conflict_snapshot
        .iter()
        .map(|file| (file.path.clone(), file.sha256.clone()))
        .collect::<Vec<_>>();
    if observed != expected {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "preserve-abort refuses edited conflict files; preserve partial resolution manually before aborting",
        )
        .with_member(target_id, &participant.path));
    }
    Ok(())
}
