use std::path::Path;

#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

use cap_std::fs::{Dir, File};

use super::super::*;
use super::snapshot::{CollisionModes, SnapshotParts};
use super::{
    LeaseBoundPreCatalogObservationV1, RawCatalogRoleObservationV1, RawPreCatalogObservationV1,
    RawPreCatalogProviderV1, index, namespace, retained, snapshot,
};
use crate::checked_artifact::bootstrap::CatalogLeaseTargetWitnessV1;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalComponent, DurableIdentityProvider, PathEquivalenceProvider,
    PrivateControlDomain,
};
use crate::checked_artifact::catalog_names::CatalogPrivateRootV1;

use super::platform::HostPlatform;
pub(in crate::checked_artifact::capability::pre_catalog) use super::retained::RetainedPlatformRoot;

pub(super) trait PlatformProviderV1:
    PathEquivalenceProvider<Dir>
    + DurableIdentityProvider<Dir, File, InvocationIdentity = Vec<u8>, RenameDomain = Vec<u8>>
    + Send
    + Sync
{
}

impl<T> PlatformProviderV1 for T where
    T: PathEquivalenceProvider<Dir>
        + DurableIdentityProvider<Dir, File, InvocationIdentity = Vec<u8>, RenameDomain = Vec<u8>>
        + Send
        + Sync
{
}

pub(super) struct FilesystemPreCatalogProvider<P> {
    platform: P,
    #[cfg(test)]
    hook: Option<TestHook>,
}

#[cfg(test)]
struct TestHook {
    callback: Arc<dyn Fn() + Send + Sync>,
    fired: AtomicBool,
}

struct Observed {
    retained_root: RetainedPlatformRoot,
    support_profile: SupportedFilesystemProfile,
    root_identity: DurableObjectIdentityV1,
    root_invocation_identity: Vec<u8>,
    rename_domain: Vec<u8>,
    path_profile: CanonicalPathIdentityV1,
    collision_snapshot_digest: [u8; 32],
    raw_roles: RawCatalogRoleObservationV1,
}

#[allow(
    dead_code,
    reason = "R2-C0 freezes the retained provider before C1 issues preflight permits"
)]
pub(super) fn platform_pre_catalog_provider() -> FilesystemPreCatalogProvider<HostPlatform> {
    FilesystemPreCatalogProvider {
        platform: HostPlatform,
        #[cfg(test)]
        hook: None,
    }
}

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

#[cfg(test)]
pub(super) fn filesystem_provider_for_test(
    platform: impl PlatformProviderV1 + 'static,
) -> FilesystemPreCatalogProvider<impl PlatformProviderV1> {
    filesystem_provider_with_hook_for_test(platform, None)
}

#[cfg(test)]
pub(super) fn filesystem_provider_with_hook_for_test<P>(
    platform: P,
    hook: Option<Arc<dyn Fn() + Send + Sync>>,
) -> FilesystemPreCatalogProvider<P>
where
    P: PlatformProviderV1 + 'static,
{
    FilesystemPreCatalogProvider {
        platform,
        hook: hook.map(|callback| TestHook {
            callback,
            fired: AtomicBool::new(false),
        }),
    }
}

impl<P> RawPreCatalogProviderV1<Path, RetainedPlatformRoot> for FilesystemPreCatalogProvider<P>
where
    P: PlatformProviderV1 + 'static,
{
    fn inspect_workspace(
        &self,
        root: &Path,
    ) -> Result<RawPreCatalogObservationV1<RetainedPlatformRoot>, CheckedFsError> {
        let observed = self.observe_workspace(root)?;
        self.run_hook();
        Ok(observed.into_raw())
    }

    fn inspect_git_directory(
        &self,
        root: &Path,
    ) -> Result<RawPreCatalogObservationV1<RetainedPlatformRoot>, CheckedFsError> {
        let observed = self.observe_git_directory(root)?;
        self.run_hook();
        Ok(observed.into_raw())
    }

    fn revalidate_workspace(
        &self,
        root: &Path,
        observation: &RawPreCatalogObservationV1<RetainedPlatformRoot>,
    ) -> Result<(), CheckedFsError> {
        observation.retained_root.revalidate(&self.platform)?;
        self.compare(self.observe_workspace(root)?, observation)
    }

    fn revalidate_git_directory(
        &self,
        root: &Path,
        observation: &RawPreCatalogObservationV1<RetainedPlatformRoot>,
    ) -> Result<(), CheckedFsError> {
        observation.retained_root.revalidate(&self.platform)?;
        self.compare(self.observe_git_directory(root)?, observation)
    }
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

    #[cfg(test)]
    pub(super) fn observe_and_revalidate_workspace_for_test(
        &self,
        root: &Path,
    ) -> Result<RawPreCatalogObservationV1<RetainedPlatformRoot>, CheckedFsError> {
        let observation = self.inspect_workspace(root)?;
        self.revalidate_workspace(root, &observation)?;
        Ok(observation)
    }

    #[cfg(test)]
    pub(super) fn observe_and_revalidate_git_directory_for_test(
        &self,
        root: &Path,
    ) -> Result<RawPreCatalogObservationV1<RetainedPlatformRoot>, CheckedFsError> {
        let observation = self.inspect_git_directory(root)?;
        self.revalidate_git_directory(root, &observation)?;
        Ok(observation)
    }

    fn observe_workspace(&self, root: &Path) -> Result<Observed, CheckedFsError> {
        let mut retained = retained::retain_workspace(root, &self.platform)?;
        let (index, index_file) = index::observe(&retained, &self.platform)?;
        retained.install_index(index_file);
        let root_kind = PreCatalogRootKindV1::Workspace;
        let private_root = CatalogPrivateRootV1::Workspace;
        let domain = PrivateControlDomain::for_root(private_root);
        snapshot::reject_private_collisions(
            &index.entries,
            &domain,
            CollisionModes {
                root: retained.root().mode(),
                private_parent: retained.private_parent().map(|parent| parent.mode()),
            },
        )?;
        self.finish(retained, root_kind, private_root, domain, Some(index))
    }

    fn observe_git_directory(&self, root: &Path) -> Result<Observed, CheckedFsError> {
        let retained = retained::retain_git_directory(root, &self.platform)?;
        self.finish(
            retained,
            PreCatalogRootKindV1::GitDirectory,
            CatalogPrivateRootV1::GitDirectory,
            PrivateControlDomain::for_root(CatalogPrivateRootV1::GitDirectory),
            None,
        )
    }

    fn finish(
        &self,
        retained: RetainedPlatformRoot,
        root_kind: PreCatalogRootKindV1,
        private_root: CatalogPrivateRootV1,
        domain: PrivateControlDomain,
        index: Option<snapshot::IndexSnapshotFacts>,
    ) -> Result<Observed, CheckedFsError> {
        let container = match private_root {
            CatalogPrivateRootV1::Workspace => b".gwz".as_slice(),
            CatalogPrivateRootV1::GitDirectory => b"gwz".as_slice(),
        };
        let root_identity = retained.root().identity().durable().clone();
        let root_invocation_identity = retained.root().identity().invocation().clone();
        let rename_domain = retained.root().rename_domain().to_vec();
        let path_profile = CanonicalPathIdentityV1::new(vec![CanonicalComponent::try_bound(
            AsciiComponent::parse(container)?,
            retained.root().mode(),
            root_identity.clone(),
            root_invocation_identity.clone(),
            rename_domain.clone(),
        )?])?;
        let namespace = namespace::observe(&retained, &self.platform, private_root)?;
        let private_parent_fact = retained
            .private_parent()
            .map(super::retained::RetainedDirectory::encoded_snapshot_fact);
        let collision_snapshot_digest = snapshot::digest(SnapshotParts {
            root_kind,
            domain: &domain,
            root_identity: &retained.root().encoded_identity(),
            repository_identity: &retained.repository().encoded_identity(),
            common_directory_identity: &retained.common_directory().encoded_identity(),
            private_parent_fact: private_parent_fact.as_deref(),
            path_profile: &path_profile,
            index: index.as_ref(),
            namespace: &namespace,
        });
        Ok(Observed {
            retained_root: retained,
            support_profile: self.platform.support_profile(),
            root_identity,
            root_invocation_identity,
            rename_domain,
            path_profile,
            collision_snapshot_digest,
            raw_roles: RawCatalogRoleObservationV1 { rows: namespace },
        })
    }

    fn compare(
        &self,
        observed: Observed,
        expected: &RawPreCatalogObservationV1<RetainedPlatformRoot>,
    ) -> Result<(), CheckedFsError> {
        if observed.support_profile != expected.support_profile
            || observed.root_identity != expected.root_identity
            || observed.root_invocation_identity != expected.root_invocation_identity
            || observed.rename_domain != expected.rename_domain
            || observed.path_profile != expected.path_profile
            || observed.collision_snapshot_digest != expected.collision_snapshot_digest
            || observed.raw_roles != expected.raw_roles
            || observed.retained_root.root_path() != expected.retained_root.root_path()
            || observed.retained_root.git_directory_path()
                != expected.retained_root.git_directory_path()
            || observed.retained_root.common_directory_path()
                != expected.retained_root.common_directory_path()
        {
            return Err(CheckedFsError::ambiguous(
                "pre-catalog snapshot",
                "retained filesystem, repository, index, or namespace facts changed",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn run_hook(&self) {
        if let Some(hook) = &self.hook
            && !hook.fired.swap(true, Ordering::SeqCst)
        {
            (hook.callback)();
        }
    }

    #[cfg(not(test))]
    fn run_hook(&self) {}
}

impl Observed {
    fn into_raw(self) -> RawPreCatalogObservationV1<RetainedPlatformRoot> {
        RawPreCatalogObservationV1 {
            retained_root: self.retained_root,
            support_profile: self.support_profile,
            root_identity: self.root_identity,
            root_invocation_identity: self.root_invocation_identity,
            rename_domain: self.rename_domain,
            path_profile: self.path_profile,
            collision_snapshot_digest: self.collision_snapshot_digest,
            raw_roles: self.raw_roles,
        }
    }
}
