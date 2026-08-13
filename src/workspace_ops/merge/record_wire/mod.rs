#[allow(
    dead_code,
    reason = "R2 freezes the durable-record owner issuer before consumer conversion"
)]
mod checked_owner;
mod decode;
mod header;
mod raw_yaml;
mod scalar;

mod archive;
mod location;
#[cfg(test)]
mod open_v0;
#[cfg(test)]
mod unknown_fields;

#[cfg(test)]
pub(in crate::workspace_ops::merge) use unknown_fields::{
    ContainerSegment, IdentityValue, SemanticIdentity, UnknownFieldLocator, UnknownFieldManifest,
};

#[cfg(test)]
pub(crate) use open_v0::{
    OpenV0Adaptation, PreparedOpenV0Upgrade, PreparedV1Upgrade, VerifiedV0Descriptor,
    adapt_open_v0_for_r3_tests, prepare_upgrade, verified_v0_descriptor,
};

#[allow(
    unused_imports,
    reason = "P4 consumes cleanup only through the test-gated archive/GC lifecycle"
)]
pub(crate) use archive::{ArchivedCleanupWorklist, ValidatedArchivedRecord, decode_archived_v0};
#[allow(
    unused_imports,
    reason = "opaque physical types are named by test-gated v1 authority consumers"
)]
pub(crate) use location::{
    CanonicalMergeLocations, CanonicalRecordKind, CanonicalRecordLeaf, CanonicalRecordPath,
    ImmutableBytes, Sha256Digest, acquire_canonical_merge_locations,
};

#[cfg(test)]
pub(crate) use archive::decode_archived_for_r3_tests as decode_archived;
#[cfg(test)]
pub(crate) use archive::{archived_fixture_for_test, decode_archived_for_r3_tests};
#[cfg(test)]
pub(crate) use location::{
    appear_archived_before_final_check_for_test, appear_open_before_final_check_for_test,
    replace_open_before_final_check_for_test, replace_parent_before_final_check_for_test,
};

#[cfg(test)]
pub(crate) fn decode_v0_for_r3_tests(
    bytes: &[u8],
) -> Result<decode::DecodedV0Record, decode::RecordDecodeError> {
    decode::decode_production_v0(bytes)
}

#[cfg(test)]
pub(crate) use decode::decode_v1_for_r3_tests;

#[allow(unused_imports)]
pub(crate) use checked_owner::{
    CheckedArchiveSourceObservation, CheckedOwnerObservationError, CheckedOwnerRecordObservation,
    CheckedOwnerRecordVersion, observe_checked_archive_source_v0,
    observe_checked_owner_v0_from_canonical,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use checked_owner::{
    MAX_CHECKED_OWNER_RECORD_BYTES, observe_checked_archive_source_v0_leaves_for_test,
    observe_checked_archive_source_v1, observe_checked_owner_v0, observe_checked_owner_v1,
    observe_checked_owner_v1_from_canonical,
};
pub(super) use decode::{RecordDecodeError, decode_production_v0};
pub(super) use header::{HeaderClassificationError, HeaderMalformedReason, MergeRecordHeader};
pub(super) use raw_yaml::StrictYamlError;

#[cfg(test)]
mod tests;
