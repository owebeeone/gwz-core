//! Pure filesystem-capability contracts for checked artifacts.
//!
//! This module deliberately contains no host implementation. Its types freeze
//! the values that platform providers and the pre-catalog collision scan must
//! prove before checked-artifact code may create private state.

use std::io;

mod collision;
mod durable_identity;

use super::protocol::generated;
pub(super) use collision::*;
pub(super) use durable_identity::DurableObjectIdentityV1;

pub(super) const MAX_PATH_COMPONENT_BYTES: usize = 255;
pub(super) const MAX_CANONICAL_PATH_IDENTITY_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct AsciiComponent(Vec<u8>);

impl AsciiComponent {
    pub(super) fn parse(value: &[u8]) -> Result<Self, CheckedFsError> {
        if value.is_empty()
            || value.len() > MAX_PATH_COMPONENT_BYTES
            || matches!(value, b"." | b"..")
            || !value.is_ascii()
            || value.contains(&0)
            || value.contains(&b'/')
            || value.contains(&b'\\')
        {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::AsciiProtocolPath,
                "path component must be nonempty normal ASCII and at most 255 bytes",
            ));
        }
        Ok(Self(value.to_vec()))
    }

    pub(super) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum PathComponentMode {
    Sensitive,
    AsciiCaseFold,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CanonicalComponent {
    original_ascii: AsciiComponent,
    parent_mode: PathComponentMode,
    canonical_ascii: Vec<u8>,
}

impl CanonicalComponent {
    pub(super) fn new(original_ascii: AsciiComponent, parent_mode: PathComponentMode) -> Self {
        let canonical_ascii = match parent_mode {
            PathComponentMode::Sensitive => original_ascii.as_bytes().to_vec(),
            PathComponentMode::AsciiCaseFold => original_ascii
                .as_bytes()
                .iter()
                .map(u8::to_ascii_lowercase)
                .collect(),
        };
        Self {
            original_ascii,
            parent_mode,
            canonical_ascii,
        }
    }

    pub(super) fn original(&self) -> &AsciiComponent {
        &self.original_ascii
    }

    pub(super) fn parent_mode(&self) -> PathComponentMode {
        self.parent_mode
    }

    pub(super) fn canonical_ascii(&self) -> &[u8] {
        &self.canonical_ascii
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CanonicalPathIdentityV1 {
    components: Vec<CanonicalComponent>,
}

impl CanonicalPathIdentityV1 {
    pub(super) fn new(components: Vec<CanonicalComponent>) -> Result<Self, CheckedFsError> {
        if components.is_empty() {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PathEquivalence,
                "canonical path identity is empty",
            ));
        }
        let value = Self { components };
        if value.encode_canonical().len() > MAX_CANONICAL_PATH_IDENTITY_BYTES {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PathEquivalence,
                "canonical path identity exceeds 4 KiB",
            ));
        }
        Ok(value)
    }

    pub(super) fn components(&self) -> &[CanonicalComponent] {
        &self.components
    }

    pub(super) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    pub(super) fn decode_canonical(bytes: &[u8]) -> Result<Self, CheckedFsError> {
        let cbor = crate::cbor::try_decode(bytes).map_err(|_| {
            CheckedFsError::ambiguous("canonical path identity", "invalid taut encoding")
        })?;
        let wire = generated::CheckedCanonicalPathIdentityV1::from_cbor(&cbor).map_err(|_| {
            CheckedFsError::ambiguous("canonical path identity", "invalid taut record shape")
        })?;
        let mut components = Vec::new();
        components
            .try_reserve_exact(wire.components.len())
            .map_err(|_| {
                CheckedFsError::unsupported(
                    PlatformCapability::PathEquivalence,
                    "canonical path component allocation failed",
                )
            })?;
        for value in wire.components {
            let mode = match value.parent_mode {
                generated::CheckedPathComponentMode::Sensitive => PathComponentMode::Sensitive,
                generated::CheckedPathComponentMode::AsciiCaseFold => {
                    PathComponentMode::AsciiCaseFold
                }
            };
            let original = AsciiComponent::parse(&value.original_ascii)?;
            let component = CanonicalComponent::new(original, mode);
            if value.canonical_ascii != component.canonical_ascii() {
                return Err(CheckedFsError::ambiguous(
                    "canonical path identity",
                    "stored component is not canonical",
                ));
            }
            components.push(component);
        }
        let value = Self::new(components)?;
        if value.encode_canonical() != bytes {
            return Err(CheckedFsError::ambiguous(
                "canonical path identity",
                "noncanonical encoding",
            ));
        }
        Ok(value)
    }

    pub(in crate::checked_artifact) fn to_generated(
        &self,
    ) -> generated::CheckedCanonicalPathIdentityV1 {
        generated::CheckedCanonicalPathIdentityV1 {
            components: self
                .components
                .iter()
                .map(|component| generated::CheckedCanonicalComponentV1 {
                    original_ascii: component.original_ascii.as_bytes().to_vec(),
                    parent_mode: match component.parent_mode {
                        PathComponentMode::Sensitive => {
                            generated::CheckedPathComponentMode::Sensitive
                        }
                        PathComponentMode::AsciiCaseFold => {
                            generated::CheckedPathComponentMode::AsciiCaseFold
                        }
                    },
                    canonical_ascii: component.canonical_ascii.clone(),
                })
                .collect(),
        }
    }
}

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
    type DurableIdentity: Clone + Eq;
    type InvocationIdentity: Clone + Eq;
    type RenameDomain: Clone + Eq;

    fn support_profile(&self) -> SupportedFilesystemProfile;

    fn dir_identity(
        &self,
        directory: &DirectoryHandle,
    ) -> Result<ObjectIdentityFact<Self::DurableIdentity, Self::InvocationIdentity>, CheckedFsError>;

    fn file_identity(
        &self,
        file: &FileHandle,
    ) -> Result<ObjectIdentityFact<Self::DurableIdentity, Self::InvocationIdentity>, CheckedFsError>;

    fn rename_domain(
        &self,
        directory: &DirectoryHandle,
    ) -> Result<Self::RenameDomain, CheckedFsError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FilesystemCapabilityProof<RootIdentity> {
    support_profile: SupportedFilesystemProfile,
    root_identity: RootIdentity,
    path_profile: CanonicalPathIdentityV1,
}

impl<RootIdentity> FilesystemCapabilityProof<RootIdentity> {
    fn new(
        support_profile: SupportedFilesystemProfile,
        root_identity: RootIdentity,
        path_profile: CanonicalPathIdentityV1,
    ) -> Self {
        Self {
            support_profile,
            root_identity,
            path_profile,
        }
    }

    pub(super) fn support_profile(&self) -> SupportedFilesystemProfile {
        self.support_profile
    }

    pub(super) fn root_identity(&self) -> &RootIdentity {
        &self.root_identity
    }

    pub(super) fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        &self.path_profile
    }
}

/// One provider-owned inspection issues the otherwise opaque capability proof.
/// Consumers cannot combine a profile, root identity, and path profile that
/// were observed by different invocations.
pub(super) trait FilesystemCapabilityPreflight<Root: ?Sized> {
    type RootIdentity: Clone + Eq;

    fn inspect(
        &self,
        root: &Root,
    ) -> Result<
        (
            SupportedFilesystemProfile,
            Self::RootIdentity,
            CanonicalPathIdentityV1,
        ),
        CheckedFsError,
    >;

    fn preflight(
        &self,
        root: &Root,
    ) -> Result<FilesystemCapabilityProof<Self::RootIdentity>, CheckedFsError> {
        let (support_profile, root_identity, path_profile) = self.inspect(root)?;
        Ok(FilesystemCapabilityProof::new(
            support_profile,
            root_identity,
            path_profile,
        ))
    }
}
