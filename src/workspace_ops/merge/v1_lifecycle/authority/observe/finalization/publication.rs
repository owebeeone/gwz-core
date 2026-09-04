use super::*;
use crate::git::{GitRepositoryState, GitScopedCommitResult, MergeAuthorityBackend};
use crate::workspace_ops::merge::PublicationStep;
use crate::workspace_ops::merge::acceptance::{
    CandidatePublicationPrefix, V1CandidateBuildInput, build_v1_candidate, candidate_artifacts,
    classify_frozen_v1_publication, v1_candidate_files, v1_composition_message,
    v1_publication_base,
};
use crate::workspace_ops::workspace_exclude_candidate;

mod live;

use live::*;

pub(super) enum ReversePublicationHandoffObservation {
    RecordEvidence(Box<VerifiedEvidenceResult>),
    Ready(PublicationHandoffFact),
}

pub(super) fn observe_reverse_handoff<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<ReversePublicationHandoffObservation> {
    let Some(progress) = current.record().publication.as_ref() else {
        // No publication progress AND no frozen acceptance: the operation is
        // `Finalizing` but acceptance never froze. There provably is no
        // candidate and no evidence commit -- both are built from the accepted
        // workspace -- so the handoff is `NoCandidate` on structural grounds,
        // and reversibility is judged against the participants alone.
        //
        // Asking `verify_accepted_inputs` here instead is what made such a
        // merge unabortable: `verify_accepted_root` demands an
        // `accepted_workspace` that does not exist and answers
        // `AcceptanceInputDrift`, which `is_semantic_drift` maps to "inexact",
        // so every merge that failed at acceptance -- root metadata invalid or
        // otherwise -- was refused its abort with "publication state is not an
        // exact reversible handoff". v0 had no such gate: `abort/mod.rs` went
        // from the open-record checks straight to its evidence preflight
        // (`git show 57502e4:src/workspace_ops/merge/abort/mod.rs`).
        if current.record().accepted_workspace.is_none() {
            require_handoff_exact(verification_is_exact(verify_participants(
                backend, current,
            ))?)?;
            return Ok(ReversePublicationHandoffObservation::Ready(
                PublicationHandoffFact::NoCandidate,
            ));
        }
        require_handoff_exact(verification_is_exact(verify_accepted_inputs(
            backend, current,
        ))?)?;
        return Ok(ReversePublicationHandoffObservation::Ready(
            PublicationHandoffFact::NoCandidate,
        ));
    };
    match progress.step {
        PublicationStep::NotStarted | PublicationStep::ValidatingResults => {
            require_handoff_exact(verification_is_exact(verify_accepted_inputs(
                backend, current,
            ))?)?;
            Ok(ReversePublicationHandoffObservation::Ready(
                PublicationHandoffFact::NoCandidate,
            ))
        }
        PublicationStep::PreparingCandidate if progress.candidate.is_none() => {
            require_handoff_exact(verification_is_exact(verify_accepted_inputs(
                backend, current,
            ))?)?;
            Ok(ReversePublicationHandoffObservation::Ready(
                PublicationHandoffFact::NoCandidate,
            ))
        }
        PublicationStep::PreparingCandidate => {
            require_handoff_exact(verification_is_exact(verify_accepted_inputs(
                backend, current,
            ))?)?;
            let Some(observed) = snapshot(backend, current)? else {
                return Err(reverse_handoff_error());
            };
            require_handoff_exact(
                observed == (CandidatePublicationPrefix::Baseline, IndexForm::Pre),
            )?;
            Ok(ReversePublicationHandoffObservation::Ready(
                candidate_handoff_fact(observed),
            ))
        }
        PublicationStep::CommittingEvidence if progress.composition_commit.is_none() => {
            if evidence_base_is_live(backend, current)? {
                return Ok(ReversePublicationHandoffObservation::Ready(
                    PublicationHandoffFact::EvidencePending,
                ));
            }
            require_handoff_exact(verification_is_exact(verify_post_evidence_inputs(
                backend, current,
            ))?)?;
            let Some(result) = observed_evidence(backend, current)? else {
                return Err(reverse_handoff_error());
            };
            let proof = VerifiedEvidenceResult::issue(
                &AuthorityIssuer::for_observer(current),
                "@publication",
                "record_evidence",
                "completed",
                evidence_payload(current.record(), result),
            )?;
            Ok(ReversePublicationHandoffObservation::RecordEvidence(
                Box::new(proof),
            ))
        }
        PublicationStep::CommittingEvidence => {
            require_post_evidence_handoff(backend, current)?;
            let Some(observed) = snapshot(backend, current)? else {
                return Err(reverse_handoff_error());
            };
            require_handoff_exact(
                observed == (CandidatePublicationPrefix::Baseline, IndexForm::Pre),
            )?;
            Ok(ReversePublicationHandoffObservation::Ready(
                candidate_handoff_fact(observed),
            ))
        }
        PublicationStep::PublishingCandidate => {
            require_post_evidence_handoff(backend, current)?;
            let Some(observed) = snapshot(backend, current)? else {
                return Err(reverse_handoff_error());
            };
            require_handoff_exact(
                publication_resolution(current, Some(observed))?
                    != PublicationResolution::Ambiguous,
            )?;
            Ok(ReversePublicationHandoffObservation::Ready(
                candidate_handoff_fact(observed),
            ))
        }
        PublicationStep::VerifyingPublication | PublicationStep::Complete
            if progress.candidate.is_none() =>
        {
            require_handoff_exact(verification_is_exact(verify_accepted_inputs(
                backend, current,
            ))?)?;
            Ok(ReversePublicationHandoffObservation::Ready(
                PublicationHandoffFact::NoCandidate,
            ))
        }
        PublicationStep::VerifyingPublication | PublicationStep::Complete => {
            require_post_evidence_handoff(backend, current)?;
            let Some(observed) = snapshot(backend, current)? else {
                return Err(reverse_handoff_error());
            };
            require_handoff_exact(
                publication_resolution(current, Some(observed))? == PublicationResolution::Complete,
            )?;
            Ok(ReversePublicationHandoffObservation::Ready(
                candidate_handoff_fact(observed),
            ))
        }
    }
}

fn require_post_evidence_handoff<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<()> {
    require_handoff_exact(verification_is_exact(verify_post_evidence_inputs(
        backend, current,
    ))?)?;
    require_handoff_exact(recorded_evidence_is_live(backend, current)?)
}

fn require_handoff_exact(exact: bool) -> ModelResult<()> {
    if exact {
        Ok(())
    } else {
        Err(reverse_handoff_error())
    }
}

fn reverse_handoff_error() -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        "publication state is not an exact reversible handoff",
    )
    .with_member("@root", ".")
}

fn candidate_handoff_fact(
    (prefix, index): (CandidatePublicationPrefix, IndexForm),
) -> PublicationHandoffFact {
    PublicationHandoffFact::Candidate {
        prefix: match prefix {
            CandidatePublicationPrefix::Baseline => PublicationHandoffPrefix::Baseline,
            CandidatePublicationPrefix::Marker => PublicationHandoffPrefix::Marker,
            CandidatePublicationPrefix::Lock => PublicationHandoffPrefix::Lock,
            CandidatePublicationPrefix::Boundary => PublicationHandoffPrefix::Boundary,
        },
        index: match index {
            IndexForm::Pre => PublicationHandoffIndex::Pre,
            IndexForm::Staged => PublicationHandoffIndex::Staged,
        },
    }
}

pub(super) fn observe<B: MergeAuthorityBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    let record = current.record();
    let Some(progress) = record.publication.as_ref() else {
        verify_accepted_inputs(backend, current)?;
        return decision(current, classify_frozen_v1_publication(record)?);
    };
    match progress.step {
        PublicationStep::NotStarted => {
            verify_accepted_inputs(backend, current)?;
            Ok(completed(CompletedObservation::Publication(
                PublicationObservation::MigratedValidationReady,
            )))
        }
        PublicationStep::ValidatingResults => {
            verify_accepted_inputs(backend, current)?;
            migrated_decision(current, classify_frozen_v1_publication(record)?)
        }
        PublicationStep::PreparingCandidate if progress.candidate.is_none() => {
            prepare_candidate(backend, context, current)
        }
        PublicationStep::PreparingCandidate => begin_evidence(backend, current),
        PublicationStep::CommittingEvidence if progress.composition_commit.is_none() => {
            observe_evidence(backend, current)
        }
        PublicationStep::CommittingEvidence => begin_publication(backend, current),
        PublicationStep::PublishingCandidate => observe_publication(backend, current),
        PublicationStep::VerifyingPublication => verify_publication(backend, current, false),
        PublicationStep::Complete => verify_publication(backend, current, true),
    }
}

pub(super) fn verify_action<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    expected: PublicationPhysicalAction,
) -> ModelResult<()> {
    let observed = action_for_state(backend, current)?;
    if observed == Some(expected) {
        Ok(())
    } else {
        Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root changed after publication action authorization",
        )
        .with_member("@root", "."))
    }
}

pub(super) fn recovery_origin_is_exact<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<bool> {
    let Some(progress) = current.record().publication.as_ref() else {
        return verification_is_exact(verify_accepted_inputs(backend, current));
    };
    match progress.step {
        PublicationStep::NotStarted | PublicationStep::ValidatingResults => {
            verification_is_exact(verify_accepted_inputs(backend, current))
        }
        PublicationStep::PreparingCandidate if progress.candidate.is_none() => {
            verification_is_exact(verify_accepted_inputs(backend, current))
        }
        PublicationStep::PreparingCandidate => Ok(verification_is_exact(verify_accepted_inputs(
            backend, current,
        ))? && snapshot(backend, current)?
            == Some((CandidatePublicationPrefix::Baseline, IndexForm::Pre))),
        PublicationStep::CommittingEvidence if progress.composition_commit.is_none() => {
            evidence_base_is_live(backend, current)
        }
        PublicationStep::CommittingEvidence => Ok(verification_is_exact(
            verify_post_evidence_inputs(backend, current),
        )? && recorded_evidence_is_live(
            backend, current,
        )? && snapshot(backend, current)?
            == Some((CandidatePublicationPrefix::Baseline, IndexForm::Pre))),
        PublicationStep::PublishingCandidate => {
            Ok(
                verification_is_exact(verify_post_evidence_inputs(backend, current))?
                    && recorded_evidence_is_live(backend, current)?
                    && publication_resolution(current, snapshot(backend, current)?)?
                        != PublicationResolution::Ambiguous,
            )
        }
        PublicationStep::VerifyingPublication | PublicationStep::Complete => {
            Ok(if progress.candidate.is_none() {
                verification_is_exact(verify_accepted_inputs(backend, current))?
            } else {
                verification_is_exact(verify_post_evidence_inputs(backend, current))?
                    && recorded_evidence_is_live(backend, current)?
                    && publication_resolution(current, snapshot(backend, current)?)?
                        == PublicationResolution::Complete
            })
        }
    }
}

fn decision(current: &StoredV1Record, required: bool) -> ModelResult<ExactObservationFact> {
    let phase = if required { "required" } else { "none" };
    let proof = BoundPublicationDecision::issue(
        &AuthorityIssuer::for_observer(current),
        "@publication",
        "classify_publication",
        phase,
        required,
    )?;
    Ok(completed(CompletedObservation::Publication(
        PublicationObservation::Decision(proof),
    )))
}

fn migrated_decision(
    current: &StoredV1Record,
    required: bool,
) -> ModelResult<ExactObservationFact> {
    let phase = if required {
        "publication_required"
    } else {
        "no_publication"
    };
    let proof = VerifiedResults::issue(
        &AuthorityIssuer::for_observer(current),
        "@publication",
        "validate_migrated_results",
        phase,
        required,
    )?;
    Ok(completed(CompletedObservation::Publication(
        PublicationObservation::MigratedResults(proof),
    )))
}

fn prepare_candidate<B: MergeAuthorityBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    verify_publication_path_parents(current.location().root())?;
    if !verification_is_exact(verify_accepted_inputs(backend, current))? {
        return ambiguity(current);
    }
    let record = current.record();
    let (manifest, lock) = candidate_artifacts(record)?;
    let (baseline_boundary, boundary) =
        workspace_exclude_candidate(backend, current.location().root(), &manifest, &lock)?;
    let root_head = backend.head(current.location().root())?;
    let marker_id = crate::workspace_ops::handle_commit::new_uuid_v7()?;
    let actor_id = context
        .attribution
        .as_ref()
        .and_then(|value| value.actor.as_ref())
        .map_or("unknown", |actor| actor.actor_id.as_str());
    let built = build_v1_candidate(
        record,
        V1CandidateBuildInput {
            marker_id: &marker_id,
            actor_id,
            root_head: &root_head,
            baseline_boundary_text: &baseline_boundary,
            boundary_text: &boundary,
        },
    )?;
    let proof = PreparedCandidate::issue(
        &AuthorityIssuer::for_observer(current),
        "@publication",
        "prepare_candidate",
        "prepared",
        CandidatePayload {
            candidate: built.candidate,
            marker_path: built.marker_path,
            lock_sha256: built.lock_sha256,
        },
    )?;
    Ok(completed(CompletedObservation::Publication(
        PublicationObservation::Candidate(Box::new(proof)),
    )))
}

fn begin_evidence<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    verify_publication_path_parents(current.location().root())?;
    verify_accepted_inputs(backend, current)?;
    if snapshot(backend, current)? != Some((CandidatePublicationPrefix::Baseline, IndexForm::Pre)) {
        return ambiguity(current);
    }
    let proof = PreparedEvidenceIntent::issue(
        &AuthorityIssuer::for_observer(current),
        "@publication",
        "begin_evidence",
        "preflight",
        (),
    )?;
    Ok(completed(CompletedObservation::Publication(
        PublicationObservation::EvidenceIntent(proof),
    )))
}

fn observe_evidence<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    if evidence_base_is_live(backend, current)? {
        return publication_action(current, PublicationPhysicalAction::EvidenceCommit);
    }
    if !verification_is_exact(verify_post_evidence_inputs(backend, current))? {
        return ambiguity(current);
    }
    let Some(result) = observed_evidence(backend, current)? else {
        return ambiguity(current);
    };
    let proof = VerifiedEvidenceResult::issue(
        &AuthorityIssuer::for_observer(current),
        "@publication",
        "record_evidence",
        "completed",
        evidence_payload(current.record(), result),
    )?;
    Ok(completed(CompletedObservation::Publication(
        PublicationObservation::EvidenceResult(Box::new(proof)),
    )))
}

fn begin_publication<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    if !verification_is_exact(verify_post_evidence_inputs(backend, current))? {
        return ambiguity(current);
    }
    if !recorded_evidence_is_live(backend, current)?
        || snapshot(backend, current)?
            != Some((CandidatePublicationPrefix::Baseline, IndexForm::Pre))
    {
        return ambiguity(current);
    }
    let proof = PreparedPublicationIntent::issue(
        &AuthorityIssuer::for_observer(current),
        "@publication",
        "begin_candidate_publication",
        "preflight",
        (),
    )?;
    Ok(completed(CompletedObservation::Publication(
        PublicationObservation::PublicationIntent(proof),
    )))
}

fn observe_publication<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    if !verification_is_exact(verify_post_evidence_inputs(backend, current))? {
        return ambiguity(current);
    }
    if !recorded_evidence_is_live(backend, current)? {
        return ambiguity(current);
    }
    match publication_resolution(current, snapshot(backend, current)?)? {
        PublicationResolution::Action(action) => publication_action(current, action),
        PublicationResolution::Complete => {
            let proof = VerifiedCandidatePublicationCompletion::issue(
                &AuthorityIssuer::for_observer(current),
                "@publication",
                "candidate_publication",
                "completed",
                (),
            )?;
            Ok(completed(CompletedObservation::Publication(
                PublicationObservation::CandidatePublished(proof),
            )))
        }
        PublicationResolution::Ambiguous => ambiguity(current),
    }
}

fn verify_publication<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    operation_complete: bool,
) -> ModelResult<ExactObservationFact> {
    let progress = current.record().publication.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "publication verification requires publication progress",
        )
    })?;
    let exact = if progress.candidate.is_none() {
        verification_is_exact(verify_accepted_inputs(backend, current))?
    } else {
        verification_is_exact(verify_post_evidence_inputs(backend, current))?
            && recorded_evidence_is_live(backend, current)?
            && publication_resolution(current, snapshot(backend, current)?)?
                == PublicationResolution::Complete
    };
    if !exact {
        return ambiguity(current);
    }
    let (owner, action) = if operation_complete {
        ("@operation", "publication_complete")
    } else {
        ("@publication", "publication")
    };
    let proof = VerifiedPublicationCompletion::issue(
        &AuthorityIssuer::for_observer(current),
        owner,
        action,
        "verified",
        (),
    )?;
    let observation = if operation_complete {
        PublicationObservation::OperationComplete(proof)
    } else {
        PublicationObservation::PublicationVerified(proof)
    };
    Ok(completed(CompletedObservation::Publication(observation)))
}

fn action_for_state<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<Option<PublicationPhysicalAction>> {
    let progress = current.record().publication.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "publication progress is missing",
        )
    })?;
    match progress.step {
        PublicationStep::CommittingEvidence if progress.composition_commit.is_none() => {
            Ok(evidence_base_is_live(backend, current)?
                .then_some(PublicationPhysicalAction::EvidenceCommit))
        }
        PublicationStep::PublishingCandidate => {
            verify_post_evidence_inputs(backend, current)?;
            if !recorded_evidence_is_live(backend, current)? {
                return Ok(None);
            }
            Ok(
                match publication_resolution(current, snapshot(backend, current)?)? {
                    PublicationResolution::Action(action) => Some(action),
                    PublicationResolution::Complete | PublicationResolution::Ambiguous => None,
                },
            )
        }
        _ => Ok(None),
    }
}

fn publication_action(
    current: &StoredV1Record,
    action: PublicationPhysicalAction,
) -> ModelResult<ExactObservationFact> {
    let proof = VerifiedPublicationAction::issue(
        &AuthorityIssuer::for_observer(current),
        "@publication",
        "publication_action",
        action_phase(action),
        action,
    )?;
    Ok(ExactObservationFact::NotStarted(
        NotStartedObservation::Publication(proof),
    ))
}

fn verify_accepted_inputs<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<()> {
    verify_participants(backend, current)?;
    verify_accepted_root(backend, current)
}

fn verify_post_evidence_inputs<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<()> {
    verify_publication_path_parents(current.location().root())?;
    verify_non_root_participants(backend, current)?;
    verify_frozen_manifest(backend, current)
}

fn evidence_base_is_live<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<bool> {
    verify_publication_path_parents(current.location().root())?;
    Ok(
        verification_is_exact(verify_accepted_inputs(backend, current))?
            && snapshot(backend, current)?
                == Some((CandidatePublicationPrefix::Baseline, IndexForm::Pre)),
    )
}

fn observed_evidence<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<Option<GitScopedCommitResult>> {
    let record = current.record();
    let root = current.location().root();
    let (parent, branch) = v1_publication_base(record)?;
    let head = backend.head(root)?;
    if backend.repository_state(root)? != GitRepositoryState::Clean
        || head.is_detached
        || head.branch.as_deref() != Some(branch)
    {
        return Ok(None);
    }
    let Some(commit) = head.commit.as_deref() else {
        return Ok(None);
    };
    if Some(commit) == parent {
        return Ok(None);
    }
    match backend.verify_gwz_paths_commit(
        root,
        commit,
        parent,
        &v1_candidate_files(record)?,
        &v1_composition_message(record),
    ) {
        Ok(result) => Ok(Some(result)),
        Err(error) if is_semantic_drift(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn verification_is_exact(result: ModelResult<()>) -> ModelResult<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if is_semantic_drift(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

fn is_semantic_drift(error: &ModelError) -> bool {
    matches!(
        error.code,
        ErrorCode::MergeDrift | ErrorCode::AcceptanceInputDrift
    )
}

fn recorded_evidence_is_live<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<bool> {
    let progress = current.record().publication.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "recorded evidence verification requires publication progress",
        )
    })?;
    let Some(recorded_commit) = progress.composition_commit.as_deref() else {
        return Ok(false);
    };
    let Some(observed) = observed_evidence(backend, current)? else {
        return Ok(false);
    };
    Ok(recorded_commit == observed.commit
        && progress.composition_tree.as_deref() == Some(observed.tree.as_str())
        && progress.candidate_hashes.len() == observed.candidate_hashes.len()
        && progress
            .candidate_hashes
            .iter()
            .zip(&observed.candidate_hashes)
            .all(|(recorded, live)| recorded.path == live.path && recorded.sha256 == live.sha256))
}

fn evidence_payload(
    record: &MergeOperationRecordV1,
    observed: GitScopedCommitResult,
) -> EvidencePayload {
    EvidencePayload {
        composition_commit: observed.commit,
        composition_tree: observed.tree,
        root_merge_commit: record
            .participants
            .get("@root")
            .and_then(|row| row.resulting_commit.clone()),
        candidate_hashes: observed
            .candidate_hashes
            .into_iter()
            .map(|hash| PublicationCandidateHash {
                path: hash.path,
                sha256: hash.sha256,
            })
            .collect(),
    }
}

fn action_phase(action: PublicationPhysicalAction) -> &'static str {
    match action {
        PublicationPhysicalAction::EvidenceCommit => "evidence_commit",
        PublicationPhysicalAction::WriteMarker => "write_marker",
        PublicationPhysicalAction::WriteLock => "write_lock",
        PublicationPhysicalAction::WriteBoundary => "write_boundary",
        PublicationPhysicalAction::StageIndex => "stage_index",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_verification_error_is_not_reclassified_as_ambiguity() {
        let error = ModelError::new(ErrorCode::IoError, "injected read failure");
        assert_eq!(
            verification_is_exact(Err(error.clone())).unwrap_err(),
            error
        );
        assert!(
            !verification_is_exact(Err(ModelError::new(
                ErrorCode::MergeDrift,
                "semantic drift"
            )))
            .unwrap()
        );
    }
}
