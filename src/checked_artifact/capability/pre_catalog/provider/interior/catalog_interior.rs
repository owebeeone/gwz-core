use super::super::filesystem::PlatformProviderV1;
use super::super::{
    RawCatalogInteriorFactV1, RawCatalogInteriorObservationV1, RawCatalogInteriorRowV1,
};
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::catalog::{
    CatalogDirectoryFactV1, CatalogNameBudgetV1, native_name_matches_ascii,
};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{
    CatalogBootstrapRecordV1, CatalogRootRowCensusV1, CatalogRootRowClassV1, InfrastructureSlotV1,
    MAX_ACTIVE_ACTION_DIRS, MAX_INFRASTRUCTURE_ENTRIES, MAX_RETIRED_ACTION_DIRS,
};
use std::ffi::OsStr;

use super::*;

pub(in crate::checked_artifact::capability::pre_catalog::provider) fn observe(
    directory: &crate::filesystem::FsDirectory,
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
        let name = entry;
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

pub(in crate::checked_artifact::capability::pre_catalog::provider) fn row(
    interior: &RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
) -> Option<&RawCatalogInteriorFactV1> {
    interior
        .rows
        .iter()
        .find(|candidate| candidate.slot == slot)
        .map(|candidate| &candidate.fact)
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
pub(crate) fn exact_row(
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

pub(crate) fn slot_index(slot: InfrastructureSlotV1) -> usize {
    InfrastructureSlotV1::ALL
        .iter()
        .position(|candidate| *candidate == slot)
        .expect("slot belongs to the closed infrastructure grammar")
}

pub(in crate::checked_artifact::capability::pre_catalog::provider) fn directory_fact(
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

/// The bounded count of retired action directories resident under the catalog's
/// `RetiredActions` root, for the callers that charge the frozen retirement
/// credit against it (`protocol/bounds.rs` `CatalogOccupancyV1`). `None` means
/// the retired root is not readable as a retired root at all, which is the same
/// fact [`completed_record`] refuses on.
pub(in crate::checked_artifact::capability::pre_catalog::provider) fn retired_action_dirs(
    interior: &RawCatalogInteriorObservationV1,
) -> Option<usize> {
    match row(interior, InfrastructureSlotV1::RetiredActions)? {
        RawCatalogInteriorFactV1::EmptyDirectory { .. } => Some(0),
        RawCatalogInteriorFactV1::RetiredActionRoot {
            unaccepted_rows,
            retired_action_dirs,
            ..
        } if *unaccepted_rows == 0 && *retired_action_dirs <= MAX_RETIRED_ACTION_DIRS => {
            Some(*retired_action_dirs)
        }
        _ => None,
    }
}

pub(crate) fn any_present(
    interior: &RawCatalogInteriorObservationV1,
    slots: &[InfrastructureSlotV1],
) -> bool {
    slots.iter().any(|slot| row(interior, *slot).is_some())
}

pub(crate) fn only_missing(
    interior: &RawCatalogInteriorObservationV1,
    slots: &[InfrastructureSlotV1],
) -> bool {
    !any_present(interior, slots)
}

pub(crate) fn later_after_roaming_missing(interior: &RawCatalogInteriorObservationV1) -> bool {
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

pub(crate) fn descriptor_and_format_missing(interior: &RawCatalogInteriorObservationV1) -> bool {
    only_missing(
        interior,
        &[
            InfrastructureSlotV1::RetiredActionsDescriptor,
            InfrastructureSlotV1::CatalogFormat,
        ],
    )
}
