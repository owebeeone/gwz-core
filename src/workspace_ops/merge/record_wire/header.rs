use serde_yaml::Value;

use super::raw_yaml::StrictYamlDocument;
use crate::MergeRecordRequiredWave;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MergeRecordHeader {
    pub(crate) schema: String,
    pub(crate) record_schema_version: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MergeRecordDispatch {
    V0,
    V1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InstalledMergeRecordVersions {
    v0: bool,
    v1: bool,
}

impl InstalledMergeRecordVersions {
    /// The A1 installed set (compatibility contract §1). Pre-A1 this was
    /// `{ v0: true, v1: false }` and every `gwz.merge-operation/v1` envelope
    /// classified `UnsupportedRecordVersion` with `required_wave: A1` — the
    /// T-2 tripwire. A1 installs v1; v2-v4 stay allocated-but-uninstalled and
    /// keep their frozen typed projection.
    pub(crate) const PRODUCTION: Self = Self { v0: true, v1: true };

    /// The v0-only set. The v0 record store still owns only v0 bodies, so its
    /// decoder classifies a v1 envelope as `UnsupportedRecordVersion` and the
    /// dispatch routes that record to the v1 lifecycle instead.
    pub(crate) const V0_ONLY: Self = Self {
        v0: true,
        v1: false,
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HeaderMalformedReason {
    MissingSchema,
    SchemaNotString,
    MissingVersion,
    VersionNotInteger,
    VersionNegative,
    VersionOutOfRange,
    RecognizedSchemaVersionMismatch {
        schema: String,
        allocated: u32,
        actual: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HeaderClassificationError {
    Malformed(HeaderMalformedReason),
    Unsupported {
        header: MergeRecordHeader,
        required_wave: Option<MergeRecordRequiredWave>,
    },
}

#[derive(Clone, Copy)]
struct Allocation {
    schema: &'static str,
    version: u32,
    dispatch: Option<MergeRecordDispatch>,
    required_wave: Option<MergeRecordRequiredWave>,
}

const ALLOCATIONS: [Allocation; 5] = [
    Allocation {
        schema: "gwz.merge-operation/v0",
        version: 0,
        dispatch: Some(MergeRecordDispatch::V0),
        required_wave: None,
    },
    Allocation {
        schema: "gwz.merge-operation/v1",
        version: 1,
        dispatch: Some(MergeRecordDispatch::V1),
        required_wave: Some(MergeRecordRequiredWave::A1),
    },
    Allocation {
        schema: "gwz.merge-operation/v2",
        version: 2,
        dispatch: None,
        required_wave: Some(MergeRecordRequiredWave::A2),
    },
    Allocation {
        schema: "gwz.merge-operation/v3",
        version: 3,
        dispatch: None,
        required_wave: Some(MergeRecordRequiredWave::A3),
    },
    Allocation {
        schema: "gwz.merge-operation/v4",
        version: 4,
        dispatch: None,
        required_wave: Some(MergeRecordRequiredWave::A4),
    },
];

pub(crate) fn read_merge_record_header(
    document: &StrictYamlDocument,
) -> Result<MergeRecordHeader, HeaderMalformedReason> {
    let mapping = document
        .root()
        .as_mapping()
        .ok_or(HeaderMalformedReason::MissingSchema)?;
    let schema = mapping
        .get(Value::String("schema".to_owned()))
        .ok_or(HeaderMalformedReason::MissingSchema)?
        .as_str()
        .ok_or(HeaderMalformedReason::SchemaNotString)?
        .to_owned();
    let version = mapping
        .get(Value::String("record_schema_version".to_owned()))
        .ok_or(HeaderMalformedReason::MissingVersion)?;
    let Value::Number(version) = version else {
        return Err(HeaderMalformedReason::VersionNotInteger);
    };
    let record_schema_version = match version.as_u64() {
        Some(value) => {
            u32::try_from(value).map_err(|_| HeaderMalformedReason::VersionOutOfRange)?
        }
        None if version.as_i64().is_some_and(|value| value < 0) => {
            return Err(HeaderMalformedReason::VersionNegative);
        }
        None => return Err(HeaderMalformedReason::VersionNotInteger),
    };
    Ok(MergeRecordHeader {
        schema,
        record_schema_version,
    })
}

pub(crate) fn classify_merge_record_header(
    header: &MergeRecordHeader,
    installed: InstalledMergeRecordVersions,
) -> Result<MergeRecordDispatch, HeaderClassificationError> {
    let Some(allocation) = ALLOCATIONS
        .iter()
        .find(|allocation| allocation.schema == header.schema)
    else {
        return Err(unsupported(header, None));
    };
    if header.record_schema_version != allocation.version {
        return Err(HeaderClassificationError::Malformed(
            HeaderMalformedReason::RecognizedSchemaVersionMismatch {
                schema: header.schema.clone(),
                allocated: allocation.version,
                actual: header.record_schema_version,
            },
        ));
    }
    match allocation.dispatch {
        Some(MergeRecordDispatch::V0) if installed.v0 => Ok(MergeRecordDispatch::V0),
        Some(MergeRecordDispatch::V1) if installed.v1 => Ok(MergeRecordDispatch::V1),
        _ => Err(unsupported(header, allocation.required_wave)),
    }
}

fn unsupported(
    header: &MergeRecordHeader,
    required_wave: Option<MergeRecordRequiredWave>,
) -> HeaderClassificationError {
    HeaderClassificationError::Unsupported {
        header: header.clone(),
        required_wave,
    }
}
