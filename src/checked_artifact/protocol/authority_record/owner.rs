//! Owner-private retained observation seam for checked authority records.

mod provider_compile;

use super::{CheckedAuthorityObservationV1, DurableLeafFingerprintV1};
use crate::checked_artifact::capability::{CanonicalPathIdentityV1, DurableObjectIdentityV1};
use crate::checked_artifact::protocol::{ActionCapacityReservationV1, ProtocolCodecErrorV1};

/// Complete facts returned by one retained observation transaction. The type
/// and its constructor are private to this owner; checked-artifact consumers
/// cannot assemble one from independently observed values.
struct AuthorityObservationFactsV1 {
    artifact_root: CanonicalPathIdentityV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
}

impl AuthorityObservationFactsV1 {
    fn new(
        artifact_root: CanonicalPathIdentityV1,
        retained_parent_identity: DurableObjectIdentityV1,
        source: DurableLeafFingerprintV1,
    ) -> Self {
        Self {
            artifact_root,
            retained_parent_identity,
            source,
        }
    }
}

/// Raw provider seam, deliberately unnameable outside the observation owner.
/// A provider instance is already targeted at one retained artifact.
trait RawAuthorityObservationProviderV1 {
    fn observe_retained(&self) -> Result<AuthorityObservationFactsV1, ProtocolCodecErrorV1>;
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
        expected_sha256: [u8; 32],
        goal_sha256: [u8; 32],
    ) -> Result<CheckedAuthorityObservationV1, ProtocolCodecErrorV1> {
        let facts = self.provider.observe_retained()?;
        CheckedAuthorityObservationV1::owner_issue(
            reservation,
            facts.artifact_root,
            facts.retained_parent_identity,
            facts.source,
            expected_sha256,
            goal_sha256,
        )
    }
}
