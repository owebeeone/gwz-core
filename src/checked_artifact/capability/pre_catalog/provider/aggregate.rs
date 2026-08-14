use std::io::Cursor;

use super::{RawCatalogBytesV1, RawCatalogEntryFactV1, RawCatalogRoleObservationV1, interior};
use crate::checked_artifact::catalog::{
    CatalogAggregateFactsV1, CatalogAttemptBindingV1, CatalogDirectoryFactV1,
    CatalogRecognizedNameV1, CatalogRecordFactV1,
};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{
    CatalogBootstrapRecordV1, decode_catalog_bootstrap_record,
};

enum RawDirectoryV1<'a> {
    Missing,
    Directory(
        &'a crate::checked_artifact::capability::DurableObjectIdentityV1,
        &'a super::RawCatalogInteriorObservationV1,
    ),
    Other,
}

pub(in crate::checked_artifact::capability::pre_catalog) fn outer_aggregate_facts(
    binding: &CatalogAttemptBindingV1,
    observed: &RawCatalogRoleObservationV1,
) -> CatalogAggregateFactsV1 {
    let mut scratch = Vec::new();
    let mut active = CatalogRecordFactV1::Missing;
    let mut staging_raw = RawDirectoryV1::Missing;
    let mut final_raw = RawDirectoryV1::Missing;
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
            (
                CatalogRecognizedNameV1::Staging,
                RawCatalogEntryFactV1::Directory {
                    durable_identity,
                    interior,
                    ..
                },
            ) => staging_raw = RawDirectoryV1::Directory(durable_identity, interior),
            (CatalogRecognizedNameV1::Staging, _) => staging_raw = RawDirectoryV1::Other,
            (
                CatalogRecognizedNameV1::Final,
                RawCatalogEntryFactV1::Directory {
                    durable_identity,
                    interior,
                    ..
                },
            ) => {
                retired = interior::retired_record(interior);
                final_raw = RawDirectoryV1::Directory(durable_identity, interior);
            }
            (CatalogRecognizedNameV1::Final, _) => {
                retired = CatalogRecordFactV1::Other;
                final_raw = RawDirectoryV1::Other;
            }
        }
    }

    let expected = merge_expected(binding, &scratch, &active, &retired);
    let staging = directory_fact(
        CatalogPrivateNameV1::BootstrapStaging,
        staging_raw,
        expected.as_ref(),
    );
    let final_directory = directory_fact(CatalogPrivateNameV1::Final, final_raw, expected.as_ref());
    CatalogAggregateFactsV1::new(scratch, active, staging, final_directory, retired)
}

fn directory_fact(
    role: CatalogPrivateNameV1,
    observed: RawDirectoryV1<'_>,
    expected: Option<&CatalogBootstrapRecordV1>,
) -> CatalogDirectoryFactV1 {
    match observed {
        RawDirectoryV1::Missing => CatalogDirectoryFactV1::Missing,
        RawDirectoryV1::Directory(identity, interior) => {
            interior::directory_fact(role, identity, interior, expected)
        }
        RawDirectoryV1::Other => CatalogDirectoryFactV1::Other,
    }
}

fn merge_expected(
    binding: &CatalogAttemptBindingV1,
    scratch: &[CatalogRecordFactV1],
    active: &CatalogRecordFactV1,
    retired: &CatalogRecordFactV1,
) -> Option<CatalogBootstrapRecordV1> {
    let mut expected = None;
    for fact in scratch.iter().chain([active, retired]) {
        let candidate = match fact {
            CatalogRecordFactV1::Scratch { name, .. } => binding.record_from_scratch(name).ok()?,
            CatalogRecordFactV1::Exact(value) if binding.accepts(value) => value.as_ref().clone(),
            CatalogRecordFactV1::Missing => continue,
            CatalogRecordFactV1::Exact(_) | CatalogRecordFactV1::Other => return None,
        };
        match &expected {
            Some(current) if current != &candidate => return None,
            Some(_) => {}
            None => expected = Some(candidate),
        }
    }
    expected
}

fn decode_record(bytes: &[u8]) -> CatalogRecordFactV1 {
    decode_catalog_bootstrap_record(Cursor::new(bytes))
        .map(CatalogRecordFactV1::exact)
        .unwrap_or(CatalogRecordFactV1::Other)
}
