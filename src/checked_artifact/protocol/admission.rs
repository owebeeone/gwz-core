//! Capacity reservation and the closed action-directory admission handoff.

use sha2::{Digest, Sha256};

use super::codec::{
    BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1, read_bounded_record_inner,
};
use super::generated;
use super::schedule::{
    ActionDigestV1, ActionScheduleV1, RecordDigestV1, RequestOwnerBindingV1, checked_array,
};
use super::slots::{CatalogRootRowClassV1, InfrastructureSlotV1, RootEntryNameV1};
use crate::checked_artifact::capability::DurableObjectIdentityV1;

mod owner;
#[cfg(test)]
mod test_support;
#[allow(
    unused_imports,
    reason = "R1 exports the admission classifier before its R2-D physical half consumes it"
)]
pub(in crate::checked_artifact) use owner::*;
#[cfg(test)]
pub(in crate::checked_artifact) use test_support::*;

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

    fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical_inner(bytes)
    }

    fn decode_canonical_inner(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
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
pub(in crate::checked_artifact) struct ActionDirectoryAdmissionV1 {
    state: ActionDirectoryAdmissionStateV1,
    record_digest: RecordDigestV1,
}

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
    pub(in crate::checked_artifact) fn idle() -> Self {
        Self::from_state(ActionDirectoryAdmissionStateV1::Idle)
    }

    pub(in crate::checked_artifact) fn preparing(
        reservation: &ActionCapacityReservationV1,
    ) -> Self {
        let action = reservation.action_digest();
        Self::from_state(ActionDirectoryAdmissionStateV1::Preparing {
            action_digest: action,
            request_owner_binding: reservation.request_owner_binding(),
            capacity_schedule_sha256: reservation.schedule().digest().bytes(),
            staging_name: InfrastructureSlotV1::ActionAdmissionStaging
                .name()
                .to_owned(),
            final_action_name: RootEntryNameV1::ActiveAction(action).name(),
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
        matches!(self.state, ActionDirectoryAdmissionStateV1::Idle)
    }

    pub(in crate::checked_artifact) const fn record_digest(&self) -> RecordDigestV1 {
        self.record_digest
    }

    fn from_state(state: ActionDirectoryAdmissionStateV1) -> Self {
        let mut value = Self {
            state,
            record_digest: RecordDigestV1::new([0; 32]),
        };
        value.record_digest = RecordDigestV1::new(Sha256::digest(value.digest_material()).into());
        value
    }

    pub(in crate::checked_artifact) fn encode_canonical(
        &self,
    ) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        Ok(crate::cbor::encode(&self.to_generated().to_cbor()))
    }

    fn digest_material(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated_with_digest(Vec::new()).to_cbor())
    }

    fn to_generated(&self) -> generated::CheckedActionDirectoryAdmissionV1 {
        self.to_generated_with_digest(self.record_digest.bytes().to_vec())
    }

    fn to_generated_with_digest(
        &self,
        record_digest: Vec<u8>,
    ) -> generated::CheckedActionDirectoryAdmissionV1 {
        match &self.state {
            ActionDirectoryAdmissionStateV1::Idle => generated::CheckedActionDirectoryAdmissionV1 {
                state: generated::CheckedAdmissionState::Idle,
                record_digest,
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
                record_digest,
            },
        }
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid admission taut encoding"))?;
        let wire = generated::CheckedActionDirectoryAdmissionV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid admission record shape"))?;
        let stored_digest = checked_array(wire.record_digest)?;
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
                Self::from_state(ActionDirectoryAdmissionStateV1::Preparing {
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
        if value.record_digest.bytes() != stored_digest || value.encode_canonical()? != bytes {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum AdmissionHandoffDecisionV1 {
    CreateStaging,
    WriteOrRewriteReservation,
    PublishStaging,
    ReplacePreparingWithIdle,
    Ambiguous,
}

/// Reads one bounded canonical admission record, so the physical driver decodes
/// the durable `ActionAdmission*` triad without reaching into this owner's
/// private codec (the `decode_catalog_bootstrap_record` idiom,
/// `protocol/catalog_bootstrap_record.rs:375-378`).
pub(in crate::checked_artifact) fn decode_action_directory_admission(
    reader: impl std::io::Read,
) -> Result<ActionDirectoryAdmissionV1, ProtocolCodecErrorV1> {
    read_bounded_record_inner::<ActionDirectoryAdmissionV1>(reader)
}

/// Reads one bounded canonical resident capacity reservation.
pub(in crate::checked_artifact) fn decode_action_capacity_reservation(
    reader: impl std::io::Read,
) -> Result<ActionCapacityReservationV1, ProtocolCodecErrorV1> {
    read_bounded_record_inner::<ActionCapacityReservationV1>(reader)
}

/// One bounded physical edge of the `GwzM5-8R4bR2ConsumerCheckpoint.md` §7
/// (:209-221) durable sequence.
///
/// Pure transition vocabulary over the already-frozen `ActionAdmission*` triad
/// and the already-frozen `RootEntryNameV1::ActiveAction` row: it mints no
/// durable record, slot, purpose, or phase (interface freeze §6).
pub(in crate::checked_artifact) enum ActionAdmissionEdgeV1<'a> {
    /// Step 3/7 write-ahead half: write or rewrite the admission scratch to the
    /// next durable state.
    WriteAdmissionScratch(&'a ActionDirectoryAdmissionV1),
    /// Step 3/7 install half, part one: retire the superseded active record so
    /// the no-replace publication has a free destination.
    RetireAdmissionRecord,
    /// Step 3/7 install half, part two: publish the scratch onto the active
    /// admission name without replacement.
    PublishAdmissionRecord,
    /// Step 4: create the one indexed staging action directory.
    CreateStagingDirectory,
    /// Step 5: write and flush the resident reservation inside staging.
    WriteResidentReservation,
    /// Step 6: publish staging to the deterministic final action name without
    /// replacement.
    PublishStagingAction,
}

/// Bounded census of the §6 (:199-201) global classification over the catalog
/// root, carried alongside the admission observation so a caller can see the
/// interior action rows without receiving raw rows or handles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogRootRowCensusV1 {
    pub(in crate::checked_artifact) infrastructure: usize,
    pub(in crate::checked_artifact) scheduled_scratch: usize,
    pub(in crate::checked_artifact) retired: usize,
    pub(in crate::checked_artifact) active_actions: usize,
    pub(in crate::checked_artifact) malformed_recognized: usize,
    pub(in crate::checked_artifact) foreign: usize,
}

impl CatalogRootRowCensusV1 {
    pub(in crate::checked_artifact) fn charge(&mut self, class: CatalogRootRowClassV1) {
        let counter = match class {
            CatalogRootRowClassV1::Infrastructure(_) => &mut self.infrastructure,
            CatalogRootRowClassV1::ScheduledScratch(_) => &mut self.scheduled_scratch,
            CatalogRootRowClassV1::Retired(_) => &mut self.retired,
            CatalogRootRowClassV1::ActiveAction(_) => &mut self.active_actions,
            CatalogRootRowClassV1::MalformedRecognized(_) => &mut self.malformed_recognized,
            CatalogRootRowClassV1::Foreign => &mut self.foreign,
        };
        *counter = counter.saturating_add(1);
    }

    /// True when the bounded global classification found a child outside the
    /// owned catalog-root grammar (§6's malformed-recognized and foreign
    /// classes), which stops admission.
    pub(in crate::checked_artifact) const fn has_unowned_row(self) -> bool {
        self.malformed_recognized > 0 || self.foreign > 0
    }
}

/// The complete owner-private admission observation of one catalog root: the
/// durable `ActionAdmission*` triad, the deterministic final action row, and the
/// bounded global row census. Carries no handle and no mutation capability.
pub(in crate::checked_artifact) struct ActionAdmissionObservationV1 {
    pub(in crate::checked_artifact) record: RecordObservationV1<ActionDirectoryAdmissionV1>,
    pub(in crate::checked_artifact) scratch: RecordObservationV1<ActionDirectoryAdmissionV1>,
    pub(in crate::checked_artifact) staging: ObservedActionDirectoryV1,
    pub(in crate::checked_artifact) final_directory: ObservedActionDirectoryV1,
    pub(in crate::checked_artifact) census: CatalogRootRowCensusV1,
}
