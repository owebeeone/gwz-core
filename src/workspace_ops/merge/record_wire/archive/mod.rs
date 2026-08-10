use serde_yaml::Value;

use super::super::model::MergeOperationRecord;
#[cfg(test)]
use super::super::model::v1::{MergeOperationRecordV1, validate_v1_record};
use super::header::{
    HeaderClassificationError, HeaderMalformedReason, InstalledMergeRecordVersions,
    MergeRecordDispatch, MergeRecordHeader, classify_merge_record_header, read_merge_record_header,
};
use super::raw_yaml::parse_strict_yaml;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::{MergeRecordCompatibilityContext, MergeRecordRequiredWave};

mod cleanup;
mod v0;
mod v0_audit;
mod v0_evidence;
#[cfg(test)]
mod v1;

pub(crate) use cleanup::ArchivedCleanupWorklist;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ValidatedArchivedRecord {
    projection: super::super::model::archive_projection::ArchivedMergeProjection,
    cleanup: ArchivedCleanupWorklist,
}

impl ValidatedArchivedRecord {
    pub(crate) fn projection(
        &self,
    ) -> &super::super::model::archive_projection::ArchivedMergeProjection {
        &self.projection
    }

    #[allow(
        dead_code,
        reason = "P4 consumes cleanup behind the disabled lifecycle"
    )]
    pub(crate) fn cleanup(&self) -> &ArchivedCleanupWorklist {
        &self.cleanup
    }
}

pub(crate) fn decode_archived_v0(
    bytes: &[u8],
    expected_merge_id: &str,
) -> ModelResult<ValidatedArchivedRecord> {
    let document =
        parse_strict_yaml(bytes).map_err(|_| archived_unreadable(expected_merge_id, None))?;
    let header = read_merge_record_header(&document).map_err(|reason| {
        let context = mismatch_header(&reason);
        archived_unreadable(expected_merge_id, context.as_ref())
    })?;
    let dispatch =
        classify_merge_record_header(&header, InstalledMergeRecordVersions::PRODUCTION_R3)
            .map_err(|error| classify_error(expected_merge_id, error))?;
    let raw = document.into_root();
    match dispatch {
        MergeRecordDispatch::V0 => decode_v0(raw, expected_merge_id, &header),
        MergeRecordDispatch::V1 => Err(unsupported(
            expected_merge_id,
            &header,
            Some(MergeRecordRequiredWave::A1),
        )),
    }
}

#[cfg(test)]
pub(crate) fn decode_archived_for_r3_tests(
    bytes: &[u8],
    expected_merge_id: &str,
) -> ModelResult<ValidatedArchivedRecord> {
    let document =
        parse_strict_yaml(bytes).map_err(|_| archived_unreadable(expected_merge_id, None))?;
    let header = read_merge_record_header(&document).map_err(|reason| {
        let context = mismatch_header(&reason);
        archived_unreadable(expected_merge_id, context.as_ref())
    })?;
    let dispatch = classify_merge_record_header(
        &header,
        InstalledMergeRecordVersions::V0_AND_V1_FOR_R3_TESTS,
    )
    .map_err(|error| classify_error(expected_merge_id, error))?;
    let raw = document.into_root();
    match dispatch {
        MergeRecordDispatch::V0 => decode_v0(raw, expected_merge_id, &header),
        MergeRecordDispatch::V1 => decode_v1(raw, expected_merge_id, &header),
    }
}

fn decode_v0(
    raw: Value,
    expected_merge_id: &str,
    header: &MergeRecordHeader,
) -> ModelResult<ValidatedArchivedRecord> {
    let record: MergeOperationRecord = serde_yaml::from_value(raw)
        .map_err(|_| archived_unreadable(expected_merge_id, Some(header)))?;
    validate_identity(&record.merge_id, expected_merge_id, header)?;
    let projection =
        v0::project(&record).map_err(|_| archived_unreadable(expected_merge_id, Some(header)))?;
    let cleanup = cleanup::from_v0(&record)
        .map_err(|error| cleanup_unreadable(expected_merge_id, header, error))?;
    Ok(ValidatedArchivedRecord {
        projection,
        cleanup,
    })
}

#[cfg(test)]
fn decode_v1(
    raw: Value,
    expected_merge_id: &str,
    header: &MergeRecordHeader,
) -> ModelResult<ValidatedArchivedRecord> {
    let record: MergeOperationRecordV1 = serde_yaml::from_value(raw)
        .map_err(|_| archived_unreadable(expected_merge_id, Some(header)))?;
    validate_identity(&record.merge_id, expected_merge_id, header)?;
    let record = validate_v1_record(record)
        .map_err(|_| archived_unreadable(expected_merge_id, Some(header)))?
        .into_record();
    let projection =
        v1::project(&record).map_err(|_| archived_unreadable(expected_merge_id, Some(header)))?;
    let cleanup = cleanup::from_v1(&record)
        .map_err(|error| cleanup_unreadable(expected_merge_id, header, error))?;
    Ok(ValidatedArchivedRecord {
        projection,
        cleanup,
    })
}

fn validate_identity(
    actual_merge_id: &str,
    expected_merge_id: &str,
    header: &MergeRecordHeader,
) -> ModelResult<()> {
    if actual_merge_id == expected_merge_id {
        Ok(())
    } else {
        Err(archived_unreadable(expected_merge_id, Some(header)))
    }
}

fn classify_error(expected_merge_id: &str, error: HeaderClassificationError) -> ModelError {
    match error {
        HeaderClassificationError::Malformed(reason) => {
            let context = mismatch_header(&reason);
            archived_unreadable(expected_merge_id, context.as_ref())
        }
        HeaderClassificationError::Unsupported {
            header,
            required_wave,
        } => unsupported(expected_merge_id, &header, required_wave),
    }
}

fn mismatch_header(reason: &HeaderMalformedReason) -> Option<MergeRecordHeader> {
    let HeaderMalformedReason::RecognizedSchemaVersionMismatch { schema, actual, .. } = reason
    else {
        return None;
    };
    Some(MergeRecordHeader {
        schema: schema.clone(),
        record_schema_version: *actual,
    })
}

fn archived_unreadable(merge_id: &str, header: Option<&MergeRecordHeader>) -> ModelError {
    archived_unreadable_reason(
        merge_id,
        header,
        "archive envelope or terminal state is contradictory",
    )
}

fn cleanup_unreadable(
    merge_id: &str,
    header: &MergeRecordHeader,
    error: cleanup::CleanupError,
) -> ModelError {
    let reason = match error {
        cleanup::CleanupError::ContradictoryEvidence => {
            "archive envelope or terminal state is contradictory"
        }
        cleanup::CleanupError::NonCanonicalRef => {
            "archive preservation ref is outside the canonical merge-owned namespace"
        }
        cleanup::CleanupError::DuplicateOwner => {
            "archive contains duplicate or colliding preservation owners"
        }
    };
    archived_unreadable_reason(merge_id, Some(header), reason)
}

fn archived_unreadable_reason(
    merge_id: &str,
    header: Option<&MergeRecordHeader>,
    reason: &str,
) -> ModelError {
    let error = ModelError::new(
        ErrorCode::ArchivedRecordUnreadable,
        format!("archived merge record '{merge_id}' is unreadable: {reason}"),
    );
    header.map_or(error.clone(), |header| {
        error.with_record_context(context(merge_id, header, None))
    })
}

fn unsupported(
    merge_id: &str,
    header: &MergeRecordHeader,
    required_wave: Option<MergeRecordRequiredWave>,
) -> ModelError {
    let message = required_wave.map_or_else(
        || format!(
            "merge record '{merge_id}' uses unrecognized schema '{}' version {}; use a compatible newer GWZ",
            header.schema, header.record_schema_version
        ),
        |wave| format!(
            "merge record '{merge_id}' uses schema '{}' version {}, which requires {}; use a compatible newer GWZ",
            header.schema,
            header.record_schema_version,
            wave_display(wave)
        ),
    );
    ModelError::new(ErrorCode::UnsupportedRecordVersion, message).with_record_context(context(
        merge_id,
        header,
        required_wave,
    ))
}

fn context(
    merge_id: &str,
    header: &MergeRecordHeader,
    required_wave: Option<MergeRecordRequiredWave>,
) -> MergeRecordCompatibilityContext {
    MergeRecordCompatibilityContext {
        merge_id: merge_id.to_owned(),
        schema: Some(header.schema.clone()),
        record_schema_version: Some(i64::from(header.record_schema_version)),
        required_wave,
        legacy_mode: None,
    }
}

fn wave_display(wave: MergeRecordRequiredWave) -> &'static str {
    match wave {
        MergeRecordRequiredWave::A1 => "A1 (v1 integration/acceptance/no-ff)",
        MergeRecordRequiredWave::A2 => "A2 (v2 branch lifecycle)",
        MergeRecordRequiredWave::A3 => "A3 (v3 snapshot source)",
        MergeRecordRequiredWave::A4 => "A4 (v4 partial composition)",
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn archived_fixture_for_test(
    version: super::super::model::v1::RecordVersion,
) -> (Vec<u8>, &'static str) {
    use tests::fixtures::{MERGE_ID, Shape, v0_record, v1_record};

    let bytes = match version {
        super::super::model::v1::RecordVersion::V0 => {
            serde_yaml::to_string(&v0_record(Shape::CompletedCandidate))
        }
        super::super::model::v1::RecordVersion::V1 => {
            serde_yaml::to_string(&v1_record(Shape::CompletedCandidate))
        }
    }
    .expect("archive fixture serializes")
    .into_bytes();
    (bytes, MERGE_ID)
}
