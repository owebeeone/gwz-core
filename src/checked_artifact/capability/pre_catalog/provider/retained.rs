use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, ambient_authority};
use cap_std::fs::{Dir, File, OpenOptions};

use super::super::*;
use super::filesystem::PlatformProviderV1;
use crate::checked_artifact::capability::{ObjectIdentityFact, PathComponentMode};
use crate::checked_artifact::catalog::{CatalogNameBudgetV1, native_name_matches_ascii};

pub(super) struct RetainedDirectory {
    handle: Dir,
    identity: ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>,
    mode: PathComponentMode,
    rename_domain: Vec<u8>,
}

impl RetainedDirectory {
    pub(super) fn handle(&self) -> &Dir {
        &self.handle
    }

    pub(super) fn identity(&self) -> &ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>> {
        &self.identity
    }

    pub(super) const fn mode(&self) -> PathComponentMode {
        self.mode
    }

    pub(super) fn rename_domain(&self) -> &[u8] {
        &self.rename_domain
    }

    pub(super) fn encoded_identity(&self) -> Vec<u8> {
        encode_identity(&self.identity)
    }

    pub(super) fn encoded_snapshot_fact(&self) -> Vec<u8> {
        let identity = self.encoded_identity();
        let mut value = Vec::with_capacity(identity.len() + self.rename_domain.len() + 17);
        value.extend_from_slice(&(identity.len() as u64).to_be_bytes());
        value.extend_from_slice(&identity);
        value.push(match self.mode {
            PathComponentMode::Sensitive => 0,
            PathComponentMode::AsciiCaseFold => 1,
        });
        value.extend_from_slice(&(self.rename_domain.len() as u64).to_be_bytes());
        value.extend_from_slice(&self.rename_domain);
        value
    }

    fn revalidate(&self, platform: &impl PlatformProviderV1) -> Result<(), CheckedFsError> {
        if platform.dir_identity(&self.handle)? != self.identity
            || platform.parent_mode(&self.handle)? != self.mode
            || platform.rename_domain(&self.handle)? != self.rename_domain
        {
            return Err(CheckedFsError::ambiguous(
                "retained pre-catalog directory",
                "identity, lookup mode, or rename domain changed",
            ));
        }
        Ok(())
    }
}

pub(super) struct RetainedFile {
    handle: File,
    identity: ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>,
}

impl RetainedFile {
    pub(super) fn handle(&self) -> &File {
        &self.handle
    }

    pub(super) fn encoded_identity(&self) -> Vec<u8> {
        encode_identity(&self.identity)
    }

    pub(super) fn encoded_durable_identity(&self) -> Vec<u8> {
        self.identity.durable().encode_canonical()
    }

    fn revalidate(&self, platform: &impl PlatformProviderV1) -> Result<(), CheckedFsError> {
        if platform.file_identity(&self.handle)? != self.identity {
            return Err(CheckedFsError::ambiguous(
                "retained Git index",
                "index file identity changed",
            ));
        }
        Ok(())
    }
}

pub(in crate::checked_artifact::capability::pre_catalog) struct RetainedPlatformRoot {
    root_path: PathBuf,
    git_directory_path: PathBuf,
    common_directory_path: PathBuf,
    root: RetainedDirectory,
    repository: RetainedDirectory,
    common_directory: RetainedDirectory,
    private_parent: Option<RetainedDirectory>,
    private_parent_alias_observation: AliasObservationV1,
    index: Option<RetainedFile>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AliasObservationV1 {
    entry_count: usize,
    encoded_name_bytes: usize,
}

impl AliasObservationV1 {
    pub(super) const fn entry_count(self) -> usize {
        self.entry_count
    }

    pub(super) const fn encoded_name_bytes(self) -> usize {
        self.encoded_name_bytes
    }
}

impl RetainedPlatformRoot {
    pub(super) fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub(super) fn git_directory_path(&self) -> &Path {
        &self.git_directory_path
    }

    pub(super) fn common_directory_path(&self) -> &Path {
        &self.common_directory_path
    }

    pub(super) fn root(&self) -> &RetainedDirectory {
        &self.root
    }

    pub(super) fn repository(&self) -> &RetainedDirectory {
        &self.repository
    }

    pub(super) fn common_directory(&self) -> &RetainedDirectory {
        &self.common_directory
    }

    pub(super) fn private_parent(&self) -> Option<&RetainedDirectory> {
        self.private_parent.as_ref()
    }

    pub(super) const fn private_parent_alias_observation(&self) -> AliasObservationV1 {
        self.private_parent_alias_observation
    }

    pub(super) fn install_index(&mut self, index: Option<RetainedFile>) {
        self.index = index;
    }

    #[cfg(test)]
    pub(super) fn swap_repository_for_test(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.repository, &mut other.repository);
    }

    pub(super) fn revalidate(
        &self,
        platform: &impl PlatformProviderV1,
    ) -> Result<(), CheckedFsError> {
        self.root.revalidate(platform)?;
        self.repository.revalidate(platform)?;
        self.common_directory.revalidate(platform)?;
        if let Some(parent) = &self.private_parent {
            parent.revalidate(platform)?;
        }
        if let Some(index) = &self.index {
            index.revalidate(platform)?;
        }
        revalidate_repository_paths(self)
    }
}

pub(super) fn retain_workspace(
    path: &Path,
    platform: &impl PlatformProviderV1,
) -> Result<RetainedPlatformRoot, CheckedFsError> {
    let root_path = canonical_directory(path, "workspace root")?;
    let repository = git2::Repository::open(&root_path).map_err(git_error)?;
    let workdir = repository.workdir().ok_or_else(|| {
        CheckedFsError::ambiguous("workspace root", "bare repository is not a workspace")
    })?;
    if canonical_directory(workdir, "repository worktree")? != root_path {
        return Err(CheckedFsError::ambiguous(
            "workspace root",
            "path is not the repository worktree root",
        ));
    }
    let git_directory_path = canonical_directory(repository.path(), "Git directory")?;
    let common_directory_path =
        canonical_directory(repository.commondir(), "common Git directory")?;
    let root = retain_ambient(&root_path, platform, "workspace root")?;
    let repository = retain_ambient(&git_directory_path, platform, "Git directory")?;
    let common_directory =
        retain_ambient(&common_directory_path, platform, "common Git directory")?;
    let (private_parent, private_parent_alias_observation) =
        retain_required_child(&root, OsStr::new(".gwz"), platform, "workspace GWZ parent")?;
    Ok(RetainedPlatformRoot {
        root_path,
        git_directory_path,
        common_directory_path,
        root,
        repository,
        common_directory,
        private_parent: Some(private_parent),
        private_parent_alias_observation,
        index: None,
    })
}

pub(super) fn retain_git_directory(
    path: &Path,
    platform: &impl PlatformProviderV1,
) -> Result<RetainedPlatformRoot, CheckedFsError> {
    let root_path = canonical_directory(path, "actual Git directory")?;
    let repository = git2::Repository::open(&root_path).map_err(git_error)?;
    let git_directory_path = canonical_directory(repository.path(), "Git directory")?;
    if git_directory_path != root_path {
        return Err(CheckedFsError::ambiguous(
            "actual Git directory",
            "path does not name the repository's actual Git directory",
        ));
    }
    let common_directory_path =
        canonical_directory(repository.commondir(), "common Git directory")?;
    let root = retain_ambient(&root_path, platform, "actual Git directory")?;
    let repository = retain_ambient(&git_directory_path, platform, "Git directory")?;
    let common_directory =
        retain_ambient(&common_directory_path, platform, "common Git directory")?;
    let (private_parent, private_parent_alias_observation) = retain_optional_child(
        &root,
        OsStr::new("gwz"),
        platform,
        "Git-directory GWZ parent",
    )?;
    Ok(RetainedPlatformRoot {
        root_path,
        git_directory_path,
        common_directory_path,
        root,
        repository,
        common_directory,
        private_parent,
        private_parent_alias_observation,
        index: None,
    })
}

pub(super) fn retain_index_file(
    parent: &RetainedDirectory,
    platform: &impl PlatformProviderV1,
) -> Result<Option<RetainedFile>, CheckedFsError> {
    let name = OsStr::new("index");
    match parent.handle.symlink_metadata(name) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(CheckedFsError::io("observe Git index", source)),
        Ok(metadata) if !metadata.is_file() || metadata.is_symlink() => Err(
            CheckedFsError::ambiguous("Git index", "index is not a no-follow regular file"),
        ),
        Ok(_) => {
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let handle = parent
                .handle
                .open_with(name, &options)
                .map_err(|source| CheckedFsError::io("open Git index no-follow", source))?;
            let identity = platform.file_identity(&handle)?;
            Ok(Some(RetainedFile { handle, identity }))
        }
    }
}

fn retain_ambient(
    path: &Path,
    platform: &impl PlatformProviderV1,
    label: &'static str,
) -> Result<RetainedDirectory, CheckedFsError> {
    reject_symlink(path, label)?;
    let handle = Dir::open_ambient_dir(path, ambient_authority())
        .map_err(|source| CheckedFsError::io("open retained pre-catalog directory", source))?;
    retain_opened(handle, platform, label)
}

fn retain_required_child(
    parent: &RetainedDirectory,
    name: &OsStr,
    platform: &impl PlatformProviderV1,
    label: &'static str,
) -> Result<(RetainedDirectory, AliasObservationV1), CheckedFsError> {
    let (child, observation) = retain_optional_child(parent, name, platform, label)?;
    child
        .map(|child| (child, observation))
        .ok_or_else(|| CheckedFsError::ambiguous(label, "required retained directory is missing"))
}

fn retain_optional_child(
    parent: &RetainedDirectory,
    name: &OsStr,
    platform: &impl PlatformProviderV1,
    label: &'static str,
) -> Result<(Option<RetainedDirectory>, AliasObservationV1), CheckedFsError> {
    let alias_observation = reject_equivalent_alias(parent.handle(), name, parent.mode(), label)?;
    match parent.handle.symlink_metadata(name) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok((None, alias_observation)),
        Err(source) => Err(CheckedFsError::io(
            "observe retained child directory",
            source,
        )),
        Ok(metadata) if !metadata.is_dir() || metadata.is_symlink() => Err(
            CheckedFsError::ambiguous(label, "expected a no-follow directory"),
        ),
        Ok(_) => parent
            .handle
            .open_dir_nofollow(name)
            .map_err(|source| CheckedFsError::io("open retained child no-follow", source))
            .and_then(|handle| retain_opened(handle, platform, label))
            .map(|child| (Some(child), alias_observation)),
    }
}

fn retain_opened(
    handle: Dir,
    platform: &impl PlatformProviderV1,
    _label: &'static str,
) -> Result<RetainedDirectory, CheckedFsError> {
    let identity = platform.dir_identity(&handle)?;
    let mode = platform.parent_mode(&handle)?;
    let rename_domain = platform.rename_domain(&handle)?;
    Ok(RetainedDirectory {
        handle,
        identity,
        mode,
        rename_domain,
    })
}

fn reject_equivalent_alias(
    parent: &Dir,
    expected: &OsStr,
    mode: PathComponentMode,
    label: &'static str,
) -> Result<AliasObservationV1, CheckedFsError> {
    if mode == PathComponentMode::Sensitive {
        return Ok(AliasObservationV1 {
            entry_count: 0,
            encoded_name_bytes: 0,
        });
    }
    let expected_bytes = expected.as_encoded_bytes();
    debug_assert!(expected_bytes.is_ascii(), "fixed GWZ names are ASCII");
    let mut budget = CatalogNameBudgetV1::new();
    for entry in parent
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate retained parent", source))?
    {
        let entry = entry.map_err(|source| CheckedFsError::io("read retained parent", source))?;
        let observed = entry.file_name();
        budget.charge_os_str(&observed)?;
        if native_name_matches_ascii(&observed, expected_bytes, mode)?
            && observed.as_os_str() != expected
        {
            return Err(CheckedFsError::ambiguous(
                label,
                "platform-equivalent alias has noncanonical spelling",
            ));
        }
    }
    Ok(AliasObservationV1 {
        entry_count: budget.entry_count(),
        encoded_name_bytes: budget.encoded_name_bytes(),
    })
}

fn revalidate_repository_paths(root: &RetainedPlatformRoot) -> Result<(), CheckedFsError> {
    let repository = git2::Repository::open(&root.root_path).map_err(git_error)?;
    if canonical_directory(repository.path(), "Git directory")? != root.git_directory_path
        || canonical_directory(repository.commondir(), "common Git directory")?
            != root.common_directory_path
    {
        return Err(CheckedFsError::ambiguous(
            "repository relationship",
            "Git or common directory changed",
        ));
    }
    Ok(())
}

fn canonical_directory(path: &Path, label: &'static str) -> Result<PathBuf, CheckedFsError> {
    reject_symlink(path, label)?;
    std::fs::canonicalize(path)
        .map_err(|source| CheckedFsError::io("canonicalize pre-catalog directory", source))
}

fn reject_symlink(path: &Path, label: &'static str) -> Result<(), CheckedFsError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|source| CheckedFsError::io("observe pre-catalog directory", source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CheckedFsError::ambiguous(
            label,
            "expected a no-follow directory",
        ));
    }
    Ok(())
}

pub(super) fn encode_identity(
    fact: &ObjectIdentityFact<DurableObjectIdentityV1, Vec<u8>>,
) -> Vec<u8> {
    let durable = fact.durable().encode_canonical();
    let mut value = Vec::with_capacity(16 + durable.len() + fact.invocation().len());
    value.extend_from_slice(&(durable.len() as u64).to_be_bytes());
    value.extend_from_slice(&durable);
    value.extend_from_slice(&(fact.invocation().len() as u64).to_be_bytes());
    value.extend_from_slice(fact.invocation());
    value
}

fn git_error(error: git2::Error) -> CheckedFsError {
    CheckedFsError::io(
        "open pre-catalog Git repository",
        io::Error::other(error.message().to_owned()),
    )
}
