//! Identity-pinned catalog infrastructure record.

use sha2::{Digest, Sha256};
use std::io::Read;

use super::codec::{
    BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1, decode_ascii,
    decode_identity, read_bounded_record_inner,
};
use super::generated;
use super::schedule::checked_array;
use crate::checked_artifact::capability::{AsciiComponent, DurableObjectIdentityV1};

const CATALOG_FORMAT_V1: i64 = 1;
const ADMISSION_ACTIVE: &[u8] = b"action-admission-active-v1";
const ADMISSION_SCRATCH: &[u8] = b"action-admission-scratch-v1";
const ADMISSION_STAGING: &[u8] = b"action-admission-staging-v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct InfrastructureRecordV1 {
    catalog_root_identity: DurableObjectIdentityV1,
    catalog_anchor_identity: DurableObjectIdentityV1,
    roaming_anchor_identity: DurableObjectIdentityV1,
    retired_root_identity: DurableObjectIdentityV1,
    catalog_bootstrap_record_id: [u8; 32],
    admission_active_name: AsciiComponent,
    admission_scratch_name: AsciiComponent,
    admission_staging_name: AsciiComponent,
    record_digest: [u8; 32],
}

impl InfrastructureRecordV1 {
    pub(in crate::checked_artifact) fn from_catalog_bootstrap(
        catalog_bootstrap: &super::CatalogBootstrapRecordV1,
        catalog_root_identity: DurableObjectIdentityV1,
        catalog_anchor_identity: DurableObjectIdentityV1,
        roaming_anchor_identity: DurableObjectIdentityV1,
        retired_root_identity: DurableObjectIdentityV1,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let value = Self::from_fields(
            catalog_root_identity,
            catalog_anchor_identity,
            roaming_anchor_identity,
            retired_root_identity,
            catalog_bootstrap.record_id(),
            AsciiComponent::parse(ADMISSION_ACTIVE).expect("fixed active name is valid"),
            AsciiComponent::parse(ADMISSION_SCRATCH).expect("fixed scratch name is valid"),
            AsciiComponent::parse(ADMISSION_STAGING).expect("fixed staging name is valid"),
        );
        value.validate_profiles()?;
        if value.catalog_root_identity.support_profile() != catalog_bootstrap.support_profile() {
            return Err(ProtocolCodecErrorV1::Invalid(
                "infrastructure profile does not match catalog bootstrap",
            ));
        }
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    fn from_fields(
        catalog_root_identity: DurableObjectIdentityV1,
        catalog_anchor_identity: DurableObjectIdentityV1,
        roaming_anchor_identity: DurableObjectIdentityV1,
        retired_root_identity: DurableObjectIdentityV1,
        catalog_bootstrap_record_id: [u8; 32],
        admission_active_name: AsciiComponent,
        admission_scratch_name: AsciiComponent,
        admission_staging_name: AsciiComponent,
    ) -> Self {
        let mut value = Self {
            catalog_root_identity,
            catalog_anchor_identity,
            roaming_anchor_identity,
            retired_root_identity,
            catalog_bootstrap_record_id,
            admission_active_name,
            admission_scratch_name,
            admission_staging_name,
            record_digest: [0; 32],
        };
        value.record_digest = Sha256::digest(value.digest_material()).into();
        value
    }

    pub(in crate::checked_artifact) const fn record_digest(&self) -> [u8; 32] {
        self.record_digest
    }

    fn validate_profiles(&self) -> Result<(), ProtocolCodecErrorV1> {
        let profile = self.catalog_root_identity.support_profile();
        if self.catalog_anchor_identity.support_profile() != profile
            || self.roaming_anchor_identity.support_profile() != profile
            || self.retired_root_identity.support_profile() != profile
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "infrastructure durable identities use different support profiles",
            ));
        }
        Ok(())
    }

    pub(in crate::checked_artifact) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid infrastructure encoding"))?;
        let wire = generated::CheckedInfrastructureV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid infrastructure shape"))?;
        if wire.catalog_format != CATALOG_FORMAT_V1 {
            return Err(ProtocolCodecErrorV1::Invalid(
                "unsupported catalog infrastructure format",
            ));
        }
        let stored_digest = checked_array(wire.record_digest)?;
        let value = Self::from_fields(
            decode_identity(wire.catalog_root_identity)?,
            decode_identity(wire.catalog_anchor_identity)?,
            decode_identity(wire.roaming_anchor_identity)?,
            decode_identity(wire.retired_root_identity)?,
            checked_array(wire.catalog_bootstrap_record_id)?,
            decode_ascii(&wire.admission_active_name)?,
            decode_ascii(&wire.admission_scratch_name)?,
            decode_ascii(&wire.admission_staging_name)?,
        );
        value.validate_profiles()?;
        if value.record_digest != stored_digest
            || value.admission_active_name.as_bytes() != ADMISSION_ACTIVE
            || value.admission_scratch_name.as_bytes() != ADMISSION_SCRATCH
            || value.admission_staging_name.as_bytes() != ADMISSION_STAGING
            || value.encode_canonical() != bytes
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "infrastructure record binding mismatch",
            ));
        }
        Ok(value)
    }

    fn digest_material(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated_with_digest(Vec::new()).to_cbor())
    }

    fn to_generated(&self) -> generated::CheckedInfrastructureV1 {
        self.to_generated_with_digest(self.record_digest.to_vec())
    }

    fn to_generated_with_digest(
        &self,
        record_digest: Vec<u8>,
    ) -> generated::CheckedInfrastructureV1 {
        generated::CheckedInfrastructureV1 {
            catalog_format: CATALOG_FORMAT_V1,
            catalog_root_identity: self.catalog_root_identity.to_generated(),
            catalog_anchor_identity: self.catalog_anchor_identity.to_generated(),
            roaming_anchor_identity: self.roaming_anchor_identity.to_generated(),
            retired_root_identity: self.retired_root_identity.to_generated(),
            catalog_bootstrap_record_id: self.catalog_bootstrap_record_id.to_vec(),
            admission_active_name: self.admission_active_name.as_bytes().to_vec(),
            admission_scratch_name: self.admission_scratch_name.as_bytes().to_vec(),
            admission_staging_name: self.admission_staging_name.as_bytes().to_vec(),
            record_digest,
        }
    }
}

impl BoundedCanonicalRecordV1 for InfrastructureRecordV1 {
    const KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::Infrastructure;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        super::codec::encode_bounded_record(Self::KIND, self.encode_canonical())
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }
}

pub(in crate::checked_artifact) struct BoundInfrastructureRecordV1(InfrastructureRecordV1);

impl BoundInfrastructureRecordV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &InfrastructureRecordV1 {
        &self.0
    }
}

pub(in crate::checked_artifact) fn read_and_match_infrastructure_record(
    reader: impl Read,
    expected: &InfrastructureRecordV1,
) -> Result<BoundInfrastructureRecordV1, ProtocolCodecErrorV1> {
    let value = read_bounded_record_inner::<InfrastructureRecordV1>(reader)?;
    if value != *expected {
        return Err(ProtocolCodecErrorV1::Invalid(
            "infrastructure does not match retained catalog identities",
        ));
    }
    Ok(BoundInfrastructureRecordV1(value))
}
