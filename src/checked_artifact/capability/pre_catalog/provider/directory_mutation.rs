//! Owner-private physical transitions for the staged and final catalog directory.

use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom, Write};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;

use super::interior::{self, StagingPlanV1};
use super::publication::{
    DestinationRecheckV1, DirectoryInteriorExpectationV1, DirectoryInteriorRecheckV1,
    PublicationSourceV1, publish_verified_no_replace,
};
use super::retained::encode_identity;
use super::{
    RawCatalogBytesV1, RawCatalogEntryFactV1, RawCatalogInteriorFactV1,
    RawCatalogInteriorObservationV1, RawCatalogRoleObservationV1, RetainedPlatformRoot,
};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, PlatformCapability,
};
use crate::checked_artifact::catalog::CatalogRecognizedNameV1;
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{CatalogBootstrapRecordV1, InfrastructureSlotV1};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CatalogDirectoryMutationFaultV1 {
    StagingAfterOpen,
    AnchorAfterPublishA,
    AnchorAfterMoveToB,
    AnchorAfterReturnA,
    FinalPublishBeforeRename,
    FinalPublishAfterInteriorRecheck,
    ActiveRetireBeforeRename,
    ActiveRetireAfterInteriorRecheck,
    CompleteAfterFinalOpen,
}

#[cfg(test)]
type CatalogDirectoryMutationFaultCallbackV1 = (CatalogDirectoryMutationFaultV1, Box<dyn FnOnce()>);

#[cfg(test)]
thread_local! {
    static NEXT_FAULT: std::cell::RefCell<Option<CatalogDirectoryMutationFaultCallbackV1>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn run_next_at(
    point: CatalogDirectoryMutationFaultV1,
    callback: impl FnOnce() + 'static,
) {
    NEXT_FAULT.with(|slot| {
        let previous = slot.replace(Some((point, Box::new(callback))));
        assert!(
            previous.is_none(),
            "catalog directory mutation fault already installed"
        );
    });
}

#[cfg(test)]
pub(super) fn run_fault(point: CatalogDirectoryMutationFaultV1) {
    NEXT_FAULT.with(|slot| {
        let execute = {
            let mut slot = slot.borrow_mut();
            match slot.as_ref() {
                Some((expected, _)) if *expected == point => slot.take(),
                _ => None,
            }
        };
        if let Some((_, callback)) = execute {
            callback();
        }
    });
}

pub(in crate::checked_artifact::capability::pre_catalog) fn prepare_or_rewrite_staging(
    retained: &RetainedPlatformRoot,
    raw_roles: &RawCatalogRoleObservationV1,
    expected: &CatalogBootstrapRecordV1,
) -> Result<(), CheckedFsError> {
    let parent = mutation_parent(retained, "catalog staging")?;
    let staging_name = OsStr::new(private_name(CatalogPrivateNameV1::BootstrapStaging));
    let Some(observed) = observed_directory(raw_roles, CatalogRecognizedNameV1::Staging)? else {
        parent
            .handle()
            .create_dir(staging_name)
            .map_err(|source| CheckedFsError::io("create catalog staging no-replace", source))?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapStagingCreate,
        );
        let directory = parent
            .handle()
            .open_dir_nofollow(staging_name)
            .map_err(|source| CheckedFsError::io("reopen catalog staging", source))?;
        super::HostPlatform.dir_identity(&directory)?;
        return sync_directory_edge(parent.handle(), "flush catalog staging creation");
    };
    let staging =
        open_observed_directory(parent.handle(), staging_name, observed, "catalog staging")?;
    #[cfg(test)]
    run_fault(CatalogDirectoryMutationFaultV1::StagingAfterOpen);
    verify_named_directory(parent.handle(), staging_name, observed, "catalog staging")?;
    match interior::staging_plan(observed.durable_identity, observed.interior, expected) {
        StagingPlanV1::CreateRetiredActions => {
            create_empty_directory(&staging, InfrastructureSlotV1::RetiredActions)?;
        }
        StagingPlanV1::WriteRoamingAnchor { create_new } => write_slot(
            &staging,
            observed.interior,
            InfrastructureSlotV1::RoamingAnchorHome,
            interior::roaming_anchor_bytes(),
            create_new,
        )?,
        StagingPlanV1::WriteCatalogAnchorB { create_new } => write_slot(
            &staging,
            observed.interior,
            InfrastructureSlotV1::CatalogAnchorB,
            interior::catalog_anchor_bytes(),
            create_new,
        )?,
        StagingPlanV1::ExerciseAnchorAndWriteDescriptor(record) => {
            exercise_catalog_anchor(&staging, observed.interior)?;
            write_slot(
                &staging,
                observed.interior,
                InfrastructureSlotV1::RetiredActionsDescriptor,
                &record.encode_canonical(),
                true,
            )?;
        }
        StagingPlanV1::WriteDescriptor { record, create_new } => write_slot(
            &staging,
            observed.interior,
            InfrastructureSlotV1::RetiredActionsDescriptor,
            &record.encode_canonical(),
            create_new,
        )?,
        StagingPlanV1::WriteFormat { record, create_new } => write_slot(
            &staging,
            observed.interior,
            InfrastructureSlotV1::CatalogFormat,
            &record.encode_canonical(),
            create_new,
        )?,
        StagingPlanV1::Complete(_) => {
            return Err(CheckedFsError::ambiguous(
                "catalog staging",
                "exact staging requires final publication rather than preparation",
            ));
        }
        StagingPlanV1::Other => {
            return Err(CheckedFsError::ambiguous(
                "catalog staging",
                "staging contents are outside the active-owned prefix grammar",
            ));
        }
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapInfrastructurePopulate,
    );
    sync_directory_edge(&staging, "flush catalog staging edge")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapInfrastructureFlush,
    );
    Ok(())
}

pub(in crate::checked_artifact::capability::pre_catalog) fn publish_final_directory(
    retained: &RetainedPlatformRoot,
    raw_roles: &RawCatalogRoleObservationV1,
    expected: &CatalogBootstrapRecordV1,
) -> Result<(), CheckedFsError> {
    let parent = mutation_parent(retained, "catalog final publication")?;
    if observed_directory(raw_roles, CatalogRecognizedNameV1::Final)?.is_some() {
        return Err(CheckedFsError::ambiguous(
            "catalog final publication",
            "final destination was present in the fresh aggregate",
        ));
    }
    let staging_observed = observed_directory(raw_roles, CatalogRecognizedNameV1::Staging)?
        .ok_or_else(|| {
            CheckedFsError::ambiguous(
                "catalog final publication",
                "exact staging source is missing",
            )
        })?;
    if !matches!(
        interior::staging_plan(
            staging_observed.durable_identity,
            staging_observed.interior,
            expected,
        ),
        StagingPlanV1::Complete(_)
    ) {
        return Err(CheckedFsError::ambiguous(
            "catalog final publication",
            "staging source is not the exact completed pre-retirement layout",
        ));
    }
    let staging_name = OsStr::new(private_name(CatalogPrivateNameV1::BootstrapStaging));
    let final_name = OsStr::new(private_name(CatalogPrivateNameV1::Final));
    let staging = open_observed_directory(
        parent.handle(),
        staging_name,
        staging_observed,
        "catalog final publication",
    )?;
    sync_directory_edge(&staging, "flush exact catalog staging")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapStagingFlush,
    );
    #[cfg(test)]
    run_fault(CatalogDirectoryMutationFaultV1::FinalPublishBeforeRename);
    verify_named_directory(
        parent.handle(),
        staging_name,
        staging_observed,
        "catalog final publication",
    )?;
    let fresh_staging = interior::observe(&staging, &super::HostPlatform)?;
    if !matches!(
        interior::staging_plan(staging_observed.durable_identity, &fresh_staging, expected),
        StagingPlanV1::Complete(_)
    ) {
        return Err(CheckedFsError::ambiguous(
            "catalog final publication",
            "staging contents changed before final publication",
        ));
    }
    // Release the caller's staging capability before the rename edge: on
    // Windows a directory rename fails with a sharing violation while any
    // handle into the source tree survives, and the sealed primitive
    // re-establishes source identity and interior through its own
    // capabilities, so nothing is verified through this handle past this
    // point (W4 catalog slice, GwzWindowsMatrix-Classification.md).
    drop(staging);
    #[cfg(test)]
    run_fault(CatalogDirectoryMutationFaultV1::FinalPublishAfterInteriorRecheck);
    publish_verified_no_replace(
        parent.handle(),
        staging_name,
        parent.handle(),
        final_name,
        PublicationSourceV1::directory(
            staging_observed.identity,
            DirectoryInteriorRecheckV1 {
                durable_identity: staging_observed.durable_identity,
                expected: DirectoryInteriorExpectationV1::CatalogStaging(expected),
            },
        ),
        DestinationRecheckV1::None,
        "publish final catalog",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapFinalPublish,
    );
    let final_directory = parent
        .handle()
        .open_dir_nofollow(final_name)
        .map_err(|source| CheckedFsError::io("reopen published final catalog", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapFinalReopen,
    );
    verify_directory_identity(
        &final_directory,
        staging_observed.identity,
        "published final catalog",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapFinalReobserve,
    );
    sync_directory_edge(parent.handle(), "flush final catalog publication")
}

pub(in crate::checked_artifact::capability::pre_catalog) fn retire_active_record(
    retained: &RetainedPlatformRoot,
    raw_roles: &RawCatalogRoleObservationV1,
    expected: &CatalogBootstrapRecordV1,
) -> Result<(), CheckedFsError> {
    let parent = mutation_parent(retained, "catalog active retirement")?;
    let final_observed = observed_directory(raw_roles, CatalogRecognizedNameV1::Final)?
        .ok_or_else(|| {
            CheckedFsError::ambiguous("catalog active retirement", "final catalog is missing")
        })?;
    if interior::completed_record(
        final_observed.durable_identity,
        final_observed.interior,
        expected,
    )
    .is_none()
        || row(
            final_observed.interior,
            InfrastructureSlotV1::CatalogBootstrapRetired,
        )
        .is_some()
    {
        return Err(CheckedFsError::ambiguous(
            "catalog active retirement",
            "final catalog is not exact pre-retirement state",
        ));
    }
    let active = observed_file(raw_roles, CatalogRecognizedNameV1::Active)?.ok_or_else(|| {
        CheckedFsError::ambiguous("catalog active retirement", "active record is missing")
    })?;
    if active.bytes != expected.encode_canonical() {
        return Err(CheckedFsError::ambiguous(
            "catalog active retirement",
            "active record bytes do not match the expected bootstrap",
        ));
    }
    let final_name = OsStr::new(private_name(CatalogPrivateNameV1::Final));
    let final_directory = open_observed_directory(
        parent.handle(),
        final_name,
        final_observed,
        "catalog active retirement",
    )?;
    #[cfg(test)]
    run_fault(CatalogDirectoryMutationFaultV1::ActiveRetireBeforeRename);
    let fresh_final = interior::observe(&final_directory, &super::HostPlatform)?;
    if interior::completed_record(final_observed.durable_identity, &fresh_final, expected).is_none()
        || row(&fresh_final, InfrastructureSlotV1::CatalogBootstrapRetired).is_some()
    {
        return Err(CheckedFsError::ambiguous(
            "catalog active retirement",
            "final catalog changed before active retirement",
        ));
    }
    #[cfg(test)]
    run_fault(CatalogDirectoryMutationFaultV1::ActiveRetireAfterInteriorRecheck);
    publish_verified_no_replace(
        parent.handle(),
        OsStr::new(private_name(CatalogPrivateNameV1::BootstrapActive)),
        &final_directory,
        OsStr::new(InfrastructureSlotV1::CatalogBootstrapRetired.name()),
        PublicationSourceV1::regular_file(active.identity, active.bytes),
        DestinationRecheckV1::PreRetirementFinal {
            durable_identity: final_observed.durable_identity,
            expected,
        },
        "retire catalog bootstrap record",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapActiveRetire,
    );
    verify_named_file(
        &final_directory,
        OsStr::new(InfrastructureSlotV1::CatalogBootstrapRetired.name()),
        active,
        "retired catalog bootstrap record",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapRetiredReobserve,
    );
    sync_directory_edge(&final_directory, "flush retired catalog record")?;
    sync_directory_edge(parent.handle(), "flush active catalog retirement")
}

#[derive(Clone, Copy)]
pub(super) struct ObservedDirectoryV1<'a> {
    pub(super) identity: &'a [u8],
    pub(super) durable_identity: &'a crate::checked_artifact::capability::DurableObjectIdentityV1,
    pub(super) interior: &'a RawCatalogInteriorObservationV1,
}

#[derive(Clone, Copy)]
pub(super) struct ObservedFileV1<'a> {
    pub(super) identity: &'a [u8],
    pub(super) bytes: &'a [u8],
}

pub(super) fn observed_directory(
    roles: &RawCatalogRoleObservationV1,
    role: CatalogRecognizedNameV1,
) -> Result<Option<ObservedDirectoryV1<'_>>, CheckedFsError> {
    let Some(fact) = roles
        .rows
        .iter()
        .find(|row| row.role == role)
        .map(|row| &row.fact)
    else {
        return Ok(None);
    };
    match fact {
        RawCatalogEntryFactV1::Directory {
            identity,
            durable_identity,
            interior,
        } => Ok(Some(ObservedDirectoryV1 {
            identity,
            durable_identity,
            interior,
        })),
        _ => Err(CheckedFsError::ambiguous(
            "catalog directory",
            "reserved directory role has the wrong kind",
        )),
    }
}

pub(super) fn observed_file(
    roles: &RawCatalogRoleObservationV1,
    role: CatalogRecognizedNameV1,
) -> Result<Option<ObservedFileV1<'_>>, CheckedFsError> {
    let Some(fact) = roles
        .rows
        .iter()
        .find(|row| row.role == role)
        .map(|row| &row.fact)
    else {
        return Ok(None);
    };
    match fact {
        RawCatalogEntryFactV1::RegularFile {
            identity,
            bytes: RawCatalogBytesV1::Bounded(bytes),
        } => Ok(Some(ObservedFileV1 { identity, bytes })),
        _ => Err(CheckedFsError::ambiguous(
            "catalog record",
            "reserved record role is not a bounded regular file",
        )),
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

fn mutation_parent<'a>(
    retained: &'a RetainedPlatformRoot,
    fact: &'static str,
) -> Result<&'a super::retained::RetainedDirectory, CheckedFsError> {
    retained
        .private_parent()
        .ok_or_else(|| CheckedFsError::ambiguous(fact, "retained private parent is missing"))
}

pub(super) fn open_observed_directory(
    parent: &cap_std::fs::Dir,
    name: &OsStr,
    observed: ObservedDirectoryV1<'_>,
    fact: &'static str,
) -> Result<cap_std::fs::Dir, CheckedFsError> {
    let directory = parent
        .open_dir_nofollow(name)
        .map_err(|source| CheckedFsError::io("open observed catalog directory", source))?;
    verify_directory_identity(&directory, observed.identity, fact)?;
    Ok(directory)
}

fn verify_named_directory(
    parent: &cap_std::fs::Dir,
    name: &OsStr,
    observed: ObservedDirectoryV1<'_>,
    fact: &'static str,
) -> Result<(), CheckedFsError> {
    let named = parent
        .open_dir_nofollow(name)
        .map_err(|source| CheckedFsError::io("reopen named catalog directory", source))?;
    verify_directory_identity(&named, observed.identity, fact)
}

fn verify_directory_identity(
    directory: &cap_std::fs::Dir,
    expected: &[u8],
    fact: &'static str,
) -> Result<(), CheckedFsError> {
    if encode_identity(&super::HostPlatform.dir_identity(directory)?) != expected {
        return Err(CheckedFsError::ambiguous(
            fact,
            "opened directory identity does not match the fresh aggregate",
        ));
    }
    Ok(())
}

fn create_empty_directory(
    parent: &cap_std::fs::Dir,
    slot: InfrastructureSlotV1,
) -> Result<(), CheckedFsError> {
    let name = OsStr::new(slot.name());
    parent
        .create_dir(name)
        .map_err(|source| CheckedFsError::io("create catalog infrastructure directory", source))?;
    let child = parent
        .open_dir_nofollow(name)
        .map_err(|source| CheckedFsError::io("reopen catalog infrastructure directory", source))?;
    super::HostPlatform.dir_identity(&child)?;
    sync_directory_edge(parent, "flush catalog infrastructure directory")
}

fn write_slot(
    directory: &cap_std::fs::Dir,
    interior: &RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    bytes: &[u8],
    create_new: bool,
) -> Result<(), CheckedFsError> {
    let name = OsStr::new(slot.name());
    let observed = row(interior, slot);
    if create_new != observed.is_none() {
        return Err(CheckedFsError::ambiguous(
            "catalog infrastructure write",
            "create/rewrite mode does not match the aggregate",
        ));
    }
    let options = durable_write_options(create_new);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open catalog infrastructure file", source))?;
    #[cfg(test)]
    if slot == InfrastructureSlotV1::CatalogAnchorB && create_new {
        crate::checked_artifact::fault_v1::hit(
            crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapAnchorScratchCreate,
        );
    }
    if let Some(observed) = observed {
        let RawCatalogInteriorFactV1::RegularFile {
            identity,
            bytes: RawCatalogBytesV1::Bounded(observed_bytes),
            ..
        } = observed
        else {
            return Err(CheckedFsError::ambiguous(
                "catalog infrastructure write",
                "rewrite source is not a bounded regular file",
            ));
        };
        let source = ObservedFileV1 {
            identity,
            bytes: observed_bytes,
        };
        verify_open_file(&mut file, source, "catalog infrastructure write")?;
        verify_named_file(directory, name, source, "catalog infrastructure write")?;
        file.set_len(0)
            .map_err(|source| CheckedFsError::io("truncate catalog infrastructure file", source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind catalog infrastructure file", source))?;
    }
    file.write_all(bytes)
        .map_err(|source| CheckedFsError::io("write catalog infrastructure file", source))?;
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush catalog infrastructure file", source))?;
    #[cfg(test)]
    if slot == InfrastructureSlotV1::CatalogAnchorB {
        crate::checked_artifact::fault_v1::hit(
            crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapAnchorScratchFlush,
        );
    }
    let identity = encode_identity(&super::HostPlatform.file_identity(&file)?);
    let written = ObservedFileV1 {
        identity: &identity,
        bytes,
    };
    verify_open_file(&mut file, written, "catalog infrastructure write")?;
    drop(file);
    verify_named_file(directory, name, written, "catalog infrastructure write")
}

fn exercise_catalog_anchor(
    directory: &cap_std::fs::Dir,
    interior: &RawCatalogInteriorObservationV1,
) -> Result<(), CheckedFsError> {
    let a = OsStr::new(InfrastructureSlotV1::CatalogAnchorA.name());
    let b = OsStr::new(InfrastructureSlotV1::CatalogAnchorB.name());
    let observed = row(interior, InfrastructureSlotV1::CatalogAnchorA)
        .or_else(|| row(interior, InfrastructureSlotV1::CatalogAnchorB))
        .ok_or_else(|| CheckedFsError::ambiguous("catalog anchor", "anchor is missing"))?;
    let RawCatalogInteriorFactV1::RegularFile {
        identity,
        bytes: RawCatalogBytesV1::Bounded(bytes),
        ..
    } = observed
    else {
        return Err(CheckedFsError::ambiguous(
            "catalog anchor",
            "anchor is not exact",
        ));
    };
    let expected = ObservedFileV1 { identity, bytes };
    if row(interior, InfrastructureSlotV1::CatalogAnchorB).is_some() {
        publish_verified_no_replace(
            directory,
            b,
            directory,
            a,
            PublicationSourceV1::regular_file(expected.identity, expected.bytes),
            DestinationRecheckV1::None,
            "publish catalog anchor A",
        )?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapAnchorPublish,
        );
        verify_named_file(directory, a, expected, "catalog anchor A")?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapAnchorReobserve,
        );
        #[cfg(test)]
        run_fault(CatalogDirectoryMutationFaultV1::AnchorAfterPublishA);
    }
    publish_verified_no_replace(
        directory,
        a,
        directory,
        b,
        PublicationSourceV1::regular_file(expected.identity, expected.bytes),
        DestinationRecheckV1::None,
        "exercise catalog anchor B",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapAnchorHomeAExercise,
    );
    verify_named_file(directory, b, expected, "catalog anchor B")?;
    #[cfg(test)]
    run_fault(CatalogDirectoryMutationFaultV1::AnchorAfterMoveToB);
    publish_verified_no_replace(
        directory,
        b,
        directory,
        a,
        PublicationSourceV1::regular_file(expected.identity, expected.bytes),
        DestinationRecheckV1::None,
        "return catalog anchor A",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1::CatalogBootstrapAnchorHomeBExercise,
    );
    verify_named_file(directory, a, expected, "catalog anchor A")?;
    #[cfg(test)]
    run_fault(CatalogDirectoryMutationFaultV1::AnchorAfterReturnA);
    Ok(())
}

pub(super) fn verify_named_file(
    parent: &cap_std::fs::Dir,
    name: &OsStr,
    expected: ObservedFileV1<'_>,
    fact: &'static str,
) -> Result<(), CheckedFsError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = parent
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("reopen named catalog file", source))?;
    verify_open_file(&mut file, expected, fact)
}

pub(super) fn verify_open_file(
    file: &mut cap_std::fs::File,
    expected: ObservedFileV1<'_>,
    fact: &'static str,
) -> Result<(), CheckedFsError> {
    if encode_identity(&super::HostPlatform.file_identity(file)?) != expected.identity {
        return Err(CheckedFsError::ambiguous(
            fact,
            "opened file identity does not match the fresh aggregate",
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind catalog file", source))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(expected.bytes.len()).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "catalog file verification allocation failed",
        )
    })?;
    file.take((expected.bytes.len() + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read catalog file", source))?;
    if bytes != expected.bytes {
        return Err(CheckedFsError::ambiguous(
            fact,
            "opened file bytes do not match the fresh aggregate",
        ));
    }
    Ok(())
}

pub(super) fn durable_write_options(create_new: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(create_new)
        .truncate(false)
        .follow(FollowSymlinks::No);
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_WRITE_THROUGH;
        options.custom_flags(FILE_FLAG_WRITE_THROUGH);
    }
    options
}

fn private_name(name: CatalogPrivateNameV1) -> &'static str {
    std::str::from_utf8(name.leaf_bytes()).expect("fixed catalog names are ASCII")
}

#[cfg(not(windows))]
pub(super) fn sync_directory_edge(
    directory: &cap_std::fs::Dir,
    operation: &'static str,
) -> Result<(), CheckedFsError> {
    crate::checked_artifact::platform::sync_parent(directory)
        .map_err(|source| CheckedFsError::io(operation, source))
}

#[cfg(windows)]
pub(super) fn sync_directory_edge(
    _directory: &cap_std::fs::Dir,
    _operation: &'static str,
) -> Result<(), CheckedFsError> {
    // Every authority-carrying file uses write-through plus FlushFileBuffers,
    // and every publication uses a write-through handle. Empty directory
    // prefixes remain restart-idempotent and carry no independent authority.
    Ok(())
}
