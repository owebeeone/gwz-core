use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use cap_std::fs::Dir;

use super::super::advisory::AdvisoryLock;
use super::super::paths::{
    RetainedDirectory, ensure_child_directory, open_child_directory, open_existing_file,
    open_or_create_file, resolve_workspace_paths, retain_ambient_directory,
    revalidate_ambient_directory, revalidate_child_directory, revalidate_file,
    revalidate_workspace_repository,
};
use super::super::{LOCKS_DIRECTORY_NAME, WORKSPACE_MUTATOR_LOCK_NAME, try_advisory_lock};
use super::alias::reject_equivalent_alias;
use super::association::{CatalogGitAssociationBindingV1, RetainedCatalogGitAssociationV1};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1, HostPlatform,
    PathComponentMode, PathEquivalenceProvider, PreCatalogRootKindV1, SupportedFilesystemProfile,
};

pub(super) const GIT_CATALOG_MUTATOR_LOCK_NAME: &str = "gwz-catalog-mutator-v1.lock";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogLeaseTargetRequestV1 {
    purpose: CatalogLeaseTargetPurposeV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CatalogLeaseTargetPurposeV1 {
    Workspace(PathBuf),
    RepositoryCommonGitDirectory(PathBuf),
}

impl CatalogLeaseTargetRequestV1 {
    pub(in crate::checked_artifact) fn workspace(path: impl Into<PathBuf>) -> Self {
        Self {
            purpose: CatalogLeaseTargetPurposeV1::Workspace(path.into()),
        }
    }

    pub(in crate::checked_artifact) fn repository_common_git_directory(
        repository: impl Into<PathBuf>,
    ) -> Self {
        Self {
            purpose: CatalogLeaseTargetPurposeV1::RepositoryCommonGitDirectory(repository.into()),
        }
    }

    #[cfg(test)]
    pub(super) fn canonical_order_key_for_test(&self) -> Result<Vec<u8>, CheckedFsError> {
        RetainedCatalogTargetV1::retain(self).map(|target| target.binding.order_key)
    }

    #[cfg(test)]
    pub(super) fn canonical_target_path_for_test(&self) -> Result<PathBuf, CheckedFsError> {
        RetainedCatalogTargetV1::retain(self).map(|target| target.binding.canonical_path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CatalogTargetBindingV1 {
    pub(super) root_kind: PreCatalogRootKindV1,
    pub(super) support_profile: SupportedFilesystemProfile,
    pub(super) durable_identity: DurableObjectIdentityV1,
    pub(super) target_invocation_identity: Vec<u8>,
    pub(super) target_rename_domain: Vec<u8>,
    pub(super) target_mode: PathComponentMode,
    pub(super) canonical_path: PathBuf,
    pub(super) related_git_directory: PathBuf,
    pub(super) related_git_durable_identity: DurableObjectIdentityV1,
    pub(super) related_git_invocation_identity: Vec<u8>,
    pub(super) related_git_rename_domain: Vec<u8>,
    pub(super) related_git_mode: PathComponentMode,
    pub(super) order_key: Vec<u8>,
}

pub(super) struct RetainedCatalogTargetV1 {
    pub(super) binding: CatalogTargetBindingV1,
    pub(super) target: RetainedDirectory,
    pub(super) related_git_directory: RetainedDirectory,
    git_association: Option<RetainedCatalogGitAssociationV1>,
}

impl RetainedCatalogTargetV1 {
    pub(super) fn retain(request: &CatalogLeaseTargetRequestV1) -> Result<Self, CheckedFsError> {
        match &request.purpose {
            CatalogLeaseTargetPurposeV1::Workspace(path) => Self::retain_workspace(path),
            CatalogLeaseTargetPurposeV1::RepositoryCommonGitDirectory(path) => {
                Self::retain_repository_common_git_directory(path)
            }
        }
    }

    fn retain_workspace(path: &Path) -> Result<Self, CheckedFsError> {
        let resolved = resolve_workspace_paths(path)?;
        let target =
            retain_ambient_directory(&resolved.workspace_root, "catalog workspace target")?;
        let related_git_directory = retain_ambient_directory(
            &resolved.workspace_git_dir,
            "catalog workspace Git directory",
        )?;
        revalidate_workspace_repository(&resolved.workspace_root, &resolved.workspace_git_dir)?;
        Self::finish(
            PreCatalogRootKindV1::Workspace,
            resolved.workspace_root,
            resolved.workspace_git_dir,
            target,
            related_git_directory,
        )
    }

    fn retain_git_directory(path: &Path) -> Result<Self, CheckedFsError> {
        let canonical_path = canonical_git_directory(path)?;
        let target = retain_ambient_directory(&canonical_path, "catalog Git-directory target")?;
        let related_git_directory =
            retain_ambient_directory(&canonical_path, "catalog Git-directory target")?;
        Self::finish(
            PreCatalogRootKindV1::GitDirectory,
            canonical_path.clone(),
            canonical_path,
            target,
            related_git_directory,
        )
    }

    fn retain_repository_common_git_directory(path: &Path) -> Result<Self, CheckedFsError> {
        let association = RetainedCatalogGitAssociationV1::retain(path)?;
        let mut target = Self::retain_git_directory(association.common_directory_path())?;
        target.git_association = Some(association);
        target.revalidate()?;
        Ok(target)
    }

    fn finish(
        root_kind: PreCatalogRootKindV1,
        canonical_path: PathBuf,
        related_git_directory_path: PathBuf,
        target: RetainedDirectory,
        related_git_directory: RetainedDirectory,
    ) -> Result<Self, CheckedFsError> {
        let platform = HostPlatform;
        let target_fact = platform.dir_identity(target.handle())?;
        let related_git_fact = platform.dir_identity(related_git_directory.handle())?;
        let support_profile = platform.support_profile();
        if target_fact.durable().support_profile() != support_profile
            || related_git_fact.durable().support_profile() != support_profile
        {
            return Err(CheckedFsError::ambiguous(
                "catalog target identity",
                "target or related Git identity does not match the host support profile",
            ));
        }
        let target_mode = platform.parent_mode(target.handle())?;
        let target_rename_domain = platform.rename_domain(target.handle())?;
        let related_git_mode = platform.parent_mode(related_git_directory.handle())?;
        let related_git_rename_domain = platform.rename_domain(related_git_directory.handle())?;
        if target_fact.invocation().is_empty()
            || target_rename_domain.is_empty()
            || related_git_fact.invocation().is_empty()
            || related_git_rename_domain.is_empty()
        {
            return Err(CheckedFsError::ambiguous(
                "catalog target identity",
                "live target and related Git identities and rename domains must be nonempty",
            ));
        }
        let durable_identity = target_fact.durable().clone();
        let order_key = canonical_order_key(support_profile, &durable_identity, root_kind);
        Ok(Self {
            binding: CatalogTargetBindingV1 {
                root_kind,
                support_profile,
                durable_identity,
                target_invocation_identity: target_fact.invocation().clone(),
                target_rename_domain,
                target_mode,
                canonical_path,
                related_git_directory: related_git_directory_path,
                related_git_durable_identity: related_git_fact.durable().clone(),
                related_git_invocation_identity: related_git_fact.invocation().clone(),
                related_git_rename_domain,
                related_git_mode,
                order_key,
            },
            target,
            related_git_directory,
            git_association: None,
        })
    }

    pub(super) fn revalidate(&self) -> Result<(), CheckedFsError> {
        revalidate_ambient_directory(
            &self.binding.canonical_path,
            &self.target,
            "catalog mutation target",
        )?;
        revalidate_ambient_directory(
            &self.binding.related_git_directory,
            &self.related_git_directory,
            "catalog target Git directory",
        )?;
        if self.binding.root_kind == PreCatalogRootKindV1::Workspace {
            revalidate_workspace_repository(
                &self.binding.canonical_path,
                &self.binding.related_git_directory,
            )?;
        } else if canonical_git_directory(&self.binding.canonical_path)?
            != self.binding.canonical_path
        {
            return Err(CheckedFsError::ambiguous(
                "catalog Git-directory target",
                "repository path binding changed",
            ));
        }
        let platform = HostPlatform;
        let target_fact = platform.dir_identity(self.target.handle())?;
        let related_git_fact = platform.dir_identity(self.related_git_directory.handle())?;
        if target_fact.durable() != &self.binding.durable_identity
            || target_fact.invocation() != &self.binding.target_invocation_identity
            || platform.support_profile() != self.binding.support_profile
            || platform.parent_mode(self.target.handle())? != self.binding.target_mode
            || platform.rename_domain(self.target.handle())? != self.binding.target_rename_domain
            || related_git_fact.durable() != &self.binding.related_git_durable_identity
            || related_git_fact.invocation() != &self.binding.related_git_invocation_identity
            || platform.parent_mode(self.related_git_directory.handle())?
                != self.binding.related_git_mode
            || platform.rename_domain(self.related_git_directory.handle())?
                != self.binding.related_git_rename_domain
        {
            return Err(CheckedFsError::ambiguous(
                "catalog mutation target",
                "stable or live target or related Git binding changed",
            ));
        }
        if let Some(association) = &self.git_association {
            association.revalidate()?;
        }
        Ok(())
    }

    pub(super) fn git_association_binding(&self) -> Option<&CatalogGitAssociationBindingV1> {
        self.git_association
            .as_ref()
            .map(RetainedCatalogGitAssociationV1::binding)
    }

    pub(super) fn into_prepared_bindings(
        self,
    ) -> (
        CatalogTargetBindingV1,
        Option<CatalogGitAssociationBindingV1>,
    ) {
        (
            self.binding,
            self.git_association
                .map(RetainedCatalogGitAssociationV1::into_binding),
        )
    }

    pub(super) fn guard_parent(&self) -> &Dir {
        self.related_git_directory.handle()
    }

    pub(super) fn prepare_final_slot(&self) -> Result<(), CheckedFsError> {
        match self.binding.root_kind {
            PreCatalogRootKindV1::Workspace => {
                let runtime = ensure_child_directory(
                    self.target.handle(),
                    OsStr::new(crate::workspace::RUNTIME_DIR),
                    "workspace runtime directory",
                )?;
                let locks = ensure_child_directory(
                    runtime.handle(),
                    OsStr::new(LOCKS_DIRECTORY_NAME),
                    "workspace locks directory",
                )?;
                reject_equivalent_alias(
                    locks.handle(),
                    OsStr::new(WORKSPACE_MUTATOR_LOCK_NAME),
                    "workspace mutator lease",
                )?;
                open_or_create_file(
                    locks.handle(),
                    OsStr::new(WORKSPACE_MUTATOR_LOCK_NAME),
                    "workspace mutator lease",
                )?;
            }
            PreCatalogRootKindV1::GitDirectory => {
                reject_equivalent_alias(
                    self.target.handle(),
                    OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
                    "Git catalog mutator lease",
                )?;
                open_or_create_file(
                    self.target.handle(),
                    OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
                    "Git catalog mutator lease",
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn acquire_final(
        self,
        associated_targets: Vec<RetainedCatalogTargetV1>,
    ) -> Result<Option<HeldCatalogTargetV1>, CheckedFsError> {
        let (runtime_dir, locks_dir, lock_file) = match self.binding.root_kind {
            PreCatalogRootKindV1::Workspace => {
                let runtime = open_child_directory(
                    self.target.handle(),
                    OsStr::new(crate::workspace::RUNTIME_DIR),
                    "workspace runtime directory",
                )?;
                let locks = open_child_directory(
                    runtime.handle(),
                    OsStr::new(LOCKS_DIRECTORY_NAME),
                    "workspace locks directory",
                )?;
                reject_equivalent_alias(
                    locks.handle(),
                    OsStr::new(WORKSPACE_MUTATOR_LOCK_NAME),
                    "workspace mutator lease",
                )?;
                let file = open_existing_file(
                    locks.handle(),
                    OsStr::new(WORKSPACE_MUTATOR_LOCK_NAME),
                    "workspace mutator lease",
                )?;
                (Some(runtime), Some(locks), file)
            }
            PreCatalogRootKindV1::GitDirectory => {
                reject_equivalent_alias(
                    self.target.handle(),
                    OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
                    "Git catalog mutator lease",
                )?;
                let file = open_existing_file(
                    self.target.handle(),
                    OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
                    "Git catalog mutator lease",
                )?;
                (None, None, file)
            }
        };
        #[cfg(test)]
        super::super::fault::run(super::super::fault::RuntimeBootstrapFault::CatalogFinalLeaseOpen);
        let Some(lock) = try_advisory_lock(lock_file)? else {
            return Ok(None);
        };
        #[cfg(test)]
        super::super::fault::run(super::super::fault::RuntimeBootstrapFault::CatalogFinalLeaseLock);
        let held = HeldCatalogTargetV1 {
            target: self,
            associated_targets,
            _runtime_dir: runtime_dir,
            _locks_dir: locks_dir,
            _lock: lock,
        };
        held.revalidate_held()?;
        Ok(Some(held))
    }
}

pub(super) struct HeldCatalogTargetV1 {
    pub(super) target: RetainedCatalogTargetV1,
    associated_targets: Vec<RetainedCatalogTargetV1>,
    _runtime_dir: Option<RetainedDirectory>,
    _locks_dir: Option<RetainedDirectory>,
    _lock: AdvisoryLock,
}

impl HeldCatalogTargetV1 {
    pub(super) fn revalidate_held(&self) -> Result<(), CheckedFsError> {
        self.target.revalidate()?;
        for target in &self.associated_targets {
            target.revalidate()?;
        }
        match self.target.binding.root_kind {
            PreCatalogRootKindV1::Workspace => {
                let runtime = self
                    ._runtime_dir
                    .as_ref()
                    .expect("workspace runtime retained");
                let locks = self._locks_dir.as_ref().expect("workspace locks retained");
                revalidate_child_directory(
                    self.target.target.handle(),
                    OsStr::new(crate::workspace::RUNTIME_DIR),
                    runtime.identity(),
                    "workspace runtime directory",
                )?;
                revalidate_child_directory(
                    runtime.handle(),
                    OsStr::new(LOCKS_DIRECTORY_NAME),
                    locks.identity(),
                    "workspace locks directory",
                )?;
                revalidate_file(
                    locks.handle(),
                    OsStr::new(WORKSPACE_MUTATOR_LOCK_NAME),
                    self._lock.file(),
                    "workspace mutator lease",
                )
            }
            PreCatalogRootKindV1::GitDirectory => revalidate_file(
                self.target.target.handle(),
                OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
                self._lock.file(),
                "Git catalog mutator lease",
            ),
        }
    }
}

fn canonical_git_directory(path: &Path) -> Result<PathBuf, CheckedFsError> {
    let input = std::fs::symlink_metadata(path)
        .map_err(|source| CheckedFsError::io("observe catalog Git directory", source))?;
    if !input.is_dir() || input.file_type().is_symlink() {
        return Err(CheckedFsError::ambiguous(
            "catalog Git-directory target",
            "expected a no-follow directory",
        ));
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|source| CheckedFsError::io("canonicalize catalog Git directory", source))?;
    let repository = git2::Repository::open(&canonical).map_err(|error| {
        CheckedFsError::io(
            "open catalog Git directory",
            std::io::Error::other(error.message().to_owned()),
        )
    })?;
    let actual = std::fs::canonicalize(repository.path())
        .map_err(|source| CheckedFsError::io("canonicalize actual Git directory", source))?;
    if actual != canonical {
        return Err(CheckedFsError::ambiguous(
            "catalog Git-directory target",
            "path does not name the repository's actual Git directory",
        ));
    }
    Ok(canonical)
}

fn canonical_order_key(
    profile: SupportedFilesystemProfile,
    identity: &DurableObjectIdentityV1,
    root_kind: PreCatalogRootKindV1,
) -> Vec<u8> {
    let identity = identity.encode_canonical();
    let mut key = Vec::with_capacity(identity.len() + 10);
    key.push(match profile {
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1 => 1,
        SupportedFilesystemProfile::MacPersistentObjectIdV1 => 2,
        SupportedFilesystemProfile::WindowsNtfsFileId128V1 => 3,
    });
    key.extend_from_slice(&(identity.len() as u64).to_be_bytes());
    key.extend_from_slice(&identity);
    key.push(match root_kind {
        PreCatalogRootKindV1::Workspace => 1,
        PreCatalogRootKindV1::GitDirectory => 2,
    });
    key
}
