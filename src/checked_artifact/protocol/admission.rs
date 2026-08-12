//! Capacity reservation and the closed action-directory admission handoff.

use sha2::{Digest, Sha256};

use super::codec::{BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1};
use super::generated;
use super::schedule::{
    ActionDigestV1, ActionScheduleV1, RecordDigestV1, RequestOwnerBindingV1, checked_array,
};
use crate::checked_artifact::capability::DurableObjectIdentityV1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ActionCapacityReservationV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    schedule: ActionScheduleV1,
    record_digest: RecordDigestV1,
}

impl ActionCapacityReservationV1 {
    pub(in crate::checked_artifact) fn new(
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        schedule: ActionScheduleV1,
    ) -> Self {
        let mut value = Self {
            action_digest,
            request_owner_binding,
            schedule,
            record_digest: RecordDigestV1::new([0; 32]),
        };
        value.record_digest = RecordDigestV1::new(Sha256::digest(value.digest_material()).into());
        value
    }

    pub(in crate::checked_artifact) const fn action_digest(&self) -> ActionDigestV1 {
        self.action_digest
    }

    pub(in crate::checked_artifact) const fn request_owner_binding(&self) -> RequestOwnerBindingV1 {
        self.request_owner_binding
    }

    pub(in crate::checked_artifact) const fn record_digest(&self) -> RecordDigestV1 {
        self.record_digest
    }

    pub(in crate::checked_artifact) fn schedule(&self) -> &ActionScheduleV1 {
        &self.schedule
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
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid capacity taut encoding"))?;
        let wire = generated::CheckedActionCapacityReservationV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid capacity record shape"))?;
        let value = Self::new(
            ActionDigestV1::new(checked_array(wire.action_digest)?),
            RequestOwnerBindingV1::new(checked_array(wire.request_owner_binding)?),
            ActionScheduleV1::decode_canonical(&crate::cbor::encode(&wire.schedule.to_cbor()))?,
        );
        let stored_digest = checked_array(wire.record_digest)?;
        if value.record_digest.bytes() != stored_digest || value.encode_canonical()? != bytes {
            return Err(ProtocolCodecErrorV1::Invalid(
                "noncanonical capacity record",
            ));
        }
        Ok(value)
    }

    fn digest_material(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated_with_digest(Vec::new()).to_cbor())
    }

    fn to_generated(&self) -> generated::CheckedActionCapacityReservationV1 {
        self.to_generated_with_digest(self.record_digest.bytes().to_vec())
    }

    fn to_generated_with_digest(
        &self,
        record_digest: Vec<u8>,
    ) -> generated::CheckedActionCapacityReservationV1 {
        generated::CheckedActionCapacityReservationV1 {
            action_digest: self.action_digest.bytes().to_vec(),
            request_owner_binding: self.request_owner_binding.bytes().to_vec(),
            schedule: self.schedule.to_generated(),
            record_digest,
        }
    }
}

impl BoundedCanonicalRecordV1 for ActionCapacityReservationV1 {
    const KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::Capacity;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        self.encode_canonical()
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ActionDirectoryAdmissionV1(ActionDirectoryAdmissionStateV1);

#[derive(Clone, Debug, Eq, PartialEq)]
enum ActionDirectoryAdmissionStateV1 {
    Idle,
    Preparing {
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        capacity_schedule_sha256: [u8; 32],
        staging_name: String,
        final_action_name: String,
        resident_reservation_sha256: [u8; 32],
    },
}

impl ActionDirectoryAdmissionV1 {
    pub(in crate::checked_artifact) const fn idle() -> Self {
        Self(ActionDirectoryAdmissionStateV1::Idle)
    }

    pub(in crate::checked_artifact) fn preparing(
        reservation: &ActionCapacityReservationV1,
    ) -> Self {
        let action = reservation.action_digest();
        Self(ActionDirectoryAdmissionStateV1::Preparing {
            action_digest: action,
            request_owner_binding: reservation.request_owner_binding(),
            capacity_schedule_sha256: reservation.schedule().digest().bytes(),
            staging_name: "action-admission-staging-v1".to_owned(),
            final_action_name: format!("action-{}-v1", action.hex()),
            resident_reservation_sha256: reservation.record_digest().bytes(),
        })
    }

    pub(in crate::checked_artifact) fn matches_reservation(
        &self,
        reservation: &ActionCapacityReservationV1,
    ) -> bool {
        self == &Self::preparing(reservation)
    }

    fn is_idle(&self) -> bool {
        matches!(self.0, ActionDirectoryAdmissionStateV1::Idle)
    }

    pub(in crate::checked_artifact) fn encode_canonical(
        &self,
    ) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        let wire = match &self.0 {
            ActionDirectoryAdmissionStateV1::Idle => generated::CheckedActionDirectoryAdmissionV1 {
                state: generated::CheckedAdmissionState::Idle,
                ..Default::default()
            },
            ActionDirectoryAdmissionStateV1::Preparing {
                action_digest,
                request_owner_binding,
                capacity_schedule_sha256,
                staging_name,
                final_action_name,
                resident_reservation_sha256,
            } => generated::CheckedActionDirectoryAdmissionV1 {
                state: generated::CheckedAdmissionState::Preparing,
                action_digest: Some(action_digest.bytes().to_vec()),
                request_owner_binding: Some(request_owner_binding.bytes().to_vec()),
                capacity_schedule_sha256: Some(capacity_schedule_sha256.to_vec()),
                staging_name: Some(staging_name.as_bytes().to_vec()),
                final_action_name: Some(final_action_name.as_bytes().to_vec()),
                resident_reservation_sha256: Some(resident_reservation_sha256.to_vec()),
            },
        };
        Ok(crate::cbor::encode(&wire.to_cbor()))
    }

    pub(in crate::checked_artifact) fn decode_canonical(
        bytes: &[u8],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid admission taut encoding"))?;
        let wire = generated::CheckedActionDirectoryAdmissionV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid admission record shape"))?;
        let value = match wire.state {
            generated::CheckedAdmissionState::Idle => {
                if wire.action_digest.is_some()
                    || wire.request_owner_binding.is_some()
                    || wire.capacity_schedule_sha256.is_some()
                    || wire.staging_name.is_some()
                    || wire.final_action_name.is_some()
                    || wire.resident_reservation_sha256.is_some()
                {
                    return Err(ProtocolCodecErrorV1::Invalid(
                        "idle admission has preparing fields",
                    ));
                }
                Self::idle()
            }
            generated::CheckedAdmissionState::Preparing => {
                Self(ActionDirectoryAdmissionStateV1::Preparing {
                    action_digest: ActionDigestV1::new(checked_array(required(
                        wire.action_digest,
                    )?)?),
                    request_owner_binding: RequestOwnerBindingV1::new(checked_array(required(
                        wire.request_owner_binding,
                    )?)?),
                    capacity_schedule_sha256: checked_array(required(
                        wire.capacity_schedule_sha256,
                    )?)?,
                    staging_name: decode_ascii_name(&required(wire.staging_name)?)?,
                    final_action_name: decode_ascii_name(&required(wire.final_action_name)?)?,
                    resident_reservation_sha256: checked_array(required(
                        wire.resident_reservation_sha256,
                    )?)?,
                })
            }
        };
        if value.encode_canonical()? != bytes {
            return Err(ProtocolCodecErrorV1::Invalid(
                "noncanonical admission record",
            ));
        }
        Ok(value)
    }
}

/// Opaque authority handoff issued only for an idle admission with one exact
/// final action directory and no staging directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct AdmittedActionV1 {
    reservation: ActionCapacityReservationV1,
    directory_identity: DurableObjectIdentityV1,
}

impl AdmittedActionV1 {
    pub(in crate::checked_artifact) fn reservation(&self) -> &ActionCapacityReservationV1 {
        &self.reservation
    }

    pub(in crate::checked_artifact) fn directory_identity(&self) -> &DurableObjectIdentityV1 {
        &self.directory_identity
    }
}

fn required<T>(value: Option<T>) -> Result<T, ProtocolCodecErrorV1> {
    value.ok_or(ProtocolCodecErrorV1::Invalid(
        "preparing admission is missing a required field",
    ))
}

impl BoundedCanonicalRecordV1 for ActionDirectoryAdmissionV1 {
    const KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::Admission;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        self.encode_canonical()
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }
}

fn decode_ascii_name(bytes: &[u8]) -> Result<String, ProtocolCodecErrorV1> {
    if bytes.is_empty() || bytes.len() > 255 || !bytes.is_ascii() {
        return Err(ProtocolCodecErrorV1::Invalid("invalid protocol name"));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid protocol name"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum RecordObservationV1<T> {
    Missing,
    PartialExpectedPrefix,
    Exact(T),
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ScratchRecordObservationV1<T> {
    Missing,
    PartialExpectedPrefix,
    Exact(T),
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum FixedReplacementDecisionV1 {
    WriteOrRewriteScratch,
    ReplaceActiveFromScratch,
    Complete,
    Ambiguous,
}

pub(in crate::checked_artifact) fn classify_fixed_replacement<T: Eq>(
    active: &RecordObservationV1<T>,
    scratch: &ScratchRecordObservationV1<T>,
    old: &T,
    new: &T,
) -> FixedReplacementDecisionV1 {
    use FixedReplacementDecisionV1::*;
    match (active, scratch) {
        (RecordObservationV1::Exact(value), ScratchRecordObservationV1::Missing)
            if value == old =>
        {
            WriteOrRewriteScratch
        }
        (RecordObservationV1::Exact(value), ScratchRecordObservationV1::PartialExpectedPrefix)
            if value == old =>
        {
            WriteOrRewriteScratch
        }
        (
            RecordObservationV1::Exact(active_value),
            ScratchRecordObservationV1::Exact(scratch_value),
        ) if active_value == old && scratch_value == new => ReplaceActiveFromScratch,
        (RecordObservationV1::Exact(value), ScratchRecordObservationV1::Missing)
            if value == new =>
        {
            Complete
        }
        _ => Ambiguous,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ActionDirectoryObservationV1 {
    Missing,
    Exact {
        identity: DurableObjectIdentityV1,
        reservation: Box<RecordObservationV1<ActionCapacityReservationV1>>,
        extra_children: usize,
    },
    Other,
}

impl ActionDirectoryObservationV1 {
    pub(in crate::checked_artifact) fn exact(
        identity: DurableObjectIdentityV1,
        reservation: RecordObservationV1<ActionCapacityReservationV1>,
    ) -> Self {
        Self::Exact {
            identity,
            reservation: Box::new(reservation),
            extra_children: 0,
        }
    }

    fn has_exact(&self, expected: &ActionCapacityReservationV1) -> bool {
        matches!(
            self,
            Self::Exact {
                reservation,
                extra_children: 0,
                ..
            } if matches!(reservation.as_ref(), RecordObservationV1::Exact(value) if value == expected)
        )
    }

    fn has_rewritable_reservation(&self) -> bool {
        matches!(
            self,
            Self::Exact {
                reservation,
                extra_children: 0,
                ..
            } if matches!(
                reservation.as_ref(),
                RecordObservationV1::Missing | RecordObservationV1::PartialExpectedPrefix
            )
        )
    }

    fn exact_identity_for(
        &self,
        expected: &ActionCapacityReservationV1,
    ) -> Option<&DurableObjectIdentityV1> {
        match self {
            Self::Exact {
                identity,
                reservation,
                extra_children: 0,
            } if matches!(reservation.as_ref(), RecordObservationV1::Exact(value) if value == expected) => {
                Some(identity)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum AdmissionHandoffDecisionV1 {
    CreateStaging,
    WriteOrRewriteReservation,
    PublishStaging,
    ReplacePreparingWithIdle,
    Ambiguous,
}

pub(in crate::checked_artifact) fn classify_handoff(
    admission: &ActionDirectoryAdmissionV1,
    expected: &ActionCapacityReservationV1,
    staging: &ActionDirectoryObservationV1,
    final_directory: &ActionDirectoryObservationV1,
) -> AdmissionHandoffDecisionV1 {
    use AdmissionHandoffDecisionV1::*;
    if !admission.matches_reservation(expected) {
        return Ambiguous;
    }
    match (staging, final_directory) {
        (ActionDirectoryObservationV1::Missing, ActionDirectoryObservationV1::Missing) => {
            CreateStaging
        }
        (value, ActionDirectoryObservationV1::Missing) if value.has_rewritable_reservation() => {
            WriteOrRewriteReservation
        }
        (value, ActionDirectoryObservationV1::Missing) if value.has_exact(expected) => {
            PublishStaging
        }
        (ActionDirectoryObservationV1::Missing, value) if value.has_exact(expected) => {
            ReplacePreparingWithIdle
        }
        _ => Ambiguous,
    }
}

pub(in crate::checked_artifact) fn admit_observed_action(
    admission: &ActionDirectoryAdmissionV1,
    expected: &ActionCapacityReservationV1,
    staging: &ActionDirectoryObservationV1,
    final_directory: &ActionDirectoryObservationV1,
) -> Option<AdmittedActionV1> {
    if !admission.is_idle() || !matches!(staging, ActionDirectoryObservationV1::Missing) {
        return None;
    }
    Some(AdmittedActionV1 {
        reservation: expected.clone(),
        directory_identity: final_directory.exact_identity_for(expected)?.clone(),
    })
}
