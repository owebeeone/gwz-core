//! Always-compiled proof that a real owner child can implement and invoke the
//! raw seam without exposing observation construction to consumers.

#![allow(dead_code, reason = "R1 compile-only authority provider shape")]

use super::*;

struct ProductionShapedAuthorityProvider {
    request_owner_binding: RequestOwnerBindingV1,
    artifact_root: CanonicalPathIdentityV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
}

impl RawAuthorityObservationProviderV1 for ProductionShapedAuthorityProvider {
    fn observe_retained_request(
        &self,
    ) -> Result<AuthorityObservationFactsV1, ProtocolCodecErrorV1> {
        Ok(AuthorityObservationFactsV1::new(
            self.request_owner_binding,
            self.artifact_root.clone(),
            self.retained_parent_identity.clone(),
            self.source.clone(),
            self.expected_sha256,
            self.goal_sha256,
        ))
    }
}

fn production_owner(
    request_owner_binding: RequestOwnerBindingV1,
    artifact_root: CanonicalPathIdentityV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
) -> CheckedAuthorityObservationOwnerV1 {
    CheckedAuthorityObservationOwnerV1::from_provider(ProductionShapedAuthorityProvider {
        request_owner_binding,
        artifact_root,
        retained_parent_identity,
        source,
        expected_sha256,
        goal_sha256,
    })
}

fn compile_observation_call(
    owner: &CheckedAuthorityObservationOwnerV1,
    reservation: &ActionCapacityReservationV1,
) -> Result<CheckedAuthorityObservationV1, ProtocolCodecErrorV1> {
    owner.observe(reservation)
}
