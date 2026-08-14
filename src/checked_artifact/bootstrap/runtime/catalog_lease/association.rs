//! Retained repository/worktree membership for one common-Git lease target.

use std::path::{Path, PathBuf};

use super::super::paths::{
    RetainedDirectory, retain_ambient_directory, revalidate_ambient_directory,
};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1, HostPlatform,
    PathComponentMode, PathEquivalenceProvider, SupportedFilesystemProfile,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct CatalogAssociationDirectoryBindingV1 {
    canonical_path: PathBuf,
    support_profile: SupportedFilesystemProfile,
    durable_identity: DurableObjectIdentityV1,
    invocation_identity: Vec<u8>,
    rename_domain: Vec<u8>,
    mode: PathComponentMode,
}

struct RetainedAssociationDirectoryV1 {
    directory: RetainedDirectory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CatalogGitAssociationBindingV1 {
    request: CatalogAssociationDirectoryBindingV1,
    worktree: Option<CatalogAssociationDirectoryBindingV1>,
    actual_git_directory: CatalogAssociationDirectoryBindingV1,
    common_git_directory: CatalogAssociationDirectoryBindingV1,
}

pub(super) struct RetainedCatalogGitAssociationV1 {
    binding: CatalogGitAssociationBindingV1,
    request: RetainedAssociationDirectoryV1,
    worktree: Option<RetainedAssociationDirectoryV1>,
    actual_git_directory: RetainedAssociationDirectoryV1,
    common_git_directory: RetainedAssociationDirectoryV1,
}

impl RetainedCatalogGitAssociationV1 {
    pub(super) fn retain(path: &Path) -> Result<Self, CheckedFsError> {
        let repository = git2::Repository::open(path).map_err(git_error)?;
        let (request_binding, request) =
            RetainedAssociationDirectoryV1::retain(path, "catalog repository/worktree request")?;
        let (worktree_binding, worktree) = match repository.workdir() {
            Some(path) => {
                let (binding, retained) =
                    RetainedAssociationDirectoryV1::retain(path, "catalog worktree")?;
                (Some(binding), Some(retained))
            }
            None => (None, None),
        };
        let (actual_git_binding, actual_git_directory) = RetainedAssociationDirectoryV1::retain(
            repository.path(),
            "catalog actual Git directory",
        )?;
        let (common_git_binding, common_git_directory) = RetainedAssociationDirectoryV1::retain(
            repository.commondir(),
            "catalog common Git directory",
        )?;
        let binding = CatalogGitAssociationBindingV1 {
            request: request_binding,
            worktree: worktree_binding,
            actual_git_directory: actual_git_binding,
            common_git_directory: common_git_binding,
        };
        let retained = Self {
            binding,
            request,
            worktree,
            actual_git_directory,
            common_git_directory,
        };
        retained.revalidate()?;
        Ok(retained)
    }

    pub(super) fn binding(&self) -> &CatalogGitAssociationBindingV1 {
        &self.binding
    }

    pub(super) fn into_binding(self) -> CatalogGitAssociationBindingV1 {
        self.binding
    }

    pub(super) fn common_directory_path(&self) -> &Path {
        &self.binding.common_git_directory.canonical_path
    }

    pub(super) fn revalidate(&self) -> Result<(), CheckedFsError> {
        self.request
            .revalidate(&self.binding.request, "catalog repository/worktree request")?;
        if let (Some(worktree), Some(binding)) = (&self.worktree, &self.binding.worktree) {
            worktree.revalidate(binding, "catalog worktree")?;
        }
        self.actual_git_directory.revalidate(
            &self.binding.actual_git_directory,
            "catalog actual Git directory",
        )?;
        self.common_git_directory.revalidate(
            &self.binding.common_git_directory,
            "catalog common Git directory",
        )?;

        let repository =
            git2::Repository::open(&self.binding.request.canonical_path).map_err(git_error)?;
        let actual = canonical_directory(repository.path(), "catalog actual Git directory")?;
        let common = canonical_directory(repository.commondir(), "catalog common Git directory")?;
        let worktree = repository
            .workdir()
            .map(|path| canonical_directory(path, "catalog worktree"))
            .transpose()?;
        if actual != self.binding.actual_git_directory.canonical_path
            || common != self.binding.common_git_directory.canonical_path
            || worktree.as_ref()
                != self
                    .binding
                    .worktree
                    .as_ref()
                    .map(|binding| &binding.canonical_path)
        {
            return Err(CheckedFsError::ambiguous(
                "catalog repository/worktree membership",
                "actual, common, or worktree relationship changed",
            ));
        }
        Ok(())
    }
}

impl CatalogGitAssociationBindingV1 {
    pub(super) fn request_path(&self) -> &Path {
        &self.request.canonical_path
    }
}

impl RetainedAssociationDirectoryV1 {
    fn retain(
        path: &Path,
        label: &'static str,
    ) -> Result<(CatalogAssociationDirectoryBindingV1, Self), CheckedFsError> {
        let canonical_path = canonical_directory(path, label)?;
        let directory = retain_ambient_directory(&canonical_path, label)?;
        let platform = HostPlatform;
        let identity = platform.dir_identity(directory.handle())?;
        let support_profile = platform.support_profile();
        let rename_domain = platform.rename_domain(directory.handle())?;
        if identity.durable().support_profile() != support_profile
            || identity.invocation().is_empty()
            || rename_domain.is_empty()
        {
            return Err(CheckedFsError::ambiguous(
                label,
                "association identity does not match the host profile or is empty",
            ));
        }
        Ok((
            CatalogAssociationDirectoryBindingV1 {
                canonical_path,
                support_profile,
                durable_identity: identity.durable().clone(),
                invocation_identity: identity.invocation().clone(),
                rename_domain,
                mode: platform.parent_mode(directory.handle())?,
            },
            Self { directory },
        ))
    }

    fn revalidate(
        &self,
        binding: &CatalogAssociationDirectoryBindingV1,
        label: &'static str,
    ) -> Result<(), CheckedFsError> {
        revalidate_ambient_directory(&binding.canonical_path, &self.directory, label)?;
        let platform = HostPlatform;
        let identity = platform.dir_identity(self.directory.handle())?;
        if platform.support_profile() != binding.support_profile
            || identity.durable() != &binding.durable_identity
            || identity.invocation() != &binding.invocation_identity
            || platform.rename_domain(self.directory.handle())? != binding.rename_domain
            || platform.parent_mode(self.directory.handle())? != binding.mode
        {
            return Err(CheckedFsError::ambiguous(
                label,
                "stable or live association binding changed",
            ));
        }
        Ok(())
    }
}

fn canonical_directory(path: &Path, label: &'static str) -> Result<PathBuf, CheckedFsError> {
    let input = std::fs::symlink_metadata(path)
        .map_err(|source| CheckedFsError::io("observe catalog association input", source))?;
    if !input.is_dir() || input.file_type().is_symlink() {
        return Err(CheckedFsError::ambiguous(
            label,
            "expected a no-follow directory",
        ));
    }
    std::fs::canonicalize(path)
        .map_err(|source| CheckedFsError::io("canonicalize catalog association directory", source))
        .and_then(|canonical| {
            let metadata = std::fs::symlink_metadata(&canonical)
                .map_err(|source| CheckedFsError::io("observe catalog association", source))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "expected a no-follow directory",
                ));
            }
            Ok(canonical)
        })
}

fn git_error(error: git2::Error) -> CheckedFsError {
    CheckedFsError::io(
        "open catalog target repository",
        std::io::Error::other(error.message().to_owned()),
    )
}
