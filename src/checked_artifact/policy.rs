use std::path::{Path, PathBuf};

use super::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};

/// Explicitly selects the filesystem that owns a checked artifact's private
/// recovery namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CheckedArtifactPolicy {
    WorkspaceArtifact { artifact_root: PathBuf },
    GitDirectoryArtifact { artifact_root: PathBuf },
}

impl CheckedArtifactPolicy {
    pub(super) fn workspace(artifact_root: &Path) -> Self {
        Self::WorkspaceArtifact {
            artifact_root: artifact_root.to_path_buf(),
        }
    }

    pub(super) fn git_directory(artifact_root: &Path) -> Self {
        Self::GitDirectoryArtifact {
            artifact_root: artifact_root.to_path_buf(),
        }
    }

    pub(super) fn artifact_root(&self) -> &Path {
        match self {
            Self::WorkspaceArtifact { artifact_root }
            | Self::GitDirectoryArtifact { artifact_root } => artifact_root,
        }
    }

    pub(super) fn private_parent(&self) -> PathBuf {
        match self {
            Self::WorkspaceArtifact { .. } => {
                CatalogPrivateNameV1::Final.relative_path(CatalogPrivateRootV1::Workspace)
            }
            Self::GitDirectoryArtifact { .. } => {
                CatalogPrivateNameV1::Final.relative_path(CatalogPrivateRootV1::GitDirectory)
            }
        }
    }
}
