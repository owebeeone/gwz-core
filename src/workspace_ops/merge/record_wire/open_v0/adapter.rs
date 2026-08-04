use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_yaml::Value;
use sha2::{Digest, Sha256};

use super::super::decode::DecodedV0Record;
use super::super::unknown_fields::UnknownFieldManifest;
use crate::artifact::{LockArtifact, ManifestArtifact};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::acceptance::construct_complete_lock;
use crate::workspace_ops::merge::model::v1::{
    AcceptedAttachedCheckoutV1, AcceptedIntegrationRefV1, AcceptedLockMemberV1, AcceptedLockV1,
    AcceptedMetadataBaseV1, AcceptedMetadataSourceV1, AcceptedRootBaseV1, AcceptedWorkspaceV1,
    CanonicalMergeRecord, MERGE_RECORD_SCHEMA_V1, MERGE_RECORD_SCHEMA_VERSION_V1,
    MemberAcceptanceV1, MergeOperationRecordV1, RootArtifactHashesV1, RootPublicationInputV1,
    validate_v1_record,
};
use crate::workspace_ops::merge::{
    MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeExecutionMode, MergeOperationRecord,
    MergeTargetKind, OperationState,
};

const REGISTRY: &str =
    include_str!("../../../../../dev-docs/GwzM5-8I2CompatibilityPredicates.json");

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OpenV0Adaptation {
    ValidUnlisted,
    Eligible {
        rule_id: String,
        next_action: String,
        record: Box<MergeOperationRecordV1>,
        canonical: Box<CanonicalMergeRecord>,
        unknown_fields: UnknownFieldManifest,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpenV0Eligibility {
    Candidate,
    ValidUnlisted,
}

fn classify_open_v0(record: &MergeOperationRecord) -> OpenV0Eligibility {
    if record.mode != MergeExecutionMode::Normal
        || record.state != OperationState::Finalizing
        || !record.operation_drift.is_empty()
        || record.selected_targets.len() != 1
        || record.selected_targets[0] == "@root"
        || record.participants.len() != 1
        || record.baseline.root_head.is_some()
    {
        return OpenV0Eligibility::ValidUnlisted;
    }
    let participant = record
        .participants
        .get(&record.selected_targets[0])
        .expect("v0 structural validation requires every selected participant");
    if participant.target_kind != MergeTargetKind::Member
        || !matches!(
            participant.state,
            crate::workspace_ops::merge::ParticipantState::FastForwarded
                | crate::workspace_ops::merge::ParticipantState::UpToDate
        )
        || participant.pending_action.is_some()
        || !participant.preservation.is_empty()
        || !participant.drift.is_empty()
    {
        return OpenV0Eligibility::ValidUnlisted;
    }
    if record.publication.as_ref().is_some_and(|publication| {
        publication.root_merge_commit.is_some()
            || publication.evidence_rolled_back
            || !publication.root_preservation.is_empty()
            || publication.preservation_prefix.is_some()
    }) {
        return OpenV0Eligibility::ValidUnlisted;
    }
    OpenV0Eligibility::Candidate
}

pub(crate) fn adapt_open_v0_for_r3_tests<B: GitBackend>(
    backend: &B,
    root: &Path,
    decoded: &DecodedV0Record,
    writer_version: &str,
) -> ModelResult<OpenV0Adaptation> {
    let record = decoded.record();
    validate_envelope(record)?;
    if record.mode == MergeExecutionMode::NoFf {
        return Err(ModelError::new(
            ErrorCode::UnsupportedLegacyMode,
            format!(
                "merge record '{}' uses legacy no-ff mode and cannot be migrated safely",
                record.merge_id
            ),
        ));
    }
    super::structural::validate_v0_structure(record)?;
    if classify_open_v0(record) == OpenV0Eligibility::ValidUnlisted {
        return Ok(OpenV0Adaptation::ValidUnlisted);
    }
    let Some(recovered) = super::baseline::recover_exact_baseline(backend, root, record) else {
        return Ok(OpenV0Adaptation::ValidUnlisted);
    };
    let record = &recovered;
    if !super::descriptor::is_v0_descriptor_domain(record)? {
        return Ok(OpenV0Adaptation::ValidUnlisted);
    }
    let descriptor = super::descriptor::verified_v0_descriptor(backend, root, record)?;
    let unknown_fields = decoded
        .unknown_fields()
        .map_v0_to_v1()
        .map_err(|error| unreadable(record, error.detail))?;
    let registry: Value = serde_yaml::from_str(REGISTRY)
        .map_err(|error| internal(format!("invalid I2 compatibility registry: {error}")))?;
    let rules = field(&registry, "migration_whitelist")
        .and_then(Value::as_sequence)
        .ok_or_else(|| internal("I2 registry migration_whitelist is missing"))?;
    let matches = rules
        .iter()
        .filter(|rule| field(rule, "descriptor") == Some(descriptor.value()))
        .collect::<Vec<_>>();
    let [rule] = matches.as_slice() else {
        return if matches.is_empty() {
            Ok(OpenV0Adaptation::ValidUnlisted)
        } else {
            Err(internal(
                "I2 compatibility descriptor matched multiple rules",
            ))
        };
    };
    let rule_id = text_field(rule, "id")?.to_owned();
    let next_action = text_field(
        field(rule, "classification")
            .ok_or_else(|| internal("I2 rule classification is missing"))?,
        "next_action",
    )?
    .to_owned();
    let live_next =
        crate::workspace_ops::merge::acceptance::finalization_next_action_for_i2(record)?;
    if live_next != next_action {
        return Err(internal(format!(
            "I2 rule '{rule_id}' next action '{next_action}' differs from R4a '{live_next}'"
        )));
    }

    let adapted = adapted_record(record, writer_version)?;
    let canonical = CanonicalMergeRecord::from(validate_v1_record(adapted.clone())?);
    Ok(OpenV0Adaptation::Eligible {
        rule_id,
        next_action,
        record: Box::new(adapted),
        canonical: Box::new(canonical),
        unknown_fields,
    })
}

fn adapted_record(
    record: &MergeOperationRecord,
    writer_version: &str,
) -> ModelResult<MergeOperationRecordV1> {
    let accepted_workspace = accepted_workspace(record)?;
    Ok(MergeOperationRecordV1 {
        schema: MERGE_RECORD_SCHEMA_V1.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION_V1,
        writer_version: writer_version.to_owned(),
        workspace_id: record.workspace_id.clone(),
        merge_id: record.merge_id.clone(),
        operation_id: record.operation_id.clone(),
        state: record.state,
        source_ref: record.source_ref.clone(),
        mode: record.mode,
        created_at: record.created_at.clone(),
        baseline: record.baseline.clone(),
        selected_targets: record.selected_targets.clone(),
        participants: record.participants.clone(),
        publication: record.publication.clone(),
        operation_drift: record.operation_drift.clone(),
        accepted_workspace: Some(accepted_workspace),
        recovery_context: None,
        pending_rollback: None,
        pending_preservation: None,
        extensions: record.extensions.clone(),
    })
}

fn accepted_workspace(record: &MergeOperationRecord) -> ModelResult<AcceptedWorkspaceV1> {
    let manifest_yaml = record
        .baseline
        .manifest_yaml
        .as_deref()
        .ok_or_else(|| acceptance_error(record, "baseline manifest bytes are missing"))?;
    let baseline_lock_yaml = record
        .baseline
        .lock_yaml
        .as_deref()
        .ok_or_else(|| acceptance_error(record, "baseline lock bytes are missing"))?;
    let manifest = ManifestArtifact::from_yaml(manifest_yaml)
        .map_err(|_| acceptance_error(record, "baseline manifest bytes are invalid"))?;
    let baseline_lock = LockArtifact::from_yaml(baseline_lock_yaml)
        .map_err(|_| acceptance_error(record, "baseline lock bytes are invalid"))?;
    let accepted_lock_yaml = record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .map(|candidate| Ok(candidate.lock_yaml.clone()))
        .unwrap_or_else(|| {
            construct_complete_lock(record, &manifest, baseline_lock)
                .map_err(|error| error.error)?
                .to_yaml()
        })?;
    let accepted_rows: AcceptedLockRows = serde_yaml::from_str(&accepted_lock_yaml)
        .map_err(|_| acceptance_error(record, "accepted lock rows are invalid"))?;
    let member_audit = member_audit(record, accepted_rows.members)?;
    let root_branch = record
        .baseline
        .root_branch
        .as_ref()
        .ok_or_else(|| acceptance_error(record, "accepted unborn root branch is missing"))?;

    Ok(AcceptedWorkspaceV1 {
        operation_baseline_lock_sha256: record.baseline.lock_sha256.clone(),
        metadata_base: AcceptedMetadataBaseV1 {
            source: AcceptedMetadataSourceV1::OperationBaseline,
            manifest_exact_yaml: manifest_yaml.to_owned(),
            manifest_sha256: record.baseline.manifest_sha256.clone(),
            lock_exact_yaml: baseline_lock_yaml.to_owned(),
            lock_sha256: record.baseline.lock_sha256.clone(),
        },
        lock: AcceptedLockV1 {
            sha256: digest(&accepted_lock_yaml),
            exact_yaml: accepted_lock_yaml,
        },
        member_audit,
        root: RootPublicationInputV1 {
            base: AcceptedRootBaseV1::UnbornAttached {
                symbolic_branch: root_branch.clone(),
            },
            publication_branch: Some(root_branch.clone()),
            baseline_artifact_hashes: RootArtifactHashesV1 {
                lock_worktree_sha256: record.baseline.lock_sha256.clone(),
                manifest_worktree_sha256: record.baseline.manifest_sha256.clone(),
                lock_commit_sha256: record.baseline.lock_commit_sha256.clone(),
                manifest_commit_sha256: record.baseline.manifest_commit_sha256.clone(),
            },
        },
    })
}

fn member_audit(
    record: &MergeOperationRecord,
    mut accepted_rows: BTreeMap<String, AcceptedLockMemberV1>,
) -> ModelResult<BTreeMap<String, MemberAcceptanceV1>> {
    let selected = record
        .selected_targets
        .first()
        .ok_or_else(|| acceptance_error(record, "selected member is missing"))?;
    let participant = record
        .participants
        .get(selected)
        .ok_or_else(|| acceptance_error(record, "selected participant is missing"))?;
    let resulting_commit = participant
        .resulting_commit
        .as_ref()
        .ok_or_else(|| acceptance_error(record, "selected result is missing"))?;
    let lock_member = accepted_rows
        .remove(selected)
        .ok_or_else(|| acceptance_error(record, "accepted selected lock row is missing"))?;
    if !accepted_rows.is_empty() {
        return Err(acceptance_error(
            record,
            "accepted lock contains an unclassified member",
        ));
    }
    Ok(BTreeMap::from([(
        selected.clone(),
        MemberAcceptanceV1::Selected {
            integration: AcceptedIntegrationRefV1 {
                branch: participant.target_branch.clone(),
                before_commit: participant.before_commit.clone(),
                resulting_commit: resulting_commit.clone(),
            },
            final_checkout: AcceptedAttachedCheckoutV1 {
                branch: participant.target_branch.clone(),
                commit: resulting_commit.clone(),
            },
            lock_member,
        },
    )]))
}

#[derive(Deserialize)]
struct AcceptedLockRows {
    members: BTreeMap<String, AcceptedLockMemberV1>,
}

fn validate_envelope(record: &MergeOperationRecord) -> ModelResult<()> {
    if record.schema != MERGE_RECORD_SCHEMA
        || record.record_schema_version != MERGE_RECORD_SCHEMA_VERSION
    {
        return Err(unreadable(record, "v0 envelope is inconsistent"));
    }
    Ok(())
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value
        .as_mapping()
        .and_then(|mapping| mapping.get(Value::String(name.to_owned())))
}

fn text_field<'a>(value: &'a Value, name: &str) -> ModelResult<&'a str> {
    field(value, name)
        .and_then(Value::as_str)
        .ok_or_else(|| internal(format!("I2 registry field '{name}' is missing")))
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn unreadable(record: &MergeOperationRecord, detail: impl Into<String>) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecordUnreadable,
        format!(
            "merge record '{}' cannot be adapted: {}",
            record.merge_id,
            detail.into()
        ),
    )
}

fn acceptance_error(record: &MergeOperationRecord, detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::AcceptanceInputDrift,
        format!(
            "merge record '{}' acceptance input is incomplete: {detail}",
            record.merge_id
        ),
    )
}

fn internal(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InternalError, detail)
}
