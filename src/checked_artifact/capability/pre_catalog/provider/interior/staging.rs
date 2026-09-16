use super::super::{RawCatalogBytesV1, RawCatalogInteriorFactV1, RawCatalogInteriorObservationV1};
use crate::checked_artifact::protocol::{
    CatalogBootstrapRecordV1, InfrastructureRecordV1, InfrastructureSlotV1, ScratchBytesV1,
    classify_expected_prefix,
};

use super::*;

/// R2-E Phase E2 shares this frozen constant with `barrier_mutation.rs`, whose
/// roaming-anchor alias is a fresh, independent copy of exactly these bytes
/// (DECISION B-5). Only the visibility moved: the predicate that consumes it
/// below is unchanged.
pub(in crate::checked_artifact::capability::pre_catalog::provider) const ROAMING_ANCHOR_BYTES:
    &[u8] = b"GWZ-ROAMING-ANCHOR-V1\n";
pub(crate) const CATALOG_ANCHOR_BYTES: &[u8] = b"GWZ-CATALOG-ANCHOR-V1\n";

pub(in crate::checked_artifact::capability::pre_catalog::provider) enum StagingPlanV1 {
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
pub(in crate::checked_artifact::capability::pre_catalog::provider) fn staging_plan(
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
        // T1's widening is deliberately **not** applied here, and the arm is
        // preserved by name rather than left to the catch-all below
        // (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §2.2, "A fourth
        // surface E3.1 must preserve deliberately, not widen"). A catalog
        // being *built* must not find action rows already retired into it: the
        // three widened gates all read a catalog that is already complete,
        // whereas this plan decides whether an incomplete staging interior may
        // be adopted, and adopting one carrying retired action rows would
        // publish a live catalog with an unexplained retirement history. Same
        // posture as the `ActionAdmission*` triad's refusal above.
        Some(RawCatalogInteriorFactV1::RetiredActionRoot { .. }) => {
            return StagingPlanV1::Other;
        }
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

pub(crate) enum FilePrefixV1<'a> {
    Missing,
    Partial,
    Exact(&'a crate::checked_artifact::capability::DurableObjectIdentityV1),
    Other,
}

pub(crate) fn file_prefix<'a>(
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

pub(in crate::checked_artifact::capability::pre_catalog::provider) const fn roaming_anchor_bytes()
-> &'static [u8] {
    ROAMING_ANCHOR_BYTES
}

pub(in crate::checked_artifact::capability::pre_catalog::provider) const fn catalog_anchor_bytes()
-> &'static [u8] {
    CATALOG_ANCHOR_BYTES
}
