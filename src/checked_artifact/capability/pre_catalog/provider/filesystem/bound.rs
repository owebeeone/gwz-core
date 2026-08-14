use super::*;

pub(in crate::checked_artifact::capability::pre_catalog) fn inspect_bound_catalog_target<'lease>(
    target: CatalogLeaseTargetWitnessV1<'lease>,
) -> Result<LeaseBoundPreCatalogObservationV1<'lease>, CheckedFsError> {
    platform_pre_catalog_provider().inspect_bound_catalog_target(target)
}

pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate_lease_root_binding(
    target: &CatalogLeaseTargetWitnessV1<'_>,
    root: &RetainedPlatformRoot,
) -> Result<(), CheckedFsError> {
    platform_pre_catalog_provider().revalidate_lease_root_binding(target, root)
}

pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate_ready_observation(
    target: &CatalogLeaseTargetWitnessV1<'_>,
    root: &RetainedPlatformRoot,
    expected: FreshObservationDigestV1,
) -> Result<(), CheckedFsError> {
    platform_pre_catalog_provider().revalidate_ready_observation(target, root, expected)
}

pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate_missing_observation(
    target: &CatalogLeaseTargetWitnessV1<'_>,
    root: &RetainedPlatformRoot,
    expected: MissingParentObservationDigestV1,
) -> Result<(), CheckedFsError> {
    platform_pre_catalog_provider().revalidate_missing_observation(target, root, expected)
}

impl<P: PlatformProviderV1 + 'static> FilesystemPreCatalogProvider<P> {
    pub(in crate::checked_artifact::capability::pre_catalog) fn inspect_bound_catalog_target<
        'lease,
    >(
        &self,
        target: CatalogLeaseTargetWitnessV1<'lease>,
    ) -> Result<LeaseBoundPreCatalogObservationV1<'lease>, CheckedFsError> {
        let facts = target.facts()?;
        let observation = match facts.root_kind() {
            PreCatalogRootKindV1::Workspace => {
                self.inspect_workspace(facts.canonical_target_path())?
            }
            PreCatalogRootKindV1::GitDirectory => {
                self.inspect_git_directory(facts.canonical_target_path())?
            }
        };
        self.revalidate_bound_target(&target, &observation)?;
        Ok(LeaseBoundPreCatalogObservationV1 {
            target,
            observation,
        })
    }

    pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate_bound_target(
        &self,
        target: &CatalogLeaseTargetWitnessV1<'_>,
        observation: &RawPreCatalogObservationV1<RetainedPlatformRoot>,
    ) -> Result<(), CheckedFsError> {
        let facts = target.facts()?;
        match facts.root_kind() {
            PreCatalogRootKindV1::Workspace => {
                self.revalidate_workspace(facts.canonical_target_path(), observation)?;
            }
            PreCatalogRootKindV1::GitDirectory => {
                self.revalidate_git_directory(facts.canonical_target_path(), observation)?;
            }
        }
        let root = &observation.retained_root;
        self.revalidate_lease_root_binding(target, root)?;
        let repository = root.repository();
        let path_component = observation
            .path_profile
            .components()
            .first()
            .ok_or_else(|| CheckedFsError::ambiguous("lease-bound target", "empty path profile"))?;
        if observation.support_profile != facts.support_profile()
            || &observation.root_identity != facts.durable_identity()
            || observation.root_invocation_identity != facts.invocation_identity()
            || observation.rename_domain != facts.rename_domain()
            || path_component.parent_mode() != facts.mode()
            || repository.identity().durable() != facts.related_git_durable_identity()
            || repository.identity().invocation() != facts.related_git_invocation_identity()
            || repository.rename_domain() != facts.related_git_rename_domain()
            || repository.mode() != facts.related_git_mode()
        {
            return Err(CheckedFsError::ambiguous(
                "lease-bound target",
                "provider root does not match the retained lease target and repository relationship",
            ));
        }
        target.revalidate()
    }

    pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate_lease_root_binding(
        &self,
        target: &CatalogLeaseTargetWitnessV1<'_>,
        root: &RetainedPlatformRoot,
    ) -> Result<(), CheckedFsError> {
        target.revalidate()?;
        root.revalidate(&self.platform)?;
        let facts = target.facts()?;
        let repository = root.repository();
        let paths_match = match facts.root_kind() {
            PreCatalogRootKindV1::Workspace => {
                root.root_path() == facts.canonical_target_path()
                    && root.git_directory_path() == facts.related_git_directory_path()
            }
            PreCatalogRootKindV1::GitDirectory => {
                root.root_path() == facts.canonical_target_path()
                    && root.git_directory_path() == facts.canonical_target_path()
                    && root.common_directory_path() == facts.canonical_target_path()
                    && facts.related_git_directory_path() == facts.canonical_target_path()
            }
        };
        if !paths_match
            || self.platform.support_profile() != facts.support_profile()
            || root.root().identity().durable() != facts.durable_identity()
            || root.root().identity().invocation() != facts.invocation_identity()
            || root.root().rename_domain() != facts.rename_domain()
            || root.root().mode() != facts.mode()
            || repository.identity().durable() != facts.related_git_durable_identity()
            || repository.identity().invocation() != facts.related_git_invocation_identity()
            || repository.rename_domain() != facts.related_git_rename_domain()
            || repository.mode() != facts.related_git_mode()
        {
            return Err(CheckedFsError::ambiguous(
                "lease-bound target",
                "retained provider root does not match the locked catalog target",
            ));
        }
        target.revalidate()
    }

    pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate_ready_observation(
        &self,
        target: &CatalogLeaseTargetWitnessV1<'_>,
        root: &RetainedPlatformRoot,
        expected: FreshObservationDigestV1,
    ) -> Result<(), CheckedFsError> {
        self.revalidate_lease_root_binding(target, root)?;
        let observed = self.observe_for_target(target)?;
        let actual = observed
            .ready_digests
            .ok_or_else(|| {
                CheckedFsError::ambiguous(
                    "catalog ready observation",
                    "retained private parent disappeared",
                )
            })?
            .fresh;
        if actual != expected {
            return Err(CheckedFsError::ambiguous(
                "catalog ready observation",
                "fresh collision or reserved-role facts changed",
            ));
        }
        target.revalidate()
    }

    pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate_missing_observation(
        &self,
        target: &CatalogLeaseTargetWitnessV1<'_>,
        root: &RetainedPlatformRoot,
        expected: MissingParentObservationDigestV1,
    ) -> Result<(), CheckedFsError> {
        self.revalidate_lease_root_binding(target, root)?;
        let observed = self.observe_for_target(target)?;
        if observed.missing_parent_digest != Some(expected) {
            return Err(CheckedFsError::ambiguous(
                "missing catalog parent observation",
                "missing-parent proof changed",
            ));
        }
        target.revalidate()
    }

    fn observe_for_target(
        &self,
        target: &CatalogLeaseTargetWitnessV1<'_>,
    ) -> Result<Observed, CheckedFsError> {
        let facts = target.facts()?;
        match facts.root_kind() {
            PreCatalogRootKindV1::Workspace => {
                self.observe_workspace(facts.canonical_target_path())
            }
            PreCatalogRootKindV1::GitDirectory => {
                self.observe_git_directory(facts.canonical_target_path())
            }
        }
    }
}
