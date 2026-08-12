//! Lossless Git facts and the opaque pre-catalog collision proof.

use super::{CanonicalPathIdentityV1, CheckedFsError, PlatformCapability};

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
            members: [
                b".gwz/checked-artifacts".as_slice(),
                b".gwz/checked-artifacts-catalog-bootstrap-v1.scratch".as_slice(),
                b".gwz/checked-artifacts-catalog-bootstrap-v1.active".as_slice(),
                b".gwz/checked-artifacts-catalog-bootstrap-v1.staging".as_slice(),
            ]
            .into_iter()
            .map(|value| GitPathBytes::new(value.to_vec()).expect("fixed path is valid"))
            .collect(),
        }
    }
    pub(in crate::checked_artifact) fn members(&self) -> &[GitPathBytes] {
        &self.members
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct PrivateNamespaceCollisionProof {
    domain: PrivateControlDomain,
    path_profile: CanonicalPathIdentityV1,
}

impl PrivateNamespaceCollisionProof {
    fn cleared(domain: &PrivateControlDomain, path_profile: &CanonicalPathIdentityV1) -> Self {
        Self {
            domain: domain.clone(),
            path_profile: path_profile.clone(),
        }
    }
    pub(in crate::checked_artifact) fn domain(&self) -> &PrivateControlDomain {
        &self.domain
    }
    pub(in crate::checked_artifact) fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        &self.path_profile
    }
}

pub(in crate::checked_artifact) trait PrivateNamespaceCollisionPreflight<Root: ?Sized> {
    fn scan(
        &self,
        root: &Root,
        domain: &PrivateControlDomain,
        path_profile: &CanonicalPathIdentityV1,
        index: &[LosslessIndexEntry],
        worktree: &[TrackedWorktreeEntry],
    ) -> Result<(), CheckedFsError>;

    fn preflight(
        &self,
        root: &Root,
        domain: &PrivateControlDomain,
        path_profile: &CanonicalPathIdentityV1,
        index: &[LosslessIndexEntry],
        worktree: &[TrackedWorktreeEntry],
    ) -> Result<PrivateNamespaceCollisionProof, CheckedFsError> {
        self.scan(root, domain, path_profile, index, worktree)?;
        Ok(PrivateNamespaceCollisionProof::cleared(
            domain,
            path_profile,
        ))
    }
}
