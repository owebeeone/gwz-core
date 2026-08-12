//! Pure ownership contracts for the three checked-artifact bootstrap layers.

use std::path::Path;

use super::capability::{
    AsciiComponent, CheckedFsError, FilesystemCapabilityProof, PrivateNamespaceCollisionProof,
};
use super::protocol::AdmittedActionV1;

pub(super) struct WorkspaceRuntimePaths<'a> {
    workspace_root: &'a Path,
    workspace_git_dir: &'a Path,
}

impl<'a> WorkspaceRuntimePaths<'a> {
    pub(super) fn new(workspace_root: &'a Path, workspace_git_dir: &'a Path) -> Self {
        Self {
            workspace_root,
            workspace_git_dir,
        }
    }

    pub(super) fn workspace_root(&self) -> &Path {
        self.workspace_root
    }

    pub(super) fn workspace_git_dir(&self) -> &Path {
        self.workspace_git_dir
    }
}

/// Capability-neutral live-process coordination. Implementors may create only
/// the fixed runtime guard, `.gwz`, `.gwz/locks`, and the final lease file.
pub(super) trait WorkspaceRuntimeBootstrapV1 {
    type Lease;

    fn try_acquire(
        &self,
        paths: WorkspaceRuntimePaths<'_>,
    ) -> Result<Option<Self::Lease>, CheckedFsError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NonWorktreeGitDirectoryProof {
    _sealed: (),
}

impl NonWorktreeGitDirectoryProof {
    fn observed() -> Self {
        Self { _sealed: () }
    }
}

/// The Git-directory owner issues the opaque exemption only after proving the
/// retained directory is not represented by a workspace worktree scan.
pub(super) trait NonWorktreeGitDirectoryPreflight<GitDirectory: ?Sized> {
    fn inspect(&self, git_directory: &GitDirectory) -> Result<(), CheckedFsError>;

    fn preflight(
        &self,
        git_directory: &GitDirectory,
    ) -> Result<NonWorktreeGitDirectoryProof, CheckedFsError> {
        self.inspect(git_directory)?;
        Ok(NonWorktreeGitDirectoryProof::observed())
    }
}

pub(super) enum CatalogBootstrapPermit<'a, Identity> {
    Workspace {
        capability: &'a FilesystemCapabilityProof<Identity>,
        collision: &'a PrivateNamespaceCollisionProof,
    },
    GitDirectory {
        capability: &'a FilesystemCapabilityProof<Identity>,
        non_worktree: &'a NonWorktreeGitDirectoryProof,
    },
}

impl<'a, Identity> CatalogBootstrapPermit<'a, Identity> {
    pub(super) fn workspace(
        capability: &'a FilesystemCapabilityProof<Identity>,
        collision: &'a PrivateNamespaceCollisionProof,
    ) -> Self {
        Self::Workspace {
            capability,
            collision,
        }
    }

    pub(super) fn git_directory(
        capability: &'a FilesystemCapabilityProof<Identity>,
        non_worktree: &'a NonWorktreeGitDirectoryProof,
    ) -> Self {
        Self::GitDirectory {
            capability,
            non_worktree,
        }
    }
}

/// Durable first-catalog bootstrap. The permit makes capability/collision
/// preflight a structural prerequisite rather than a caller convention.
pub(super) trait CatalogBootstrapV1<Identity> {
    type Catalog;

    fn recover_or_create(
        &self,
        permit: CatalogBootstrapPermit<'_, Identity>,
    ) -> Result<Self::Catalog, CheckedFsError>;
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum ManagedParentPurpose {
    MergeStore,
    MergeArchive,
    PreservationBundles,
    RootPreservationMarkers,
}

impl ManagedParentPurpose {
    pub(super) const ALL: &'static [Self] = &[
        Self::MergeStore,
        Self::MergeArchive,
        Self::PreservationBundles,
        Self::RootPreservationMarkers,
    ];

    fn path(self) -> &'static [&'static [u8]] {
        match self {
            Self::MergeStore => &[b".gwz", b"merge"],
            Self::MergeArchive => &[b".gwz", b"merge", b"done"],
            Self::PreservationBundles => &[b".gwz", b"stash", b"bundles"],
            Self::RootPreservationMarkers => &[b"gwz.conf", b"markers"],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ManagedParentSpec {
    purpose: ManagedParentPurpose,
    components: Vec<AsciiComponent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ManagedParentBootstrapRequest {
    specs: Vec<ManagedParentSpec>,
}

impl ManagedParentBootstrapRequest {
    pub(super) fn try_new(mut specs: Vec<ManagedParentSpec>) -> Result<Self, CheckedFsError> {
        if specs.is_empty() || specs.len() > 8 {
            return Err(CheckedFsError::unsupported(
                super::capability::PlatformCapability::ManagedParentBootstrap,
                "managed-parent bootstrap requires 1..=8 declared purposes",
            ));
        }
        specs.sort_by_key(ManagedParentSpec::purpose);
        if specs
            .windows(2)
            .any(|pair| pair[0].purpose() == pair[1].purpose())
        {
            return Err(CheckedFsError::ambiguous(
                "managed-parent bootstrap",
                "duplicate managed-parent purpose",
            ));
        }
        Ok(Self { specs })
    }

    pub(super) fn specs(&self) -> &[ManagedParentSpec] {
        &self.specs
    }
}

impl ManagedParentSpec {
    pub(super) fn for_purpose(purpose: ManagedParentPurpose) -> Self {
        let components = purpose
            .path()
            .iter()
            .map(|value| AsciiComponent::parse(value).expect("fixed managed path is valid"))
            .collect();
        Self {
            purpose,
            components,
        }
    }

    pub(super) fn purpose(&self) -> ManagedParentPurpose {
        self.purpose
    }

    pub(super) fn components(&self) -> &[AsciiComponent] {
        &self.components
    }
}

/// Catalog-backed managed-parent creation. The admitted action is opaque and
/// provider plans remain associated types, so consumers cannot substitute raw
/// paths or a plan issued by another implementation.
pub(super) trait ManagedParentBootstrap {
    type Plan;
    type RetainedParents;

    fn preflight(
        &self,
        request: &ManagedParentBootstrapRequest,
    ) -> Result<Self::Plan, CheckedFsError>;

    fn execute(
        &self,
        admitted_action: &AdmittedActionV1,
        plan: &Self::Plan,
    ) -> Result<Self::RetainedParents, CheckedFsError>;
}
