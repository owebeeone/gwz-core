//! Always-compiled proof that a real owner child can implement and invoke the
//! raw seam without exposing observation construction to consumers.

#![allow(dead_code, reason = "R1 compile-only authority provider shape")]

use super::*;

struct ProductionShapedAuthorityProvider {
    artifact_root: CanonicalPathIdentityV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
}

impl RawAuthorityObservationProviderV1 for ProductionShapedAuthorityProvider {
    fn observe_retained(&self) -> Result<AuthorityObservationFactsV1, ProtocolCodecErrorV1> {
        Ok(AuthorityObservationFactsV1::new(
            self.artifact_root.clone(),
            self.retained_parent_identity.clone(),
            self.source.clone(),
        ))
    }
}

fn production_owner(
    provider: ProductionShapedAuthorityProvider,
) -> CheckedAuthorityObservationOwnerV1 {
    CheckedAuthorityObservationOwnerV1::from_provider(provider)
}

fn compile_observation_call(
    owner: &CheckedAuthorityObservationOwnerV1,
    reservation: &ActionCapacityReservationV1,
) -> Result<CheckedAuthorityObservationV1, ProtocolCodecErrorV1> {
    owner.observe(reservation, [1; 32], [2; 32])
}
