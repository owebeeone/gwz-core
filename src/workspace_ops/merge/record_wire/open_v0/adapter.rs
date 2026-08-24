use std::path::Path;

use serde_yaml::Value;

use super::super::decode::DecodedV0Record;
use super::super::unknown_fields::UnknownFieldManifest;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::acceptance::{
    V1AcceptanceMetadata, V1AcceptanceRecord, build_v1_acceptance,
};
use crate::workspace_ops::merge::model::v1::{
    CanonicalMergeRecord, MERGE_RECORD_SCHEMA_V1, MERGE_RECORD_SCHEMA_VERSION_V1,
    MergeOperationRecordV1, validate_v1_record,
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

pub(crate) fn adapt_open_v0<B: GitBackend>(
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
    let mut unknown_fields = decoded
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
    let adapted_raw = serde_yaml::to_value(&adapted)
        .map_err(|error| internal(format!("cannot inspect adapted v1 record: {error}")))?;
    let adapted_unknowns = UnknownFieldManifest::extract_v1(&adapted_raw).map_err(|error| {
        internal(format!(
            "cannot inspect adapted v1 unknowns: {}",
            error.detail
        ))
    })?;
    unknown_fields
        .authorize_derived_accepted_lock_members(&adapted_unknowns)
        .map_err(|error| unreadable(record, error.detail))?;
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
    let accepted = build_v1_acceptance(
        V1AcceptanceRecord::V0(record),
        V1AcceptanceMetadata::OperationBaseline,
    )?;
    if accepted.publication_required()
        != crate::workspace_ops::merge::acceptance::publication_required(record)
    {
        return Err(internal(
            "shared v1 acceptance publication classification differs from R4a",
        ));
    }
    let accepted_workspace = accepted.into_accepted_workspace();
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
        preservation_publication_handoff: None,
        extensions: record.extensions.clone(),
    })
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

fn internal(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InternalError, detail)
}
