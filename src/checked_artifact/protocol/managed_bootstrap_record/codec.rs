//! Owner-private Taut codec and bounded reservation-binding entry point.

use std::io::Read;

use super::{
    ManagedBootstrapComponentRecordV1, ManagedBootstrapPhaseV1, ManagedParentBootstrapIntentV1,
};
use crate::checked_artifact::bootstrap::{BoundManagedParentPlanV1, ManagedParentPurpose};
use crate::checked_artifact::capability::PathComponentMode;
#[cfg(test)]
use crate::checked_artifact::protocol::ActionCapacityReservationV1;
use crate::checked_artifact::protocol::codec::{
    BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1, decode_ascii,
    decode_identity, decode_path, read_bounded_record_inner,
};
use crate::checked_artifact::protocol::generated;
use crate::checked_artifact::protocol::schedule::{
    ActionDigestV1, BootstrapComponentOrdinalV1, BootstrapGenerationV1, BootstrapOrdinalV1,
    RecordDigestV1, RequestOwnerBindingV1, ScheduleDigestV1, checked_array, checked_usize,
};

impl ManagedParentBootstrapIntentV1 {
    pub(in crate::checked_artifact) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid bootstrap intent encoding"))?;
        let wire = generated::CheckedManagedParentBootstrapIntentV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid bootstrap intent shape"))?;
        let stored_id = checked_array(wire.intent_id)?;
        let components = wire
            .components
            .into_iter()
            .map(|component| {
                Ok(ManagedBootstrapComponentRecordV1 {
                    component_ascii: decode_ascii(&component.component_ascii)?,
                    staging_name: decode_ascii(&component.staging_name)?,
                    final_name: decode_ascii(&component.final_name)?,
                    marker_name: decode_ascii(&component.marker_name)?,
                    global_component_ordinal: BootstrapComponentOrdinalV1::new(checked_usize(
                        component.global_component_ordinal,
                    )?)
                    .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid component ordinal"))?,
                    ownership_marker_id: component
                        .ownership_marker_id
                        .map(checked_array)
                        .transpose()?,
                    ownership_marker_intent_id: component
                        .ownership_marker_intent_id
                        .map(checked_array)
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, ProtocolCodecErrorV1>>()?;
        let value = Self::from_fields(
            ActionDigestV1::new(checked_array(wire.action_digest)?),
            RequestOwnerBindingV1::new(checked_array(wire.request_owner_binding)?),
            RecordDigestV1::new(checked_array(wire.reservation_digest)?),
            ScheduleDigestV1::new(checked_array(wire.schedule_digest)?),
            checked_array(wire.spec_digest)?,
            decode_purpose(wire.purpose),
            checked_array(wire.managed_plan_digest)?,
            BootstrapOrdinalV1::new(checked_usize(wire.bootstrap_ordinal)?)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid bootstrap ordinal"))?,
            BootstrapGenerationV1::new(checked_usize(wire.generation_ordinal)?)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid generation ordinal"))?,
            checked_usize(wire.generation_start)?,
            checked_usize(wire.component_start)?,
            decode_identity(wire.retained_parent_identity)?,
            decode_mode(wire.retained_parent_mode),
            decode_path(wire.retained_parent_path)?,
            components,
            checked_array(wire.ownership_token)?,
            wire.predecessor_intent_id.map(checked_array).transpose()?,
            decode_phase(wire.phase),
            checked_usize(wire.cursor)?,
        )?;
        if value.intent_id != stored_id || value.encode_canonical() != bytes {
            return Err(ProtocolCodecErrorV1::Invalid(
                "bootstrap intent binding mismatch",
            ));
        }
        Ok(value)
    }

    pub(super) fn digest_material(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated_with_id(Vec::new()).to_cbor())
    }

    fn to_generated(&self) -> generated::CheckedManagedParentBootstrapIntentV1 {
        self.to_generated_with_id(self.intent_id.to_vec())
    }

    fn to_generated_with_id(
        &self,
        intent_id: Vec<u8>,
    ) -> generated::CheckedManagedParentBootstrapIntentV1 {
        generated::CheckedManagedParentBootstrapIntentV1 {
            action_digest: self.action_digest.bytes().to_vec(),
            request_owner_binding: self.request_owner_binding.bytes().to_vec(),
            reservation_digest: self.reservation_digest.bytes().to_vec(),
            schedule_digest: self.schedule_digest.bytes().to_vec(),
            spec_digest: self.spec_digest.to_vec(),
            purpose: encode_purpose(self.purpose),
            managed_plan_digest: self.managed_plan_digest.to_vec(),
            bootstrap_ordinal: self.bootstrap_ordinal.index() as i64,
            generation_ordinal: self.generation_ordinal.index() as i64,
            generation_start: self.generation_start as i64,
            component_start: self.component_start as i64,
            retained_parent_identity: self.retained_parent_identity.to_generated(),
            retained_parent_mode: encode_mode(self.retained_parent_mode),
            retained_parent_path: self.retained_parent_path.to_generated(),
            components: self
                .components
                .iter()
                .map(|component| generated::CheckedManagedBootstrapComponentV1 {
                    component_ascii: component.component_ascii.as_bytes().to_vec(),
                    staging_name: component.staging_name.as_bytes().to_vec(),
                    final_name: component.final_name.as_bytes().to_vec(),
                    marker_name: component.marker_name.as_bytes().to_vec(),
                    global_component_ordinal: component.global_component_ordinal.index() as i64,
                    ownership_marker_id: component.ownership_marker_id.map(|value| value.to_vec()),
                    ownership_marker_intent_id: component
                        .ownership_marker_intent_id
                        .map(|value| value.to_vec()),
                })
                .collect(),
            ownership_token: self.ownership_token.to_vec(),
            predecessor_intent_id: self.predecessor_intent_id.map(|value| value.to_vec()),
            phase: encode_phase(self.phase),
            cursor: self.cursor as i64,
            intent_id,
        }
    }
}

impl BoundedCanonicalRecordV1 for ManagedParentBootstrapIntentV1 {
    const KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::BootstrapIntent;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        crate::checked_artifact::protocol::codec::encode_bounded_record(
            Self::KIND,
            self.encode_canonical(),
        )
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }
}

pub(in crate::checked_artifact) struct BoundManagedParentBootstrapIntentV1(
    ManagedParentBootstrapIntentV1,
);

impl BoundManagedParentBootstrapIntentV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &ManagedParentBootstrapIntentV1 {
        &self.0
    }
}

pub(in crate::checked_artifact) fn read_and_bind_managed_bootstrap_intent(
    reader: impl Read,
    bound_plan: &BoundManagedParentPlanV1,
    purpose: ManagedParentPurpose,
    expected_generation: BootstrapGenerationV1,
    expected_predecessor: Option<[u8; 32]>,
) -> Result<BoundManagedParentBootstrapIntentV1, ProtocolCodecErrorV1> {
    let value = read_bounded_record_inner::<ManagedParentBootstrapIntentV1>(reader)?;
    if !value.matches_bound_plan(bound_plan, purpose)
        || value.generation_ordinal != expected_generation
        || value.predecessor_intent_id != expected_predecessor
    {
        return Err(ProtocolCodecErrorV1::Invalid(
            "bootstrap intent does not match resident schedule or predecessor",
        ));
    }
    Ok(BoundManagedParentBootstrapIntentV1(value))
}

#[cfg(test)]
pub(in crate::checked_artifact) fn read_and_bind_managed_bootstrap_intent_for_test(
    reader: impl Read,
    reservation: &ActionCapacityReservationV1,
    expected_generation: BootstrapGenerationV1,
    expected_predecessor: Option<[u8; 32]>,
) -> Result<BoundManagedParentBootstrapIntentV1, ProtocolCodecErrorV1> {
    let value = read_bounded_record_inner::<ManagedParentBootstrapIntentV1>(reader)?;
    if !value.matches_reservation(reservation)
        || value.generation_ordinal != expected_generation
        || value.predecessor_intent_id != expected_predecessor
    {
        return Err(ProtocolCodecErrorV1::Invalid(
            "bootstrap intent does not match resident schedule or predecessor",
        ));
    }
    Ok(BoundManagedParentBootstrapIntentV1(value))
}

fn encode_mode(value: PathComponentMode) -> generated::CheckedPathComponentMode {
    match value {
        PathComponentMode::Sensitive => generated::CheckedPathComponentMode::Sensitive,
        PathComponentMode::AsciiCaseFold => generated::CheckedPathComponentMode::AsciiCaseFold,
    }
}

fn decode_mode(value: generated::CheckedPathComponentMode) -> PathComponentMode {
    match value {
        generated::CheckedPathComponentMode::Sensitive => PathComponentMode::Sensitive,
        generated::CheckedPathComponentMode::AsciiCaseFold => PathComponentMode::AsciiCaseFold,
    }
}

fn encode_phase(value: ManagedBootstrapPhaseV1) -> generated::CheckedManagedBootstrapPhase {
    match value {
        ManagedBootstrapPhaseV1::InstallComponents => {
            generated::CheckedManagedBootstrapPhase::InstallComponents
        }
        ManagedBootstrapPhaseV1::RetireMarkers => {
            generated::CheckedManagedBootstrapPhase::RetireMarkers
        }
        ManagedBootstrapPhaseV1::Complete => generated::CheckedManagedBootstrapPhase::Complete,
    }
}

fn decode_phase(value: generated::CheckedManagedBootstrapPhase) -> ManagedBootstrapPhaseV1 {
    match value {
        generated::CheckedManagedBootstrapPhase::InstallComponents => {
            ManagedBootstrapPhaseV1::InstallComponents
        }
        generated::CheckedManagedBootstrapPhase::RetireMarkers => {
            ManagedBootstrapPhaseV1::RetireMarkers
        }
        generated::CheckedManagedBootstrapPhase::Complete => ManagedBootstrapPhaseV1::Complete,
    }
}

fn encode_purpose(value: ManagedParentPurpose) -> generated::CheckedManagedParentPurpose {
    match value {
        ManagedParentPurpose::MergeStore => generated::CheckedManagedParentPurpose::MergeStore,
        ManagedParentPurpose::MergeArchive => generated::CheckedManagedParentPurpose::MergeArchive,
        ManagedParentPurpose::PreservationBundles => {
            generated::CheckedManagedParentPurpose::PreservationBundles
        }
        ManagedParentPurpose::RootPreservationMarkers => {
            generated::CheckedManagedParentPurpose::RootPreservationMarkers
        }
    }
}

fn decode_purpose(value: generated::CheckedManagedParentPurpose) -> ManagedParentPurpose {
    match value {
        generated::CheckedManagedParentPurpose::MergeStore => ManagedParentPurpose::MergeStore,
        generated::CheckedManagedParentPurpose::MergeArchive => ManagedParentPurpose::MergeArchive,
        generated::CheckedManagedParentPurpose::PreservationBundles => {
            ManagedParentPurpose::PreservationBundles
        }
        generated::CheckedManagedParentPurpose::RootPreservationMarkers => {
            ManagedParentPurpose::RootPreservationMarkers
        }
    }
}
