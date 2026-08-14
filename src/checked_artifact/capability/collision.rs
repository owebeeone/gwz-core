//! Lossless Git facts and the opaque pre-catalog collision proof.

use sha2::{Digest, Sha256};

use super::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::checked_artifact) struct GitPathBytes(Vec<u8>);

impl GitPathBytes {
    pub(in crate::checked_artifact) fn new(value: Vec<u8>) -> Result<Self, CheckedFsError> {
        if value.is_empty() || value.contains(&0) {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "Git path is empty or contains NUL",
            ));
        }
        Ok(Self(value))
    }

    pub(in crate::checked_artifact) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum IndexStage {
    Normal,
    Base,
    Ours,
    Theirs,
}

impl IndexStage {
    fn parse(value: u8) -> Result<Self, CheckedFsError> {
        match value {
            0 => Ok(Self::Normal),
            1 => Ok(Self::Base),
            2 => Ok(Self::Ours),
            3 => Ok(Self::Theirs),
            _ => Err(CheckedFsError::ambiguous(
                "Git index",
                "index entry has an invalid stage",
            )),
        }
    }

    pub(super) const fn code(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Base => 1,
            Self::Ours => 2,
            Self::Theirs => 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct IndexTimestampV1 {
    seconds: i32,
    nanoseconds: u32,
}

impl IndexTimestampV1 {
    pub(in crate::checked_artifact) fn new(
        seconds: i32,
        nanoseconds: u32,
    ) -> Result<Self, CheckedFsError> {
        if nanoseconds >= 1_000_000_000 {
            return Err(CheckedFsError::ambiguous(
                "Git index",
                "index timestamp nanoseconds are out of range",
            ));
        }
        Ok(Self {
            seconds,
            nanoseconds,
        })
    }

    pub(in crate::checked_artifact) fn seconds(&self) -> i32 {
        self.seconds
    }

    pub(in crate::checked_artifact) fn nanoseconds(&self) -> u32 {
        self.nanoseconds
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct LosslessIndexMetadataV1 {
    ctime: IndexTimestampV1,
    mtime: IndexTimestampV1,
    stat: [u32; 5],
    object_id: Vec<u8>,
}

impl LosslessIndexMetadataV1 {
    pub(in crate::checked_artifact) fn new(
        ctime: IndexTimestampV1,
        mtime: IndexTimestampV1,
        stat: [u32; 5],
        object_id: Vec<u8>,
    ) -> Result<Self, CheckedFsError> {
        if object_id.is_empty() {
            return Err(CheckedFsError::ambiguous(
                "Git index",
                "index object identity is empty",
            ));
        }
        Ok(Self {
            ctime,
            mtime,
            stat,
            object_id,
        })
    }

    pub(in crate::checked_artifact) fn ctime(&self) -> &IndexTimestampV1 {
        &self.ctime
    }

    pub(in crate::checked_artifact) fn mtime(&self) -> &IndexTimestampV1 {
        &self.mtime
    }

    /// Returns device, inode, uid, gid, and file size in Git-index order.
    pub(in crate::checked_artifact) fn stat(&self) -> &[u32; 5] {
        &self.stat
    }

    pub(in crate::checked_artifact) fn object_id(&self) -> &[u8] {
        &self.object_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct LosslessIndexEntry {
    path: GitPathBytes,
    stage: IndexStage,
    mode: u32,
    raw_flags: u16,
    raw_extended_flags: u16,
    metadata: LosslessIndexMetadataV1,
}

impl LosslessIndexEntry {
    pub(in crate::checked_artifact) fn new(
        path: GitPathBytes,
        stage: u8,
        mode: u32,
        raw_flags: u16,
        raw_extended_flags: u16,
        metadata: LosslessIndexMetadataV1,
    ) -> Result<Self, CheckedFsError> {
        Ok(Self {
            path,
            stage: IndexStage::parse(stage)?,
            mode,
            raw_flags,
            raw_extended_flags,
            metadata,
        })
    }

    pub(in crate::checked_artifact) fn path(&self) -> &GitPathBytes {
        &self.path
    }
    pub(in crate::checked_artifact) fn stage(&self) -> IndexStage {
        self.stage
    }
    pub(in crate::checked_artifact) fn mode(&self) -> u32 {
        self.mode
    }
    pub(in crate::checked_artifact) fn raw_flags(&self) -> u16 {
        self.raw_flags
    }
    pub(in crate::checked_artifact) fn raw_extended_flags(&self) -> u16 {
        self.raw_extended_flags
    }
    pub(in crate::checked_artifact) fn metadata(&self) -> &LosslessIndexMetadataV1 {
        &self.metadata
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum TrackedWorktreeKind {
    Missing,
    RegularFile,
    Symlink,
    Directory,
    Gitlink,
    Other,
}

impl TrackedWorktreeKind {
    pub(super) const fn code(self) -> u8 {
        match self {
            Self::Missing => 0,
            Self::RegularFile => 1,
            Self::Symlink => 2,
            Self::Directory => 3,
            Self::Gitlink => 4,
            Self::Other => 5,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct TrackedWorktreeEntry {
    path: GitPathBytes,
    kind: TrackedWorktreeKind,
}

impl TrackedWorktreeEntry {
    pub(in crate::checked_artifact) fn new(path: GitPathBytes, kind: TrackedWorktreeKind) -> Self {
        Self { path, kind }
    }
    pub(in crate::checked_artifact) fn path(&self) -> &GitPathBytes {
        &self.path
    }
    pub(in crate::checked_artifact) fn kind(&self) -> TrackedWorktreeKind {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct PrivateControlDomain {
    members: Vec<GitPathBytes>,
    scratch_family: GitPathBytes,
}

impl PrivateControlDomain {
    pub(in crate::checked_artifact) fn checked_v1() -> Self {
        Self::for_root(CatalogPrivateRootV1::Workspace)
    }

    pub(super) fn for_root(root: CatalogPrivateRootV1) -> Self {
        let scratch_family =
            GitPathBytes::new(CatalogPrivateNameV1::BootstrapScratch.relative_bytes(root))
                .expect("fixed scratch-family path is valid");
        Self {
            members: CatalogPrivateNameV1::ALL
                .iter()
                .map(|name| {
                    GitPathBytes::new(name.relative_bytes(root)).expect("fixed path is valid")
                })
                .collect(),
            scratch_family,
        }
    }
    pub(in crate::checked_artifact) fn members(&self) -> &[GitPathBytes] {
        &self.members
    }

    pub(in crate::checked_artifact) fn scratch_family(&self) -> &GitPathBytes {
        &self.scratch_family
    }

    pub(in crate::checked_artifact) fn version_digest(&self) -> [u8; 32] {
        let mut material = Vec::new();
        material.extend_from_slice(b"gwz-private-control-domain-v2\0dynamic-scratch-family\0");
        for member in &self.members {
            material.extend_from_slice(&(member.as_bytes().len() as u64).to_le_bytes());
            material.extend_from_slice(member.as_bytes());
        }
        material.extend_from_slice(&(self.scratch_family.as_bytes().len() as u64).to_le_bytes());
        material.extend_from_slice(self.scratch_family.as_bytes());
        Sha256::digest(material).into()
    }
}
