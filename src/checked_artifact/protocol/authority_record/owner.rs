//! Owner-private retained observation seam for checked authority records.

mod host;

#[allow(
    unused_imports,
    reason = "R2-D Step 2.4 installs the production seam; plan §4 Step 3.3 wires its consumer"
)]
pub(in crate::checked_artifact) use host::{
    AuthorityFactsIssuerV1, RetainedAuthorityFactsV1, RetainedAuthorityRequestV1,
    retained_authority_observation_owner,
};

use super::{CheckedAuthorityObservationV1, DurableLeafFingerprintV1};
use crate::checked_artifact::capability::{CanonicalPathIdentityV1, DurableObjectIdentityV1};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ProtocolCodecErrorV1, RequestOwnerBindingV1,
};

/// Complete facts returned by one retained observation transaction. The type
/// and its constructor are private to this owner; checked-artifact consumers
/// cannot assemble one from independently observed values.
struct AuthorityObservationFactsV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    artifact_root: CanonicalPathIdentityV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
}

impl AuthorityObservationFactsV1 {
    fn new(
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        artifact_root: CanonicalPathIdentityV1,
        retained_parent_identity: DurableObjectIdentityV1,
        source: DurableLeafFingerprintV1,
        expected_sha256: [u8; 32],
        goal_sha256: [u8; 32],
    ) -> Self {
        Self {
            action_digest,
            request_owner_binding,
            artifact_root,
            retained_parent_identity,
            source,
            expected_sha256,
            goal_sha256,
        }
    }
}

/// Raw provider seam, deliberately unnameable outside the observation owner.
/// A provider instance is already targeted at one retained artifact and one
/// request transaction.
trait RawAuthorityObservationProviderV1 {
    fn observe_retained_request(&self)
    -> Result<AuthorityObservationFactsV1, ProtocolCodecErrorV1>;
}

/// Consumer-facing authority-observation owner. Production construction stays
/// inside this module subtree, while R2 consumers can invoke the one coherent
/// observation operation without naming or implementing the raw provider.
pub(in crate::checked_artifact) struct CheckedAuthorityObservationOwnerV1 {
    provider: Box<dyn RawAuthorityObservationProviderV1>,
}

impl CheckedAuthorityObservationOwnerV1 {
    fn from_provider(provider: impl RawAuthorityObservationProviderV1 + 'static) -> Self {
        Self {
            provider: Box::new(provider),
        }
    }

    pub(in crate::checked_artifact) fn observe(
        &self,
        reservation: &ActionCapacityReservationV1,
    ) -> Result<CheckedAuthorityObservationV1, ProtocolCodecErrorV1> {
        let facts = self.provider.observe_retained_request()?;
        // Phase 3 settle item 8. Before this gate the observation's action
        // digest was *copied from the reservation argument* by `owner_issue`,
        // so a transaction that streamed under action B and was issued against
        // action A's reservation produced a record that agreed with itself and
        // could only be caught downstream, by a consumer that remembered to
        // check provenance. Comparing the transaction's own digest against the
        // reservation's closes it here, at the seam, for every consumer.
        if facts.action_digest != reservation.action_digest() {
            return Err(ProtocolCodecErrorV1::Invalid(
                "authority observation action digest does not match resident reservation",
            ));
        }
        if facts.request_owner_binding != reservation.request_owner_binding() {
            return Err(ProtocolCodecErrorV1::Invalid(
                "authority observation request binding does not match resident reservation",
            ));
        }
        CheckedAuthorityObservationV1::owner_issue(
            reservation,
            facts.artifact_root,
            facts.retained_parent_identity,
            facts.source,
            facts.expected_sha256,
            facts.goal_sha256,
        )
    }
}

#[cfg(test)]
struct SyntheticAuthorityObservationProviderV1 {
    facts: AuthorityObservationFactsV1,
}

#[cfg(test)]
impl RawAuthorityObservationProviderV1 for SyntheticAuthorityObservationProviderV1 {
    fn observe_retained_request(
        &self,
    ) -> Result<AuthorityObservationFactsV1, ProtocolCodecErrorV1> {
        Ok(AuthorityObservationFactsV1::new(
            self.facts.action_digest,
            self.facts.request_owner_binding,
            self.facts.artifact_root.clone(),
            self.facts.retained_parent_identity.clone(),
            self.facts.source.clone(),
            self.facts.expected_sha256,
            self.facts.goal_sha256,
        ))
    }
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_authority_observation_owner(
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    artifact_root: CanonicalPathIdentityV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
) -> CheckedAuthorityObservationOwnerV1 {
    CheckedAuthorityObservationOwnerV1::from_provider(SyntheticAuthorityObservationProviderV1 {
        facts: AuthorityObservationFactsV1::new(
            action_digest,
            request_owner_binding,
            artifact_root,
            retained_parent_identity,
            source,
            expected_sha256,
            goal_sha256,
        ),
    })
}
