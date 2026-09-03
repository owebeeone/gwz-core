use std::collections::BTreeSet;

use super::super::super::model::archive_projection::*;
use super::MergeOperationRecordV0;
use super::super::super::{
    OperationState, PublicationProgress, PublicationStep,
};
use super::v0_audit::{complete_audit, legacy_members};
use super::v0_evidence::{
    BaselineEvidence, project_root, validate_baseline, validate_candidate, validate_common,
};

pub(super) fn project(record: &MergeOperationRecordV0) -> Result<ArchivedMergeProjection, ()> {
    validate_common(record)?;
    let baseline = validate_baseline(record)?;
    let terminal_outcome = match record.state {
        OperationState::Completed => ArchivedTerminalOutcome::Completed,
        OperationState::Aborted => ArchivedTerminalOutcome::Aborted,
        _ => return Err(()),
    };
    let acceptance = match record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
    {
        Some(_) => project_candidate(record, &baseline)?,
        None if terminal_outcome == ArchivedTerminalOutcome::Aborted => {
            validate_pre_acceptance(record)?;
            ArchivedAcceptanceProjection::NotAccepted
        }
        None => project_baseline(record, &baseline)?,
    };
    Ok(ArchivedMergeProjection {
        source_version: ArchiveSourceVersion::V0,
        terminal_outcome,
        acceptance,
    })
}

fn project_candidate(
    record: &MergeOperationRecordV0,
    baseline: &BaselineEvidence,
) -> Result<ArchivedAcceptanceProjection, ()> {
    let publication = record.publication.as_ref().ok_or(())?;
    let candidate = publication.candidate.as_ref().ok_or(())?;
    let source_required = match record.state {
        OperationState::Completed => {
            crate::workspace_ops::merge::acceptance::publication_required(&record.participants)
        }
        OperationState::Aborted => record.participants.values().any(|participant| {
            participant.state == super::super::super::ParticipantState::RolledBack
        }),
        _ => false,
    };
    if !source_required {
        return Err(());
    }
    let validated = validate_candidate(record, publication)?;
    validate_candidate_terminal(record, publication, validated.composition_complete)?;
    let root = project_root(record, Some(&candidate.root_branch))?;
    let audit = complete_audit(
        record,
        baseline.manifest.as_ref(),
        &validated.lock,
        Some(&validated.metadata_base_lock),
    )?;
    let mut missing = BTreeSet::new();
    if !validated.exact_lock {
        missing.insert(LegacyAcceptanceGap::ExactLockBytes);
    }
    if audit.is_none() {
        missing.insert(LegacyAcceptanceGap::CompleteMemberAudit);
    }
    if root.is_none() {
        missing.insert(LegacyAcceptanceGap::AcceptedRootInput);
    }
    if !validated.publication_complete {
        missing.insert(LegacyAcceptanceGap::PublicationEvidence);
    }
    if missing.is_empty() {
        return Ok(ArchivedAcceptanceProjection::LegacyComplete {
            workspace: ArchivedAcceptedWorkspace {
                baseline_lock_sha256: record.baseline.lock_sha256.clone(),
                lock_yaml: candidate.lock_yaml.clone(),
                lock_sha256: publication.candidate_lock_sha256.clone().ok_or(())?,
                members: audit.ok_or(())?,
                root: root.ok_or(())?,
            },
            source: LegacyAcceptanceSource::Candidate,
        });
    }
    Ok(ArchivedAcceptanceProjection::LegacyUnavailable {
        available: evidence(
            record,
            Some(&candidate.lock_yaml),
            publication.candidate_lock_sha256.as_deref(),
            Some(&validated.lock),
            root,
            Some(publication),
        )?,
        missing,
    })
}

fn project_baseline(
    record: &MergeOperationRecordV0,
    baseline: &BaselineEvidence,
) -> Result<ArchivedAcceptanceProjection, ()> {
    validate_no_publication_completion(record)?;
    let root = project_root(record, None)?;
    let audit = baseline
        .lock
        .as_ref()
        .map(|lock| complete_audit(record, baseline.manifest.as_ref(), lock, Some(lock)))
        .transpose()?
        .flatten();
    let mut missing = BTreeSet::new();
    if baseline.lock.is_none() {
        missing.insert(LegacyAcceptanceGap::ExactLockBytes);
    }
    if audit.is_none() {
        missing.insert(LegacyAcceptanceGap::CompleteMemberAudit);
    }
    if root.is_none() {
        missing.insert(LegacyAcceptanceGap::AcceptedRootInput);
    }
    if missing.is_empty() {
        return Ok(ArchivedAcceptanceProjection::LegacyComplete {
            workspace: ArchivedAcceptedWorkspace {
                baseline_lock_sha256: record.baseline.lock_sha256.clone(),
                lock_yaml: record.baseline.lock_yaml.clone().ok_or(())?,
                lock_sha256: record.baseline.lock_sha256.clone(),
                members: audit.ok_or(())?,
                root: root.ok_or(())?,
            },
            source: LegacyAcceptanceSource::BaselineNoPublication,
        });
    }
    Ok(ArchivedAcceptanceProjection::LegacyUnavailable {
        available: evidence(
            record,
            record.baseline.lock_yaml.as_deref(),
            baseline
                .lock
                .as_ref()
                .map(|_| record.baseline.lock_sha256.as_str()),
            baseline.lock.as_ref(),
            root,
            record.publication.as_ref(),
        )?,
        missing,
    })
}

fn validate_candidate_terminal(
    record: &MergeOperationRecordV0,
    publication: &PublicationProgress,
    composition_complete: bool,
) -> Result<(), ()> {
    let composition_absent = publication.composition_commit.is_none()
        && publication.composition_tree.is_none()
        && publication.candidate_hashes.is_empty();
    let phase_valid = match publication.step {
        PublicationStep::PreparingCandidate => !composition_complete,
        PublicationStep::CommittingEvidence => true,
        PublicationStep::PublishingCandidate | PublicationStep::VerifyingPublication => {
            composition_complete
        }
        PublicationStep::Complete => {
            composition_complete || record.state == OperationState::Completed && composition_absent
        }
        PublicationStep::NotStarted | PublicationStep::ValidatingResults => false,
    };
    if !phase_valid {
        return Err(());
    }
    match record.state {
        OperationState::Completed => {
            if publication.step != PublicationStep::Complete || publication.evidence_rolled_back {
                return Err(());
            }
        }
        OperationState::Aborted => {
            if composition_complete != publication.evidence_rolled_back {
                return Err(());
            }
        }
        _ => return Err(()),
    }
    Ok(())
}

fn validate_no_publication_completion(record: &MergeOperationRecordV0) -> Result<(), ()> {
    let publication = record.publication.as_ref().ok_or(())?;
    let has_output = publication.candidate_lock_sha256.is_some()
        || publication.candidate_marker_path.is_some()
        || publication.root_merge_commit.is_some()
        || publication.composition_commit.is_some()
        || publication.composition_tree.is_some()
        || !publication.candidate_hashes.is_empty()
        || publication.evidence_rolled_back
        || !publication.root_preservation.is_empty()
        || publication.preservation_prefix.is_some();
    if publication.step == PublicationStep::Complete
        && !has_output
        && !crate::workspace_ops::merge::acceptance::publication_required(&record.participants)
        && record
            .participants
            .values()
            .all(|participant| participant.state == super::super::super::ParticipantState::UpToDate)
    {
        Ok(())
    } else {
        Err(())
    }
}

fn validate_pre_acceptance(record: &MergeOperationRecordV0) -> Result<(), ()> {
    let Some(publication) = record.publication.as_ref() else {
        return Ok(());
    };
    let output = publication.candidate_lock_sha256.is_some()
        || publication.candidate_marker_path.is_some()
        || publication.root_merge_commit.is_some()
        || publication.composition_commit.is_some()
        || publication.composition_tree.is_some()
        || !publication.candidate_hashes.is_empty()
        || publication.evidence_rolled_back
        || !publication.root_preservation.is_empty()
        || publication.preservation_prefix.is_some();
    if output
        || !matches!(
            publication.step,
            PublicationStep::NotStarted
                | PublicationStep::ValidatingResults
                | PublicationStep::PreparingCandidate
        )
    {
        Err(())
    } else {
        Ok(())
    }
}

fn evidence(
    record: &MergeOperationRecordV0,
    lock_yaml: Option<&str>,
    lock_sha256: Option<&str>,
    lock: Option<&crate::artifact::LockArtifact>,
    root: Option<AcceptedRootProjection>,
    publication: Option<&PublicationProgress>,
) -> Result<LegacyAcceptanceEvidence, ()> {
    Ok(LegacyAcceptanceEvidence {
        lock_yaml: lock_yaml.map(str::to_owned),
        lock_sha256: lock_sha256.map(str::to_owned),
        members: legacy_members(record, lock)?,
        root,
        composition_commit: publication.and_then(|value| value.composition_commit.clone()),
        composition_tree: publication.and_then(|value| value.composition_tree.clone()),
        candidate_hashes: publication
            .into_iter()
            .flat_map(|value| &value.candidate_hashes)
            .map(|hash| AcceptedCandidateHashProjection {
                path: hash.path.clone(),
                sha256: hash.sha256.clone(),
            })
            .collect(),
    })
}
