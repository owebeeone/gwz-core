//! Pure filesystem-capability contracts for checked artifacts.
//!
//! This module deliberately contains no host implementation. Its types freeze
//! the values that platform providers and the pre-catalog collision scan must
//! prove before checked-artifact code may create private state.

use std::io;

mod collision;
mod durable_identity;
mod path;
mod pre_catalog;

#[allow(
    unused_imports,
    reason = "R2 retains collision fact vocabulary for the sealed platform provider"
)]
pub(super) use collision::*;
pub(super) use durable_identity::DurableObjectIdentityV1;
pub(super) use path::*;
pub(super) use pre_catalog::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SupportedFilesystemProfile {
    LinuxExt4FsIocGetFsUuidV1,
    MacPersistentObjectIdV1,
    WindowsNtfsFileId128V1,
}

impl SupportedFilesystemProfile {
    pub(super) const ALL: &'static [Self] = &[
        Self::LinuxExt4FsIocGetFsUuidV1,
        Self::MacPersistentObjectIdV1,
        Self::WindowsNtfsFileId128V1,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum PlatformCapability {
    AsciiProtocolPath,
    PathEquivalence,
    DurableObjectIdentity,
    AtomicRenameDomain,
    NamespaceDurability,
    PrivateNamespaceCollisionScan,
    RuntimeAdvisoryLock,
    ManagedParentBootstrap,
}

#[derive(Debug)]
pub(super) enum CheckedFsError {
    Unsupported {
        capability: PlatformCapability,
        detail: String,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Ambiguous {
        fact: &'static str,
        detail: String,
    },
}

impl CheckedFsError {
    pub(super) fn unsupported(capability: PlatformCapability, detail: impl Into<String>) -> Self {
        Self::Unsupported {
            capability,
            detail: detail.into(),
        }
    }

    pub(super) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub(super) fn ambiguous(fact: &'static str, detail: impl Into<String>) -> Self {
        Self::Ambiguous {
            fact,
            detail: detail.into(),
        }
    }
}

pub(super) trait PathEquivalenceProvider<DirectoryHandle: ?Sized> {
    fn parent_mode(&self, parent: &DirectoryHandle) -> Result<PathComponentMode, CheckedFsError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ObjectIdentityFact<DurableIdentity, InvocationIdentity> {
    durable: DurableIdentity,
    invocation: InvocationIdentity,
}

impl<DurableIdentity, InvocationIdentity> ObjectIdentityFact<DurableIdentity, InvocationIdentity> {
    pub(super) fn new(durable: DurableIdentity, invocation: InvocationIdentity) -> Self {
        Self {
            durable,
            invocation,
        }
    }

    pub(super) fn durable(&self) -> &DurableIdentity {
        &self.durable
    }

    pub(super) fn invocation(&self) -> &InvocationIdentity {
        &self.invocation
    }
}

pub(super) trait DurableIdentityProvider<DirectoryHandle: ?Sized, FileHandle: ?Sized> {
    type InvocationIdentity: Clone + Eq;
    type RenameDomain: Clone + Eq;

    fn support_profile(&self) -> SupportedFilesystemProfile;

    fn dir_identity(
        &self,
        directory: &DirectoryHandle,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Self::InvocationIdentity>, CheckedFsError>;

    fn file_identity(
        &self,
        file: &FileHandle,
    ) -> Result<ObjectIdentityFact<DurableObjectIdentityV1, Self::InvocationIdentity>, CheckedFsError>;

    fn rename_domain(
        &self,
        directory: &DirectoryHandle,
    ) -> Result<Self::RenameDomain, CheckedFsError>;
}
