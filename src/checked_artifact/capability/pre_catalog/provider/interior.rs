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
    ActionCapacityReservationV1, ActionSlotV1, BaseActionSlotV1, CatalogBootstrapRecordV1,
    CatalogRootRowCensusV1, CatalogRootRowClassV1, InfrastructureRecordV1, InfrastructureSlotV1,
    MAX_ACTION_SLOTS, MAX_ACTIVE_ACTION_DIRS, MAX_INFRASTRUCTURE_ENTRIES, MAX_ROOT_ENTRIES,
    ObservedActionDirectoryV1, ProtocolRecordKindV1, RecordObservationV1, ScratchBytesV1,
    classify_expected_prefix, decode_catalog_bootstrap_record,
};

const ROAMING_ANCHOR_BYTES: &[u8] = b"GWZ-ROAMING-ANCHOR-V1\n";
const CATALOG_ANCHOR_BYTES: &[u8] = b"GWZ-CATALOG-ANCHOR-V1\n";
/// The catalog root's bound, widened from the ten infrastructure slots to the
/// already-frozen `MAX_ROOT_ENTRIES` (= 74 = 10 infrastructure + 64 active
/// action directories, `protocol/bounds.rs:21-23`) so a published
/// `RootEntryNameV1::ActiveAction` row survives reobservation.
///
/// `GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 2 (C-3) fact 3: "Ten is exactly
/// `|InfrastructureSlotV1::ALL|`, so a fully-populated catalog root has zero
/// headroom: the cap and the grammar have to move together." Both move here,
/// and both move only onto vocabulary R1+C0 already froze. Widening only: the
/// per-family caps below keep every previously-accepted interior accepted.
const MAX_INTERIOR_ENTRIES: usize = MAX_ROOT_ENTRIES;

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
    let mut action_rows = Vec::new();
    let mut census = CatalogRootRowCensusV1::default();
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate catalog interior", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read catalog interior", source))?;
        let name = entry.file_name();
        budget.charge_os_str(&name)?;
        if rows.len() + action_rows.len() == MAX_INTERIOR_ENTRIES {
            return Err(interior_bound_exceeded());
        }
        let class = exact_row(&name, mode)?;
        census.charge(class);
        if let CatalogRootRowClassV1::ActiveAction(action) = class {
            if action_rows.len() == MAX_ACTIVE_ACTION_DIRS {
                return Err(interior_bound_exceeded());
            }
            reserve_one(&mut action_rows)?;
            action_rows.push(action);
            continue;
        }
        // `exact_row` above refuses every `MalformedRecognized` and `Foreign`
        // child before it can be classified onto this path, so the only classes
        // that reach here are the slot-bearing ones and this cannot panic. Any
        // future widening of that refusal must keep the guarantee or convert
        // this into a typed refusal — the same invariant the driver's
        // `census.has_unowned_row()` stop is deliberately kept for
        // (`admission/driver.rs`).
        let slot = class
            .infrastructure_slot()
            .expect("a classified non-action catalog row owns an infrastructure slot");
        if rows.len() == MAX_INFRASTRUCTURE_ENTRIES {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "catalog interior exceeds the ten-slot bound",
            ));
        }
        let fact = observe_slot(
            directory,
            &name,
            slot == InfrastructureSlotV1::RetiredActions,
            platform,
        )?;
        reserve_one(&mut rows)?;
        rows.push(RawCatalogInteriorRowV1 { slot, fact });
    }
    rows.sort_unstable_by_key(|row| slot_index(row.slot));
    if rows.windows(2).any(|pair| pair[0].slot == pair[1].slot) {
        return Err(CheckedFsError::ambiguous(
            "catalog interior",
            "multiple native entries resolve to one infrastructure slot",
        ));
    }
    action_rows.sort_unstable_by_key(|action| action.bytes());
    if action_rows.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(CheckedFsError::ambiguous(
            "catalog interior",
            "multiple native entries resolve to one action row",
        ));
    }
    Ok(RawCatalogInteriorObservationV1 {
        entry_count: budget.entry_count(),
        encoded_name_bytes: budget.encoded_name_bytes(),
        rows,
        action_rows,
        census,
    })
}

fn interior_bound_exceeded() -> CheckedFsError {
    CheckedFsError::unsupported(
        PlatformCapability::PrivateNamespaceCollisionScan,
        "catalog interior exceeds the frozen root-entry bound",
    )
}

fn reserve_one<T>(values: &mut Vec<T>) -> Result<(), CheckedFsError> {
    values.try_reserve_exact(1).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog interior row allocation failed",
        )
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
    // The C-3 widening (interface freeze §4.4 Class 2 fact 2) is deliberately
    // **not** applied here. Admission runs only against a complete catalog's
    // root, never into a bootstrap-staging interior, so no cooperating history
    // ever places an `ActionAdmission*` slot inside a staging directory: the
    // recorded breakage chain (`recover_or_create` -> `execute_owner_complete`
    // -> `retain_completed_catalog`) runs through `completed_record`, which is
    // where the drop is owed and taken. Dropping the triad here instead widened
    // the bootstrap's *adoption* grammar with no flow that needs it — a staging
    // directory planted with a stray-but-valid admission record alongside the
    // six exact roles would classify `Complete`, pass the CatalogStaging
    // source-interior recheck, and publish as a live catalog carrying an
    // unexplained admission row. Beyond the amendment §4.1 trust boundary the
    // R1 posture is to fail closed, so the refusal stays.
    // `CatalogBootstrapRetired` keeps its own refusal: it is the bootstrap
    // owner's pre-retirement discriminator.
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
    // C-3 widening (interface freeze §4.4 Class 2 facts 2 and the "Consequence"
    // paragraph): with an `ActionAdmissionActive` slot resident — the steady
    // state after a successful admission — this predicate used to be `None`,
    // which broke `retain_completed_catalog` (`completed.rs:61`) and therefore
    // ConsumerCheckpoint §7 step 8's reobservation. The admission triad is now
    // admitted; `CatalogAnchorB` keeps its refusal because an unexercised
    // B anchor still means the catalog is mid-bootstrap.
    if any_present(interior, &[Slot::CatalogAnchorB]) {
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

/// Classifies one catalog-root child into the §6 (:199-201) grammar.
///
/// C-3 widening (`GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 2 fact 1): the
/// observer previously walked `InfrastructureSlotV1::ALL` alone and refused
/// every other child, so it "does not yet admit the very row E3 publishes".
/// It now walks the whole frozen `RootEntryNameV1` grammar, whose second arm is
/// the active-action row. Widening only: the platform-alias and unowned-child
/// refusals are byte-identical, so every interior that classified before still
/// classifies the same way, and only `action-<hex>-v1` rows are newly admitted.
fn exact_row(
    name: &OsStr,
    mode: crate::checked_artifact::capability::PathComponentMode,
) -> Result<CatalogRootRowClassV1, CheckedFsError> {
    for slot in InfrastructureSlotV1::ALL.iter().copied() {
        if native_name_matches_ascii(name, slot.name().as_bytes(), mode)? {
            return if name == OsStr::new(slot.name()) {
                Ok(CatalogRootRowClassV1::classify(slot.name().as_bytes()))
            } else {
                Err(CheckedFsError::ambiguous(
                    "catalog interior",
                    "platform-equivalent infrastructure alias is noncanonical",
                ))
            };
        }
    }
    let class = CatalogRootRowClassV1::classify(native_ascii_bytes(name).unwrap_or(&[]));
    if matches!(class, CatalogRootRowClassV1::ActiveAction(_)) {
        return Ok(class);
    }
    Err(CheckedFsError::ambiguous(
        "catalog interior",
        "catalog directory contains an unowned child",
    ))
}

/// The canonical ASCII spelling of a native name, or `None` when the platform
/// name is not ASCII. An action row's grammar is ASCII-only
/// (`protocol/slots.rs:385-400`), so a non-ASCII child can never be one and
/// falls through to the unchanged unowned-child refusal.
fn native_ascii_bytes(name: &OsStr) -> Option<&[u8]> {
    let bytes = name.to_str()?.as_bytes();
    bytes.is_ascii().then_some(bytes)
}

fn observe_slot(
    directory: &cap_std::fs::Dir,
    name: &OsStr,
    probe_empty_directory: bool,
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
        if probe_empty_directory {
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

/// Bounded observation of one action directory through the frozen
/// [`ActionSlotV1`] grammar.
///
/// It lives in this file because `GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 1
/// records that the verification a recheck arm drives "lives in a different
/// file of the same owner" — `publication.rs` decides, `interior.rs` verifies.
/// It returns the frozen R1 observation type, so neither the admission driver
/// nor the sealed primitive receives a handle or a raw row.
pub(super) fn observe_action_directory(
    parent: &cap_std::fs::Dir,
    name: &OsStr,
    expected: &ActionCapacityReservationV1,
    platform: &impl PlatformProviderV1,
) -> Result<ObservedActionDirectoryV1, CheckedFsError> {
    let metadata = match parent.symlink_metadata(name) {
        Ok(value) => value,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ObservedActionDirectoryV1::Missing);
        }
        Err(source) => return Err(CheckedFsError::io("observe action directory", source)),
    };
    if !metadata.is_dir() || metadata.is_symlink() {
        return Ok(ObservedActionDirectoryV1::Other);
    }
    let directory = parent
        .open_dir_nofollow(name)
        .map_err(|source| CheckedFsError::io("open action directory", source))?;
    let identity = platform.dir_identity(&directory)?;
    let observed = observe_action_interior(&directory, expected)?;
    Ok(ObservedActionDirectoryV1::exact(
        identity.durable().clone(),
        observed.reservation,
        observed.extra_children,
    ))
}

/// Bounded interior of an already-open action directory, so the sealed
/// publication primitive can re-verify its retained source handle without
/// reopening the name it is about to consume.
pub(super) struct ActionInteriorObservationV1 {
    pub(super) reservation: RecordObservationV1<ActionCapacityReservationV1>,
    pub(super) extra_children: usize,
}

impl ActionInteriorObservationV1 {
    /// The §7 (:220-221) exactness predicate: the deterministic resident
    /// reservation and no extra children.
    pub(super) fn is_exact(&self, expected: &ActionCapacityReservationV1) -> bool {
        self.extra_children == 0
            && matches!(&self.reservation, RecordObservationV1::Exact(value) if value == expected)
    }
}

pub(super) fn observe_action_interior(
    directory: &cap_std::fs::Dir,
    expected: &ActionCapacityReservationV1,
) -> Result<ActionInteriorObservationV1, CheckedFsError> {
    let reservation_name =
        ActionSlotV1::Base(BaseActionSlotV1::Reservation).name(expected.action_digest());
    let reservation_name = OsStr::new(reservation_name.as_str());
    let expected_bytes = expected.encode_canonical().map_err(|_| {
        CheckedFsError::ambiguous(
            "action capacity reservation",
            "expected capacity record is not canonically encodable",
        )
    })?;
    let mut budget = CatalogNameBudgetV1::new();
    let mut reservation = RecordObservationV1::Missing;
    let mut extra_children = 0_usize;
    let mut seen = 0_usize;
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate action directory", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read action directory", source))?;
        let child = entry.file_name();
        budget.charge_os_str(&child)?;
        seen += 1;
        if seen > MAX_ACTION_SLOTS {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "action directory exceeds the frozen action-slot bound",
            ));
        }
        if child == reservation_name {
            reservation = observe_reservation(directory, &child, &expected_bytes, expected)?;
            continue;
        }
        extra_children += 1;
    }
    Ok(ActionInteriorObservationV1 {
        reservation,
        extra_children,
    })
}

fn observe_reservation(
    directory: &cap_std::fs::Dir,
    name: &OsStr,
    expected_bytes: &[u8],
    expected: &ActionCapacityReservationV1,
) -> Result<RecordObservationV1<ActionCapacityReservationV1>, CheckedFsError> {
    let metadata = directory
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe resident reservation", source))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Ok(RecordObservationV1::Other);
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open resident reservation", source))?;
    let RawCatalogBytesV1::Bounded(bytes) =
        read_bounded_with(&mut file, ProtocolRecordKindV1::Capacity.max_bytes())?
    else {
        return Ok(RecordObservationV1::Other);
    };
    Ok(match classify_expected_prefix(&bytes, expected_bytes) {
        ScratchBytesV1::Exact => RecordObservationV1::Exact(expected.clone()),
        ScratchBytesV1::PartialExpectedPrefix => RecordObservationV1::PartialExpectedPrefix,
        ScratchBytesV1::Missing | ScratchBytesV1::Other => RecordObservationV1::Other,
    })
}

fn read_bounded(file: &mut cap_std::fs::File) -> Result<RawCatalogBytesV1, CheckedFsError> {
    read_bounded_with(
        file,
        ProtocolRecordKindV1::Infrastructure
            .max_bytes()
            .max(ProtocolRecordKindV1::CatalogBootstrap.max_bytes()),
    )
}

fn read_bounded_with(
    file: &mut cap_std::fs::File,
    limit: usize,
) -> Result<RawCatalogBytesV1, CheckedFsError> {
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
