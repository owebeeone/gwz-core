use std::path::Path;

#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

use cap_std::fs::{Dir, File};

use super::super::*;
use super::snapshot::{CollisionModes, SnapshotParts};
use super::{
    RawCatalogRoleObservationV1, RawPreCatalogObservationV1, RawPreCatalogProviderV1, index,
    namespace, retained, snapshot,
};
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
pub(in crate::checked_artifact::capability::pre_catalog) fn platform_pre_catalog_provider()
-> FilesystemPreCatalogProvider<HostPlatform> {
    FilesystemPreCatalogProvider {
        platform: HostPlatform,
        #[cfg(test)]
        hook: None,
    }
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
