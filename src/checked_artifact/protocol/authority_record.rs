//! Taut-owned, reservation-bound checked-artifact authority record.

use sha2::{Digest, Sha256};
use std::io::Read;

use super::cleanup::DurableLeafFingerprintV1;
use super::codec::{
    BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1, decode_fingerprint,
    decode_identity, decode_path, encode_fingerprint, read_bounded_record_inner,
};
use super::generated;
use super::schedule::{ActionDigestV1, RecordDigestV1, RequestOwnerBindingV1, ScheduleDigestV1};
use super::{ActionCapacityReservationV1, CanonicalPathIdentityV1};
use crate::checked_artifact::capability::DurableObjectIdentityV1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CheckedAuthorityRecordV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    schedule_digest: ScheduleDigestV1,
    reservation_digest: RecordDigestV1,
    artifact_root: CanonicalPathIdentityV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
    record_id: [u8; 32],
}

impl CheckedAuthorityRecordV1 {
    pub(in crate::checked_artifact) fn new(
        reservation: &ActionCapacityReservationV1,
        artifact_root: CanonicalPathIdentityV1,
        retained_parent_identity: DurableObjectIdentityV1,
        source: DurableLeafFingerprintV1,
        expected_sha256: [u8; 32],
        goal_sha256: [u8; 32],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let value = Self::from_fields(
            reservation.action_digest(),
            reservation.request_owner_binding(),
            reservation.schedule().digest(),
            reservation.record_digest(),
            artifact_root,
            retained_parent_identity,
            source,
            expected_sha256,
            goal_sha256,
        );
        value.validate_profiles()?;
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    fn from_fields(
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        schedule_digest: ScheduleDigestV1,
        reservation_digest: RecordDigestV1,
        artifact_root: CanonicalPathIdentityV1,
        retained_parent_identity: DurableObjectIdentityV1,
        source: DurableLeafFingerprintV1,
        expected_sha256: [u8; 32],
        goal_sha256: [u8; 32],
    ) -> Self {
        let mut value = Self {
            action_digest,
            request_owner_binding,
            schedule_digest,
            reservation_digest,
            artifact_root,
            retained_parent_identity,
            source,
            expected_sha256,
            goal_sha256,
            record_id: [0; 32],
        };
        value.record_id = Sha256::digest(value.digest_material()).into();
        value
    }

    pub(in crate::checked_artifact) const fn record_id(&self) -> [u8; 32] {
        self.record_id
    }

    pub(in crate::checked_artifact) fn matches_reservation(
        &self,
        reservation: &ActionCapacityReservationV1,
    ) -> bool {
        self.action_digest == reservation.action_digest()
            && self.request_owner_binding == reservation.request_owner_binding()
            && self.schedule_digest == reservation.schedule().digest()
            && self.reservation_digest == reservation.record_digest()
    }

    fn validate_profiles(&self) -> Result<(), ProtocolCodecErrorV1> {
        let profile = self.retained_parent_identity.support_profile();
        if self.source.identity().support_profile() != profile
            || !super::codec::path_matches_profile(&self.artifact_root, profile)
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "authority durable identities use different support profiles",
            ));
        }
        Ok(())
    }

    pub(in crate::checked_artifact) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid authority taut encoding"))?;
        let wire = generated::CheckedAuthorityV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid authority record shape"))?;
        let stored_id = super::schedule::checked_array(wire.record_id)?;
        let value = Self::from_fields(
            ActionDigestV1::new(super::schedule::checked_array(wire.action_digest)?),
            RequestOwnerBindingV1::new(super::schedule::checked_array(wire.request_owner_binding)?),
            ScheduleDigestV1::new(super::schedule::checked_array(wire.schedule_digest)?),
            RecordDigestV1::new(super::schedule::checked_array(wire.reservation_digest)?),
            decode_path(wire.artifact_root)?,
            decode_identity(wire.retained_parent_identity)?,
            decode_fingerprint(wire.source)?,
            super::schedule::checked_array(wire.expected_sha256)?,
            super::schedule::checked_array(wire.goal_sha256)?,
        );
        value.validate_profiles()?;
        if value.record_id != stored_id || value.encode_canonical() != bytes {
            return Err(ProtocolCodecErrorV1::Invalid(
                "authority record binding mismatch",
            ));
        }
        Ok(value)
    }

    fn digest_material(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated_with_id(Vec::new()).to_cbor())
    }

    fn to_generated(&self) -> generated::CheckedAuthorityV1 {
        self.to_generated_with_id(self.record_id.to_vec())
    }

    fn to_generated_with_id(&self, record_id: Vec<u8>) -> generated::CheckedAuthorityV1 {
        generated::CheckedAuthorityV1 {
            action_digest: self.action_digest.bytes().to_vec(),
            request_owner_binding: self.request_owner_binding.bytes().to_vec(),
            schedule_digest: self.schedule_digest.bytes().to_vec(),
            reservation_digest: self.reservation_digest.bytes().to_vec(),
            artifact_root: self.artifact_root.to_generated(),
            retained_parent_identity: self.retained_parent_identity.to_generated(),
            source: encode_fingerprint(&self.source),
            expected_sha256: self.expected_sha256.to_vec(),
            goal_sha256: self.goal_sha256.to_vec(),
            record_id,
        }
    }
}

impl BoundedCanonicalRecordV1 for CheckedAuthorityRecordV1 {
    const KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::Authority;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        super::codec::encode_bounded_record(Self::KIND, self.encode_canonical())
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        Self::decode_canonical(bytes)
    }
}

pub(in crate::checked_artifact) struct BoundCheckedAuthorityRecordV1(CheckedAuthorityRecordV1);

impl BoundCheckedAuthorityRecordV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &CheckedAuthorityRecordV1 {
        &self.0
    }
}

pub(in crate::checked_artifact) fn read_and_bind_authority_record(
    reader: impl Read,
    reservation: &ActionCapacityReservationV1,
) -> Result<BoundCheckedAuthorityRecordV1, ProtocolCodecErrorV1> {
    let value = read_bounded_record_inner::<CheckedAuthorityRecordV1>(reader)?;
    if !value.matches_reservation(reservation) {
        return Err(ProtocolCodecErrorV1::Invalid(
            "authority does not match resident reservation",
        ));
    }
    Ok(BoundCheckedAuthorityRecordV1(value))
}
