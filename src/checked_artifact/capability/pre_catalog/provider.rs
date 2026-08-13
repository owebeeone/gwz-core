//! Owner-private raw pre-catalog provider seam.
//!
//! Platform implementations belong in this module subtree. Keeping the trait
//! private prevents other checked-artifact siblings from self-issuing permits.

use super::*;

pub(super) struct RawPreCatalogObservationV1<RetainedRoot> {
    pub(super) retained_root: RetainedRoot,
    pub(super) support_profile: SupportedFilesystemProfile,
    pub(super) root_identity: DurableObjectIdentityV1,
    pub(super) root_invocation_identity: Vec<u8>,
    pub(super) rename_domain: Vec<u8>,
    pub(super) path_profile: CanonicalPathIdentityV1,
}

pub(super) trait RawPreCatalogProviderV1<Root: ?Sized, RetainedRoot> {
    fn inspect_and_scan(
        &self,
        root: &Root,
        root_kind: PreCatalogRootKindV1,
        domain: &PrivateControlDomain,
        index: &[LosslessIndexEntry],
        worktree: &[TrackedWorktreeEntry],
    ) -> Result<RawPreCatalogObservationV1<RetainedRoot>, CheckedFsError>;

    fn revalidate(
        &self,
        root: &Root,
        permit: &PreCatalogPermitV1<RetainedRoot>,
    ) -> Result<(), CheckedFsError>;
}

#[cfg(test)]
mod test_support {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct ProbeState {
        events: Vec<&'static str>,
        reject_revalidation: bool,
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
        fn inspect_and_scan(
            &self,
            _root: &Path,
            _root_kind: PreCatalogRootKindV1,
            _domain: &PrivateControlDomain,
            _index: &[LosslessIndexEntry],
            _worktree: &[TrackedWorktreeEntry],
        ) -> Result<RawPreCatalogObservationV1<RetainedRoot>, CheckedFsError> {
            self.probe.state.lock().unwrap().events.push("observe");
            Ok(RawPreCatalogObservationV1 {
                retained_root: self.retained_root.clone(),
                support_profile: self.support_profile,
                root_identity: self.root_identity.clone(),
                root_invocation_identity: self.root_invocation_identity.clone(),
                rename_domain: self.rename_domain.clone(),
                path_profile: self.path_profile.clone(),
            })
        }

        fn revalidate(
            &self,
            _root: &Path,
            permit: &PreCatalogPermitV1<RetainedRoot>,
        ) -> Result<(), CheckedFsError> {
            let mut state = self.probe.state.lock().unwrap();
            state.events.push("revalidate");
            if state.reject_revalidation
                || permit.retained_root() != &self.retained_root
                || permit.root_identity() != &self.root_identity
                || permit.root_invocation_identity() != self.root_invocation_identity
                || permit.rename_domain() != self.rename_domain
                || permit.path_profile() != &self.path_profile
            {
                return Err(CheckedFsError::ambiguous("retained path", "replaced"));
            }
            Ok(())
        }
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
pub(in crate::checked_artifact) use test_support::{
    SyntheticPreCatalogProbeV1, synthetic_pre_catalog_owner,
};

#[allow(
    dead_code,
    reason = "always-compiled proof that a platform provider fits the sealed owner seam"
)]
mod production_provider_compile {
    use std::path::Path;

    use super::*;

    struct PlatformRetainedRoot;
    struct PlatformProvider;

    impl RawPreCatalogProviderV1<Path, PlatformRetainedRoot> for PlatformProvider {
        fn inspect_and_scan(
            &self,
            _root: &Path,
            _root_kind: PreCatalogRootKindV1,
            _domain: &PrivateControlDomain,
            _index: &[LosslessIndexEntry],
            _worktree: &[TrackedWorktreeEntry],
        ) -> Result<RawPreCatalogObservationV1<PlatformRetainedRoot>, CheckedFsError> {
            Err(CheckedFsError::ambiguous(
                "compile-only platform provider",
                "not executed",
            ))
        }

        fn revalidate(
            &self,
            _root: &Path,
            _permit: &PreCatalogPermitV1<PlatformRetainedRoot>,
        ) -> Result<(), CheckedFsError> {
            Err(CheckedFsError::ambiguous(
                "compile-only platform provider",
                "not executed",
            ))
        }
    }

    struct PlatformBootstrap;

    impl crate::checked_artifact::bootstrap::CatalogBootstrapV1<PlatformRetainedRoot>
        for PlatformBootstrap
    {
        type Catalog = ();

        fn recover_or_create(
            &self,
            _permit: RevalidatedPreCatalogPermitV1<'_, PlatformRetainedRoot>,
        ) -> Result<Self::Catalog, CheckedFsError> {
            Ok(())
        }
    }

    fn compile_provider(root: &Path) -> Result<(), CheckedFsError> {
        PreCatalogOwnerV1::from_provider(PlatformProvider).recover_or_create(
            root,
            PreCatalogRootKindV1::Workspace,
            [1; 32],
            &PrivateControlDomain::checked_v1(),
            &[],
            &[],
            &PlatformBootstrap,
        )
    }

    #[cfg(test)]
    #[test]
    fn production_shaped_provider_can_be_owned_without_exposing_raw_seam() {
        assert!(compile_provider(Path::new(".")).is_err());
    }
}
