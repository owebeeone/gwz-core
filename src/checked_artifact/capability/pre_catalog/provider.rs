//! Owner-private raw pre-catalog provider seam.
//!
//! Platform implementations belong in this module subtree. Keeping the trait
//! private prevents other checked-artifact siblings from self-issuing permits.

use super::*;

mod filesystem;
mod index;
mod namespace;
mod platform;
mod retained;
mod snapshot;

pub(super) use filesystem::{RetainedPlatformRoot, platform_pre_catalog_owner};

pub(super) struct RawPreCatalogObservationV1<RetainedRoot> {
    pub(super) retained_root: RetainedRoot,
    pub(super) support_profile: SupportedFilesystemProfile,
    pub(super) root_identity: DurableObjectIdentityV1,
    pub(super) root_invocation_identity: Vec<u8>,
    pub(super) rename_domain: Vec<u8>,
    pub(super) path_profile: CanonicalPathIdentityV1,
    pub(super) collision_snapshot_digest: [u8; 32],
}

pub(super) trait RawPreCatalogProviderV1<Root: ?Sized, RetainedRoot> {
    fn inspect_workspace(
        &self,
        root: &Root,
    ) -> Result<RawPreCatalogObservationV1<RetainedRoot>, CheckedFsError>;

    fn inspect_git_directory(
        &self,
        root: &Root,
    ) -> Result<RawPreCatalogObservationV1<RetainedRoot>, CheckedFsError>;

    fn revalidate_workspace(
        &self,
        root: &Root,
        permit: &PreCatalogPermitV1<RetainedRoot>,
    ) -> Result<(), CheckedFsError>;

    fn revalidate_git_directory(
        &self,
        root: &Root,
        permit: &PreCatalogPermitV1<RetainedRoot>,
    ) -> Result<(), CheckedFsError>;
}

#[cfg(test)]
pub(in crate::checked_artifact) use test_support::{
    SyntheticPreCatalogProbeV1, synthetic_pre_catalog_owner,
};

#[cfg(test)]
mod test_support {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct ProbeState {
        events: Vec<&'static str>,
        reject_revalidation: bool,
        change_snapshot_after_observe: bool,
        collision_epoch: u8,
    }

    #[derive(Clone, Default)]
    pub(in crate::checked_artifact) struct SyntheticPreCatalogProbeV1 {
        state: Arc<Mutex<ProbeState>>,
    }

    impl SyntheticPreCatalogProbeV1 {
        pub(in crate::checked_artifact) fn events(&self) -> Vec<&'static str> {
            self.state.lock().unwrap().events.clone()
        }

        pub(in crate::checked_artifact) fn clear_events(&self) {
            self.state.lock().unwrap().events.clear();
        }

        pub(in crate::checked_artifact) fn reject_revalidation(&self) {
            self.state.lock().unwrap().reject_revalidation = true;
        }

        pub(in crate::checked_artifact) fn change_snapshot_after_observe(&self) {
            self.state.lock().unwrap().change_snapshot_after_observe = true;
        }

        pub(in crate::checked_artifact) fn note_bootstrap(&self) {
            self.state.lock().unwrap().events.push("bootstrap");
        }
    }

    struct SyntheticProvider<RetainedRoot> {
        retained_root: RetainedRoot,
        support_profile: SupportedFilesystemProfile,
        root_identity: DurableObjectIdentityV1,
        root_invocation_identity: Vec<u8>,
        rename_domain: Vec<u8>,
        path_profile: CanonicalPathIdentityV1,
        probe: SyntheticPreCatalogProbeV1,
    }

    impl<RetainedRoot> RawPreCatalogProviderV1<Path, RetainedRoot> for SyntheticProvider<RetainedRoot>
    where
        RetainedRoot: Clone + Eq,
    {
        fn inspect_workspace(
            &self,
            _root: &Path,
        ) -> Result<RawPreCatalogObservationV1<RetainedRoot>, CheckedFsError> {
            self.observe(PreCatalogRootKindV1::Workspace)
        }

        fn inspect_git_directory(
            &self,
            _root: &Path,
        ) -> Result<RawPreCatalogObservationV1<RetainedRoot>, CheckedFsError> {
            self.observe(PreCatalogRootKindV1::GitDirectory)
        }

        fn revalidate_workspace(
            &self,
            _root: &Path,
            permit: &PreCatalogPermitV1<RetainedRoot>,
        ) -> Result<(), CheckedFsError> {
            self.revalidate(PreCatalogRootKindV1::Workspace, permit)
        }

        fn revalidate_git_directory(
            &self,
            _root: &Path,
            permit: &PreCatalogPermitV1<RetainedRoot>,
        ) -> Result<(), CheckedFsError> {
            self.revalidate(PreCatalogRootKindV1::GitDirectory, permit)
        }
    }

    impl<RetainedRoot> SyntheticProvider<RetainedRoot>
    where
        RetainedRoot: Clone + Eq,
    {
        fn observe(
            &self,
            root_kind: PreCatalogRootKindV1,
        ) -> Result<RawPreCatalogObservationV1<RetainedRoot>, CheckedFsError> {
            let mut state = self.probe.state.lock().unwrap();
            state.events.push(match root_kind {
                PreCatalogRootKindV1::Workspace => "observe-workspace",
                PreCatalogRootKindV1::GitDirectory => "observe-git-directory",
            });
            let collision_snapshot_digest = synthetic_snapshot(root_kind, state.collision_epoch);
            if state.change_snapshot_after_observe {
                state.collision_epoch = state.collision_epoch.wrapping_add(1);
            }
            Ok(RawPreCatalogObservationV1 {
                retained_root: self.retained_root.clone(),
                support_profile: self.support_profile,
                root_identity: self.root_identity.clone(),
                root_invocation_identity: self.root_invocation_identity.clone(),
                rename_domain: self.rename_domain.clone(),
                path_profile: self.path_profile.clone(),
                collision_snapshot_digest,
            })
        }

        fn revalidate(
            &self,
            root_kind: PreCatalogRootKindV1,
            permit: &PreCatalogPermitV1<RetainedRoot>,
        ) -> Result<(), CheckedFsError> {
            let mut state = self.probe.state.lock().unwrap();
            state.events.push(match root_kind {
                PreCatalogRootKindV1::Workspace => "revalidate-workspace",
                PreCatalogRootKindV1::GitDirectory => "revalidate-git-directory",
            });
            if state.reject_revalidation
                || permit.retained_root() != &self.retained_root
                || permit.root_identity() != &self.root_identity
                || permit.root_invocation_identity() != self.root_invocation_identity
                || permit.rename_domain() != self.rename_domain
                || permit.path_profile() != &self.path_profile
                || permit.root_kind() != root_kind
                || permit.collision_snapshot_digest()
                    != synthetic_snapshot(root_kind, state.collision_epoch)
            {
                return Err(CheckedFsError::ambiguous(
                    "pre-catalog snapshot",
                    "retained path or collision facts changed",
                ));
            }
            Ok(())
        }
    }

    fn synthetic_snapshot(root_kind: PreCatalogRootKindV1, epoch: u8) -> [u8; 32] {
        let kind = match root_kind {
            PreCatalogRootKindV1::Workspace => 0x40,
            PreCatalogRootKindV1::GitDirectory => 0x80,
        };
        [kind ^ epoch; 32]
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "test setup mirrors the one raw observation"
    )]
    pub(in crate::checked_artifact) fn synthetic_pre_catalog_owner<RetainedRoot>(
        retained_root: RetainedRoot,
        support_profile: SupportedFilesystemProfile,
        root_identity: DurableObjectIdentityV1,
        root_invocation_identity: Vec<u8>,
        rename_domain: Vec<u8>,
        path_profile: CanonicalPathIdentityV1,
    ) -> (
        PreCatalogOwnerV1<Path, RetainedRoot>,
        SyntheticPreCatalogProbeV1,
    )
    where
        RetainedRoot: Clone + Eq + 'static,
    {
        let probe = SyntheticPreCatalogProbeV1::default();
        let provider = SyntheticProvider {
            retained_root,
            support_profile,
            root_identity,
            root_invocation_identity,
            rename_domain,
            path_profile,
            probe: probe.clone(),
        };
        (PreCatalogOwnerV1::from_provider(provider), probe)
    }
}

#[cfg(test)]
mod production_tests;
