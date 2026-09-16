use super::super::{RawCatalogBytesV1, RawCatalogInteriorFactV1, RawCatalogInteriorObservationV1};
use crate::checked_artifact::protocol::{
    CatalogBootstrapRecordV1, InfrastructureRecordV1, InfrastructureSlotV1,
    MAX_RETIRED_ACTION_DIRS, decode_catalog_bootstrap_record,
};

use super::*;

pub(in crate::checked_artifact::capability::pre_catalog::provider) fn completed_record(
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
    // T1 widening gate 1 of 3 (E0.2b §2.2). This was
    // `empty_directory_identity(interior, Slot::RetiredActions)?`, which
    // returned `Some` only for an *empty* retired root — so the catalog became
    // unobservable, and therefore unrecoverable, at its own first terminal
    // retirement. `retired_root_identity` accepts the same empty root plus a
    // populated one whose every child is a `RootEntryNameV1::ActiveAction` row
    // and whose count is within `MAX_RETIRED_ACTION_DIRS`. The identity it
    // returns is the retired root's own durable identity in both arms, which a
    // child addition does not change, so `RetiredActionsDescriptor` and
    // `CatalogFormat` stay byte-identical across the widening.
    let retired = retired_root_identity(interior)?;
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

pub(in crate::checked_artifact::capability::pre_catalog::provider) fn retired_record(
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

pub(crate) fn exact_file_identity(
    interior: &RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    expected: &[u8],
) -> Option<crate::checked_artifact::capability::DurableObjectIdentityV1> {
    match file_prefix(interior, slot, expected) {
        FilePrefixV1::Exact(identity) => Some(identity.clone()),
        _ => None,
    }
}

/// The `RetiredActions` root's own durable identity, under T1's widened
/// reading (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §2, both axes
/// concurring; the freeze's Class 2 sanction `:1443-1450`).
///
/// Two arms, and only two: the empty root every catalog is created with
/// (`directory_mutation.rs`'s single `CreateRetiredActions` arm), and a
/// populated one whose children are **exclusively**
/// `RootEntryNameV1::ActiveAction` rows.
///
/// **This bound is checked explicitly and is not inherited.** The reused
/// reader's own caps are `MAX_INTERIOR_ENTRIES` (= `MAX_ROOT_ENTRIES` = 74)
/// and `MAX_ACTIVE_ACTION_DIRS` (= 64) — neither of them
/// `MAX_RETIRED_ACTION_DIRS`. They are numerically safe today only because
/// `bounds.rs:1` and `:2` are both 64, which silently couples the retired-root
/// bound to the active one; the explicit comparison below is what makes a
/// future edit to either constant fail closed here instead of decoupling them
/// unnoticed (E0.2b §3.2 ground 3, Code round-2 [P3-R1]).
pub(crate) fn retired_root_identity(
    interior: &RawCatalogInteriorObservationV1,
) -> Option<crate::checked_artifact::capability::DurableObjectIdentityV1> {
    match row(interior, InfrastructureSlotV1::RetiredActions)? {
        RawCatalogInteriorFactV1::EmptyDirectory {
            durable_identity, ..
        } => Some(durable_identity.clone()),
        RawCatalogInteriorFactV1::RetiredActionRoot {
            durable_identity,
            unaccepted_rows,
            retired_action_dirs,
            ..
        } if *unaccepted_rows == 0 && *retired_action_dirs <= MAX_RETIRED_ACTION_DIRS => {
            Some(durable_identity.clone())
        }
        _ => None,
    }
}
