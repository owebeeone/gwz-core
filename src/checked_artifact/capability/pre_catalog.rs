//! Target-bound pre-catalog authority types.
//!
//! R2-C0 freezes the live/durable type boundary. The C1 aggregate provider is
//! the only future issuer; C0 deliberately exposes no catalog writer.

use super::{
    CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1, SupportedFilesystemProfile,
};
use crate::checked_artifact::bootstrap::CatalogMutationLeaseV1;

mod provider;

pub(in crate::checked_artifact) use provider::HostPlatform;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::checked_artifact) enum PreCatalogRootKindV1 {
    Workspace,
    GitDirectory,
}

macro_rules! digest_newtype {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(in crate::checked_artifact) struct $name([u8; 32]);

        impl $name {
            pub(in crate::checked_artifact) const fn owner_issue(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub(in crate::checked_artifact) const fn bytes(self) -> [u8; 32] {
                self.0
            }
        }
    };
}

digest_newtype!(FreshObservationDigestV1);
digest_newtype!(DurableCatalogTargetDigestV1);
digest_newtype!(HistoricalCollisionDigestV1);
digest_newtype!(MissingParentObservationDigestV1);

/// Closed result of the retained pre-catalog transaction.
pub(in crate::checked_artifact) enum CatalogPreflightV1<'lease> {
    MissingGitPrivateParent(Box<MissingCatalogParentPermitV1<'lease>>),
    Ready(Box<CatalogPermitV1<'lease>>),
}

/// A ready, live-only permit. Its target cannot be substituted because the
/// lease is carried inside the permit rather than supplied beside it.
pub(in crate::checked_artifact) struct CatalogPermitV1<'lease> {
    _catalog_target_lease: CatalogMutationLeaseV1<'lease>,
    _retained_root: provider::RetainedPlatformRoot,
    _support_profile: SupportedFilesystemProfile,
    _root_identity: DurableObjectIdentityV1,
    _root_invocation_identity: Vec<u8>,
    _rename_domain: Vec<u8>,
    _path_profile: CanonicalPathIdentityV1,
    _raw_roles: provider::RawCatalogRoleObservationV1,
    _fresh_observation_digest: FreshObservationDigestV1,
    _durable_target_digest: DurableCatalogTargetDigestV1,
    _historical_collision_digest: HistoricalCollisionDigestV1,
}

/// A disjoint, live-only authorization for the one fixed Git `gwz` parent
/// creation edge. It contains no ready-catalog digest or catalog authority.
pub(in crate::checked_artifact) struct MissingCatalogParentPermitV1<'lease> {
    _catalog_target_lease: CatalogMutationLeaseV1<'lease>,
    _retained_root: provider::RetainedPlatformRoot,
    _missing_parent_observation_digest: MissingParentObservationDigestV1,
}

impl CatalogPermitV1<'_> {
    #[allow(dead_code, reason = "R2-C1 consumes the frozen ready-permit fields")]
    pub(in crate::checked_artifact) const fn support_profile(&self) -> SupportedFilesystemProfile {
        self._support_profile
    }

    #[allow(dead_code, reason = "R2-C1 consumes the frozen ready-permit fields")]
    pub(in crate::checked_artifact) fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        &self._path_profile
    }
}
