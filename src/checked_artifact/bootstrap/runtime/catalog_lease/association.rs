//! Retained repository/worktree membership for one common-Git lease target.

use std::path::{Path, PathBuf};

use super::super::paths::{
    RetainedDirectory, retain_ambient_directory_in, revalidate_ambient_directory,
};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1, HostPlatform,
    PathComponentMode, PathEquivalenceProvider, SupportedFilesystemProfile,
};
use crate::filesystem::{FileSystem, FsKind};
use crate::operation_context::OperationServices;

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
    context: OperationServices,
    binding: CatalogGitAssociationBindingV1,
    request: RetainedAssociationDirectoryV1,
    worktree: Option<RetainedAssociationDirectoryV1>,
    actual_git_directory: RetainedAssociationDirectoryV1,
    common_git_directory: RetainedAssociationDirectoryV1,
}

impl RetainedCatalogGitAssociationV1 {
    pub(super) fn retain(context: &OperationServices, path: &Path) -> Result<Self, CheckedFsError> {
        let repository = context
            .repository()
            .repository_paths(path)
            .map_err(git_error)?;
        let (request_binding, request) = RetainedAssociationDirectoryV1::retain(
            context.filesystem(),
            path,
            "catalog repository/worktree request",
        )?;
        let (worktree_binding, worktree) = match repository.worktree.as_deref() {
            Some(path) => {
                let (binding, retained) = RetainedAssociationDirectoryV1::retain(
                    context.filesystem(),
                    path,
                    "catalog worktree",
                )?;
                (Some(binding), Some(retained))
            }
            None => (None, None),
        };
        let (actual_git_binding, actual_git_directory) = RetainedAssociationDirectoryV1::retain(
            context.filesystem(),
            &repository.git_dir,
            "catalog actual Git directory",
        )?;
        let (common_git_binding, common_git_directory) = RetainedAssociationDirectoryV1::retain(
            context.filesystem(),
            &repository.common_dir,
            "catalog common Git directory",
        )?;
        let binding = CatalogGitAssociationBindingV1 {
            request: request_binding,
            worktree: worktree_binding,
            actual_git_directory: actual_git_binding,
            common_git_directory: common_git_binding,
        };
        let retained = Self {
            context: context.clone(),
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

        let repository = self
            .context
            .repository()
            .repository_paths(&self.binding.request.canonical_path)
            .map_err(git_error)?;
        let actual = canonical_directory(
            self.context.filesystem(),
            &repository.git_dir,
            "catalog actual Git directory",
        )?;
        let common = canonical_directory(
            self.context.filesystem(),
            &repository.common_dir,
            "catalog common Git directory",
        )?;
        let worktree = repository
            .worktree
            .as_deref()
            .map(|path| canonical_directory(self.context.filesystem(), path, "catalog worktree"))
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
        filesystem: &dyn FileSystem,
        path: &Path,
        label: &'static str,
    ) -> Result<(CatalogAssociationDirectoryBindingV1, Self), CheckedFsError> {
        let canonical_path = canonical_directory(filesystem, path, label)?;
        let directory = retain_ambient_directory_in(filesystem, &canonical_path, label)?;
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

fn canonical_directory(
    filesystem: &dyn FileSystem,
    path: &Path,
    label: &'static str,
) -> Result<PathBuf, CheckedFsError> {
    let input = filesystem
        .metadata(path)
        .map_err(|source| CheckedFsError::io("observe catalog association input", source))?;
    if input.kind != FsKind::Directory {
        return Err(CheckedFsError::ambiguous(
            label,
            "expected a no-follow directory",
        ));
    }
    filesystem
        .canonical_path(path)
        .map_err(|source| CheckedFsError::io("canonicalize catalog association directory", source))
        .and_then(|canonical| {
            let metadata = filesystem
                .metadata(&canonical)
                .map_err(|source| CheckedFsError::io("observe catalog association", source))?;
            if metadata.kind != FsKind::Directory {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "expected a no-follow directory",
                ));
            }
            Ok(canonical)
        })
}

fn git_error(error: crate::model::ModelError) -> CheckedFsError {
    CheckedFsError::io(
        "open catalog target repository",
        std::io::Error::other(error.to_string()),
    )
}
