//! Bounded observation and exact-prefix classification for first-catalog interiors.

use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;

use super::filesystem::PlatformProviderV1;
use super::retained::encode_identity;
use super::{
    RawCatalogBytesV1, RawCatalogInteriorFactV1, RawCatalogInteriorObservationV1,
    RawCatalogInteriorRowV1,
};
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::catalog::{
    CatalogDirectoryFactV1, CatalogNameBudgetV1, native_name_matches_ascii,
};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{
    CatalogBootstrapRecordV1, InfrastructureRecordV1, InfrastructureSlotV1, ProtocolRecordKindV1,
    ScratchBytesV1, classify_expected_prefix, decode_catalog_bootstrap_record,
};

const ROAMING_ANCHOR_BYTES: &[u8] = b"GWZ-ROAMING-ANCHOR-V1\n";
const CATALOG_ANCHOR_BYTES: &[u8] = b"GWZ-CATALOG-ANCHOR-V1\n";
const MAX_INTERIOR_ENTRIES: usize = 10;

pub(super) enum StagingPlanV1 {
    CreateRetiredActions,
    WriteRoamingAnchor {
        create_new: bool,
    },
    WriteCatalogAnchorB {
        create_new: bool,
    },
    ExerciseAnchorAndWriteDescriptor(InfrastructureRecordV1),
    WriteDescriptor {
        record: InfrastructureRecordV1,
        create_new: bool,
    },
    WriteFormat {
        record: InfrastructureRecordV1,
        create_new: bool,
    },
    Complete(InfrastructureRecordV1),
    Other,
}

pub(super) fn observe(
    directory: &cap_std::fs::Dir,
    platform: &impl PlatformProviderV1,
) -> Result<RawCatalogInteriorObservationV1, CheckedFsError> {
    let mode = platform.parent_mode(directory)?;
    let mut budget = CatalogNameBudgetV1::new();
    let mut rows = Vec::new();
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate catalog interior", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read catalog interior", source))?;
        let name = entry.file_name();
        budget.charge_os_str(&name)?;
        if rows.len() == MAX_INTERIOR_ENTRIES {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "catalog interior exceeds the ten-slot bound",
            ));
        }
        let slot = exact_slot(&name, mode)?;
        let fact = observe_slot(directory, &name, slot, platform)?;
        rows.try_reserve_exact(1).map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "catalog interior row allocation failed",
            )
        })?;
        rows.push(RawCatalogInteriorRowV1 { slot, fact });
    }
    rows.sort_unstable_by_key(|row| slot_index(row.slot));
    if rows.windows(2).any(|pair| pair[0].slot == pair[1].slot) {
        return Err(CheckedFsError::ambiguous(
            "catalog interior",
            "multiple native entries resolve to one infrastructure slot",
        ));
    }
    Ok(RawCatalogInteriorObservationV1 {
        entry_count: budget.entry_count(),
        encoded_name_bytes: budget.encoded_name_bytes(),
        rows,
    })
}

pub(super) fn directory_fact(
    role: CatalogPrivateNameV1,
    directory_identity: &crate::checked_artifact::capability::DurableObjectIdentityV1,
    interior: &RawCatalogInteriorObservationV1,
    expected: Option<&CatalogBootstrapRecordV1>,
) -> CatalogDirectoryFactV1 {
    let Some(expected) = expected else {
        return CatalogDirectoryFactV1::Other;
    };
    match role {
        CatalogPrivateNameV1::BootstrapStaging => {
            match staging_plan(directory_identity, interior, expected) {
                StagingPlanV1::Complete(_) => CatalogDirectoryFactV1::ExactOwned,
                StagingPlanV1::Other => CatalogDirectoryFactV1::Other,
                _ => CatalogDirectoryFactV1::ActiveOwnedPrefix,
            }
        }
        CatalogPrivateNameV1::Final => {
            if completed_record(directory_identity, interior, expected).is_some() {
                CatalogDirectoryFactV1::ExactOwned
            } else {
                CatalogDirectoryFactV1::Other
            }
        }
        _ => CatalogDirectoryFactV1::Other,
    }
}

pub(super) fn staging_plan(
    directory_identity: &crate::checked_artifact::capability::DurableObjectIdentityV1,
    interior: &RawCatalogInteriorObservationV1,
    expected: &CatalogBootstrapRecordV1,
) -> StagingPlanV1 {
    use InfrastructureSlotV1 as Slot;
    if any_present(
        interior,
        &[
            Slot::CatalogBootstrapRetired,
            Slot::ActionAdmissionActive,
            Slot::ActionAdmissionScratch,
            Slot::ActionAdmissionStaging,
        ],
    ) {
        return StagingPlanV1::Other;
    }
    let retired = match row(interior, Slot::RetiredActions) {
        None => {
            return if only_missing(
                interior,
                &[
                    Slot::RoamingAnchorHome,
                    Slot::CatalogAnchorA,
                    Slot::CatalogAnchorB,
                    Slot::RetiredActionsDescriptor,
                    Slot::CatalogFormat,
                ],
            ) {
                StagingPlanV1::CreateRetiredActions
            } else {
                StagingPlanV1::Other
            };
        }
        Some(RawCatalogInteriorFactV1::EmptyDirectory {
            durable_identity, ..
        }) => durable_identity,
        _ => return StagingPlanV1::Other,
    };
    let roaming = match file_prefix(interior, Slot::RoamingAnchorHome, ROAMING_ANCHOR_BYTES) {
        FilePrefixV1::Missing => {
            return if later_after_roaming_missing(interior) {
                StagingPlanV1::WriteRoamingAnchor { create_new: true }
            } else {
                StagingPlanV1::Other
            };
        }
        FilePrefixV1::Partial => {
            return if later_after_roaming_missing(interior) {
                StagingPlanV1::WriteRoamingAnchor { create_new: false }
            } else {
                StagingPlanV1::Other
            };
        }
        FilePrefixV1::Exact(identity) => identity,
        FilePrefixV1::Other => return StagingPlanV1::Other,
    };

    let (anchor, needs_exercise) = match (
        file_prefix(interior, Slot::CatalogAnchorA, CATALOG_ANCHOR_BYTES),
        file_prefix(interior, Slot::CatalogAnchorB, CATALOG_ANCHOR_BYTES),
    ) {
        (FilePrefixV1::Missing, FilePrefixV1::Missing)
            if descriptor_and_format_missing(interior) =>
        {
            return StagingPlanV1::WriteCatalogAnchorB { create_new: true };
        }
        (FilePrefixV1::Missing, FilePrefixV1::Partial)
            if descriptor_and_format_missing(interior) =>
        {
            return StagingPlanV1::WriteCatalogAnchorB { create_new: false };
        }
        (FilePrefixV1::Missing, FilePrefixV1::Exact(identity))
            if descriptor_and_format_missing(interior) =>
        {
            (identity, true)
        }
        (FilePrefixV1::Exact(identity), FilePrefixV1::Missing) => (identity, true),
        _ => return StagingPlanV1::Other,
    };
    let infrastructure = match InfrastructureRecordV1::owner_issue_for_catalog(
        expected,
        directory_identity.clone(),
        anchor.clone(),
        roaming.clone(),
        retired.clone(),
    ) {
        Ok(value) => value,
        Err(_) => return StagingPlanV1::Other,
    };
    let descriptor = file_prefix(
        interior,
        Slot::RetiredActionsDescriptor,
        &infrastructure.encode_canonical(),
    );
    if needs_exercise
        && matches!(descriptor, FilePrefixV1::Missing)
        && row(interior, Slot::CatalogFormat).is_none()
    {
        return StagingPlanV1::ExerciseAnchorAndWriteDescriptor(infrastructure);
    }
    match descriptor {
        FilePrefixV1::Missing if row(interior, Slot::CatalogFormat).is_none() => {
            StagingPlanV1::WriteDescriptor {
                record: infrastructure,
                create_new: true,
            }
        }
        FilePrefixV1::Partial if row(interior, Slot::CatalogFormat).is_none() => {
            StagingPlanV1::WriteDescriptor {
                record: infrastructure,
                create_new: false,
            }
        }
        FilePrefixV1::Exact(_) => match file_prefix(
            interior,
            Slot::CatalogFormat,
            &infrastructure.encode_canonical(),
        ) {
            FilePrefixV1::Missing => StagingPlanV1::WriteFormat {
                record: infrastructure,
                create_new: true,
            },
            FilePrefixV1::Partial => StagingPlanV1::WriteFormat {
                record: infrastructure,
                create_new: false,
            },
            FilePrefixV1::Exact(_) => StagingPlanV1::Complete(infrastructure),
            FilePrefixV1::Other => StagingPlanV1::Other,
        },
        _ => StagingPlanV1::Other,
    }
}

pub(super) fn completed_record(
    directory_identity: &crate::checked_artifact::capability::DurableObjectIdentityV1,
    interior: &RawCatalogInteriorObservationV1,
    expected: &CatalogBootstrapRecordV1,
) -> Option<InfrastructureRecordV1> {
    use InfrastructureSlotV1 as Slot;
    if any_present(
        interior,
        &[
            Slot::CatalogAnchorB,
            Slot::ActionAdmissionActive,
            Slot::ActionAdmissionScratch,
            Slot::ActionAdmissionStaging,
        ],
    ) {
        return None;
    }
    let retired = empty_directory_identity(interior, Slot::RetiredActions)?;
    let roaming = exact_file_identity(interior, Slot::RoamingAnchorHome, ROAMING_ANCHOR_BYTES)?;
    let anchor = exact_file_identity(interior, Slot::CatalogAnchorA, CATALOG_ANCHOR_BYTES)?;
    let infrastructure = InfrastructureRecordV1::owner_issue_for_catalog(
        expected,
        directory_identity.clone(),
        anchor,
        roaming,
        retired,
    )
    .ok()?;
    let bytes = infrastructure.encode_canonical();
    exact_file_identity(interior, Slot::RetiredActionsDescriptor, &bytes)?;
    exact_file_identity(interior, Slot::CatalogFormat, &bytes)?;
    Some(infrastructure)
}

pub(super) fn retired_record(
    interior: &RawCatalogInteriorObservationV1,
) -> crate::checked_artifact::catalog::CatalogRecordFactV1 {
    use crate::checked_artifact::catalog::CatalogRecordFactV1;
    let Some(fact) = row(interior, InfrastructureSlotV1::CatalogBootstrapRetired) else {
        return CatalogRecordFactV1::Missing;
    };
    let RawCatalogInteriorFactV1::RegularFile {
        bytes: RawCatalogBytesV1::Bounded(bytes),
        ..
    } = fact
    else {
        return CatalogRecordFactV1::Other;
    };
    decode_catalog_bootstrap_record(std::io::Cursor::new(bytes))
        .map(CatalogRecordFactV1::exact)
        .unwrap_or(CatalogRecordFactV1::Other)
}

fn exact_slot(
    name: &OsStr,
    mode: crate::checked_artifact::capability::PathComponentMode,
) -> Result<InfrastructureSlotV1, CheckedFsError> {
    for slot in InfrastructureSlotV1::ALL.iter().copied() {
        if native_name_matches_ascii(name, slot.name().as_bytes(), mode)? {
            return if name == OsStr::new(slot.name()) {
                Ok(slot)
            } else {
                Err(CheckedFsError::ambiguous(
                    "catalog interior",
                    "platform-equivalent infrastructure alias is noncanonical",
                ))
            };
        }
    }
    Err(CheckedFsError::ambiguous(
        "catalog interior",
        "catalog directory contains an unowned child",
    ))
}

fn observe_slot(
    directory: &cap_std::fs::Dir,
    name: &OsStr,
    slot: InfrastructureSlotV1,
    platform: &impl PlatformProviderV1,
) -> Result<RawCatalogInteriorFactV1, CheckedFsError> {
    let metadata = directory
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe catalog interior slot", source))?;
    if metadata.is_dir() && !metadata.is_symlink() {
        let child = directory
            .open_dir_nofollow(name)
            .map_err(|source| CheckedFsError::io("open catalog interior directory", source))?;
        let identity = platform.dir_identity(&child)?;
        if slot == InfrastructureSlotV1::RetiredActions {
            let mut entries = child
                .entries()
                .map_err(|source| CheckedFsError::io("enumerate retired-action root", source))?;
            if entries.next().is_none() {
                return Ok(RawCatalogInteriorFactV1::EmptyDirectory {
                    identity: encode_identity(&identity),
                    durable_identity: identity.durable().clone(),
                });
            }
        }
    } else if metadata.is_file() && !metadata.is_symlink() {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = directory
            .open_with(name, &options)
            .map_err(|source| CheckedFsError::io("open catalog interior file", source))?;
        let identity = platform.file_identity(&file)?;
        return Ok(RawCatalogInteriorFactV1::RegularFile {
            identity: encode_identity(&identity),
            durable_identity: identity.durable().clone(),
            bytes: read_bounded(&mut file)?,
        });
    }
    let mut value = Vec::new();
    value.push(if metadata.is_symlink() { 3 } else { 4 });
    value.extend_from_slice(&metadata.dev().to_be_bytes());
    value.extend_from_slice(&metadata.ino().to_be_bytes());
    Ok(RawCatalogInteriorFactV1::Other(value))
}

fn read_bounded(file: &mut cap_std::fs::File) -> Result<RawCatalogBytesV1, CheckedFsError> {
    let limit = ProtocolRecordKindV1::Infrastructure
        .max_bytes()
        .max(ProtocolRecordKindV1::CatalogBootstrap.max_bytes());
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog interior read allocation failed",
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind catalog interior file", source))?;
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read catalog interior file", source))?;
    Ok(if bytes.len() > limit {
        RawCatalogBytesV1::Oversize
    } else {
        RawCatalogBytesV1::Bounded(bytes)
    })
}

enum FilePrefixV1<'a> {
    Missing,
    Partial,
    Exact(&'a crate::checked_artifact::capability::DurableObjectIdentityV1),
    Other,
}

fn file_prefix<'a>(
    interior: &'a RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    expected: &[u8],
) -> FilePrefixV1<'a> {
    let Some(fact) = row(interior, slot) else {
        return FilePrefixV1::Missing;
    };
    let RawCatalogInteriorFactV1::RegularFile {
        durable_identity,
        bytes: RawCatalogBytesV1::Bounded(bytes),
        ..
    } = fact
    else {
        return FilePrefixV1::Other;
    };
    match classify_expected_prefix(bytes, expected) {
        ScratchBytesV1::PartialExpectedPrefix => FilePrefixV1::Partial,
        ScratchBytesV1::Exact => FilePrefixV1::Exact(durable_identity),
        ScratchBytesV1::Missing | ScratchBytesV1::Other => FilePrefixV1::Other,
    }
}

pub(super) fn row(
    interior: &RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
) -> Option<&RawCatalogInteriorFactV1> {
    interior
        .rows
        .iter()
        .find(|candidate| candidate.slot == slot)
        .map(|candidate| &candidate.fact)
}

fn exact_file_identity(
    interior: &RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    expected: &[u8],
) -> Option<crate::checked_artifact::capability::DurableObjectIdentityV1> {
    match file_prefix(interior, slot, expected) {
        FilePrefixV1::Exact(identity) => Some(identity.clone()),
        _ => None,
    }
}

fn empty_directory_identity(
    interior: &RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
) -> Option<crate::checked_artifact::capability::DurableObjectIdentityV1> {
    match row(interior, slot)? {
        RawCatalogInteriorFactV1::EmptyDirectory {
            durable_identity, ..
        } => Some(durable_identity.clone()),
        _ => None,
    }
}

fn any_present(interior: &RawCatalogInteriorObservationV1, slots: &[InfrastructureSlotV1]) -> bool {
    slots.iter().any(|slot| row(interior, *slot).is_some())
}

fn only_missing(
    interior: &RawCatalogInteriorObservationV1,
    slots: &[InfrastructureSlotV1],
) -> bool {
    !any_present(interior, slots)
}

fn later_after_roaming_missing(interior: &RawCatalogInteriorObservationV1) -> bool {
    only_missing(
        interior,
        &[
            InfrastructureSlotV1::CatalogAnchorA,
            InfrastructureSlotV1::CatalogAnchorB,
            InfrastructureSlotV1::RetiredActionsDescriptor,
            InfrastructureSlotV1::CatalogFormat,
        ],
    )
}

fn descriptor_and_format_missing(interior: &RawCatalogInteriorObservationV1) -> bool {
    only_missing(
        interior,
        &[
            InfrastructureSlotV1::RetiredActionsDescriptor,
            InfrastructureSlotV1::CatalogFormat,
        ],
    )
}

fn slot_index(slot: InfrastructureSlotV1) -> usize {
    InfrastructureSlotV1::ALL
        .iter()
        .position(|candidate| *candidate == slot)
        .expect("slot belongs to the closed infrastructure grammar")
}

pub(super) const fn roaming_anchor_bytes() -> &'static [u8] {
    ROAMING_ANCHOR_BYTES
}

pub(super) const fn catalog_anchor_bytes() -> &'static [u8] {
    CATALOG_ANCHOR_BYTES
}
