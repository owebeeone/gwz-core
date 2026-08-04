use std::path::Path;

use serde::Serialize;
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};

use crate::artifact::{LockArtifact, ManifestArtifact};
use crate::git::{GitBackend, GitRepositoryState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::{
    MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeExecutionMode, MergeOperationRecord,
    MergeParticipantRecord, MergeTargetKind, OperationState, ParticipantState, PublicationStep,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DescriptorEvidence {
    participant_state: &'static str,
    result_relation: &'static str,
    publication_presence: &'static str,
    publication_step: &'static str,
    candidate_relation: &'static str,
    composition_relation: &'static str,
    hash_relation: &'static str,
    root_observation: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VerifiedV0Descriptor {
    value: Value,
}

impl VerifiedV0Descriptor {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }
}

pub(super) fn is_v0_descriptor_domain(record: &MergeOperationRecord) -> ModelResult<bool> {
    let lock_yaml = record
        .baseline
        .lock_yaml
        .as_deref()
        .ok_or_else(|| acceptance_drift(record, "baseline lock bytes are missing"))?;
    let manifest_yaml = record
        .baseline
        .manifest_yaml
        .as_deref()
        .ok_or_else(|| acceptance_drift(record, "baseline manifest bytes are missing"))?;
    let manifest = ManifestArtifact::from_yaml(manifest_yaml)
        .map_err(|_| acceptance_drift(record, "baseline manifest is invalid"))?;
    let lock = LockArtifact::from_yaml(lock_yaml)
        .map_err(|_| acceptance_drift(record, "baseline lock is invalid"))?;
    Ok(manifest.members.len() == 1 && lock.members.len() == 1)
}

pub(crate) fn verified_v0_descriptor<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<VerifiedV0Descriptor> {
    let mut evidence = validate_record_evidence(record)?;
    evidence.root_observation = validate_live_evidence(backend, root, record)?;
    Ok(VerifiedV0Descriptor {
        value: descriptor_value(evidence),
    })
}

fn validate_record_evidence(record: &MergeOperationRecord) -> ModelResult<DescriptorEvidence> {
    if record.schema != MERGE_RECORD_SCHEMA
        || record.record_schema_version != MERGE_RECORD_SCHEMA_VERSION
        || record.mode != MergeExecutionMode::Normal
        || record.state != OperationState::Finalizing
        || !record.operation_drift.is_empty()
        || record.selected_targets.len() != 1
        || record.selected_targets[0] == "@root"
        || record.participants.len() != 1
    {
        return Err(unreadable(
            record,
            "record is outside the migration descriptor domain",
        ));
    }
    let participant = record
        .participants
        .get(&record.selected_targets[0])
        .ok_or_else(|| unreadable(record, "selected participant is missing"))?;
    if participant.target_kind != MergeTargetKind::Member
        || participant.pending_action.is_some()
        || !participant.conflict_paths.is_empty()
        || !participant.conflict_snapshot.is_empty()
        || participant.error.is_some()
        || !participant.preservation.is_empty()
        || !participant.drift.is_empty()
    {
        return Err(unreadable(
            record,
            "participant is outside the migration descriptor domain",
        ));
    }
    let result = participant
        .resulting_commit
        .as_deref()
        .ok_or_else(|| acceptance_drift(record, "selected participant result is missing"))?;
    let derived_participant = match participant.state {
        ParticipantState::FastForwarded
            if result != participant.before_commit && result == participant.source_commit =>
        {
            ("fast_forwarded", "changed_exact")
        }
        ParticipantState::UpToDate if result == participant.before_commit => {
            ("up_to_date", "equals_before")
        }
        _ => {
            return Err(acceptance_drift(
                record,
                "participant result relation is not exact",
            ));
        }
    };
    validate_baseline(record, participant)?;
    let derived_publication = publication_shape(record)?;
    Ok(DescriptorEvidence {
        participant_state: derived_participant.0,
        result_relation: derived_participant.1,
        publication_presence: derived_publication.0,
        publication_step: derived_publication.1,
        candidate_relation: derived_publication.2,
        composition_relation: derived_publication.3,
        hash_relation: derived_publication.4,
        root_observation: "",
    })
}

fn validate_live_evidence<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<&'static str> {
    let participant = &record.participants[&record.selected_targets[0]];
    let result = participant.resulting_commit.as_deref().unwrap();
    let member_path = root.join(&participant.path);
    for commit in [
        participant.before_commit.as_str(),
        participant.source_commit.as_str(),
        result,
    ] {
        if !backend
            .commit_exists(&member_path, commit)
            .map_err(|_| acceptance_drift(record, "participant commit evidence is unavailable"))?
        {
            return Err(acceptance_drift(
                record,
                "participant commit evidence is unavailable",
            ));
        }
    }
    let head = backend
        .head(&member_path)
        .map_err(|_| acceptance_drift(record, "participant HEAD is unavailable"))?;
    let target_ref = backend
        .read_ref(
            &member_path,
            &format!("refs/heads/{}", participant.target_branch),
        )
        .map_err(|_| acceptance_drift(record, "participant target ref is unavailable"))?;
    if head.is_detached
        || head.branch.as_deref() != Some(participant.target_branch.as_str())
        || head.commit.as_deref() != Some(result)
        || target_ref.as_deref() != Some(result)
        || backend
            .repository_state(&member_path)
            .map_err(|_| acceptance_drift(record, "participant repository state is unavailable"))?
            != GitRepositoryState::Clean
        || backend
            .status(&member_path)
            .map_err(|_| acceptance_drift(record, "participant status is unavailable"))?
            .is_dirty
    {
        return Err(acceptance_drift(
            record,
            "participant live state is not the exact migration input",
        ));
    }
    let observation = crate::workspace_ops::merge::publication::normalized_i2_root_observation(
        backend, root, record,
    )
    .map_err(|failure| root_observation_error(record, failure))?;
    Ok(observation)
}

fn validate_baseline(
    record: &MergeOperationRecord,
    participant: &MergeParticipantRecord,
) -> ModelResult<()> {
    let lock_yaml = record
        .baseline
        .lock_yaml
        .as_deref()
        .ok_or_else(|| acceptance_drift(record, "baseline lock bytes are missing"))?;
    let manifest_yaml = record
        .baseline
        .manifest_yaml
        .as_deref()
        .ok_or_else(|| acceptance_drift(record, "baseline manifest bytes are missing"))?;
    if digest(lock_yaml) != record.baseline.lock_sha256
        || digest(manifest_yaml) != record.baseline.manifest_sha256
        || record.baseline.lock_commit_sha256.is_some()
        || record.baseline.manifest_commit_sha256.is_some()
        || record.baseline.root_head.is_some()
        || record.baseline.root_branch.is_none()
    {
        return Err(acceptance_drift(
            record,
            "baseline descriptor evidence is inconsistent",
        ));
    }
    let manifest = ManifestArtifact::from_yaml(manifest_yaml)
        .map_err(|_| acceptance_drift(record, "baseline manifest is invalid"))?;
    let lock = LockArtifact::from_yaml(lock_yaml)
        .map_err(|_| acceptance_drift(record, "baseline lock is invalid"))?;
    let selected = &record.selected_targets[0];
    let member = manifest
        .members
        .iter()
        .find(|member| member.id == *selected && member.active)
        .ok_or_else(|| acceptance_drift(record, "selected manifest member is missing"))?;
    let locked = lock
        .members
        .get(selected)
        .ok_or_else(|| acceptance_drift(record, "selected baseline lock row is missing"))?;
    if manifest.workspace.id != record.workspace_id
        || lock.workspace_id != record.workspace_id
        || manifest.members.len() != 1
        || lock.members.len() != 1
        || member.path != participant.path
        || locked.path != participant.path
        || locked.source_id.as_deref() != Some(member.source_id.as_str())
        || locked.source_kind != member.source_kind
        || locked.commit.as_deref() != Some(participant.before_commit.as_str())
    {
        return Err(acceptance_drift(
            record,
            "baseline member identity is inconsistent",
        ));
    }
    Ok(())
}

fn publication_shape(
    record: &MergeOperationRecord,
) -> ModelResult<(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)> {
    let Some(publication) = record.publication.as_ref() else {
        return Ok(("absent", "absent", "absent", "absent", "empty"));
    };
    if publication.root_merge_commit.is_some()
        || publication.evidence_rolled_back
        || !publication.root_preservation.is_empty()
        || publication.preservation_prefix.is_some()
    {
        return Err(unreadable(
            record,
            "publication owns excluded root or reverse evidence",
        ));
    }
    let candidate = if publication.candidate.is_some() {
        crate::workspace_ops::merge::finalize::validate_candidate_for_i2_fixture(record)?;
        "complete_valid"
    } else if publication.candidate_lock_sha256.is_none()
        && publication.candidate_marker_path.is_none()
    {
        "absent"
    } else {
        return Err(unreadable(
            record,
            "publication candidate evidence is partial",
        ));
    };
    let (composition, hashes) = match (
        publication.composition_commit.as_ref(),
        publication.composition_tree.as_ref(),
        publication.candidate_hashes.is_empty(),
    ) {
        (None, None, true) => ("absent", "empty"),
        (Some(_), Some(_), false) => ("complete_valid", "canonical_valid"),
        _ => return Err(unreadable(record, "composition evidence is partial")),
    };
    Ok((
        "present",
        publication_step(publication.step),
        candidate,
        composition,
        hashes,
    ))
}

fn descriptor_value(evidence: DescriptorEvidence) -> Value {
    object([
        ("location", value("open")),
        ("mode", value("normal")),
        (
            "operation",
            object([
                ("state", value("finalizing")),
                ("drift", value(Vec::<String>::new())),
            ]),
        ),
        (
            "selection",
            object([
                ("ordered_ids", value(["p0"])),
                ("root_selected", value(false)),
            ]),
        ),
        (
            "participants",
            value([object([
                ("id", value("p0")),
                ("path", value("selected_path")),
                ("target_kind", value("member")),
                ("target_branch", value("attached_live_branch")),
                ("state", value(evidence.participant_state)),
                ("result", value(evidence.result_relation)),
                (
                    "pending",
                    object([
                        ("kind", value("absent")),
                        ("expected", value("absent")),
                        ("commit_spec", value("absent")),
                    ]),
                ),
                ("conflict", value("absent")),
                ("error", value("absent")),
                ("preservation", value("absent")),
                ("drift", value(Vec::<String>::new())),
            ])]),
        ),
        (
            "baseline",
            object([
                ("lock_yaml", value("present_digest_valid")),
                ("manifest_yaml", value("present_digest_valid")),
                ("lock_commit_hash", value("absent")),
                ("manifest_commit_hash", value("absent")),
                ("root_checkout", value("unborn_attached")),
                ("root_commit_hash", value("absent")),
            ]),
        ),
        (
            "publication",
            object([
                ("presence", value(evidence.publication_presence)),
                ("step", value(evidence.publication_step)),
                ("candidate", value(evidence.candidate_relation)),
                ("composition", value(evidence.composition_relation)),
                ("hashes", value(evidence.hash_relation)),
                ("root_merge", value("absent")),
                ("evidence_rolled_back", value(false)),
                ("root_preservation", value("absent")),
                ("preservation_prefix", value("absent")),
            ]),
        ),
        (
            "observation",
            object([
                (
                    "participants",
                    value([object([
                        ("id", value("p0")),
                        ("action", value("none")),
                        ("head", value("equals_result")),
                        ("target_ref", value("equals_result")),
                        ("index", value("clean")),
                        ("worktree", value("clean")),
                    ])]),
                ),
                ("root", value(evidence.root_observation)),
                ("preservation", value("none")),
                ("rollback", value("none")),
            ]),
        ),
    ])
}

fn publication_step(step: PublicationStep) -> &'static str {
    match step {
        PublicationStep::NotStarted => "not_started",
        PublicationStep::ValidatingResults => "validating_results",
        PublicationStep::PreparingCandidate => "preparing_candidate",
        PublicationStep::CommittingEvidence => "committing_evidence",
        PublicationStep::PublishingCandidate => "publishing_candidate",
        PublicationStep::VerifyingPublication => "verifying_publication",
        PublicationStep::Complete => "complete",
    }
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn unreadable(record: &MergeOperationRecord, detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecordUnreadable,
        format!(
            "merge record '{}' is not migration-safe: {detail}",
            record.merge_id
        ),
    )
}

fn acceptance_drift(record: &MergeOperationRecord, detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::AcceptanceInputDrift,
        format!(
            "merge record '{}' acceptance input changed: {detail}",
            record.merge_id
        ),
    )
}

fn root_observation_error(
    record: &MergeOperationRecord,
    failure: crate::workspace_ops::merge::publication::I2RootObservationFailure,
) -> ModelError {
    use crate::workspace_ops::merge::publication::I2RootObservationFailure as Failure;

    let (code, detail) = match failure {
        Failure::AcceptanceInputDrift => (
            ErrorCode::AcceptanceInputDrift,
            "the live root no longer matches the accepted baseline metadata or checkout",
        ),
        Failure::CandidateIntegrityMismatch => (
            ErrorCode::CandidateIntegrityMismatch,
            "the durable publication candidate is internally inconsistent",
        ),
        Failure::AmbiguousEvidenceCommit => (
            ErrorCode::AmbiguousEvidenceCommit,
            "live root is neither the accepted base nor one exact unrecorded evidence commit",
        ),
        Failure::RecordedEvidenceDrift => (
            ErrorCode::RecordedEvidenceDrift,
            "recorded composition commit, tree, parent, message, files, or hashes changed",
        ),
        Failure::PublicationPrefixMismatch => (
            ErrorCode::PublicationPrefixMismatch,
            "filesystem or index does not match one legal recorded publication prefix",
        ),
    };
    ModelError::new(
        code,
        format!(
            "merge record '{}' root observation failed: {detail}",
            record.merge_id
        ),
    )
}

fn value<T: Serialize>(input: T) -> Value {
    serde_yaml::to_value(input).expect("compatibility descriptor must serialize")
}

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Mapping(
        fields
            .into_iter()
            .map(|(key, value)| (Value::String(key.to_owned()), value))
            .collect::<Mapping>(),
    )
}
