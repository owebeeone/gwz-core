//! Taut-owned, reservation-bound checked-artifact authority record.
//!
//! R2-D Phase 2 Step 2.4 (`GwzM5-8R2D-Plan.md` §4) splits this owner's job in
//! two, and the split is structural rather than advisory:
//!
//! * **This file is the bounded parse owner.** Everything it reads is a
//!   *protocol record*, so its one and only budget is
//!   [`ProtocolRecordKindV1::Authority`]'s frozen 16 KiB bound, taken through
//!   the shared `limit + 1` reader. It names no payload, no leaf and no
//!   filesystem, so a payload byte can never widen a record bound
//!   (`GwzM5-8R4bR2ConsumerCheckpoint.md` §8 :232-237).
//! * **Source and goal proof is not here.** The record binds a source
//!   fingerprint and two payload digests, never payload bytes; those facts are
//!   *streamed* through the landed `LeafObserver`
//!   (`capability/pre_catalog/provider/authority_record_binding.rs`), whose
//!   budget is the payload's own length and never a record kind.
//!
//! The four `record.*` parse boundaries this owner announces —
//! `bounded_read`, `decode`, `canonical_reencode`, `binding_validate` — are
//! therefore the complete set of record-sized boundaries of one authority
//! read, and no payload byte crosses any of them.

use sha2::{Digest, Sha256};
use std::io::Read;

use super::ActionCapacityReservationV1;
use super::cleanup::DurableLeafFingerprintV1;
use super::codec::{
    BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1, decode_fingerprint,
    decode_identity, decode_path, encode_fingerprint, read_bounded_bytes,
};
use super::generated;
use super::schedule::{ActionDigestV1, RecordDigestV1, RequestOwnerBindingV1, ScheduleDigestV1};
use crate::checked_artifact::capability::{
    CanonicalPathIdentityV1, DurableObjectIdentityV1, DurablePathV1,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;

mod owner;

#[allow(
    unused_imports,
    reason = "R1 freezes the opaque owner entry before R2 installs its provider"
)]
pub(in crate::checked_artifact) use owner::CheckedAuthorityObservationOwnerV1;
#[cfg(test)]
pub(in crate::checked_artifact) use owner::synthetic_authority_observation_owner;
/// R2-D Step 2.4 — the production installation of the R1 owner seam.
#[allow(
    unused_imports,
    reason = "Step 2.4 installs the seam; plan §4 Step 3.3 wires its production consumer"
)]
pub(in crate::checked_artifact) use owner::{
    AuthorityFactsIssuerV1, RetainedAuthorityFactsV1, RetainedAuthorityRequestV1,
    retained_authority_observation_owner,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CheckedAuthorityRecordV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    schedule_digest: ScheduleDigestV1,
    reservation_digest: RecordDigestV1,
    artifact_root: DurablePathV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
    record_id: [u8; 32],
}

/// One coherent retained observation issued by the authority owner.
///
/// The fields deliberately have no checked-artifact-visible constructor. A
/// record can therefore bind only facts observed together by the retained
/// authority transaction, rather than a path, parent and source assembled by
/// a consumer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CheckedAuthorityObservationV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    schedule_digest: ScheduleDigestV1,
    reservation_digest: RecordDigestV1,
    artifact_root: DurablePathV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
}

impl CheckedAuthorityObservationV1 {
    fn owner_issue(
        reservation: &ActionCapacityReservationV1,
        artifact_root: CanonicalPathIdentityV1,
        retained_parent_identity: DurableObjectIdentityV1,
        source: DurableLeafFingerprintV1,
        expected_sha256: [u8; 32],
        goal_sha256: [u8; 32],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let value = Self {
            action_digest: reservation.action_digest(),
            request_owner_binding: reservation.request_owner_binding(),
            schedule_digest: reservation.schedule().digest(),
            reservation_digest: reservation.record_digest(),
            artifact_root: DurablePathV1::from_live(&artifact_root).map_err(|_| {
                ProtocolCodecErrorV1::Invalid("authority observation has invalid durable path")
            })?,
            retained_parent_identity,
            source,
            expected_sha256,
            goal_sha256,
        };
        let profile = value.retained_parent_identity.support_profile();
        if value.source.identity().support_profile() != profile
            || !super::codec::path_matches_profile(&value.artifact_root, profile)
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "authority observation durable identities use different support profiles",
            ));
        }
        Ok(value)
    }
}

impl CheckedAuthorityRecordV1 {
    pub(in crate::checked_artifact) fn issue(
        observation: &CheckedAuthorityObservationV1,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let value = Self::from_fields(
            observation.action_digest,
            observation.request_owner_binding,
            observation.schedule_digest,
            observation.reservation_digest,
            observation.artifact_root.clone(),
            observation.retained_parent_identity.clone(),
            observation.source.clone(),
            observation.expected_sha256,
            observation.goal_sha256,
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
        artifact_root: DurablePathV1,
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

    /// The source payload leaf this record binds. Step 2.4's terminal relation
    /// compares it against the *streamed* source proof; it is a fingerprint,
    /// never bytes, which is why an arbitrarily large payload leaves this
    /// bounded record's size unchanged.
    pub(in crate::checked_artifact) const fn source(&self) -> &DurableLeafFingerprintV1 {
        &self.source
    }

    /// The exact digest of the content the action expects to find resident.
    pub(in crate::checked_artifact) const fn expected_sha256(&self) -> [u8; 32] {
        self.expected_sha256
    }

    /// The exact digest of the content the action will install.
    pub(in crate::checked_artifact) const fn goal_sha256(&self) -> [u8; 32] {
        self.goal_sha256
    }

    /// The durable identity of the retained action directory this record's
    /// payloads were **observed through**.
    ///
    /// This is the one binding field that does *not* come from the reservation a
    /// caller passed to the issuer: `CheckedAuthorityObservationV1::owner_issue`
    /// copies the four reservation fields from its argument, but takes this one
    /// from the observation facts, which `observe_streamed_payloads` mints from
    /// the capability it actually streamed through
    /// (`capability/pre_catalog/provider/authority_record_binding.rs`). It is
    /// therefore the record's observed provenance, and the only field a consumer
    /// can check an observation's *origin* against rather than its caller's
    /// restatement of it (R2-D Step-3.3 review [P1-1]).
    pub(in crate::checked_artifact) const fn retained_parent_identity(
        &self,
    ) -> &DurableObjectIdentityV1 {
        &self.retained_parent_identity
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

    fn matches_observation(&self, observation: &CheckedAuthorityObservationV1) -> bool {
        self.action_digest == observation.action_digest
            && self.request_owner_binding == observation.request_owner_binding
            && self.schedule_digest == observation.schedule_digest
            && self.reservation_digest == observation.reservation_digest
            && self.artifact_root == observation.artifact_root
            && self.retained_parent_identity == observation.retained_parent_identity
            && self.source == observation.source
            && self.expected_sha256 == observation.expected_sha256
            && self.goal_sha256 == observation.goal_sha256
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

#[derive(Debug)]
pub(in crate::checked_artifact) struct BoundCheckedAuthorityRecordV1(
    CheckedAuthorityRecordV1,
    usize,
);

impl BoundCheckedAuthorityRecordV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &CheckedAuthorityRecordV1 {
        &self.0
    }
}

impl BoundCheckedAuthorityRecordV1 {
    /// How many bytes the bounded read accepted. Step 2.4's terminal relation
    /// asserts this against the record kind's frozen bound, which is the
    /// checkable form of "payload size is not confused with protocol-record
    /// size" (`GwzM5-8R4bR2ConsumerCheckpoint.md` §8 :236-237).
    pub(in crate::checked_artifact) const fn record_bytes(&self) -> usize {
        self.1
    }
}

/// The bounded authority parse, in its four announced stages.
///
/// The stages are the frozen `record.*` parse boundaries, and each is crossed
/// exactly once per read: the `limit + 1` bounded read of at most
/// [`ProtocolRecordKindV1::Authority`]'s 16 KiB; the owner-private canonical
/// decode; the canonical re-encode that refuses a non-canonical byte sequence
/// that happens to decode; and the binding validation against the resident
/// reservation and the retained observation.
///
/// The stages were previously fused inside `read_bounded_record_inner`. They
/// are spelled out here — over the same shared `read_bounded_bytes` primitive,
/// not a private re-implementation of it — because Step 2.4 must be able to
/// interrupt each of them, and because a reader of this function should be
/// able to see that nothing between the bound and the binding ever consults a
/// payload.
pub(in crate::checked_artifact) fn read_and_bind_authority_record(
    reader: impl Read,
    reservation: &ActionCapacityReservationV1,
    observation: &CheckedAuthorityObservationV1,
) -> Result<BoundCheckedAuthorityRecordV1, ProtocolCodecErrorV1> {
    // `record.bounded_read` — the only budget on this path, and it is the
    // record kind's, never a payload's.
    let bytes = read_bounded_bytes(reader, ProtocolRecordKindV1::Authority.max_bytes())?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordBoundedRead);

    // `record.decode` — the owner-private canonical decoder, run only after the
    // shared bounded read accepted the complete byte sequence.
    let value = CheckedAuthorityRecordV1::decode_canonical(&bytes)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordDecode);

    // `record.canonical_reencode` — `decode_canonical` already refuses a
    // non-canonical encoding; re-proving it here keeps the boundary a real
    // stage of this function rather than an implementation detail of a
    // helper, so an interruption at it is an interruption at a stage.
    if value.encode_record()? != bytes {
        return Err(ProtocolCodecErrorV1::Invalid(
            "authority record is not canonically encoded",
        ));
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordCanonicalReencode);

    // `record.binding_validate` — the record is bound to the resident
    // reservation and to the one coherent retained observation, never to
    // facts a consumer assembled.
    if !value.matches_reservation(reservation) || !value.matches_observation(observation) {
        return Err(ProtocolCodecErrorV1::Invalid(
            "authority does not match resident reservation and retained observation",
        ));
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordBindingValidate);
    Ok(BoundCheckedAuthorityRecordV1(value, bytes.len()))
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_authority_observation(
    reservation: &ActionCapacityReservationV1,
    action_digest: ActionDigestV1,
    artifact_root: CanonicalPathIdentityV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
) -> Result<CheckedAuthorityObservationV1, ProtocolCodecErrorV1> {
    synthetic_authority_observation_owner(
        action_digest,
        reservation.request_owner_binding(),
        artifact_root,
        retained_parent_identity,
        source,
        expected_sha256,
        goal_sha256,
    )
    .observe(reservation)
}
