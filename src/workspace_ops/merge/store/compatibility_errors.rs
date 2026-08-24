use std::path::Path;

use super::{RecordLocation, unreadable};
use crate::model::{ErrorCode, ModelError};
use crate::workspace_ops::merge::record_wire::{
    HeaderClassificationError, HeaderMalformedReason, MergeRecordHeader, RecordDecodeError,
    StrictYamlError,
};

pub(super) fn decode_error(
    path: &Path,
    merge_id: &str,
    location: RecordLocation,
    error: RecordDecodeError,
) -> ModelError {
    match error {
        RecordDecodeError::Raw(error) => {
            location_unreadable(path, merge_id, location, strict_yaml_reason(&error))
        }
        RecordDecodeError::Header(HeaderClassificationError::Malformed(reason)) => {
            let context = mismatch_context(merge_id, &reason);
            let error = location_unreadable(path, merge_id, location, malformed_reason(&reason));
            context.map_or(error.clone(), |context| error.with_record_context(context))
        }
        RecordDecodeError::Header(HeaderClassificationError::Unsupported {
            header,
            required_wave,
        }) => unsupported_record(merge_id, header, required_wave),
        RecordDecodeError::Body { header, detail } => location_unreadable(
            path,
            merge_id,
            location,
            format!("invalid record: {detail}"),
        )
        .with_record_context(record_context(merge_id, &header, None)),
        RecordDecodeError::Validation { header, error } => match location {
            RecordLocation::Open => {
                error.with_record_context(record_context(merge_id, &header, None))
            }
            RecordLocation::Archived => archived_contradiction(merge_id, &header),
        },
        RecordDecodeError::UnknownFields { header, error } => match location {
            RecordLocation::Open => location_unreadable(
                path,
                merge_id,
                location,
                format!("invalid unknown-field manifest: {}", error.detail),
            )
            .with_record_context(record_context(merge_id, &header, None)),
            RecordLocation::Archived => archived_contradiction(merge_id, &header),
        },
    }
}

pub(super) fn location_unreadable(
    path: &Path,
    merge_id: &str,
    location: RecordLocation,
    reason: impl std::fmt::Display,
) -> ModelError {
    match location {
        RecordLocation::Open => unreadable(Some(path), reason),
        RecordLocation::Archived => archived_unreadable(merge_id),
    }
}

pub(super) fn archived_contradiction(merge_id: &str, header: &MergeRecordHeader) -> ModelError {
    archived_unreadable(merge_id).with_record_context(record_context(merge_id, header, None))
}

pub(super) fn record_context(
    merge_id: &str,
    header: &MergeRecordHeader,
    required_wave: Option<crate::MergeRecordRequiredWave>,
) -> crate::MergeRecordCompatibilityContext {
    crate::MergeRecordCompatibilityContext {
        merge_id: merge_id.to_owned(),
        schema: Some(header.schema.clone()),
        record_schema_version: Some(i64::from(header.record_schema_version)),
        required_wave,
        legacy_mode: None,
    }
}

fn strict_yaml_reason(error: &StrictYamlError) -> String {
    match (error.line, error.column) {
        (Some(line), Some(column)) => format!(
            "invalid YAML: {} at line {line}, column {column}",
            error.detail
        ),
        (Some(line), None) => format!("invalid YAML: {} at line {line}", error.detail),
        _ => format!("invalid YAML: {}", error.detail),
    }
}

fn archived_unreadable(merge_id: &str) -> ModelError {
    ModelError::new(
        ErrorCode::ArchivedRecordUnreadable,
        format!(
            "archived merge record '{merge_id}' is unreadable: archive envelope or terminal state is contradictory"
        ),
    )
}

fn unsupported_record(
    merge_id: &str,
    header: MergeRecordHeader,
    required_wave: Option<crate::MergeRecordRequiredWave>,
) -> ModelError {
    let message = required_wave.map_or_else(
        || {
            format!(
                "merge record '{merge_id}' uses unrecognized schema '{}' version {}; use a compatible newer GWZ",
                header.schema, header.record_schema_version
            )
        },
        |wave| {
            format!(
                "merge record '{merge_id}' uses schema '{}' version {}, which requires {}; use a compatible newer GWZ",
                header.schema,
                header.record_schema_version,
                required_wave_display(wave)
            )
        },
    );
    ModelError::new(ErrorCode::UnsupportedRecordVersion, message)
        .with_record_context(record_context(merge_id, &header, required_wave))
}

fn mismatch_context(
    merge_id: &str,
    reason: &HeaderMalformedReason,
) -> Option<crate::MergeRecordCompatibilityContext> {
    let HeaderMalformedReason::RecognizedSchemaVersionMismatch { schema, actual, .. } = reason
    else {
        return None;
    };
    Some(record_context(
        merge_id,
        &MergeRecordHeader {
            schema: schema.clone(),
            record_schema_version: *actual,
        },
        None,
    ))
}

fn malformed_reason(reason: &HeaderMalformedReason) -> String {
    match reason {
        HeaderMalformedReason::MissingSchema => "missing schema".to_owned(),
        HeaderMalformedReason::SchemaNotString => "schema is not a string".to_owned(),
        HeaderMalformedReason::MissingVersion => "missing record_schema_version".to_owned(),
        HeaderMalformedReason::VersionNotInteger => {
            "record_schema_version is not an integer".to_owned()
        }
        HeaderMalformedReason::VersionNegative => "record_schema_version is negative".to_owned(),
        HeaderMalformedReason::VersionOutOfRange => "record_schema_version exceeds u32".to_owned(),
        HeaderMalformedReason::RecognizedSchemaVersionMismatch {
            schema,
            allocated,
            actual,
        } => format!("recognized schema '{schema}' is allocated version {allocated}, not {actual}"),
    }
}

fn required_wave_display(wave: crate::MergeRecordRequiredWave) -> &'static str {
    match wave {
        crate::MergeRecordRequiredWave::A1 => "A1 (v1 integration/acceptance/no-ff)",
        crate::MergeRecordRequiredWave::A2 => "A2 (v2 branch lifecycle)",
        crate::MergeRecordRequiredWave::A3 => "A3 (v3 snapshot source)",
        crate::MergeRecordRequiredWave::A4 => "A4 (v4 partial composition)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validation_error() -> RecordDecodeError {
        RecordDecodeError::Validation {
            header: MergeRecordHeader {
                schema: "gwz.merge-operation/v1".to_owned(),
                record_schema_version: 1,
            },
            error: ModelError::new(
                ErrorCode::UnexpectedAcceptanceEvidence,
                "accepted evidence is contradictory",
            ),
        }
    }

    #[test]
    fn v1_validation_error_projection_depends_on_record_location() {
        let path = Path::new(".gwz/merge/merge_1.yaml");
        let open = decode_error(path, "merge_1", RecordLocation::Open, validation_error());
        let archived = decode_error(
            path,
            "merge_1",
            RecordLocation::Archived,
            validation_error(),
        );

        assert_eq!(open.code, ErrorCode::UnexpectedAcceptanceEvidence);
        assert_eq!(archived.code, ErrorCode::ArchivedRecordUnreadable);
        for error in [&open, &archived] {
            let context = error.record_context.as_ref().unwrap();
            assert_eq!(context.merge_id, "merge_1");
            assert_eq!(context.schema.as_deref(), Some("gwz.merge-operation/v1"));
            assert_eq!(context.record_schema_version, Some(1));
        }
    }
}
