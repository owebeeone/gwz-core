use std::io::Cursor;

use sha2::{Digest, Sha256};

use super::snapshot::{IndexSnapshotFacts, SnapshotParts};
use super::{
    RawCatalogBytesV1, RawCatalogEntryFactV1, RawCatalogInteriorFactV1, RawCatalogRoleObservationV1,
};
use crate::checked_artifact::capability::{
    CanonicalPathIdentityV1, CheckedFsError, DurableCatalogTargetDigestV1, DurablePathV1,
    FreshObservationDigestV1, HistoricalCollisionDigestV1, MissingParentObservationDigestV1,
    PreCatalogRootKindV1, PrivateControlDomain, SupportedFilesystemProfile,
};
use crate::checked_artifact::catalog::CatalogRecognizedNameV1;
use crate::checked_artifact::protocol::{
    CatalogBootstrapOwnershipTokenV1, InfrastructureSlotV1, decode_catalog_bootstrap_record,
};

use super::retained::RetainedPlatformRoot;

pub(super) struct ReadyDigestInputsV1<'a> {
    pub(super) root_kind: PreCatalogRootKindV1,
    pub(super) support_profile: SupportedFilesystemProfile,
    pub(super) domain: &'a PrivateControlDomain,
    pub(super) retained: &'a RetainedPlatformRoot,
    pub(super) path_profile: &'a CanonicalPathIdentityV1,
    pub(super) index: Option<&'a IndexSnapshotFacts>,
    pub(super) namespace: &'a RawCatalogRoleObservationV1,
    pub(super) private_parent_fact: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact::capability::pre_catalog) struct ReadyObservationDigestsV1 {
    pub(in crate::checked_artifact::capability::pre_catalog) fresh: FreshObservationDigestV1,
    pub(in crate::checked_artifact::capability::pre_catalog) target: DurableCatalogTargetDigestV1,
    pub(in crate::checked_artifact::capability::pre_catalog) historical:
        HistoricalCollisionDigestV1,
}

pub(super) fn derive_ready_digests(
    inputs: ReadyDigestInputsV1<'_>,
) -> Result<ReadyObservationDigestsV1, CheckedFsError> {
    let durable_path = DurablePathV1::from_live(inputs.path_profile)?;
    let target = DurableCatalogTargetDigestV1::owner_issue(target_digest(
        inputs.root_kind,
        inputs.support_profile,
        inputs.retained,
        &durable_path,
    ));
    let current_historical = HistoricalCollisionDigestV1::owner_issue(historical_digest(
        target,
        inputs.domain,
        inputs.index,
    ));
    let historical = select_historical(inputs.namespace, target, current_historical)?;
    let snapshot = super::snapshot::digest(SnapshotParts {
        root_kind: inputs.root_kind,
        domain: inputs.domain,
        root_identity: &inputs.retained.root().encoded_identity(),
        repository_identity: &inputs.retained.repository().encoded_identity(),
        common_directory_identity: &inputs.retained.common_directory().encoded_identity(),
        private_parent_fact: Some(inputs.private_parent_fact),
        path_profile: inputs.path_profile,
        index: inputs.index,
        namespace: &inputs.namespace.rows,
        namespace_entry_count: inputs.namespace.enumeration.entry_count(),
        namespace_encoded_name_bytes: inputs.namespace.enumeration.encoded_name_bytes(),
    });
    let mut fresh_digest = Sha256::new();
    frame(&mut fresh_digest, b"gwz-fresh-observation-v1\0");
    frame(&mut fresh_digest, &[profile_code(inputs.support_profile)]);
    frame(&mut fresh_digest, &snapshot);
    Ok(ReadyObservationDigestsV1 {
        fresh: FreshObservationDigestV1::owner_issue(fresh_digest.finalize().into()),
        target,
        historical,
    })
}

pub(in crate::checked_artifact::capability::pre_catalog) fn derive_missing_digest(
    root_kind: PreCatalogRootKindV1,
    profile: SupportedFilesystemProfile,
    retained: &RetainedPlatformRoot,
    path_profile: &CanonicalPathIdentityV1,
) -> MissingParentObservationDigestV1 {
    let alias = retained.private_parent_alias_observation();
    let mut digest = Sha256::new();
    frame(&mut digest, b"gwz-missing-catalog-parent-observation-v1\0");
    frame(&mut digest, &[root_kind_code(root_kind)]);
    frame(&mut digest, &[profile_code(profile)]);
    for directory in [
        retained.root(),
        retained.repository(),
        retained.common_directory(),
    ] {
        frame(&mut digest, &directory.encoded_snapshot_fact());
    }
    frame(&mut digest, &path_profile.fresh_digest_material());
    frame(&mut digest, b"fixed-child-absent-v1\0gwz");
    frame(&mut digest, &(alias.entry_count() as u64).to_be_bytes());
    frame(
        &mut digest,
        &(alias.encoded_name_bytes() as u64).to_be_bytes(),
    );
    MissingParentObservationDigestV1::owner_issue(digest.finalize().into())
}

fn target_digest(
    root_kind: PreCatalogRootKindV1,
    profile: SupportedFilesystemProfile,
    retained: &RetainedPlatformRoot,
    path: &DurablePathV1,
) -> [u8; 32] {
    let parent = retained
        .private_parent()
        .expect("ready digest requires retained mutation parent");
    let mut digest = Sha256::new();
    frame(&mut digest, b"gwz-durable-catalog-target-v1\0");
    frame(&mut digest, &[root_kind_code(root_kind)]);
    frame(&mut digest, &[profile_code(profile)]);
    for directory in [
        retained.root(),
        retained.repository(),
        retained.common_directory(),
        parent,
    ] {
        frame(
            &mut digest,
            &directory.identity().durable().encode_canonical(),
        );
        frame(&mut digest, &[mode_code(directory.mode())]);
    }
    frame(&mut digest, &path.encode_canonical());
    digest.finalize().into()
}

fn historical_digest(
    target: DurableCatalogTargetDigestV1,
    domain: &PrivateControlDomain,
    index: Option<&IndexSnapshotFacts>,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    frame(&mut digest, b"gwz-historical-collision-v1\0");
    frame(&mut digest, &target.bytes());
    frame(&mut digest, &domain.version_digest());
    frame(&mut digest, b"catalog-roles-initially-absent-v1\0");
    match index {
        None => frame(&mut digest, &[0]),
        Some(index) => {
            frame(&mut digest, &[1]);
            frame_optional(&mut digest, index.file_durable_identity.as_deref());
            frame_optional(
                &mut digest,
                index.content_digest.as_ref().map(<[u8; 32]>::as_slice),
            );
            frame(&mut digest, &(index.entries.len() as u64).to_be_bytes());
            for entry in &index.entries {
                frame(&mut digest, entry.path().as_bytes());
                frame(&mut digest, &[entry.stage().code()]);
                frame(&mut digest, &entry.mode().to_be_bytes());
                frame(&mut digest, &entry.raw_flags().to_be_bytes());
                frame(&mut digest, &entry.raw_extended_flags().to_be_bytes());
                frame(
                    &mut digest,
                    &entry.metadata().ctime().seconds().to_be_bytes(),
                );
                frame(
                    &mut digest,
                    &entry.metadata().ctime().nanoseconds().to_be_bytes(),
                );
                frame(
                    &mut digest,
                    &entry.metadata().mtime().seconds().to_be_bytes(),
                );
                frame(
                    &mut digest,
                    &entry.metadata().mtime().nanoseconds().to_be_bytes(),
                );
                for value in entry.metadata().stat() {
                    frame(&mut digest, &value.to_be_bytes());
                }
                frame(&mut digest, entry.metadata().object_id());
            }
            frame(&mut digest, &(index.worktree.len() as u64).to_be_bytes());
            for entry in &index.worktree {
                frame(&mut digest, entry.path().as_bytes());
                frame(&mut digest, &[entry.kind().code()]);
            }
        }
    }
    digest.finalize().into()
}

fn select_historical(
    namespace: &RawCatalogRoleObservationV1,
    target: DurableCatalogTargetDigestV1,
    current: HistoricalCollisionDigestV1,
) -> Result<HistoricalCollisionDigestV1, CheckedFsError> {
    let mut source: Option<AttemptSourceV1> = None;
    for row in &namespace.rows {
        match (&row.role, &row.fact) {
            (
                CatalogRecognizedNameV1::Scratch(name),
                RawCatalogEntryFactV1::RegularFile {
                    bytes: RawCatalogBytesV1::Bounded(_),
                    ..
                },
            ) => merge_source(
                &mut source,
                AttemptSourceV1 {
                    target: name.durable_target_digest(),
                    historical: name.historical_collision_digest(),
                    token: name.ownership_token(),
                },
            )?,
            (
                CatalogRecognizedNameV1::Active,
                RawCatalogEntryFactV1::RegularFile {
                    bytes: RawCatalogBytesV1::Bounded(bytes),
                    ..
                },
            ) => merge_source(&mut source, decode_source(bytes)?)?,
            (CatalogRecognizedNameV1::Staging, RawCatalogEntryFactV1::Directory { .. }) => {}
            (CatalogRecognizedNameV1::Final, RawCatalogEntryFactV1::Directory { interior, .. }) => {
                match interior
                    .rows
                    .iter()
                    .find(|row| row.slot == InfrastructureSlotV1::CatalogBootstrapRetired)
                {
                    None => {}
                    Some(row) => match &row.fact {
                        RawCatalogInteriorFactV1::RegularFile {
                            bytes: RawCatalogBytesV1::Bounded(bytes),
                            ..
                        } => merge_source(&mut source, decode_source(bytes)?)?,
                        _ => return Err(ambiguous_roles()),
                    },
                }
            }
            _ => return Err(ambiguous_roles()),
        }
    }
    match source {
        Some(source) if source.target == target => Ok(source.historical),
        Some(_) => Err(CheckedFsError::ambiguous(
            "catalog recovery target",
            "durable recovery evidence belongs to another catalog target",
        )),
        None if namespace.rows.is_empty() => Ok(current),
        None => Err(ambiguous_roles()),
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct AttemptSourceV1 {
    target: DurableCatalogTargetDigestV1,
    historical: HistoricalCollisionDigestV1,
    token: CatalogBootstrapOwnershipTokenV1,
}

fn decode_source(bytes: &[u8]) -> Result<AttemptSourceV1, CheckedFsError> {
    let record =
        decode_catalog_bootstrap_record(Cursor::new(bytes)).map_err(|_| ambiguous_roles())?;
    Ok(AttemptSourceV1 {
        target: record.durable_target_digest(),
        historical: record.historical_collision_digest(),
        token: record.bootstrap_ownership_token(),
    })
}

fn merge_source(
    current: &mut Option<AttemptSourceV1>,
    candidate: AttemptSourceV1,
) -> Result<(), CheckedFsError> {
    match current {
        Some(expected) if *expected != candidate => Err(ambiguous_roles()),
        Some(_) => Ok(()),
        None => {
            *current = Some(candidate);
            Ok(())
        }
    }
}

fn ambiguous_roles() -> CheckedFsError {
    CheckedFsError::ambiguous(
        "catalog recovery roles",
        "reserved role kind, bytes, or recovery attempt is not uniquely owned",
    )
}

fn root_kind_code(value: PreCatalogRootKindV1) -> u8 {
    match value {
        PreCatalogRootKindV1::Workspace => 0,
        PreCatalogRootKindV1::GitDirectory => 1,
    }
}

fn profile_code(value: SupportedFilesystemProfile) -> u8 {
    match value {
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1 => 0,
        SupportedFilesystemProfile::MacPersistentObjectIdV1 => 1,
        SupportedFilesystemProfile::WindowsNtfsFileId128V1 => 2,
    }
}

fn mode_code(value: crate::checked_artifact::capability::PathComponentMode) -> u8 {
    match value {
        crate::checked_artifact::capability::PathComponentMode::Sensitive => 0,
        crate::checked_artifact::capability::PathComponentMode::AsciiCaseFold => 1,
    }
}

fn frame(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn frame_optional(digest: &mut Sha256, value: Option<&[u8]>) {
    match value {
        Some(value) => {
            frame(digest, &[1]);
            frame(digest, value);
        }
        None => frame(digest, &[0]),
    }
}
