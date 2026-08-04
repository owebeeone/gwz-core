use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelResult};
use crate::operation::{EventEmitter, OperationContext};

use super::acceptance::{FinalizationNextAction, finalization_next_action, publication_required};
use super::finalize::{
    CandidatePreparationError, ensure_composition_commit, prepare_candidate, publish_candidate,
    validate_candidate, verify_publication,
};
use super::finalize_support::{
    block_root, complete_and_archive, progress, progress_mut, record_root_metadata_invalid,
    set_step, sha256, unreadable, verified_participants,
};
use super::marker::marker_merge_from_verified;
use super::{MergeOperationRecord, MergeStore, PublicationProgress, PublicationStep};

pub(super) fn finalize<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
) -> ModelResult<bool> {
    let mut previous_action = None;
    let mut verified_for_candidate = None;
    loop {
        let action = finalization_next_action(record)?;
        match action {
            FinalizationNextAction::ArchiveCompleted => {
                super::archive_merge_record(store, root, &record.merge_id, emitter)?;
                return Ok(true);
            }
            FinalizationNextAction::ValidateResults => {
                ensure_publication_progress(store, root, record, emitter)?;
                set_step(
                    store,
                    root,
                    record,
                    PublicationStep::ValidatingResults,
                    emitter,
                )?;
                let Some(verified) = verified_participants(backend, store, root, record, emitter)?
                else {
                    return Ok(false);
                };
                if !publication_required(record) {
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
                verified_for_candidate = Some(verified);
            }
            FinalizationNextAction::PrepareCandidate => {
                let verified = match verified_for_candidate.take() {
                    Some(verified) => verified,
                    None => {
                        let Some(verified) =
                            verified_participants(backend, store, root, record, emitter)?
                        else {
                            return Ok(false);
                        };
                        verified
                    }
                };
                if !prepare_and_record_candidate(
                    backend, store, root, record, context, emitter, &verified,
                )? {
                    return Ok(false);
                }
                set_step(
                    store,
                    root,
                    record,
                    PublicationStep::CommittingEvidence,
                    emitter,
                )?;
            }
            FinalizationNextAction::CreateOrAdoptEvidence => {
                if previous_action != Some(FinalizationNextAction::PrepareCandidate) {
                    let Some(_) = verified_participants(backend, store, root, record, emitter)?
                    else {
                        return Ok(false);
                    };
                }
                validate_candidate(record)?;
                set_step(
                    store,
                    root,
                    record,
                    PublicationStep::CommittingEvidence,
                    emitter,
                )?;
                let Some(verified) = verified_participants(backend, store, root, record, emitter)?
                else {
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
            }
            FinalizationNextAction::PublishCandidate => {
                if previous_action != Some(FinalizationNextAction::CreateOrAdoptEvidence)
                    && !replay_evidence_stage(backend, store, root, record, emitter)?
                {
                    return Ok(false);
                }
                set_step(
                    store,
                    root,
                    record,
                    PublicationStep::PublishingCandidate,
                    emitter,
                )?;
                let Some(verified) = verified_participants(backend, store, root, record, emitter)?
                else {
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
            }
            FinalizationNextAction::VerifyPublication => {
                if previous_action != Some(FinalizationNextAction::PublishCandidate)
                    && !replay_publication_stage(backend, store, root, record, emitter)?
                {
                    return Ok(false);
                }
                let Some(verified) = verified_participants(backend, store, root, record, emitter)?
                else {
                    return Ok(false);
                };
                marker_merge_from_verified(record, &verified)?;
                if !verify_publication(backend, store, root, record, emitter)? {
                    return Ok(false);
                }
                set_step(store, root, record, PublicationStep::Complete, emitter)?;
                complete_and_archive(store, root, record, emitter)?;
                return Ok(true);
            }
            FinalizationNextAction::CompleteNoPublication => {
                let Some(_) = verified_participants(backend, store, root, record, emitter)? else {
                    return Ok(false);
                };
                if publication_required(record) {
                    return Err(unreadable("publication candidate is missing"));
                }
                complete_and_archive(store, root, record, emitter)?;
                return Ok(true);
            }
        }
        previous_action = Some(action);
    }
}

fn ensure_publication_progress<S: MergeStore>(
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<()> {
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
            root_preservation: Vec::new(),
            preservation_prefix: None,
        });
        super::persist_merge_record(store, root, record, emitter)?;
    }
    Ok(())
}

fn prepare_and_record_candidate<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    context: &OperationContext,
    emitter: &EventEmitter<'_>,
    verified: &[super::marker::VerifiedMergeParticipant],
) -> ModelResult<bool> {
    if progress(record)?.candidate.is_none() {
        let prepared = match prepare_candidate(backend, root, record, context, verified) {
            Ok(prepared) => prepared,
            Err(CandidatePreparationError::Other(error)) if error.code == ErrorCode::MergeDrift => {
                return block_root(store, root, record, emitter, &error.message);
            }
            Err(CandidatePreparationError::Metadata(error)) => {
                record_root_metadata_invalid(store, root, record, emitter, &error.message)?;
                return Err(error);
            }
            Err(CandidatePreparationError::Other(error)) => return Err(error),
        };
        super::finalize_support::clear_root_metadata_drift(record);
        let root_merge_commit = super::root::root_merge_commit(record)?.map(str::to_owned);
        let publication = progress_mut(record)?;
        publication.candidate_lock_sha256 = Some(sha256(prepared.lock_yaml.as_bytes()));
        publication.candidate_marker_path = Some(format!(
            "{}/{}.yaml",
            artifact::MARKER_DIR,
            prepared.marker_id
        ));
        publication.root_merge_commit = root_merge_commit;
        publication.candidate = Some(prepared);
        super::persist_merge_record(store, root, record, emitter)?;
    }
    validate_candidate(record)?;
    Ok(true)
}

fn replay_evidence_stage<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<bool> {
    let Some(_) = verified_participants(backend, store, root, record, emitter)? else {
        return Ok(false);
    };
    validate_candidate(record)?;
    let Some(verified) = verified_participants(backend, store, root, record, emitter)? else {
        return Ok(false);
    };
    marker_merge_from_verified(record, &verified)?;
    ensure_composition_commit(backend, store, root, record, emitter)
}

fn replay_publication_stage<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    record: &mut MergeOperationRecord,
    emitter: &EventEmitter<'_>,
) -> ModelResult<bool> {
    if !replay_evidence_stage(backend, store, root, record, emitter)? {
        return Ok(false);
    }
    let Some(verified) = verified_participants(backend, store, root, record, emitter)? else {
        return Ok(false);
    };
    marker_merge_from_verified(record, &verified)?;
    publish_candidate(backend, store, root, record, emitter)
}
