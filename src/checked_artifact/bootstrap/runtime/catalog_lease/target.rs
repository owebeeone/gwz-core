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
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1, HostPlatform,
    PathComponentMode, PathEquivalenceProvider, PreCatalogRootKindV1, SupportedFilesystemProfile,
};

pub(super) const GIT_CATALOG_MUTATOR_LOCK_NAME: &str = "gwz-catalog-mutator-v1.lock";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CatalogLeaseTargetRequestV1 {
    root_kind: PreCatalogRootKindV1,
    path: PathBuf,
}

impl CatalogLeaseTargetRequestV1 {
    pub(in crate::checked_artifact) fn workspace(path: impl Into<PathBuf>) -> Self {
        Self {
            root_kind: PreCatalogRootKindV1::Workspace,
            path: path.into(),
        }
    }

    pub(in crate::checked_artifact) fn git_directory(path: impl Into<PathBuf>) -> Self {
        Self {
            root_kind: PreCatalogRootKindV1::GitDirectory,
            path: path.into(),
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
    support_profile: SupportedFilesystemProfile,
    durable_identity: DurableObjectIdentityV1,
    pub(super) canonical_path: PathBuf,
    related_git_directory: PathBuf,
    pub(super) order_key: Vec<u8>,
}

pub(super) struct RetainedCatalogTargetV1 {
    pub(super) binding: CatalogTargetBindingV1,
    target: RetainedDirectory,
    related_git_directory: RetainedDirectory,
    target_invocation_identity: Vec<u8>,
    target_rename_domain: Vec<u8>,
    target_mode: PathComponentMode,
}

impl RetainedCatalogTargetV1 {
    pub(super) fn retain(request: &CatalogLeaseTargetRequestV1) -> Result<Self, CheckedFsError> {
        match request.root_kind {
            PreCatalogRootKindV1::Workspace => Self::retain_workspace(&request.path),
            PreCatalogRootKindV1::GitDirectory => Self::retain_git_directory(&request.path),
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

    fn finish(
        root_kind: PreCatalogRootKindV1,
        canonical_path: PathBuf,
        related_git_directory_path: PathBuf,
        target: RetainedDirectory,
        related_git_directory: RetainedDirectory,
    ) -> Result<Self, CheckedFsError> {
        let platform = HostPlatform;
        let fact = platform.dir_identity(target.handle())?;
        let support_profile = platform.support_profile();
        if fact.durable().support_profile() != support_profile {
            return Err(CheckedFsError::ambiguous(
                "catalog target identity",
                "durable identity does not match the host support profile",
            ));
        }
        let target_mode = platform.parent_mode(target.handle())?;
        let target_rename_domain = platform.rename_domain(target.handle())?;
        if fact.invocation().is_empty() || target_rename_domain.is_empty() {
            return Err(CheckedFsError::ambiguous(
                "catalog target identity",
                "live target identity and rename domain must be nonempty",
            ));
        }
        let durable_identity = fact.durable().clone();
        let order_key = canonical_order_key(support_profile, &durable_identity, root_kind);
        Ok(Self {
            binding: CatalogTargetBindingV1 {
                root_kind,
                support_profile,
                durable_identity,
                canonical_path,
                related_git_directory: related_git_directory_path,
                order_key,
            },
            target,
            related_git_directory,
            target_invocation_identity: fact.invocation().clone(),
            target_rename_domain,
            target_mode,
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
        let fact = platform.dir_identity(self.target.handle())?;
        if fact.durable() != &self.binding.durable_identity
            || fact.invocation() != &self.target_invocation_identity
            || platform.support_profile() != self.binding.support_profile
            || platform.parent_mode(self.target.handle())? != self.target_mode
            || platform.rename_domain(self.target.handle())? != self.target_rename_domain
        {
            return Err(CheckedFsError::ambiguous(
                "catalog mutation target",
                "stable or live target binding changed",
            ));
        }
        Ok(())
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

    pub(super) fn acquire_final(self) -> Result<Option<HeldCatalogTargetV1>, CheckedFsError> {
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
        self.revalidate()?;
        match self.binding.root_kind {
            PreCatalogRootKindV1::Workspace => {
                let runtime = runtime_dir.as_ref().expect("workspace runtime retained");
                let locks = locks_dir.as_ref().expect("workspace locks retained");
                revalidate_child_directory(
                    self.target.handle(),
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
                    lock.file(),
                    "workspace mutator lease",
                )?;
            }
            PreCatalogRootKindV1::GitDirectory => revalidate_file(
                self.target.handle(),
                OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
                lock.file(),
                "Git catalog mutator lease",
            )?,
        }
        Ok(Some(HeldCatalogTargetV1 {
            target: self,
            _runtime_dir: runtime_dir,
            _locks_dir: locks_dir,
            _lock: lock,
        }))
    }
}

pub(super) struct HeldCatalogTargetV1 {
    pub(super) target: RetainedCatalogTargetV1,
    _runtime_dir: Option<RetainedDirectory>,
    _locks_dir: Option<RetainedDirectory>,
    _lock: AdvisoryLock,
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

pub(super) fn reject_equivalent_alias(
    parent: &Dir,
    expected: &OsStr,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let platform = HostPlatform;
    let mode = platform.parent_mode(parent)?;
    let expected = expected
        .to_str()
        .expect("fixed catalog lease names are ASCII");
    for entry in parent
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate catalog lease parent", source))?
    {
        let entry =
            entry.map_err(|source| CheckedFsError::io("read catalog lease parent", source))?;
        let observed = entry.file_name();
        let Some(observed) = observed.to_str() else {
            continue;
        };
        let equivalent = match mode {
            PathComponentMode::Sensitive => observed == expected,
            PathComponentMode::AsciiCaseFold => {
                observed.is_ascii() && observed.eq_ignore_ascii_case(expected)
            }
        };
        if equivalent && observed != expected {
            return Err(CheckedFsError::ambiguous(
                label,
                "platform-equivalent alias has noncanonical spelling",
            ));
        }
    }
    Ok(())
}
