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
    AsciiComponent, DurableCatalogTargetDigestV1, DurableObjectIdentityV1, DurablePathV1,
    HistoricalCollisionDigestV1, PreCatalogRootKindV1, SupportedFilesystemProfile,
};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogBootstrapOwnershipTokenV1([u8; 32]);

impl CatalogBootstrapOwnershipTokenV1 {
    /// Wraps 256 bits produced by the catalog owner's cryptographic random
    /// source. Zero is reserved so an uninitialized token cannot be durable.
    pub(in crate::checked_artifact) fn try_from_random_bytes(
        bytes: [u8; 32],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if bytes == [0; 32] {
            return Err(ProtocolCodecErrorV1::Invalid(
                "catalog bootstrap ownership token is zero",
            ));
        }
        Ok(Self(bytes))
    }

    pub(in crate::checked_artifact) const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogBootstrapRecordV1 {
    root_kind: PreCatalogRootKindV1,
    support_profile: SupportedFilesystemProfile,
    durable_target_digest: DurableCatalogTargetDigestV1,
    historical_collision_digest: HistoricalCollisionDigestV1,
    retained_parent_identity: DurableObjectIdentityV1,
    retained_parent_path: DurablePathV1,
    staging_name: AsciiComponent,
    final_name: AsciiComponent,
    catalog_anchor_a_name: AsciiComponent,
    catalog_anchor_b_name: AsciiComponent,
    bootstrap_ownership_token: CatalogBootstrapOwnershipTokenV1,
    record_id: [u8; 32],
}

impl CatalogBootstrapRecordV1 {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::checked_artifact) fn synthetic_for_test(
        root_kind: PreCatalogRootKindV1,
        support_profile: SupportedFilesystemProfile,
        durable_target_digest: DurableCatalogTargetDigestV1,
        historical_collision_digest: HistoricalCollisionDigestV1,
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_path: DurablePathV1,
        bootstrap_ownership_token: CatalogBootstrapOwnershipTokenV1,
    ) -> Self {
        Self::from_fields(
            root_kind,
            support_profile,
            durable_target_digest,
            historical_collision_digest,
            retained_parent_identity,
            retained_parent_path,
            catalog_component(CatalogPrivateNameV1::BootstrapStaging),
            catalog_component(CatalogPrivateNameV1::Final),
            AsciiComponent::parse(
                super::InfrastructureSlotV1::CatalogAnchorA
                    .name()
                    .as_bytes(),
            )
            .expect("infrastructure slot name is valid ASCII"),
            AsciiComponent::parse(
                super::InfrastructureSlotV1::CatalogAnchorB
                    .name()
                    .as_bytes(),
            )
            .expect("infrastructure slot name is valid ASCII"),
            bootstrap_ownership_token,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_fields(
        root_kind: PreCatalogRootKindV1,
        support_profile: SupportedFilesystemProfile,
        durable_target_digest: DurableCatalogTargetDigestV1,
        historical_collision_digest: HistoricalCollisionDigestV1,
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_path: DurablePathV1,
        staging_name: AsciiComponent,
        final_name: AsciiComponent,
        catalog_anchor_a_name: AsciiComponent,
        catalog_anchor_b_name: AsciiComponent,
        bootstrap_ownership_token: CatalogBootstrapOwnershipTokenV1,
    ) -> Self {
        let mut value = Self {
            root_kind,
            support_profile,
            durable_target_digest,
            historical_collision_digest,
            retained_parent_identity,
            retained_parent_path,
            staging_name,
            final_name,
            catalog_anchor_a_name,
            catalog_anchor_b_name,
            bootstrap_ownership_token,
            record_id: [0; 32],
        };
        value.record_id = Sha256::digest(value.digest_material()).into();
        value
    }

    pub(in crate::checked_artifact) const fn record_id(&self) -> [u8; 32] {
        self.record_id
    }

    pub(in crate::checked_artifact) const fn bootstrap_ownership_token(
        &self,
    ) -> CatalogBootstrapOwnershipTokenV1 {
        self.bootstrap_ownership_token
    }

    pub(super) const fn support_profile(&self) -> SupportedFilesystemProfile {
        self.support_profile
    }

    pub(in crate::checked_artifact) const fn durable_target_digest(
        &self,
    ) -> DurableCatalogTargetDigestV1 {
        self.durable_target_digest
    }

    pub(in crate::checked_artifact) const fn historical_collision_digest(
        &self,
    ) -> HistoricalCollisionDigestV1 {
        self.historical_collision_digest
    }

    pub(in crate::checked_artifact) fn retained_parent_identity(&self) -> &DurableObjectIdentityV1 {
        &self.retained_parent_identity
    }

    pub(in crate::checked_artifact) fn retained_parent_path(&self) -> &DurablePathV1 {
        &self.retained_parent_path
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
            DurableCatalogTargetDigestV1::owner_issue(checked_array(wire.durable_target_digest)?),
            HistoricalCollisionDigestV1::owner_issue(checked_array(
                wire.historical_collision_digest,
            )?),
            decode_identity(wire.retained_parent_identity)?,
            decode_path(wire.retained_parent_path)?,
            super::codec::decode_ascii(&wire.staging_name)?,
            super::codec::decode_ascii(&wire.final_name)?,
            super::codec::decode_ascii(&wire.catalog_anchor_a_name)?,
            super::codec::decode_ascii(&wire.catalog_anchor_b_name)?,
            CatalogBootstrapOwnershipTokenV1::try_from_random_bytes(checked_array(
                wire.bootstrap_ownership_token,
            )?)?,
        );
        if value.record_id != stored_id
            || value.retained_parent_identity.support_profile() != value.support_profile
            || !super::codec::path_matches_profile(
                &value.retained_parent_path,
                value.support_profile,
            )
            || value.staging_name != catalog_component(CatalogPrivateNameV1::BootstrapStaging)
            || value.final_name != catalog_component(CatalogPrivateNameV1::Final)
            || value.catalog_anchor_a_name.as_bytes()
                != super::InfrastructureSlotV1::CatalogAnchorA
                    .name()
                    .as_bytes()
            || value.catalog_anchor_b_name.as_bytes()
                != super::InfrastructureSlotV1::CatalogAnchorB
                    .name()
                    .as_bytes()
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
            durable_target_digest: self.durable_target_digest.bytes().to_vec(),
            historical_collision_digest: self.historical_collision_digest.bytes().to_vec(),
            retained_parent_identity: self.retained_parent_identity.to_generated(),
            retained_parent_path: self.retained_parent_path.to_generated(),
            staging_name: self.staging_name.as_bytes().to_vec(),
            final_name: self.final_name.as_bytes().to_vec(),
            catalog_anchor_a_name: self.catalog_anchor_a_name.as_bytes().to_vec(),
            catalog_anchor_b_name: self.catalog_anchor_b_name.as_bytes().to_vec(),
            record_id,
            bootstrap_ownership_token: self.bootstrap_ownership_token.0.to_vec(),
        }
    }
}

fn catalog_component(name: CatalogPrivateNameV1) -> AsciiComponent {
    AsciiComponent::parse(name.leaf_bytes()).expect("fixed catalog name is valid ASCII")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogRecordObservationV1 {
    Missing,
    PartialExpectedPrefix,
    Exact(Box<CatalogBootstrapRecordV1>),
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
