use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::artifact::{LockArtifact, ManifestArtifact};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::super::{OperationState, ParticipantState};
use super::super::{
    AcceptedLockMemberV1, AcceptedMetadataSourceV1, AcceptedRootBaseV1, AcceptedWorkspaceV1,
    MemberAcceptanceV1, MergeOperationRecordV1,
};

pub(crate) fn validate_v1_acceptance(record: &MergeOperationRecordV1) -> ModelResult<()> {
    validate_lifetime(record)?;
    let Some(accepted) = record.accepted_workspace.as_ref() else {
        return Ok(());
    };
    validate_operation_baseline(record)?;
    validate_metadata_base(record, accepted)?;
    let accepted_lock = parse_lock(record, &accepted.lock.exact_yaml)?;
    if digest(&accepted.lock.exact_yaml) != accepted.lock.sha256 {
        return Err(acceptance_input_error(record));
    }
    let accepted_rows = parse_lock_rows(record, &accepted.lock.exact_yaml)?;
    validate_audit(record, accepted, &accepted_lock, &accepted_rows)?;
    validate_root(record, accepted)?;
    validate_candidate(record, accepted)
}

fn validate_operation_baseline(record: &MergeOperationRecordV1) -> ModelResult<()> {
    let exact = record
        .baseline
        .manifest_yaml
        .as_deref()
        .is_some_and(|yaml| digest(yaml) == record.baseline.manifest_sha256)
        && record
            .baseline
            .lock_yaml
            .as_deref()
            .is_some_and(|yaml| digest(yaml) == record.baseline.lock_sha256);
    if exact {
        Ok(())
    } else {
        Err(acceptance_input_error(record))
    }
}

fn validate_lifetime(record: &MergeOperationRecordV1) -> ModelResult<()> {
    let acceptance_too_early = record.accepted_workspace.is_some()
        && matches!(
            record.state,
            OperationState::Executing | OperationState::AwaitingResolution | OperationState::Halted
        );
    if acceptance_too_early {
        return Err(unexpected_acceptance(
            record,
            "accepted workspace is present before complete participant validation",
        ));
    }
    // The publication object is itself the durable classification decision.
    // Acceptance must therefore precede even an empty progress object and the
    // deterministic no-publication `complete` shape.
    let publication_without_acceptance =
        record.accepted_workspace.is_none() && record.publication.is_some();
    if publication_without_acceptance {
        return Err(unexpected_acceptance(
            record,
            "publication evidence exists without accepted workspace",
        ));
    }
    Ok(())
}

fn validate_metadata_base(
    record: &MergeOperationRecordV1,
    accepted: &AcceptedWorkspaceV1,
) -> ModelResult<()> {
    if accepted.operation_baseline_lock_sha256 != record.baseline.lock_sha256
        || digest(&accepted.metadata_base.manifest_exact_yaml)
            != accepted.metadata_base.manifest_sha256
        || digest(&accepted.metadata_base.lock_exact_yaml) != accepted.metadata_base.lock_sha256
    {
        return Err(acceptance_input_error(record));
    }
    let manifest = parse_manifest(record, &accepted.metadata_base.manifest_exact_yaml)?;
    let lock = parse_lock(record, &accepted.metadata_base.lock_exact_yaml)?;
    if manifest.workspace.id != record.workspace_id || lock.workspace_id != record.workspace_id {
        return Err(acceptance_input_error(record));
    }
    let selected_root = selected_root(record);
    match (&accepted.metadata_base.source, selected_root) {
        (AcceptedMetadataSourceV1::OperationBaseline, None) => {
            if record.baseline.manifest_yaml.as_deref()
                != Some(accepted.metadata_base.manifest_exact_yaml.as_str())
                || record.baseline.lock_yaml.as_deref()
                    != Some(accepted.metadata_base.lock_exact_yaml.as_str())
                || record.baseline.manifest_sha256 != accepted.metadata_base.manifest_sha256
                || record.baseline.lock_sha256 != accepted.metadata_base.lock_sha256
            {
                return Err(acceptance_input_error(record));
            }
        }
        (AcceptedMetadataSourceV1::SelectedRootResult { commit }, Some(root)) => {
            if root.resulting_commit.as_deref() != Some(commit.as_str()) {
                return Err(selected_result_error(record));
            }
        }
        _ => return Err(acceptance_input_error(record)),
    }
    Ok(())
}

fn validate_audit(
    record: &MergeOperationRecordV1,
    accepted: &AcceptedWorkspaceV1,
    accepted_lock: &LockArtifact,
    accepted_rows: &BTreeMap<String, AcceptedLockMemberV1>,
) -> ModelResult<()> {
    if accepted_lock.workspace_id != record.workspace_id {
        return Err(acceptance_input_error(record));
    }
    let manifest = parse_manifest(record, &accepted.metadata_base.manifest_exact_yaml)?;
    let metadata_lock = parse_lock(record, &accepted.metadata_base.lock_exact_yaml)?;
    let metadata_rows = parse_lock_rows(record, &accepted.metadata_base.lock_exact_yaml)?;
    let baseline_manifest = parse_manifest(
        record,
        record
            .baseline
            .manifest_yaml
            .as_deref()
            .ok_or_else(|| acceptance_input_error(record))?,
    )?;
    let baseline_rows = parse_lock_rows(
        record,
        record
            .baseline
            .lock_yaml
            .as_deref()
            .ok_or_else(|| acceptance_input_error(record))?,
    )?;
    let selected_members: BTreeSet<&str> = record
        .selected_targets
        .iter()
        .filter(|target| target.as_str() != "@root")
        .map(String::as_str)
        .collect();
    let mut domain: BTreeSet<String> = metadata_lock.members.keys().cloned().collect();
    domain.extend(
        manifest
            .members
            .iter()
            .filter(|member| member.active)
            .map(|member| member.id.clone()),
    );
    domain.extend(selected_members.iter().map(|member| (*member).to_owned()));
    if accepted
        .member_audit
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        != domain
    {
        return Err(acceptance_input_error(record));
    }

    let mut expected_lock_keys = BTreeSet::new();
    for (member_id, audit) in &accepted.member_audit {
        match audit {
            MemberAcceptanceV1::Selected {
                integration,
                final_checkout,
                lock_member,
            } => {
                let Some(participant) = record.participants.get(member_id) else {
                    return Err(selected_result_error(record));
                };
                let result = participant.resulting_commit.as_deref();
                if !selected_members.contains(member_id.as_str())
                    || !accepted_result_state_is_legal(record, participant)
                    || integration.branch != participant.target_branch
                    || integration.before_commit != participant.before_commit
                    || result != Some(integration.resulting_commit.as_str())
                    || final_checkout.branch != integration.branch
                    || final_checkout.commit != integration.resulting_commit
                    || accepted_rows.get(member_id) != Some(lock_member)
                    || lock_member.path != participant.path
                    || lock_member.commit.as_deref() != result
                    || lock_member.branch.as_deref() != Some(participant.target_branch.as_str())
                    || lock_member.detached != Some(false)
                    || lock_member.dirty != Some(false)
                    || lock_member.materialized != Some(true)
                {
                    return Err(selected_result_error(record));
                }
                let metadata_identity = manifest
                    .members
                    .iter()
                    .find(|member| member.id == *member_id);
                let baseline_identity = baseline_manifest
                    .members
                    .iter()
                    .find(|member| member.id == *member_id);
                let baseline_lock_identity = baseline_rows.get(member_id);
                let frozen_present =
                    baseline_identity.is_some() || baseline_lock_identity.is_some();
                let baseline_manifest_matches = baseline_identity.is_none_or(|member| {
                    member.path == lock_member.path
                        && member.source_id == lock_member.source_id
                        && member.source_kind == lock_member.source_kind
                });
                let baseline_lock_matches = baseline_lock_identity.is_none_or(|member| {
                    member.path == lock_member.path
                        && member.source_id == lock_member.source_id
                        && member.source_kind == lock_member.source_kind
                });
                let metadata_matches = metadata_identity.is_none_or(|member| {
                    member.path == lock_member.path
                        && member.source_id == lock_member.source_id
                        && member.source_kind == lock_member.source_kind
                });
                let identity_matches = frozen_present
                    && baseline_manifest_matches
                    && baseline_lock_matches
                    && metadata_matches;
                if !identity_matches {
                    return Err(acceptance_input_error(record));
                }
                expected_lock_keys.insert(member_id.clone());
            }
            MemberAcceptanceV1::UnselectedPresent { lock_member } => {
                if selected_members.contains(member_id.as_str())
                    || metadata_rows.get(member_id) != Some(lock_member)
                    || accepted_rows.get(member_id) != Some(lock_member)
                {
                    return Err(acceptance_input_error(record));
                }
                expected_lock_keys.insert(member_id.clone());
            }
            MemberAcceptanceV1::Absent => {
                if selected_members.contains(member_id.as_str())
                    || accepted_rows.contains_key(member_id)
                    || metadata_rows.contains_key(member_id)
                {
                    return Err(acceptance_input_error(record));
                }
            }
        }
    }
    if accepted_rows.keys().cloned().collect::<BTreeSet<_>>() != expected_lock_keys {
        return Err(acceptance_input_error(record));
    }
    Ok(())
}

fn validate_root(
    record: &MergeOperationRecordV1,
    accepted: &AcceptedWorkspaceV1,
) -> ModelResult<()> {
    let hashes = &accepted.root.baseline_artifact_hashes;
    if hashes.lock_worktree_sha256 != record.baseline.lock_sha256
        || hashes.manifest_worktree_sha256 != record.baseline.manifest_sha256
        || hashes.lock_commit_sha256 != record.baseline.lock_commit_sha256
        || hashes.manifest_commit_sha256 != record.baseline.manifest_commit_sha256
    {
        return Err(acceptance_input_error(record));
    }
    if let Some(root) = selected_root(record) {
        let expected = root.resulting_commit.as_deref();
        if !accepted_result_state_is_legal(record, root)
            || !matches!(
                &accepted.root.base,
                AcceptedRootBaseV1::BornAttached { commit, symbolic_branch }
                    if Some(commit.as_str()) == expected && symbolic_branch == &root.target_branch
            )
            || accepted.root.publication_branch.as_deref() != Some(root.target_branch.as_str())
            || hashes.lock_commit_sha256.is_none()
            || hashes.manifest_commit_sha256.is_none()
        {
            return Err(acceptance_input_error(record));
        }
    } else {
        match &accepted.root.base {
            AcceptedRootBaseV1::BornAttached {
                commit,
                symbolic_branch,
            } => {
                if record.baseline.root_head.as_deref() != Some(commit.as_str())
                    || record.baseline.root_branch.as_deref() != Some(symbolic_branch.as_str())
                    || accepted.root.publication_branch.as_deref() != Some(symbolic_branch.as_str())
                {
                    return Err(acceptance_input_error(record));
                }
            }
            AcceptedRootBaseV1::BornDetached { commit } => {
                if record.baseline.root_head.as_deref() != Some(commit.as_str())
                    || record.baseline.root_branch.is_some()
                    || accepted.root.publication_branch.is_some()
                    || publication_required_for_v1(record)
                {
                    return Err(acceptance_input_error(record));
                }
            }
            AcceptedRootBaseV1::UnbornAttached { symbolic_branch } => {
                if record.baseline.root_head.is_some()
                    || record.baseline.root_branch.as_deref() != Some(symbolic_branch.as_str())
                    || accepted.root.publication_branch.as_deref() != Some(symbolic_branch.as_str())
                {
                    return Err(acceptance_input_error(record));
                }
            }
        }
    }
    Ok(())
}

fn validate_candidate(
    record: &MergeOperationRecordV1,
    accepted: &AcceptedWorkspaceV1,
) -> ModelResult<()> {
    let Some(publication) = record.publication.as_ref() else {
        return Ok(());
    };
    let Some(candidate) = publication.candidate.as_ref() else {
        return Ok(());
    };
    if candidate.lock_yaml != accepted.lock.exact_yaml
        || publication.candidate_lock_sha256.as_deref() != Some(accepted.lock.sha256.as_str())
        || candidate.baseline_lock_yaml != accepted.metadata_base.lock_exact_yaml
        || accepted.root.publication_branch.as_deref() != Some(candidate.root_branch.as_str())
        || validate_candidate_semantics_for_v1(record).is_err()
    {
        return Err(candidate_error(record));
    }
    Ok(())
}

fn selected_root(
    record: &MergeOperationRecordV1,
) -> Option<&super::super::super::MergeParticipantRecord> {
    record
        .selected_targets
        .iter()
        .any(|target| target == "@root")
        .then(|| record.participants.get("@root"))
        .flatten()
}

pub(crate) fn publication_required_for_v1(record: &MergeOperationRecordV1) -> bool {
    crate::workspace_ops::merge::acceptance::publication_required(&accepted_semantic_view(record))
}

pub(crate) fn validate_candidate_semantics_for_v1(
    record: &MergeOperationRecordV1,
) -> ModelResult<()> {
    crate::workspace_ops::merge::finalize::validate_candidate_for_i2_fixture(
        &accepted_semantic_view(record),
    )
}

fn accepted_semantic_view(
    record: &MergeOperationRecordV1,
) -> super::super::super::MergeOperationRecordV0 {
    let mut view = record.v0_common_view();
    if record.accepted_workspace.is_some() {
        for participant in view.participants.values_mut() {
            participant.state = match participant.state {
                ParticipantState::Aborted => ParticipantState::UpToDate,
                ParticipantState::RolledBack => ParticipantState::Merged,
                state => state,
            };
        }
    }
    view
}

fn accepted_result_state_is_legal(
    record: &MergeOperationRecordV1,
    participant: &super::super::super::MergeParticipantRecord,
) -> bool {
    let changed_result = participant
        .resulting_commit
        .as_deref()
        .is_some_and(|result| result != participant.before_commit);
    match participant.state {
        ParticipantState::UpToDate if !changed_result => return true,
        ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued
            if changed_result =>
        {
            return true;
        }
        _ => {}
    }
    let rollback_lifecycle = matches!(
        record.state,
        OperationState::RollingBack | OperationState::Aborted
    ) || record.state == OperationState::RecoveryRequired
        && record.recovery_context.as_ref().is_some_and(|context| {
            context.origin_state == super::super::RecoveryOriginStateV1::RollingBack
        });
    if !rollback_lifecycle {
        return false;
    }
    match participant.state {
        ParticipantState::Aborted => {
            participant.resulting_commit.as_deref() == Some(participant.before_commit.as_str())
        }
        ParticipantState::RolledBack => changed_result,
        _ => false,
    }
}

fn parse_manifest(record: &MergeOperationRecordV1, yaml: &str) -> ModelResult<ManifestArtifact> {
    ManifestArtifact::from_yaml(yaml).map_err(|_| acceptance_input_error(record))
}

fn parse_lock(record: &MergeOperationRecordV1, yaml: &str) -> ModelResult<LockArtifact> {
    LockArtifact::from_yaml(yaml).map_err(|_| acceptance_input_error(record))
}

#[derive(Deserialize)]
struct AcceptedLockRows {
    members: BTreeMap<String, AcceptedLockMemberV1>,
}

fn parse_lock_rows(
    record: &MergeOperationRecordV1,
    yaml: &str,
) -> ModelResult<BTreeMap<String, AcceptedLockMemberV1>> {
    serde_yaml::from_str::<AcceptedLockRows>(yaml)
        .map(|lock| lock.members)
        .map_err(|_| acceptance_input_error(record))
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn unexpected_acceptance(record: &MergeOperationRecordV1, reason: &str) -> ModelError {
    typed_error(
        record,
        ErrorCode::UnexpectedAcceptanceEvidence,
        "has unexpected acceptance evidence",
        reason,
    )
}

fn acceptance_input_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::AcceptanceInputDrift,
        "acceptance input changed",
        "the accepted metadata base cannot be verified from its recorded source",
    )
}

fn selected_result_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::AcceptanceInputDrift,
        "acceptance input changed",
        "a selected participant result no longer matches its durable result",
    )
}

fn candidate_error(record: &MergeOperationRecordV1) -> ModelError {
    typed_error(
        record,
        ErrorCode::CandidateIntegrityMismatch,
        "candidate integrity check failed",
        "candidate bytes or digest do not match accepted workspace",
    )
}

fn typed_error(
    record: &MergeOperationRecordV1,
    code: ErrorCode,
    prefix: &str,
    reason: &str,
) -> ModelError {
    ModelError::new(
        code,
        format!("merge record '{}' {prefix}: {reason}", record.merge_id),
    )
}
