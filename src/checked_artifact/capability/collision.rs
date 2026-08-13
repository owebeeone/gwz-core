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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct LosslessIndexEntry {
    path: GitPathBytes,
    stage: IndexStage,
    mode: u32,
    raw_flags: u16,
    raw_extended_flags: u16,
}

impl LosslessIndexEntry {
    pub(in crate::checked_artifact) fn new(
        path: GitPathBytes,
        stage: u8,
        mode: u32,
        raw_flags: u16,
        raw_extended_flags: u16,
    ) -> Result<Self, CheckedFsError> {
        Ok(Self {
            path,
            stage: IndexStage::parse(stage)?,
            mode,
            raw_flags,
            raw_extended_flags,
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
}

impl PrivateControlDomain {
    pub(in crate::checked_artifact) fn checked_v1() -> Self {
        Self {
            members: CatalogPrivateNameV1::ALL
                .iter()
                .map(|name| {
                    GitPathBytes::new(name.relative_bytes(CatalogPrivateRootV1::Workspace))
                        .expect("fixed path is valid")
                })
                .collect(),
        }
    }
    pub(in crate::checked_artifact) fn members(&self) -> &[GitPathBytes] {
        &self.members
    }

    pub(in crate::checked_artifact) fn version_digest(&self) -> [u8; 32] {
        let mut material = Vec::new();
        for member in &self.members {
            material.extend_from_slice(&(member.as_bytes().len() as u64).to_le_bytes());
            material.extend_from_slice(member.as_bytes());
        }
        Sha256::digest(material).into()
    }
}
