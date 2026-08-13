//! Immutable ownership marker for one staged managed-parent component.

use sha2::{Digest, Sha256};
use std::io::Read;

use super::codec::{
    BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1, decode_ascii,
    read_bounded_record_inner,
};
use super::generated;
use super::managed_bootstrap_record::{
    ManagedBootstrapComponentRecordV1, ManagedBootstrapPhaseV1, ManagedParentBootstrapIntentV1,
};
use super::schedule::{
    ActionDigestV1, BootstrapComponentOrdinalV1, BootstrapOrdinalV1, RequestOwnerBindingV1,
    ScheduleDigestV1, checked_array, checked_usize,
};
use crate::checked_artifact::capability::AsciiComponent;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct OwnershipMarkerV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    schedule_digest: ScheduleDigestV1,
    intent_id: [u8; 32],
    bootstrap_ordinal: BootstrapOrdinalV1,
    local_component_ordinal: BootstrapComponentOrdinalV1,
    global_component_ordinal: BootstrapComponentOrdinalV1,
    component_ascii: AsciiComponent,
    staging_name: AsciiComponent,
    final_name: AsciiComponent,
    ownership_token: [u8; 32],
    marker_id: [u8; 32],
}

impl OwnershipMarkerV1 {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn derived_id_for_component(
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        schedule_digest: ScheduleDigestV1,
        intent_id: [u8; 32],
        bootstrap_ordinal: BootstrapOrdinalV1,
        local_component_ordinal: usize,
        component: &ManagedBootstrapComponentRecordV1,
        ownership_token: [u8; 32],
    ) -> Result<[u8; 32], ProtocolCodecErrorV1> {
        Ok(Self::from_fields(
            action_digest,
            request_owner_binding,
            schedule_digest,
            intent_id,
            bootstrap_ordinal,
            BootstrapComponentOrdinalV1::new(local_component_ordinal)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid local component ordinal"))?,
            component.global_component_ordinal(),
            component.final_name().clone(),
            component.staging_name().clone(),
            component.final_name().clone(),
            ownership_token,
        )?
        .marker_id())
    }

    pub(in crate::checked_artifact) fn for_current_component(
        intent: &ManagedParentBootstrapIntentV1,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if intent.phase() != ManagedBootstrapPhaseV1::InstallComponents
            || intent.cursor() >= intent.components().len()
            || intent.components()[intent.cursor()]
                .ownership_marker_id()
                .is_some()
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "intent has no unmarked current component",
            ));
        }
        let component = &intent.components()[intent.cursor()];
        Self::from_fields(
            intent.action_digest(),
            intent.request_owner_binding(),
            intent.schedule_digest(),
            intent.intent_id(),
            intent.bootstrap_ordinal(),
            BootstrapComponentOrdinalV1::new(intent.cursor())
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid local component ordinal"))?,
            component.global_component_ordinal(),
            component.final_name().clone(),
            component.staging_name().clone(),
            component.final_name().clone(),
            intent.ownership_token(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_fields(
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        schedule_digest: ScheduleDigestV1,
        intent_id: [u8; 32],
        bootstrap_ordinal: BootstrapOrdinalV1,
        local_component_ordinal: BootstrapComponentOrdinalV1,
        global_component_ordinal: BootstrapComponentOrdinalV1,
        component_ascii: AsciiComponent,
        staging_name: AsciiComponent,
        final_name: AsciiComponent,
        ownership_token: [u8; 32],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if ownership_token == [0; 32] || component_ascii != final_name {
            return Err(ProtocolCodecErrorV1::Invalid(
                "invalid ownership marker binding",
            ));
        }
        let mut value = Self {
            action_digest,
            request_owner_binding,
            schedule_digest,
            intent_id,
            bootstrap_ordinal,
            local_component_ordinal,
            global_component_ordinal,
            component_ascii,
            staging_name,
            final_name,
            ownership_token,
            marker_id: [0; 32],
        };
        value.marker_id = Sha256::digest(value.digest_material()).into();
        Ok(value)
    }

    pub(in crate::checked_artifact) const fn marker_id(&self) -> [u8; 32] {
        self.marker_id
    }

    pub(super) fn matches_component(
        &self,
        intent: &ManagedParentBootstrapIntentV1,
        local: usize,
    ) -> bool {
        self.intent_id == intent.intent_id() && self.matches_static_component(intent, local)
    }

    pub(super) fn matches_static_component(
        &self,
        intent: &ManagedParentBootstrapIntentV1,
        local: usize,
    ) -> bool {
        let Some(component) = intent.components().get(local) else {
            return false;
        };
        self.action_digest == intent.action_digest()
            && self.request_owner_binding == intent.request_owner_binding()
            && self.schedule_digest == intent.schedule_digest()
            && self.bootstrap_ordinal == intent.bootstrap_ordinal()
            && self.local_component_ordinal.index() == local
            && self.global_component_ordinal == component.global_component_ordinal()
            && self.component_ascii == *component.final_name()
            && self.staging_name == *component.staging_name()
            && self.final_name == *component.final_name()
            && self.ownership_token == intent.ownership_token()
    }

    pub(in crate::checked_artifact) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid ownership marker encoding"))?;
        let wire = generated::CheckedOwnershipMarkerV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid ownership marker shape"))?;
        let stored_id = checked_array(wire.marker_id)?;
        let value = Self::from_fields(
            ActionDigestV1::new(checked_array(wire.action_digest)?),
            RequestOwnerBindingV1::new(checked_array(wire.request_owner_binding)?),
            ScheduleDigestV1::new(checked_array(wire.schedule_digest)?),
            checked_array(wire.intent_id)?,
            BootstrapOrdinalV1::new(checked_usize(wire.bootstrap_ordinal)?)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid bootstrap ordinal"))?,
            BootstrapComponentOrdinalV1::new(checked_usize(wire.local_component_ordinal)?)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid local component ordinal"))?,
            BootstrapComponentOrdinalV1::new(checked_usize(wire.global_component_ordinal)?)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid global component ordinal"))?,
            decode_ascii(&wire.component_ascii)?,
            decode_ascii(&wire.staging_name)?,
            decode_ascii(&wire.final_name)?,
            checked_array(wire.ownership_token)?,
        )?;
        if value.marker_id != stored_id || value.encode_canonical() != bytes {
            return Err(ProtocolCodecErrorV1::Invalid(
                "ownership marker binding mismatch",
            ));
        }
        Ok(value)
    }

    fn digest_material(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated_with_id(Vec::new()).to_cbor())
    }

    fn to_generated(&self) -> generated::CheckedOwnershipMarkerV1 {
        self.to_generated_with_id(self.marker_id.to_vec())
    }

    fn to_generated_with_id(&self, marker_id: Vec<u8>) -> generated::CheckedOwnershipMarkerV1 {
        generated::CheckedOwnershipMarkerV1 {
            action_digest: self.action_digest.bytes().to_vec(),
            request_owner_binding: self.request_owner_binding.bytes().to_vec(),
            schedule_digest: self.schedule_digest.bytes().to_vec(),
            intent_id: self.intent_id.to_vec(),
            bootstrap_ordinal: self.bootstrap_ordinal.index() as i64,
            local_component_ordinal: self.local_component_ordinal.index() as i64,
            global_component_ordinal: self.global_component_ordinal.index() as i64,
            component_ascii: self.component_ascii.as_bytes().to_vec(),
            staging_name: self.staging_name.as_bytes().to_vec(),
            final_name: self.final_name.as_bytes().to_vec(),
            ownership_token: self.ownership_token.to_vec(),
            marker_id,
        }
    }
}

impl BoundedCanonicalRecordV1 for OwnershipMarkerV1 {
    const KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::Marker;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        super::codec::encode_bounded_record(Self::KIND, self.encode_canonical())
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }
}

pub(in crate::checked_artifact) struct BoundOwnershipMarkerV1(OwnershipMarkerV1);

impl BoundOwnershipMarkerV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &OwnershipMarkerV1 {
        &self.0
    }
}

pub(in crate::checked_artifact) fn read_and_bind_ownership_marker(
    reader: impl Read,
    intent: &ManagedParentBootstrapIntentV1,
    local_component: usize,
) -> Result<BoundOwnershipMarkerV1, ProtocolCodecErrorV1> {
    let value = read_bounded_record_inner::<OwnershipMarkerV1>(reader)?;
    let recorded = intent
        .components()
        .get(local_component)
        .and_then(|component| component.ownership_marker_id());
    let valid = match recorded {
        Some(marker_id) => {
            value.marker_id == marker_id && value.matches_static_component(intent, local_component)
        }
        None => value.matches_component(intent, local_component),
    };
    if !valid {
        return Err(ProtocolCodecErrorV1::Invalid(
            "ownership marker does not match intent component",
        ));
    }
    Ok(BoundOwnershipMarkerV1(value))
}
