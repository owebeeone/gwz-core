//! Sealed source-associated catalog namespace publication.

use std::ffi::OsStr;
use std::io::Read;

use cap_std::fs::Dir;

use super::interior::{self, StagingPlanV1};
use super::platform::HostPlatform;
use super::retained::encode_identity;
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1, PlatformCapability,
};
use crate::checked_artifact::catalog::CatalogRecordFactV1;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, CatalogBootstrapRecordV1, InfrastructureSlotV1,
    MAX_ACTIVE_ACTION_DIRS, OwnershipMarkerV1, RootEntryNameV1,
};
use crate::model::ErrorCode;

pub(super) enum PublicationSourceV1<'a> {
    RegularFile {
        expected_identity: &'a [u8],
        expected_bytes: &'a [u8],
    },
    Directory {
        expected_identity: &'a [u8],
        interior: DirectoryInteriorRecheckV1<'a>,
    },
}

/// Source-interior expectation re-verified through the primitive's own
/// directory capability inside the acquisition window, so pre-acquisition
/// interior drift rejects before publication (amendment §4.1
/// drift-rejection paragraph, as clarified by the 2026-08-15 erratum); the
/// remaining residue after this re-check is post-acquisition and falls
/// inside the accepted same-user namespace boundary.
pub(super) struct DirectoryInteriorRecheckV1<'a> {
    pub(super) durable_identity: &'a DurableObjectIdentityV1,
    pub(super) expected: DirectoryInteriorExpectationV1<'a>,
}

/// C-2, the recheck-arm class, source-interior half
/// (`GwzM5-8R2DInterfaceFreeze.md` §4.4 Class 1: "the `expected` field is
/// mandatory and catalog-typed, so the admission arm is a *generalization of
/// that struct or its field type*, not the addition of a variant"; §6 restates
/// it as "a generalization of the `DirectoryInteriorRecheckV1` **struct**'s
/// `expected` field").
///
/// Every arm is a lifetime-parameterized reference holder with no encode path:
/// they are built by the mutation owners, consumed read-only inside
/// [`publish_verified_no_replace`], and dropped. Nothing is serialized and
/// nothing is reachable from a durable-record root.
#[allow(
    clippy::enum_variant_names,
    reason = "the shared `Staging` postfix is the invariant, not noise: each arm \
              names the staged-directory layout of exactly one converting package, \
              and the parallel names are what make the §4.4 Class 1 arm inventory \
              readable against the freeze's own table"
)]
pub(super) enum DirectoryInteriorExpectationV1<'a> {
    /// R2-C2's arm: the interior is a completed catalog staging layout.
    CatalogStaging(&'a CatalogBootstrapRecordV1),
    /// R2-D Phase 1's arm (edge E3): the interior is a resident
    /// `ActionCapacityReservationV1`, not a `CatalogBootstrapRecordV1`.
    AdmissionStaging(&'a ActionCapacityReservationV1),
    /// R2-D Step 2.3's arm (edge E15), the §4.4 Class 1 row "managed
    /// source-interior": a staged managed component's interior "is neither
    /// record type" — it is exactly the frozen ownership-marker leaf carrying
    /// this exact `OwnershipMarkerV1`. Like both arms above it is a borrowed
    /// protocol record with no encode path out of this owner; the marker is
    /// encoded once, in memory, inside the acquisition window and compared
    /// byte-exact.
    ManagedStaging(&'a OwnershipMarkerV1),
}

/// Destination-interior expectation re-verified through the retained
/// destination capability immediately before the rename edge.
pub(super) enum DestinationRecheckV1<'a> {
    None,
    PreRetirementFinal {
        durable_identity: &'a DurableObjectIdentityV1,
        expected: &'a CatalogBootstrapRecordV1,
    },
    /// C-2, the recheck-arm class, destination half (interface freeze §4.4
    /// Class 1 row "admission destination", edges E3 and E7; §6: "a variant on
    /// the `DestinationRecheckV1` **enum**"). The destination of every
    /// admission edge is the completed catalog interior itself, so the arm
    /// re-verifies that the catalog is still exactly complete *and* that the
    /// deterministic destination row is still free — the no-replace property
    /// restated as an in-window expectation rather than only as a rename flag.
    AdmissionCatalogInterior {
        durable_identity: &'a DurableObjectIdentityV1,
        expected: &'a CatalogBootstrapRecordV1,
        absent: RootEntryNameV1,
    },
}

impl<'a> PublicationSourceV1<'a> {
    pub(super) const fn regular_file(
        expected_identity: &'a [u8],
        expected_bytes: &'a [u8],
    ) -> Self {
        Self::RegularFile {
            expected_identity,
            expected_bytes,
        }
    }

    pub(super) const fn directory(
        expected_identity: &'a [u8],
        interior: DirectoryInteriorRecheckV1<'a>,
    ) -> Self {
        Self::Directory {
            expected_identity,
            interior,
        }
    }

    const fn expected_identity(&self) -> &[u8] {
        match self {
            Self::RegularFile {
                expected_identity, ..
            }
            | Self::Directory {
                expected_identity, ..
            } => expected_identity,
        }
    }
}

pub(super) fn publish_verified_no_replace(
    source_dir: &Dir,
    source: &OsStr,
    destination_dir: &Dir,
    destination: &OsStr,
    expected: PublicationSourceV1<'_>,
    destination_recheck: DestinationRecheckV1<'_>,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let mut source_handle = crate::checked_artifact::platform::open_rename_source(
        source_dir,
        source,
        ErrorCode::IoError,
        label,
    )
    .map_err(|source| CheckedFsError::ambiguous(label, source.message))?;
    if encode_identity(&HostPlatform.file_identity(source_handle.file())?)
        != expected.expected_identity()
    {
        return Err(CheckedFsError::ambiguous(
            label,
            "publication source identity changed",
        ));
    }
    match &expected {
        PublicationSourceV1::RegularFile { expected_bytes, .. } => {
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(expected_bytes.len() + 1)
                .map_err(|_| {
                    CheckedFsError::unsupported(
                        PlatformCapability::PrivateNamespaceCollisionScan,
                        "publication source verification allocation failed",
                    )
                })?;
            source_handle
                .file_mut()
                .by_ref()
                .take((expected_bytes.len() + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|source| CheckedFsError::io("read publication source", source))?;
            if bytes != *expected_bytes {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "publication source bytes changed",
                ));
            }
        }
        PublicationSourceV1::Directory {
            expected_identity,
            interior: recheck,
        } => {
            let directory =
                crate::checked_artifact::platform::open_dir_share_delete(source_dir, source)
                    .map_err(|source| {
                        CheckedFsError::io("reopen publication source directory", source)
                    })?;
            if encode_identity(&HostPlatform.dir_identity(&directory)?) != *expected_identity {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "publication source identity changed",
                ));
            }
            let exact = match recheck.expected {
                DirectoryInteriorExpectationV1::CatalogStaging(expected) => {
                    let fresh = interior::observe(&directory, &HostPlatform)?;
                    matches!(
                        interior::staging_plan(recheck.durable_identity, &fresh, expected),
                        StagingPlanV1::Complete(_)
                    )
                }
                DirectoryInteriorExpectationV1::AdmissionStaging(expected) => {
                    interior::observe_action_interior(&directory, expected)?.is_exact(expected)
                }
                DirectoryInteriorExpectationV1::ManagedStaging(expected) => {
                    interior::observe_managed_component_interior(&directory, expected)?.is_exact()
                }
            };
            if !exact {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "publication source interior changed inside the acquisition window",
                ));
            }
        }
    }
    match &destination_recheck {
        DestinationRecheckV1::None => {}
        DestinationRecheckV1::PreRetirementFinal {
            durable_identity,
            expected,
        } => {
            let fresh = interior::observe(destination_dir, &HostPlatform)?;
            if interior::completed_record(durable_identity, &fresh, expected).is_none()
                || interior::row(&fresh, InfrastructureSlotV1::CatalogBootstrapRetired).is_some()
            {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "publication destination interior changed inside the acquisition window",
                ));
            }
        }
        DestinationRecheckV1::AdmissionCatalogInterior {
            durable_identity,
            expected,
            absent,
        } => {
            let fresh = interior::observe(destination_dir, &HostPlatform)?;
            let (occupied, full) = match absent {
                RootEntryNameV1::Infrastructure(slot) => {
                    (interior::row(&fresh, *slot).is_some(), false)
                }
                RootEntryNameV1::ActiveAction(action) => (
                    fresh.action_rows.contains(action),
                    fresh.action_rows.len() >= MAX_ACTIVE_ACTION_DIRS,
                ),
            };
            // The frozen active-action budget, re-proved inside the acquisition
            // window. `interior::observe` refuses a root that already exceeds it,
            // and no admission edge can remove an action row, so publishing the
            // 65th row would leave a catalog that no sealed path — including
            // `recover_or_create` — can observe again. This is the race-free
            // half of the refusal: it also closes an admission resumed after
            // other admissions filled the root, which never re-enters the
            // driver's new-admission gate.
            if full {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "publication would exceed the frozen active-action bound",
                ));
            }
            if occupied
                || interior::completed_record(durable_identity, &fresh, expected).is_none()
                || !matches!(
                    interior::retired_record(&fresh),
                    CatalogRecordFactV1::Exact(value) if value.as_ref() == *expected
                )
            {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "publication destination interior changed inside the acquisition window",
                ));
            }
        }
    }
    crate::checked_artifact::platform::rename_open_source(
        &source_handle,
        destination_dir,
        destination,
        false,
        ErrorCode::IoError,
        label,
    )
    .map_err(|source| CheckedFsError::ambiguous(label, source.message))
}
