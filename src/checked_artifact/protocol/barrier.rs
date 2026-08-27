//! Fully bound, canonically encoded namespace-barrier intent.

use sha2::{Digest, Sha256};
use std::io::Read;

use super::codec::{
    BoundedCanonicalRecordV1, ProtocolCodecErrorV1, ProtocolRecordKindV1, read_bounded_record_inner,
};
use super::generated;
use super::schedule::{
    ActionDigestV1, BarrierOrdinalV1, RecordDigestV1, RequestOwnerBindingV1, ScheduleDigestV1,
};
use super::schedule::{checked_array, checked_usize};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, DurableObjectIdentityV1, DurablePathV1,
    RoamingAnchorHomeWitnessV1,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct BarrierIntentV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    reservation_digest: RecordDigestV1,
    schedule_digest: ScheduleDigestV1,
    ordinal: BarrierOrdinalV1,
    catalog_anchor_identity: DurableObjectIdentityV1,
    private_home_parent_identity: DurableObjectIdentityV1,
    private_home_name: AsciiComponent,
    target_parent_identity: DurableObjectIdentityV1,
    target_path_profile: DurablePathV1,
    reserved_target_leaf: AsciiComponent,
    intent_id: [u8; 32],
}

impl BarrierIntentV1 {
    /// Issues one barrier intent from facts nobody restated.
    ///
    /// **Pass the witness the capability owner minted; there is no route from a
    /// caller-supplied identity.** `home` is a [`RoamingAnchorHomeWitnessV1`],
    /// constructible only inside the pre-catalog provider owner from
    /// `RetainedCompletedCatalogV1`'s own retained `catalog_anchor` and
    /// `final_directory` handles — so the catalog anchor's identity and the
    /// roaming anchor's home parent identity are *observed* by the owner that
    /// holds those capabilities, and the home's name is not observed at all: it
    /// is the frozen `InfrastructureSlotV1::RoamingAnchorHome.name()` the
    /// witness derives, which is why it is no longer a parameter.
    ///
    /// This is the O6 obligation of `GwzM5-8R2DSettledTuple.md` §11.1
    /// (`:653-658`) discharged in the Step-4.3 shape
    /// (`GwzM5-8R2DPhase4Closure.md` §4): the owner observes, the owner refuses
    /// on disagreement, and the derivation obligation is written here, on the
    /// issuer's own signature, because this is the only place a future
    /// transaction author will look. The refusal has two arms — the mint's, at
    /// `CompletedCatalogPermitV1::observe_roaming_anchor_home`, which
    /// revalidates the retained catalog before it mints anything; and the
    /// read's, at [`read_and_bind_barrier_intent`], which requires the same
    /// witness and refuses a resident record whose three identity facts
    /// disagree with it. The second exists because `decode_canonical` rebuilds
    /// through `from_bound_fields` and bypasses this constructor entirely, so
    /// without it the restatement class would survive a restart
    /// (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §5).
    #[allow(
        clippy::too_many_arguments,
        reason = "the intent deliberately binds each independent retained namespace fact"
    )]
    pub(in crate::checked_artifact) fn issue(
        _authority: &crate::checked_artifact::namespace::NamespaceBarrierAuthority,
        reservation: &super::ActionCapacityReservationV1,
        ordinal: BarrierOrdinalV1,
        home: &RoamingAnchorHomeWitnessV1,
        target_parent_identity: DurableObjectIdentityV1,
        target_path_profile: CanonicalPathIdentityV1,
        reserved_target_leaf: AsciiComponent,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if ordinal.index() >= reservation.schedule().barrier_count() {
            return Err(ProtocolCodecErrorV1::Invalid(
                "barrier ordinal is not reserved by the action schedule",
            ));
        }
        Ok(BarrierIntentV1::from_bound_fields(
            reservation.action_digest(),
            reservation.request_owner_binding(),
            reservation.record_digest(),
            reservation.schedule().digest(),
            ordinal,
            home.catalog_anchor_identity().clone(),
            home.private_home_parent_identity().clone(),
            home.private_home_name().clone(),
            target_parent_identity,
            DurablePathV1::from_live(&target_path_profile).map_err(|_| {
                ProtocolCodecErrorV1::Invalid("barrier target has invalid durable path")
            })?,
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
        reservation_digest: RecordDigestV1,
        schedule_digest: ScheduleDigestV1,
        ordinal: BarrierOrdinalV1,
        catalog_anchor_identity: DurableObjectIdentityV1,
        private_home_parent_identity: DurableObjectIdentityV1,
        private_home_name: AsciiComponent,
        target_parent_identity: DurableObjectIdentityV1,
        target_path_profile: DurablePathV1,
        reserved_target_leaf: AsciiComponent,
    ) -> Self {
        let mut value = Self {
            action_digest,
            request_owner_binding,
            reservation_digest,
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

    pub(in crate::checked_artifact) const fn reservation_digest(&self) -> RecordDigestV1 {
        self.reservation_digest
    }

    /// The target parent this barrier bound at issue. R2-E Phase E2's
    /// `barrier.target_reobserve` re-proves the live target against it, and
    /// this one *is* a real identity check because the field exists
    /// (E0.2b §1.5 row #9).
    pub(in crate::checked_artifact) const fn target_parent_identity(
        &self,
    ) -> &DurableObjectIdentityV1 {
        &self.target_parent_identity
    }

    /// The leaf the schedule reserved for the roaming anchor's alias inside the
    /// target parent. A restart learns it from this durable record rather than
    /// from its caller, which is why the alias lifecycle needs no alias-name
    /// argument on the resume path.
    pub(in crate::checked_artifact) const fn reserved_target_leaf(&self) -> &AsciiComponent {
        &self.reserved_target_leaf
    }

    pub(in crate::checked_artifact) fn encode_canonical(
        &self,
    ) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
        Ok(crate::cbor::encode(&self.to_generated().to_cbor()))
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid barrier taut encoding"))?;
        let wire = generated::CheckedBarrierIntentV1::from_cbor(&cbor)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid barrier record shape"))?;
        let action = ActionDigestV1::new(checked_array(wire.action_digest)?);
        let owner = RequestOwnerBindingV1::new(checked_array(wire.request_owner_binding)?);
        let reservation = RecordDigestV1::new(checked_array(wire.reservation_digest)?);
        let schedule = ScheduleDigestV1::new(checked_array(wire.schedule_digest)?);
        let ordinal = BarrierOrdinalV1::new(checked_usize(wire.ordinal)?)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid barrier ordinal"))?;
        let catalog = decode_identity(wire.catalog_anchor_identity)?;
        let home = decode_identity(wire.private_home_parent_identity)?;
        let home_name = AsciiComponent::parse(&wire.private_home_name)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid private home name"))?;
        let target = decode_identity(wire.target_parent_identity)?;
        let path = super::codec::decode_path(wire.target_path_profile)?;
        let target_leaf = AsciiComponent::parse(&wire.reserved_target_leaf)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid reserved target leaf"))?;
        let intent_id = checked_array(wire.intent_id)?;
        let value = Self::from_bound_fields(
            action,
            owner,
            reservation,
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
            reservation_digest: self.reservation_digest.bytes().to_vec(),
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

#[cfg(test)]
impl BarrierIntentV1 {
    #[allow(
        clippy::too_many_arguments,
        reason = "protocol-private semantic test binds every persisted field"
    )]
    pub(in crate::checked_artifact) fn test_issue(
        reservation: &super::ActionCapacityReservationV1,
        ordinal: BarrierOrdinalV1,
        home: &RoamingAnchorHomeWitnessV1,
        target_parent_identity: DurableObjectIdentityV1,
        target_path_profile: CanonicalPathIdentityV1,
        reserved_target_leaf: AsciiComponent,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let authority = crate::checked_artifact::namespace::NamespaceBarrierAuthority::test_only();
        Self::issue(
            &authority,
            reservation,
            ordinal,
            home,
            target_parent_identity,
            target_path_profile,
            reserved_target_leaf,
        )
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

pub(in crate::checked_artifact) struct BoundBarrierIntentV1(BarrierIntentV1);

impl BoundBarrierIntentV1 {
    pub(in crate::checked_artifact) fn value(&self) -> &BarrierIntentV1 {
        &self.0
    }
}

/// Reads one resident barrier intent bounded and binds it to the resident
/// reservation, ordinal **and roaming-anchor home**.
///
/// The five reservation/ordinal checks are R2-D's. The three identity checks
/// beside them are R2-E Phase E2's completion of O6
/// (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §5.2): the barrier owner
/// re-mints the witness from its own retained capabilities on **every** resume
/// and this seam refuses typed when the resident record's
/// `catalog_anchor_identity`, `private_home_parent_identity` or
/// `private_home_name` disagrees with it.
///
/// The comparison lives here, beside the other five, and **not** inside
/// `BarrierIntentV1::decode_canonical`, which has no capability and must stay a
/// pure codec. Requiring the witness rather than comparing an optional one is
/// what closes the class: there is no route to bind a resident intent without
/// the owner's own observation of the home it names.
pub(in crate::checked_artifact) fn read_and_bind_barrier_intent(
    reader: impl Read,
    reservation: &super::ActionCapacityReservationV1,
    expected_ordinal: BarrierOrdinalV1,
    home: &RoamingAnchorHomeWitnessV1,
) -> Result<BoundBarrierIntentV1, ProtocolCodecErrorV1> {
    let value = read_bounded_record_inner::<BarrierIntentV1>(reader)?;
    if value.action_digest != reservation.action_digest()
        || value.request_owner_binding != reservation.request_owner_binding()
        || value.reservation_digest != reservation.record_digest()
        || value.schedule_digest != reservation.schedule().digest()
        || value.ordinal != expected_ordinal
        || expected_ordinal.index() >= reservation.schedule().barrier_count()
    {
        return Err(ProtocolCodecErrorV1::Invalid(
            "barrier intent does not match resident reservation and ordinal",
        ));
    }
    if &value.catalog_anchor_identity != home.catalog_anchor_identity()
        || &value.private_home_parent_identity != home.private_home_parent_identity()
        || &value.private_home_name != home.private_home_name()
    {
        return Err(ProtocolCodecErrorV1::Invalid(
            "barrier intent does not match the observed roaming anchor home",
        ));
    }
    Ok(BoundBarrierIntentV1(value))
}

fn decode_identity(
    value: generated::CheckedDurableObjectIdentityV1,
) -> Result<DurableObjectIdentityV1, ProtocolCodecErrorV1> {
    DurableObjectIdentityV1::decode_canonical(&crate::cbor::encode(&value.to_cbor()))
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid durable identity"))
}
