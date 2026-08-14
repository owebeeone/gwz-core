use std::io::Cursor;

use super::{
    RawCatalogBytesV1, RawCatalogEntryFactV1, RawCatalogRetiredFactV1, RawCatalogRoleObservationV1,
};
use crate::checked_artifact::catalog::{
    CatalogAggregateFactsV1, CatalogDirectoryFactV1, CatalogRecognizedNameV1, CatalogRecordFactV1,
};
use crate::checked_artifact::protocol::decode_catalog_bootstrap_record;

pub(in crate::checked_artifact::capability::pre_catalog) fn outer_aggregate_facts(
    observed: &RawCatalogRoleObservationV1,
) -> CatalogAggregateFactsV1 {
    let mut scratch = Vec::new();
    let mut active = CatalogRecordFactV1::Missing;
    let mut staging = CatalogDirectoryFactV1::Missing;
    let mut final_directory = CatalogDirectoryFactV1::Missing;
    let mut retired = CatalogRecordFactV1::Missing;

    for row in &observed.rows {
        match (&row.role, &row.fact) {
            (
                CatalogRecognizedNameV1::Scratch(name),
                RawCatalogEntryFactV1::RegularFile {
                    bytes: RawCatalogBytesV1::Bounded(bytes),
                    ..
                },
            ) => scratch.push(CatalogRecordFactV1::scratch(
                name.as_ref().clone(),
                bytes.clone(),
            )),
            (CatalogRecognizedNameV1::Scratch(_), _) => scratch.push(CatalogRecordFactV1::Other),
            (
                CatalogRecognizedNameV1::Active,
                RawCatalogEntryFactV1::RegularFile {
                    bytes: RawCatalogBytesV1::Bounded(bytes),
                    ..
                },
            ) => active = decode_record(bytes),
            (CatalogRecognizedNameV1::Active, _) => active = CatalogRecordFactV1::Other,
            (CatalogRecognizedNameV1::Staging, RawCatalogEntryFactV1::Directory { .. }) => {
                staging = CatalogDirectoryFactV1::Other;
            }
            (CatalogRecognizedNameV1::Staging, _) => staging = CatalogDirectoryFactV1::Other,
            (
                CatalogRecognizedNameV1::Final,
                RawCatalogEntryFactV1::Directory {
                    retired: retired_fact,
                    ..
                },
            ) => {
                final_directory = CatalogDirectoryFactV1::Other;
                retired = match retired_fact {
                    RawCatalogRetiredFactV1::Missing => CatalogRecordFactV1::Missing,
                    RawCatalogRetiredFactV1::RegularFile {
                        bytes: RawCatalogBytesV1::Bounded(bytes),
                        ..
                    } => decode_record(bytes),
                    RawCatalogRetiredFactV1::RegularFile {
                        bytes: RawCatalogBytesV1::Oversize,
                        ..
                    }
                    | RawCatalogRetiredFactV1::Other(_) => CatalogRecordFactV1::Other,
                };
            }
            (CatalogRecognizedNameV1::Final, _) => {
                final_directory = CatalogDirectoryFactV1::Other;
                retired = CatalogRecordFactV1::Other;
            }
        }
    }

    CatalogAggregateFactsV1::new(scratch, active, staging, final_directory, retired)
}

fn decode_record(bytes: &[u8]) -> CatalogRecordFactV1 {
    decode_catalog_bootstrap_record(Cursor::new(bytes))
        .map(CatalogRecordFactV1::exact)
        .unwrap_or(CatalogRecordFactV1::Other)
}
