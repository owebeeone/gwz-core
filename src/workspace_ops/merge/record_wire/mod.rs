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
mod unknown_fields;

pub(in crate::workspace_ops::merge) use unknown_fields::{
    ContainerSegment, IdentityValue, SemanticIdentity, UnknownFieldLocator, UnknownFieldManifest,
};

pub(in crate::workspace_ops::merge) use archive::MergeOperationRecordV0;
pub(crate) use archive::{ArchivedCleanupWorklist, ValidatedArchivedRecord, decode_archived};
#[allow(
    unused_imports,
    reason = "opaque physical types are named by the v1 authority's checked consumers"
)]
pub(crate) use location::{
    CanonicalMergeLocations, CanonicalRecordKind, CanonicalRecordLeaf, CanonicalRecordPath,
    ImmutableBytes, Sha256Digest, acquire_canonical_merge_locations,
};

#[cfg(test)]
pub(crate) use archive::archived_fixture_for_test;
pub(in crate::workspace_ops::merge) use location::{
    FileIdentity, identity_at_named_path, identity_from_file, open_named_path,
};
#[cfg(test)]
pub(crate) use location::{
    appear_archived_before_final_check_for_test, appear_open_before_final_check_for_test,
    replace_open_before_final_check_for_test, replace_parent_before_final_check_for_test,
};

pub(crate) use decode::{decode_archived_common, decode_production_v1};

#[cfg(test)]
pub(crate) use checked_owner::observe_checked_archive_source_v0_leaves_for_test;
#[allow(unused_imports)]
pub(crate) use checked_owner::{
    CheckedArchiveSourceObservation, CheckedOwnerObservationError, CheckedOwnerRecordObservation,
    CheckedOwnerRecordVersion, observe_checked_archive_source_v0,
    observe_checked_owner_v0_from_canonical,
};
#[allow(unused_imports)]
pub(crate) use checked_owner::{
    MAX_CHECKED_OWNER_RECORD_BYTES, observe_checked_archive_source_v1, observe_checked_owner_v0,
    observe_checked_owner_v1, observe_checked_owner_v1_from_canonical,
};
pub(super) use decode::RecordDecodeError;
use decode::decode_v0_parts;
pub(crate) use header::InstalledMergeRecordVersions;
pub(super) use header::read_merge_record_header;
pub(super) use header::{
    HeaderClassificationError, HeaderMalformedReason, MergeRecordDispatch, MergeRecordHeader,
    classify_merge_record_header,
};
pub(super) use raw_yaml::StrictYamlError;
pub(super) use raw_yaml::parse_strict_yaml;

#[cfg(test)]
mod tests;
