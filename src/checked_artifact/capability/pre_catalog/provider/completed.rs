//! Retained exact-completion capability returned by the first-catalog owner.

use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;

use super::directory_mutation::{
    ObservedDirectoryV1, ObservedFileV1, observed_directory, open_observed_directory, row,
    verify_named_file,
};
use super::interior;
use super::{
    RawCatalogBytesV1, RawCatalogInteriorFactV1, RawCatalogRoleObservationV1, RetainedPlatformRoot,
};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, ObjectIdentityFact, PlatformCapability,
};
use crate::checked_artifact::catalog::{CatalogRecognizedNameV1, CatalogRecordFactV1};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{CatalogBootstrapRecordV1, InfrastructureSlotV1};

type IdentityV1 =
    ObjectIdentityFact<crate::checked_artifact::capability::DurableObjectIdentityV1, Vec<u8>>;

struct RetainedCatalogFileV1 {
    handle: cap_std::fs::File,
    identity: IdentityV1,
}

struct RetainedCatalogDirectoryV1 {
    handle: cap_std::fs::Dir,
    identity: IdentityV1,
}

/// Exact final catalog plus every fixed authority-bearing interior handle.
pub(in crate::checked_artifact::capability::pre_catalog) struct RetainedCompletedCatalogV1 {
    final_directory: RetainedCatalogDirectoryV1,
    catalog_format: RetainedCatalogFileV1,
    catalog_anchor: RetainedCatalogFileV1,
    roaming_anchor: RetainedCatalogFileV1,
    retired_actions: RetainedCatalogDirectoryV1,
    retired_descriptor: RetainedCatalogFileV1,
    retired_bootstrap: RetainedCatalogFileV1,
    expected_bootstrap: CatalogBootstrapRecordV1,
}

pub(in crate::checked_artifact::capability::pre_catalog) fn retain_completed_catalog(
    retained: &RetainedPlatformRoot,
    raw_roles: &RawCatalogRoleObservationV1,
    expected: &CatalogBootstrapRecordV1,
) -> Result<RetainedCompletedCatalogV1, CheckedFsError> {
    let parent = retained.private_parent().ok_or_else(|| {
        CheckedFsError::ambiguous("completed catalog", "retained private parent is missing")
    })?;
    let observed =
        observed_directory(raw_roles, CatalogRecognizedNameV1::Final)?.ok_or_else(|| {
            CheckedFsError::ambiguous("completed catalog", "final catalog is missing")
        })?;
    if interior::completed_record(observed.durable_identity, observed.interior, expected).is_none()
        || !matches!(
            interior::retired_record(observed.interior),
            CatalogRecordFactV1::Exact(value) if value.as_ref() == expected
        )
    {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "final catalog does not have the exact retired layout",
        ));
    }
    let final_directory = open_observed_directory(
        parent.handle(),
        OsStr::new(private_name(CatalogPrivateNameV1::Final)),
        ObservedDirectoryV1 {
            identity: observed.identity,
            durable_identity: observed.durable_identity,
            interior: observed.interior,
        },
        "completed catalog",
    )?;
    let final_identity = super::HostPlatform.dir_identity(&final_directory)?;
    #[cfg(test)]
    super::directory_mutation::run_fault(
        super::directory_mutation::CatalogDirectoryMutationFaultV1::CompleteAfterFinalOpen,
    );
    let catalog_format = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::CatalogFormat,
    )?;
    let catalog_anchor = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::CatalogAnchorA,
    )?;
    let roaming_anchor = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::RoamingAnchorHome,
    )?;
    let retired_actions = retain_directory(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::RetiredActions,
    )?;
    let retired_descriptor = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::RetiredActionsDescriptor,
    )?;
    let retired_bootstrap = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::CatalogBootstrapRetired,
    )?;
    let completed = RetainedCompletedCatalogV1 {
        final_directory: RetainedCatalogDirectoryV1 {
            handle: final_directory,
            identity: final_identity,
        },
        catalog_format,
        catalog_anchor,
        roaming_anchor,
        retired_actions,
        retired_descriptor,
        retired_bootstrap,
        expected_bootstrap: expected.clone(),
    };
    completed.revalidate(retained)?;
    Ok(completed)
}

impl RetainedCompletedCatalogV1 {
    pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate(
        &self,
        retained: &RetainedPlatformRoot,
    ) -> Result<(), CheckedFsError> {
        let parent = retained.private_parent().ok_or_else(|| {
            CheckedFsError::ambiguous("completed catalog", "retained private parent is missing")
        })?;
        let named = parent
            .handle()
            .open_dir_nofollow(OsStr::new(private_name(CatalogPrivateNameV1::Final)))
            .map_err(|source| CheckedFsError::io("reopen named completed catalog", source))?;
        let named_identity = super::HostPlatform.dir_identity(&named)?;
        if named_identity != self.final_directory.identity
            || super::HostPlatform.dir_identity(&self.final_directory.handle)?
                != self.final_directory.identity
        {
            return Err(CheckedFsError::ambiguous(
                "completed catalog",
                "retained final directory is no longer the named catalog",
            ));
        }
        for file in [
            &self.catalog_format,
            &self.catalog_anchor,
            &self.roaming_anchor,
            &self.retired_descriptor,
            &self.retired_bootstrap,
        ] {
            if super::HostPlatform.file_identity(&file.handle)? != file.identity {
                return Err(CheckedFsError::ambiguous(
                    "completed catalog",
                    "retained catalog file identity changed",
                ));
            }
        }
        if super::HostPlatform.dir_identity(&self.retired_actions.handle)?
            != self.retired_actions.identity
        {
            return Err(CheckedFsError::ambiguous(
                "completed catalog",
                "retained retired-action directory identity changed",
            ));
        }
        let observed = interior::observe(&self.final_directory.handle, &super::HostPlatform)?;
        for (slot, file) in [
            (InfrastructureSlotV1::CatalogFormat, &self.catalog_format),
            (InfrastructureSlotV1::CatalogAnchorA, &self.catalog_anchor),
            (
                InfrastructureSlotV1::RoamingAnchorHome,
                &self.roaming_anchor,
            ),
            (
                InfrastructureSlotV1::RetiredActionsDescriptor,
                &self.retired_descriptor,
            ),
            (
                InfrastructureSlotV1::CatalogBootstrapRetired,
                &self.retired_bootstrap,
            ),
        ] {
            require_named_file_identity(&observed, slot, &file.identity)?;
        }
        require_named_directory_identity(
            &observed,
            InfrastructureSlotV1::RetiredActions,
            &self.retired_actions.identity,
        )?;
        if interior::completed_record(
            self.final_directory.identity.durable(),
            &observed,
            &self.expected_bootstrap,
        )
        .is_none()
            || !matches!(
                interior::retired_record(&observed),
                CatalogRecordFactV1::Exact(value) if value.as_ref() == &self.expected_bootstrap
            )
        {
            return Err(CheckedFsError::ambiguous(
                "completed catalog",
                "fresh retained-catalog observation is not exact",
            ));
        }
        Ok(())
    }
}

fn retain_file(
    directory: &cap_std::fs::Dir,
    interior: &super::RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
) -> Result<RetainedCatalogFileV1, CheckedFsError> {
    let Some(RawCatalogInteriorFactV1::RegularFile {
        identity,
        bytes: RawCatalogBytesV1::Bounded(bytes),
        ..
    }) = row(interior, slot)
    else {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "required retained catalog file is missing or invalid",
        ));
    };
    let expected = ObservedFileV1 { identity, bytes };
    let name = OsStr::new(slot.name());
    verify_named_file(directory, name, expected, "completed catalog")?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut handle = directory
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("retain completed catalog file", source))?;
    verify_open_bytes(&mut handle, expected)?;
    let identity = super::HostPlatform.file_identity(&handle)?;
    if super::retained::encode_identity(&identity) != *expected.identity {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog file is not the freshly observed named object",
        ));
    }
    Ok(RetainedCatalogFileV1 { handle, identity })
}

fn retain_directory(
    directory: &cap_std::fs::Dir,
    interior: &super::RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
) -> Result<RetainedCatalogDirectoryV1, CheckedFsError> {
    let Some(RawCatalogInteriorFactV1::EmptyDirectory { identity, .. }) = row(interior, slot)
    else {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "required retained catalog directory is missing or invalid",
        ));
    };
    let handle = directory
        .open_dir_nofollow(OsStr::new(slot.name()))
        .map_err(|source| CheckedFsError::io("retain completed catalog directory", source))?;
    let opened = super::HostPlatform.dir_identity(&handle)?;
    if super::retained::encode_identity(&opened) != *identity {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog directory identity changed",
        ));
    }
    Ok(RetainedCatalogDirectoryV1 {
        handle,
        identity: opened,
    })
}

fn verify_open_bytes(
    file: &mut cap_std::fs::File,
    expected: ObservedFileV1<'_>,
) -> Result<(), CheckedFsError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind retained catalog file", source))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(expected.bytes.len()).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "retained catalog verification allocation failed",
        )
    })?;
    file.take((expected.bytes.len() + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read retained catalog file", source))?;
    if bytes != expected.bytes {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog file bytes changed",
        ));
    }
    Ok(())
}

fn require_named_file_identity(
    interior: &super::RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    retained: &IdentityV1,
) -> Result<(), CheckedFsError> {
    let Some(RawCatalogInteriorFactV1::RegularFile { identity, .. }) = row(interior, slot) else {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog file is no longer present at its named slot",
        ));
    };
    if *identity != super::retained::encode_identity(retained) {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog file is no longer the named slot object",
        ));
    }
    Ok(())
}

fn require_named_directory_identity(
    interior: &super::RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    retained: &IdentityV1,
) -> Result<(), CheckedFsError> {
    let Some(RawCatalogInteriorFactV1::EmptyDirectory { identity, .. }) = row(interior, slot)
    else {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog directory is no longer present at its named slot",
        ));
    };
    if *identity != super::retained::encode_identity(retained) {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog directory is no longer the named slot object",
        ));
    }
    Ok(())
}

fn private_name(name: CatalogPrivateNameV1) -> &'static str {
    std::str::from_utf8(name.leaf_bytes()).expect("fixed catalog names are ASCII")
}
