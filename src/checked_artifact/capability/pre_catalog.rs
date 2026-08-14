//! Target-bound pre-catalog authority types.
//!
//! R2-C0 freezes the live/durable type boundary. The C1 aggregate provider is
//! the only future issuer; C0 deliberately exposes no catalog writer.

use super::{
    CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1, SupportedFilesystemProfile,
};
use crate::checked_artifact::bootstrap::CatalogLeaseTargetWitnessV1;
use crate::checked_artifact::catalog::CatalogScratchNameV1;
use crate::checked_artifact::catalog::{
    CatalogAttemptBindingV1, CatalogClassificationV1, classify_catalog_attempt,
};
use crate::checked_artifact::protocol::CatalogBootstrapOwnershipTokenV1;
#[cfg(test)]
use crate::checked_artifact::protocol::CatalogBootstrapRecordV1;

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

pub(in crate::checked_artifact) fn preflight_catalog_target(
    target: CatalogLeaseTargetWitnessV1<'_>,
) -> Result<CatalogPreflightV1<'_>, CheckedFsError> {
    let bound = provider::inspect_bound_catalog_target(target)?;
    match bound.observation.ready_digests {
        Some(digests) => Ok(CatalogPreflightV1::Ready(Box::new(
            CatalogPermitV1::owner_issue(bound, digests.fresh, digests.target, digests.historical)?,
        ))),
        None => {
            let digest = bound.observation.missing_parent_digest.ok_or_else(|| {
                CheckedFsError::ambiguous(
                    "catalog preflight state",
                    "observation is neither a ready target nor a missing Git parent",
                )
            })?;
            Ok(CatalogPreflightV1::MissingGitPrivateParent(Box::new(
                MissingCatalogParentPermitV1::owner_issue(bound, digest)?,
            )))
        }
    }
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
    _attempt_binding: CatalogAttemptBindingV1,
}

/// A disjoint, live-only authorization for the one fixed Git `gwz` parent
/// creation edge. It contains no ready-catalog digest or catalog authority.
pub(in crate::checked_artifact) struct MissingCatalogParentPermitV1<'lease> {
    _catalog_target: CatalogLeaseTargetWitnessV1<'lease>,
    _retained_root: provider::RetainedPlatformRoot,
    _missing_parent_observation_digest: MissingParentObservationDigestV1,
}

impl<'lease> CatalogPermitV1<'lease> {
    #[allow(dead_code, reason = "R2-C2 revalidates before every physical edge")]
    pub(in crate::checked_artifact) fn revalidate_target_binding(
        &self,
    ) -> Result<(), CheckedFsError> {
        provider::revalidate_lease_root_binding(&self._catalog_target, &self._retained_root)
    }

    pub(in crate::checked_artifact) fn revalidate_observation(&self) -> Result<(), CheckedFsError> {
        provider::revalidate_ready_observation(
            &self._catalog_target,
            &self._retained_root,
            self._fresh_observation_digest,
        )
    }

    #[allow(dead_code, reason = "R2-C1 consumes the frozen ready-permit fields")]
    pub(in crate::checked_artifact) const fn support_profile(&self) -> SupportedFilesystemProfile {
        self._support_profile
    }

    #[allow(dead_code, reason = "R2-C1 consumes the frozen ready-permit fields")]
    pub(in crate::checked_artifact) fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        &self._path_profile
    }

    pub(in crate::checked_artifact) const fn digests(
        &self,
    ) -> (
        FreshObservationDigestV1,
        DurableCatalogTargetDigestV1,
        HistoricalCollisionDigestV1,
    ) {
        (
            self._fresh_observation_digest,
            self._durable_target_digest,
            self._historical_collision_digest,
        )
    }

    pub(in crate::checked_artifact) fn attempt_binding(&self) -> &CatalogAttemptBindingV1 {
        &self._attempt_binding
    }

    pub(in crate::checked_artifact) fn classify_observed(&self) -> CatalogClassificationV1 {
        classify_catalog_attempt(
            &self._attempt_binding,
            provider::outer_aggregate_facts(&self._raw_roles),
        )
    }

    pub(in crate::checked_artifact) fn execute_write_or_rewrite_scratch(
        self: Box<Self>,
        token: CatalogBootstrapOwnershipTokenV1,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        self.revalidate_observation()?;
        let scratch = CatalogScratchNameV1::new(
            self._durable_target_digest,
            self._historical_collision_digest,
            token,
        );
        let record = self
            ._attempt_binding
            .record_from_scratch(&scratch)
            .map_err(|_| {
                CheckedFsError::ambiguous(
                    "catalog scratch",
                    "scratch values do not match the retained attempt binding",
                )
            })?;
        let classification = self.classify_observed();
        let create_new = classification.expected_record().is_none();
        if classification.decision()
            != crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch
            || classification
                .expected_record()
                .is_some_and(|expected| expected != &record)
        {
            return Err(CheckedFsError::ambiguous(
                "catalog scratch",
                "ready permit does not authorize this scratch edge",
            ));
        }
        provider::write_or_rewrite_scratch(&self._retained_root, &scratch, &record, create_new)?;
        Ok(self.into_target())
    }

    pub(in crate::checked_artifact) fn execute_publish_active(
        self: Box<Self>,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        self.revalidate_observation()?;
        let classification = self.classify_observed();
        if classification.decision()
            != crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1::PublishActive
        {
            return Err(CheckedFsError::ambiguous(
                "catalog active publication",
                "ready permit does not authorize active publication",
            ));
        }
        let record = classification.expected_record().ok_or_else(|| {
            CheckedFsError::ambiguous(
                "catalog active publication",
                "published scratch has no expected record",
            )
        })?;
        let scratch = CatalogScratchNameV1::new(
            record.durable_target_digest(),
            record.historical_collision_digest(),
            record.bootstrap_ownership_token(),
        );
        provider::publish_active_record(&self._retained_root, &scratch)?;
        Ok(self.into_target())
    }

    fn into_target(self: Box<Self>) -> CatalogLeaseTargetWitnessV1<'lease> {
        let Self {
            _catalog_target,
            _retained_root: _,
            _support_profile: _,
            _root_identity: _,
            _root_invocation_identity: _,
            _rename_domain: _,
            _path_profile: _,
            _raw_roles: _,
            _fresh_observation_digest: _,
            _durable_target_digest: _,
            _historical_collision_digest: _,
            _attempt_binding: _,
        } = *self;
        _catalog_target
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn record_for_test(
        &self,
        token: CatalogBootstrapOwnershipTokenV1,
    ) -> CatalogBootstrapRecordV1 {
        self._attempt_binding
            .record_from_scratch(&CatalogScratchNameV1::new(
                self._durable_target_digest,
                self._historical_collision_digest,
                token,
            ))
            .expect("permit digests match its attempt binding")
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
        let attempt_binding =
            provider::attempt_binding(&bound, durable_target_digest, historical_collision_digest)?;
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
            _attempt_binding: attempt_binding,
        })
    }
}

impl<'lease> MissingCatalogParentPermitV1<'lease> {
    #[allow(dead_code, reason = "R2-C2 revalidates the one missing-parent edge")]
    pub(in crate::checked_artifact) fn revalidate_target_binding(
        &self,
    ) -> Result<(), CheckedFsError> {
        provider::revalidate_lease_root_binding(&self._catalog_target, &self._retained_root)
    }

    pub(in crate::checked_artifact) fn revalidate_observation(&self) -> Result<(), CheckedFsError> {
        provider::revalidate_missing_observation(
            &self._catalog_target,
            &self._retained_root,
            self._missing_parent_observation_digest,
        )
    }

    pub(in crate::checked_artifact) const fn observation_digest(
        &self,
    ) -> MissingParentObservationDigestV1 {
        self._missing_parent_observation_digest
    }

    pub(in crate::checked_artifact) fn execute_create_and_retry(
        self: Box<Self>,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        self.revalidate_observation()?;
        provider::create_git_private_parent(&self._retained_root)?;
        let Self {
            _catalog_target,
            _retained_root: _,
            _missing_parent_observation_digest: _,
        } = *self;
        Ok(_catalog_target)
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
