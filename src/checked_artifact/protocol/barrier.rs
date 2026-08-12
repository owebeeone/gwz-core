//! Fully bound, canonically encoded namespace-barrier intent.

use sha2::{Digest, Sha256};

use super::codec::{BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1};
use super::generated;
use super::schedule::{ActionDigestV1, BarrierOrdinalV1, RequestOwnerBindingV1, ScheduleDigestV1};
use super::schedule::{checked_array, checked_usize};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, DurableObjectIdentityV1,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct BarrierIntentV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    schedule_digest: ScheduleDigestV1,
    ordinal: BarrierOrdinalV1,
    catalog_anchor_identity: DurableObjectIdentityV1,
    private_home_parent_identity: DurableObjectIdentityV1,
    private_home_name: AsciiComponent,
    target_parent_identity: DurableObjectIdentityV1,
    target_path_profile: CanonicalPathIdentityV1,
    reserved_target_leaf: AsciiComponent,
    intent_id: [u8; 32],
}

impl BarrierIntentV1 {
    #[allow(
        clippy::too_many_arguments,
        reason = "the intent deliberately binds each independent retained namespace fact"
    )]
    pub(in crate::checked_artifact) fn try_new(
        reservation: &super::ActionCapacityReservationV1,
        ordinal: BarrierOrdinalV1,
        catalog_anchor_identity: DurableObjectIdentityV1,
        private_home_parent_identity: DurableObjectIdentityV1,
        private_home_name: AsciiComponent,
        target_parent_identity: DurableObjectIdentityV1,
        target_path_profile: CanonicalPathIdentityV1,
        reserved_target_leaf: AsciiComponent,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if ordinal.index() >= reservation.schedule().barrier_count() {
            return Err(ProtocolCodecErrorV1::Invalid(
                "barrier ordinal is not reserved by the action schedule",
            ));
        }
        Ok(Self::from_bound_fields(
            reservation.action_digest(),
            reservation.request_owner_binding(),
            reservation.schedule().digest(),
            ordinal,
            catalog_anchor_identity,
            private_home_parent_identity,
            private_home_name,
            target_parent_identity,
            target_path_profile,
            reserved_target_leaf,
        ))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "decoder validates every persisted binding explicitly"
    )]
    fn from_bound_fields(
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        schedule_digest: ScheduleDigestV1,
        ordinal: BarrierOrdinalV1,
        catalog_anchor_identity: DurableObjectIdentityV1,
        private_home_parent_identity: DurableObjectIdentityV1,
        private_home_name: AsciiComponent,
        target_parent_identity: DurableObjectIdentityV1,
        target_path_profile: CanonicalPathIdentityV1,
        reserved_target_leaf: AsciiComponent,
    ) -> Self {
        let mut value = Self {
            action_digest,
            request_owner_binding,
            schedule_digest,
            ordinal,
            catalog_anchor_identity,
            private_home_parent_identity,
            private_home_name,
            target_parent_identity,
            target_path_profile,
            reserved_target_leaf,
            intent_id: [0; 32],
        };
        value.intent_id = Sha256::digest(value.digest_material()).into();
        value
    }

    pub(in crate::checked_artifact) const fn intent_id(&self) -> [u8; 32] {
        self.intent_id
    }

    pub(in crate::checked_artifact) fn encode_canonical(
        &self,
    ) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        Ok(crate::cbor::encode(&self.to_generated().to_cbor()))
    }

    pub(in crate::checked_artifact) fn decode_canonical(
        bytes: &[u8],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid barrier taut encoding"))?;
        let wire = generated::CheckedBarrierIntentV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid barrier record shape"))?;
        let action = ActionDigestV1::new(checked_array(wire.action_digest)?);
        let owner = RequestOwnerBindingV1::new(checked_array(wire.request_owner_binding)?);
        let schedule = ScheduleDigestV1::new(checked_array(wire.schedule_digest)?);
        let ordinal = BarrierOrdinalV1::new(checked_usize(wire.ordinal)?)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid barrier ordinal"))?;
        let catalog = decode_identity(wire.catalog_anchor_identity)?;
        let home = decode_identity(wire.private_home_parent_identity)?;
        let home_name = AsciiComponent::parse(&wire.private_home_name)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid private home name"))?;
        let target = decode_identity(wire.target_parent_identity)?;
        let path = CanonicalPathIdentityV1::decode_canonical(&crate::cbor::encode(
            &wire.target_path_profile.to_cbor(),
        ))
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid path identity"))?;
        let target_leaf = AsciiComponent::parse(&wire.reserved_target_leaf)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid reserved target leaf"))?;
        let intent_id = checked_array(wire.intent_id)?;
        let value = Self::from_bound_fields(
            action,
            owner,
            schedule,
            ordinal,
            catalog,
            home,
            home_name,
            target,
            path,
            target_leaf,
        );
        if value.intent_id != intent_id || value.encode_canonical()? != bytes {
            return Err(ProtocolCodecErrorV1::Invalid(
                "barrier intent binding mismatch",
            ));
        }
        Ok(value)
    }

    fn digest_material(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated_with_id(Vec::new()).to_cbor())
    }

    fn to_generated(&self) -> generated::CheckedBarrierIntentV1 {
        self.to_generated_with_id(self.intent_id.to_vec())
    }

    fn to_generated_with_id(&self, intent_id: Vec<u8>) -> generated::CheckedBarrierIntentV1 {
        generated::CheckedBarrierIntentV1 {
            action_digest: self.action_digest.bytes().to_vec(),
            request_owner_binding: self.request_owner_binding.bytes().to_vec(),
            schedule_digest: self.schedule_digest.bytes().to_vec(),
            ordinal: self.ordinal.index() as i64,
            catalog_anchor_identity: self.catalog_anchor_identity.to_generated(),
            private_home_parent_identity: self.private_home_parent_identity.to_generated(),
            private_home_name: self.private_home_name.as_bytes().to_vec(),
            target_parent_identity: self.target_parent_identity.to_generated(),
            target_path_profile: self.target_path_profile.to_generated(),
            reserved_target_leaf: self.reserved_target_leaf.as_bytes().to_vec(),
            intent_id,
        }
    }
}

impl BoundedCanonicalRecordV1 for BarrierIntentV1 {
    const KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::BarrierIntent;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        self.encode_canonical()
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }
}

fn decode_identity(
    value: generated::CheckedDurableObjectIdentityV1,
) -> Result<DurableObjectIdentityV1, ProtocolCodecErrorV1> {
    DurableObjectIdentityV1::decode_canonical(&crate::cbor::encode(&value.to_cbor()))
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid durable identity"))
}
