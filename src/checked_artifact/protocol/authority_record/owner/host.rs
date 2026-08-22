//! The production entry to the owner-private authority-observation seam.
//!
//! R1 froze `CheckedAuthorityObservationOwnerV1` with an unnameable raw
//! provider and a compile-only proof that a real owner child could implement
//! it ("R1 freezes the opaque owner entry before R2 installs its provider",
//! `authority_record.rs`). R2-D Step 2.4 is that installation, and this file is
//! deliberately the *whole* of it: the raw seam stays private to the owner, and
//! what leaves this subtree is a transaction trait plus a sealed issuer.
//!
//! The property R1 bought with the private seam is kept exactly:
//!
//! * [`AuthorityObservationFactsV1`] is still unnameable outside the owner;
//! * [`RetainedAuthorityFactsV1`] — the only value a transaction may return —
//!   has no public constructor, so the sole way to produce one is to call
//!   [`AuthorityFactsIssuerV1::issue`] on the issuer handed *into* one
//!   `observe_retained_request` call;
//! * the issuer is neither `Clone` nor `Copy` nor storable beyond that call's
//!   borrow, so the six facts a record binds can only ever be minted together,
//!   inside one retained transaction.
//!
//! A consumer therefore still cannot assemble an observation from a path, a
//! parent and a source it looked up separately
//! (`GwzM5-8R4bR2ConsumerCheckpoint.md` §8 :239-240, §14 first bullet).

use super::{
    AuthorityObservationFactsV1, CheckedAuthorityObservationOwnerV1,
    RawAuthorityObservationProviderV1,
};
use crate::checked_artifact::capability::{CanonicalPathIdentityV1, DurableObjectIdentityV1};
use crate::checked_artifact::protocol::{
    DurableLeafFingerprintV1, ProtocolCodecErrorV1, RequestOwnerBindingV1,
};

/// The complete facts of one retained authority transaction, sealed so that
/// only [`AuthorityFactsIssuerV1`] can mint one.
pub(in crate::checked_artifact) struct RetainedAuthorityFactsV1(AuthorityObservationFactsV1);

/// The issuing token, handed to a transaction for the duration of one call.
///
/// It is a borrow with no constructor of its own outside this module, so it
/// cannot be retained, cloned, or used to mint a second, independently
/// observed set of facts.
pub(in crate::checked_artifact) struct AuthorityFactsIssuerV1 {
    _sealed: (),
}

impl AuthorityFactsIssuerV1 {
    /// Mints the facts of one coherent observation. Every field is a fact the
    /// caller observed inside this single transaction; `expected_sha256` and
    /// `goal_sha256` are digests of *streamed* payloads and `source` is a
    /// fingerprint, so no payload byte reaches this record.
    pub(in crate::checked_artifact) fn issue(
        &self,
        request_owner_binding: RequestOwnerBindingV1,
        artifact_root: CanonicalPathIdentityV1,
        retained_parent_identity: DurableObjectIdentityV1,
        source: DurableLeafFingerprintV1,
        expected_sha256: [u8; 32],
        goal_sha256: [u8; 32],
    ) -> RetainedAuthorityFactsV1 {
        RetainedAuthorityFactsV1(AuthorityObservationFactsV1::new(
            request_owner_binding,
            artifact_root,
            retained_parent_identity,
            source,
            expected_sha256,
            goal_sha256,
        ))
    }
}

/// One retained authority transaction, implemented outside this owner.
///
/// An implementor is already targeted at one retained artifact and one request
/// transaction — the same contract the private raw seam states — and it proves
/// that by being unable to return anything but facts the issuer minted during
/// its own call.
pub(in crate::checked_artifact) trait RetainedAuthorityRequestV1 {
    fn observe_retained_request(
        &self,
        issue: &AuthorityFactsIssuerV1,
    ) -> Result<RetainedAuthorityFactsV1, ProtocolCodecErrorV1>;
}

/// Adapts a production transaction onto the private raw seam.
struct RetainedAuthorityProviderV1<Request>(Request);

impl<Request: RetainedAuthorityRequestV1> RawAuthorityObservationProviderV1
    for RetainedAuthorityProviderV1<Request>
{
    fn observe_retained_request(
        &self,
    ) -> Result<AuthorityObservationFactsV1, ProtocolCodecErrorV1> {
        self.0
            .observe_retained_request(&AuthorityFactsIssuerV1 { _sealed: () })
            .map(|facts| facts.0)
    }
}

/// The one production constructor of the authority-observation owner.
pub(in crate::checked_artifact) fn retained_authority_observation_owner(
    request: impl RetainedAuthorityRequestV1 + 'static,
) -> CheckedAuthorityObservationOwnerV1 {
    CheckedAuthorityObservationOwnerV1::from_provider(RetainedAuthorityProviderV1(request))
}
