//! Lease-owned target witness consumed by pre-catalog authority.

use std::path::{Path, PathBuf};

use super::{CatalogMutationLeaseSourceV1, CatalogMutationLeaseV1};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1, HostPlatform,
    PathComponentMode, PathEquivalenceProvider, PreCatalogRootKindV1, SupportedFilesystemProfile,
};

pub(crate) struct CatalogLeaseTargetWitnessV1<'lease> {
    lease: CatalogMutationLeaseV1<'lease>,
}

pub(in crate::checked_artifact) struct CatalogLeaseTargetFactsV1 {
    root_kind: PreCatalogRootKindV1,
    support_profile: SupportedFilesystemProfile,
    durable_identity: DurableObjectIdentityV1,
    invocation_identity: Vec<u8>,
    rename_domain: Vec<u8>,
    mode: PathComponentMode,
    canonical_target_path: PathBuf,
    related_git_directory_path: PathBuf,
    related_git_durable_identity: DurableObjectIdentityV1,
    related_git_invocation_identity: Vec<u8>,
    related_git_rename_domain: Vec<u8>,
    related_git_mode: PathComponentMode,
}

impl<'lease> CatalogLeaseTargetWitnessV1<'lease> {
    pub(super) fn try_new(lease: CatalogMutationLeaseV1<'lease>) -> Result<Self, CheckedFsError> {
        let witness = Self { lease };
        witness.revalidate()?;
        Ok(witness)
    }

    pub(in crate::checked_artifact) fn revalidate(&self) -> Result<(), CheckedFsError> {
        match self.lease.source {
            CatalogMutationLeaseSourceV1::WorkspaceRuntime(runtime) => {
                runtime.revalidate_catalog_target()
            }
            CatalogMutationLeaseSourceV1::LeaseSet(held) => held.revalidate_held(),
        }
    }

    pub(in crate::checked_artifact) fn facts(
        &self,
    ) -> Result<CatalogLeaseTargetFactsV1, CheckedFsError> {
        self.revalidate()?;
        let platform = HostPlatform;
        match self.lease.source {
            CatalogMutationLeaseSourceV1::WorkspaceRuntime(runtime) => {
                let target = platform.dir_identity(runtime.workspace_root_handle())?;
                let related_git = platform.dir_identity(runtime.workspace_git_dir_handle())?;
                Ok(CatalogLeaseTargetFactsV1 {
                    root_kind: PreCatalogRootKindV1::Workspace,
                    support_profile: platform.support_profile(),
                    durable_identity: target.durable().clone(),
                    invocation_identity: target.invocation().clone(),
                    rename_domain: platform.rename_domain(runtime.workspace_root_handle())?,
                    mode: platform.parent_mode(runtime.workspace_root_handle())?,
                    canonical_target_path: runtime.workspace_root_path().to_path_buf(),
                    related_git_directory_path: runtime.workspace_git_dir_path().to_path_buf(),
                    related_git_durable_identity: related_git.durable().clone(),
                    related_git_invocation_identity: related_git.invocation().clone(),
                    related_git_rename_domain: platform
                        .rename_domain(runtime.workspace_git_dir_handle())?,
                    related_git_mode: platform.parent_mode(runtime.workspace_git_dir_handle())?,
                })
            }
            CatalogMutationLeaseSourceV1::LeaseSet(held) => {
                let target = &held.target;
                Ok(CatalogLeaseTargetFactsV1 {
                    root_kind: target.binding.root_kind,
                    support_profile: target.binding.support_profile,
                    durable_identity: target.binding.durable_identity.clone(),
                    invocation_identity: target.binding.target_invocation_identity.clone(),
                    rename_domain: target.binding.target_rename_domain.clone(),
                    mode: target.binding.target_mode,
                    canonical_target_path: target.binding.canonical_path.clone(),
                    related_git_directory_path: target.binding.related_git_directory.clone(),
                    related_git_durable_identity: target
                        .binding
                        .related_git_durable_identity
                        .clone(),
                    related_git_invocation_identity: target
                        .binding
                        .related_git_invocation_identity
                        .clone(),
                    related_git_rename_domain: target.binding.related_git_rename_domain.clone(),
                    related_git_mode: target.binding.related_git_mode,
                })
            }
        }
    }

    #[cfg(test)]
    pub(super) fn revalidate_for_test(&self) -> Result<(), CheckedFsError> {
        self.revalidate()
    }

    #[cfg(test)]
    pub(super) fn root_kind_for_test(&self) -> Result<PreCatalogRootKindV1, CheckedFsError> {
        Ok(self.facts()?.root_kind())
    }

    #[cfg(test)]
    pub(super) fn canonical_target_path_for_test(&self) -> Result<PathBuf, CheckedFsError> {
        Ok(self.facts()?.canonical_target_path().to_path_buf())
    }
}

impl CatalogLeaseTargetFactsV1 {
    pub(in crate::checked_artifact) const fn root_kind(&self) -> PreCatalogRootKindV1 {
        self.root_kind
    }

    pub(in crate::checked_artifact) const fn support_profile(&self) -> SupportedFilesystemProfile {
        self.support_profile
    }

    pub(in crate::checked_artifact) fn durable_identity(&self) -> &DurableObjectIdentityV1 {
        &self.durable_identity
    }

    pub(in crate::checked_artifact) fn invocation_identity(&self) -> &[u8] {
        &self.invocation_identity
    }

    pub(in crate::checked_artifact) fn rename_domain(&self) -> &[u8] {
        &self.rename_domain
    }

    pub(in crate::checked_artifact) const fn mode(&self) -> PathComponentMode {
        self.mode
    }

    pub(in crate::checked_artifact) fn canonical_target_path(&self) -> &Path {
        &self.canonical_target_path
    }

    pub(in crate::checked_artifact) fn related_git_directory_path(&self) -> &Path {
        &self.related_git_directory_path
    }

    pub(in crate::checked_artifact) fn related_git_durable_identity(
        &self,
    ) -> &DurableObjectIdentityV1 {
        &self.related_git_durable_identity
    }

    pub(in crate::checked_artifact) fn related_git_invocation_identity(&self) -> &[u8] {
        &self.related_git_invocation_identity
    }

    pub(in crate::checked_artifact) fn related_git_rename_domain(&self) -> &[u8] {
        &self.related_git_rename_domain
    }

    pub(in crate::checked_artifact) const fn related_git_mode(&self) -> PathComponentMode {
        self.related_git_mode
    }
}
