//! Target-bound pre-catalog authority types.
//!
//! R2-C0 freezes the live/durable type boundary. The C1 aggregate provider is
//! the only future issuer; C0 deliberately exposes no catalog writer.

use super::{
    CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1, SupportedFilesystemProfile,
};
use crate::checked_artifact::bootstrap::CatalogLeaseTargetWitnessV1;

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
    _catalog_target: CatalogLeaseTargetWitnessV1<'lease>,
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
    _catalog_target: CatalogLeaseTargetWitnessV1<'lease>,
    _retained_root: provider::RetainedPlatformRoot,
    _missing_parent_observation_digest: MissingParentObservationDigestV1,
}

impl CatalogPermitV1<'_> {
    #[allow(dead_code, reason = "R2-C2 revalidates before every physical edge")]
    pub(in crate::checked_artifact) fn revalidate_target_binding(
        &self,
    ) -> Result<(), CheckedFsError> {
        provider::revalidate_lease_root_binding(&self._catalog_target, &self._retained_root)
    }

    #[allow(dead_code, reason = "R2-C1 consumes the frozen ready-permit fields")]
    pub(in crate::checked_artifact) const fn support_profile(&self) -> SupportedFilesystemProfile {
        self._support_profile
    }

    #[allow(dead_code, reason = "R2-C1 consumes the frozen ready-permit fields")]
    pub(in crate::checked_artifact) fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        &self._path_profile
    }
}

impl<'lease> CatalogPermitV1<'lease> {
    #[allow(
        dead_code,
        reason = "R2-C1 derives the typed digests and issues the ready permit"
    )]
    fn owner_issue(
        bound: provider::LeaseBoundPreCatalogObservationV1<'lease>,
        fresh_observation_digest: FreshObservationDigestV1,
        durable_target_digest: DurableCatalogTargetDigestV1,
        historical_collision_digest: HistoricalCollisionDigestV1,
    ) -> Result<Self, CheckedFsError> {
        provider::revalidate_bound_observation(&bound)?;
        if !provider::has_private_parent(&bound) {
            return Err(CheckedFsError::ambiguous(
                "catalog preflight state",
                "ready permit requires the retained private parent",
            ));
        }
        let provider::LeaseBoundPreCatalogObservationV1 {
            target,
            observation,
        } = bound;
        Ok(Self {
            _catalog_target: target,
            _retained_root: observation.retained_root,
            _support_profile: observation.support_profile,
            _root_identity: observation.root_identity,
            _root_invocation_identity: observation.root_invocation_identity,
            _rename_domain: observation.rename_domain,
            _path_profile: observation.path_profile,
            _raw_roles: observation.raw_roles,
            _fresh_observation_digest: fresh_observation_digest,
            _durable_target_digest: durable_target_digest,
            _historical_collision_digest: historical_collision_digest,
        })
    }
}

impl MissingCatalogParentPermitV1<'_> {
    #[allow(dead_code, reason = "R2-C2 revalidates the one missing-parent edge")]
    pub(in crate::checked_artifact) fn revalidate_target_binding(
        &self,
    ) -> Result<(), CheckedFsError> {
        provider::revalidate_lease_root_binding(&self._catalog_target, &self._retained_root)
    }
}

impl<'lease> MissingCatalogParentPermitV1<'lease> {
    #[allow(dead_code, reason = "R2-C1 issues the disjoint missing-parent permit")]
    fn owner_issue(
        bound: provider::LeaseBoundPreCatalogObservationV1<'lease>,
        missing_parent_observation_digest: MissingParentObservationDigestV1,
    ) -> Result<Self, CheckedFsError> {
        provider::revalidate_bound_observation(&bound)?;
        if bound.target.facts()?.root_kind() != PreCatalogRootKindV1::GitDirectory
            || provider::has_private_parent(&bound)
        {
            return Err(CheckedFsError::ambiguous(
                "catalog preflight state",
                "missing-parent permit requires one lease-bound Git target with no private parent",
            ));
        }
        let provider::LeaseBoundPreCatalogObservationV1 {
            target,
            observation,
        } = bound;
        Ok(Self {
            _catalog_target: target,
            _retained_root: observation.retained_root,
            _missing_parent_observation_digest: missing_parent_observation_digest,
        })
    }
}
