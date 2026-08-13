//! Durable first-catalog bootstrap record and its fixed namespace bindings.

use sha2::{Digest, Sha256};
use std::io::Read;

use super::codec::{
    BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1, decode_identity,
    decode_path, read_bounded_record_inner,
};
use super::generated;
use super::schedule::checked_array;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, DurableObjectIdentityV1, PreCatalogPermitV1,
    PreCatalogRootKindV1, SupportedFilesystemProfile,
};

const STAGING_NAME: &[u8] = b"checked-artifacts-catalog-bootstrap-v1.staging";
const FINAL_NAME: &[u8] = b"checked-artifacts";
const ANCHOR_A_NAME: &[u8] = b"catalog-anchor-a-v1";
const ANCHOR_B_NAME: &[u8] = b"catalog-anchor-b-v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogBootstrapRecordV1 {
    root_kind: PreCatalogRootKindV1,
    support_profile: SupportedFilesystemProfile,
    invocation_identity: Vec<u8>,
    rename_domain: Vec<u8>,
    lease_binding: [u8; 32],
    collision_domain_digest: [u8; 32],
    retained_parent_identity: DurableObjectIdentityV1,
    retained_parent_path: CanonicalPathIdentityV1,
    staging_name: AsciiComponent,
    final_name: AsciiComponent,
    catalog_anchor_a_name: AsciiComponent,
    catalog_anchor_b_name: AsciiComponent,
    record_id: [u8; 32],
}

impl CatalogBootstrapRecordV1 {
    pub(in crate::checked_artifact) fn from_permit<RetainedRoot>(
        permit: &PreCatalogPermitV1<RetainedRoot>,
    ) -> Self {
        Self::from_fields(
            permit.root_kind(),
            permit.support_profile(),
            permit.root_invocation_identity().to_vec(),
            permit.rename_domain().to_vec(),
            permit.lease_binding(),
            permit.collision_domain_digest(),
            permit.root_identity().clone(),
            permit.path_profile().clone(),
            AsciiComponent::parse(STAGING_NAME).expect("fixed staging name is valid ASCII"),
            AsciiComponent::parse(FINAL_NAME).expect("fixed final name is valid ASCII"),
            AsciiComponent::parse(ANCHOR_A_NAME).expect("fixed anchor name is valid ASCII"),
            AsciiComponent::parse(ANCHOR_B_NAME).expect("fixed anchor name is valid ASCII"),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_fields(
        root_kind: PreCatalogRootKindV1,
        support_profile: SupportedFilesystemProfile,
        invocation_identity: Vec<u8>,
        rename_domain: Vec<u8>,
        lease_binding: [u8; 32],
        collision_domain_digest: [u8; 32],
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_path: CanonicalPathIdentityV1,
        staging_name: AsciiComponent,
        final_name: AsciiComponent,
        catalog_anchor_a_name: AsciiComponent,
        catalog_anchor_b_name: AsciiComponent,
    ) -> Self {
        let mut value = Self {
            root_kind,
            support_profile,
            invocation_identity,
            rename_domain,
            lease_binding,
            collision_domain_digest,
            retained_parent_identity,
            retained_parent_path,
            staging_name,
            final_name,
            catalog_anchor_a_name,
            catalog_anchor_b_name,
            record_id: [0; 32],
        };
        value.record_id = Sha256::digest(value.digest_material()).into();
        value
    }

    pub(in crate::checked_artifact) const fn record_id(&self) -> [u8; 32] {
        self.record_id
    }

    pub(super) const fn support_profile(&self) -> SupportedFilesystemProfile {
        self.support_profile
    }

    pub(in crate::checked_artifact) fn staging_name(&self) -> &AsciiComponent {
        &self.staging_name
    }

    pub(in crate::checked_artifact) fn final_name(&self) -> &AsciiComponent {
        &self.final_name
    }

    pub(in crate::checked_artifact) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid catalog bootstrap encoding"))?;
        let wire = generated::CheckedCatalogBootstrapV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid catalog bootstrap record shape"))?;
        let stored_id = checked_array(wire.record_id)?;
        let value = Self::from_fields(
            match wire.root_kind {
                generated::CheckedCatalogRootKind::Workspace => PreCatalogRootKindV1::Workspace,
                generated::CheckedCatalogRootKind::GitDirectory => {
                    PreCatalogRootKindV1::GitDirectory
                }
            },
            decode_profile(wire.support_profile),
            wire.invocation_identity,
            wire.rename_domain,
            checked_array(wire.lease_binding)?,
            checked_array(wire.collision_domain_digest)?,
            decode_identity(wire.retained_parent_identity)?,
            decode_path(wire.retained_parent_path)?,
            super::codec::decode_ascii(&wire.staging_name)?,
            super::codec::decode_ascii(&wire.final_name)?,
            super::codec::decode_ascii(&wire.catalog_anchor_a_name)?,
            super::codec::decode_ascii(&wire.catalog_anchor_b_name)?,
        );
        if value.record_id != stored_id
            || value.retained_parent_identity.support_profile() != value.support_profile
            || value.invocation_identity.is_empty()
            || value.rename_domain.is_empty()
            || value.staging_name.as_bytes() != STAGING_NAME
            || value.final_name.as_bytes() != FINAL_NAME
            || value.catalog_anchor_a_name.as_bytes() != ANCHOR_A_NAME
            || value.catalog_anchor_b_name.as_bytes() != ANCHOR_B_NAME
            || value.encode_canonical() != bytes
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "catalog bootstrap binding mismatch",
            ));
        }
        Ok(value)
    }

    fn digest_material(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated_with_id(Vec::new()).to_cbor())
    }

    fn to_generated(&self) -> generated::CheckedCatalogBootstrapV1 {
        self.to_generated_with_id(self.record_id.to_vec())
    }

    fn to_generated_with_id(&self, record_id: Vec<u8>) -> generated::CheckedCatalogBootstrapV1 {
        generated::CheckedCatalogBootstrapV1 {
            root_kind: match self.root_kind {
                PreCatalogRootKindV1::Workspace => generated::CheckedCatalogRootKind::Workspace,
                PreCatalogRootKindV1::GitDirectory => {
                    generated::CheckedCatalogRootKind::GitDirectory
                }
            },
            support_profile: encode_profile(self.support_profile),
            invocation_identity: self.invocation_identity.clone(),
            rename_domain: self.rename_domain.clone(),
            lease_binding: self.lease_binding.to_vec(),
            collision_domain_digest: self.collision_domain_digest.to_vec(),
            retained_parent_identity: self.retained_parent_identity.to_generated(),
            retained_parent_path: self.retained_parent_path.to_generated(),
            staging_name: self.staging_name.as_bytes().to_vec(),
            final_name: self.final_name.as_bytes().to_vec(),
            catalog_anchor_a_name: self.catalog_anchor_a_name.as_bytes().to_vec(),
            catalog_anchor_b_name: self.catalog_anchor_b_name.as_bytes().to_vec(),
            record_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogRecordObservationV1 {
    Missing,
    PartialExpectedPrefix,
    Exact(Box<CatalogBootstrapRecordV1>),
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogDirectoryObservationV1 {
    Missing,
    PartialExpectedContents,
    Exact(Box<super::InfrastructureRecordV1>),
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogBootstrapRecoveryDecisionV1 {
    WriteOrRewriteScratch,
    PublishActive,
    PrepareOrRewriteStaging,
    PublishFinal,
    RetireActive,
    Complete,
    Ambiguous,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::checked_artifact) fn classify_catalog_bootstrap_recovery(
    expected_record: &CatalogBootstrapRecordV1,
    expected_infrastructure: &super::InfrastructureRecordV1,
    scratch: &CatalogRecordObservationV1,
    active: &CatalogRecordObservationV1,
    staging: &CatalogDirectoryObservationV1,
    final_directory: &CatalogDirectoryObservationV1,
    retired: &CatalogRecordObservationV1,
) -> CatalogBootstrapRecoveryDecisionV1 {
    use CatalogBootstrapRecoveryDecisionV1::*;
    use CatalogDirectoryObservationV1 as Directory;
    use CatalogRecordObservationV1 as Record;

    match (scratch, active, staging, final_directory, retired) {
        (
            Record::Missing | Record::PartialExpectedPrefix,
            Record::Missing,
            Directory::Missing,
            Directory::Missing,
            Record::Missing,
        ) => WriteOrRewriteScratch,
        (
            Record::Exact(value),
            Record::Missing,
            Directory::Missing,
            Directory::Missing,
            Record::Missing,
        ) if value.as_ref() == expected_record => PublishActive,
        (
            Record::Missing,
            Record::Exact(value),
            Directory::Missing | Directory::PartialExpectedContents,
            Directory::Missing,
            Record::Missing,
        ) if value.as_ref() == expected_record => PrepareOrRewriteStaging,
        (
            Record::Missing,
            Record::Exact(active_value),
            Directory::Exact(staging_value),
            Directory::Missing,
            Record::Missing,
        ) if active_value.as_ref() == expected_record
            && staging_value.as_ref() == expected_infrastructure =>
        {
            PublishFinal
        }
        (
            Record::Missing,
            Record::Exact(active_value),
            Directory::Missing,
            Directory::Exact(final_value),
            Record::Missing,
        ) if active_value.as_ref() == expected_record
            && final_value.as_ref() == expected_infrastructure =>
        {
            RetireActive
        }
        (
            Record::Missing,
            Record::Missing,
            Directory::Missing,
            Directory::Exact(final_value),
            Record::Exact(retired_value),
        ) if final_value.as_ref() == expected_infrastructure
            && retired_value.as_ref() == expected_record =>
        {
            Complete
        }
        _ => Ambiguous,
    }
}

fn encode_profile(value: SupportedFilesystemProfile) -> generated::CheckedFilesystemProfile {
    match value {
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1 => {
            generated::CheckedFilesystemProfile::LinuxExt4FsIocGetfsuuidV1
        }
        SupportedFilesystemProfile::MacPersistentObjectIdV1 => {
            generated::CheckedFilesystemProfile::MacPersistentObjectIdV1
        }
        SupportedFilesystemProfile::WindowsNtfsFileId128V1 => {
            generated::CheckedFilesystemProfile::WindowsNtfsFileId128V1
        }
    }
}

fn decode_profile(value: generated::CheckedFilesystemProfile) -> SupportedFilesystemProfile {
    match value {
        generated::CheckedFilesystemProfile::LinuxExt4FsIocGetfsuuidV1 => {
            SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1
        }
        generated::CheckedFilesystemProfile::MacPersistentObjectIdV1 => {
            SupportedFilesystemProfile::MacPersistentObjectIdV1
        }
        generated::CheckedFilesystemProfile::WindowsNtfsFileId128V1 => {
            SupportedFilesystemProfile::WindowsNtfsFileId128V1
        }
    }
}

impl BoundedCanonicalRecordV1 for CatalogBootstrapRecordV1 {
    const KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::CatalogBootstrap;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        super::codec::encode_bounded_record(Self::KIND, self.encode_canonical())
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }
}

pub(in crate::checked_artifact) struct BoundCatalogBootstrapRecordV1(CatalogBootstrapRecordV1);

impl BoundCatalogBootstrapRecordV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &CatalogBootstrapRecordV1 {
        &self.0
    }
}

pub(in crate::checked_artifact) fn read_and_match_catalog_bootstrap_record(
    reader: impl Read,
    expected: &CatalogBootstrapRecordV1,
) -> Result<BoundCatalogBootstrapRecordV1, ProtocolCodecErrorV1> {
    let value = read_bounded_record_inner::<CatalogBootstrapRecordV1>(reader)?;
    if value != *expected {
        return Err(ProtocolCodecErrorV1::Invalid(
            "catalog bootstrap does not match retained permit",
        ));
    }
    Ok(BoundCatalogBootstrapRecordV1(value))
}
