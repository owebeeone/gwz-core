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

    /// The LEGACY leaf writer's private area — not the catalog's Final
    /// directory. R2-F R1.1, 2026-09-01: these two arms were the second
    /// consumer of `CatalogPrivateNameV1::Final`, and pinning them to
    /// `LegacyPrivate` is the split (`GwzM5-8R2F-RelocationPlan.md` §1). The
    /// leaf bytes are unchanged — `checked-artifacts` — so the legacy area does
    /// not move, its residue is not orphaned, and its git-status dirt exemption
    /// and preservation-image blindness stay correct where they already are.
    ///
    /// The `GitDirectoryArtifact` arm is symmetry, not behaviour: its only
    /// production construction site is `entry.rs:182`, reached only from
    /// `observe_merge_preservation_git_directory`, which never mutates — no
    /// production write lands under `<git-dir>/gwz/` through this policy
    /// ([P3-7]).
    pub(super) fn private_parent(&self) -> PathBuf {
        match self {
            Self::WorkspaceArtifact { .. } => {
                CatalogPrivateNameV1::LegacyPrivate.relative_path(CatalogPrivateRootV1::Workspace)
            }
            Self::GitDirectoryArtifact { .. } => CatalogPrivateNameV1::LegacyPrivate
                .relative_path(CatalogPrivateRootV1::GitDirectory),
        }
    }
}
