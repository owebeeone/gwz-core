//! Filesystem facts, independent of catalog policy and wire representations.
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "facts from all supported platforms share one representation"
)]
pub(crate) enum FsSupportProfile {
    LinuxPersistentHandle,
    MacPersistentId,
    WindowsFileId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FsLookupMode {
    Sensitive,
    AsciiCaseFold,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "facts from all supported platforms share one representation"
)]
pub(crate) enum FsPersistentIdentity {
    Linux {
        volume: [u8; 16],
        handle_type: i32,
        handle: Vec<u8>,
    },
    Mac {
        volume: [u8; 16],
        object: [u8; 8],
    },
    Windows {
        volume: Vec<u16>,
        file_id: [u8; 16],
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FsObjectIdentity {
    pub(crate) persistent: FsPersistentIdentity,
    pub(crate) invocation: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FsVolumeDescription {
    pub(crate) name: Option<String>,
    pub(crate) remote: bool,
    pub(crate) volatile: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code, reason = "capabilities differ across supported platforms")]
pub(crate) enum FsCapability {
    PersistentFilesystemIdentity,
    PathEquivalence,
    AtomicRenameDomain,
}

#[derive(Debug)]
pub(crate) enum FsProbeError {
    Unsupported {
        capability: FsCapability,
        detail: String,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

impl FsProbeError {
    pub(crate) fn unsupported(capability: FsCapability, detail: impl Into<String>) -> Self {
        Self::Unsupported {
            capability,
            detail: detail.into(),
        }
    }
    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "the private authority decoder recognizes identities written on every supported OS"
)]
pub(crate) enum FsLegacyDurableIdentity {
    Linux {
        filesystem_id: Vec<u8>,
        handle_type: i32,
        file_handle: Vec<u8>,
    },
    Mac {
        volume_uuid: [u8; 16],
        persistent_object_id: [u8; 8],
    },
    Windows {
        volume_guid: Vec<u16>,
        file_id: [u8; 16],
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "the cross-platform identity model is exhaustively tested on every host"
)]
pub(crate) enum FsLegacyInvocationIdentity {
    Unix {
        device: u64,
        inode: u64,
    },
    Windows {
        volume_guid: Vec<u16>,
        file_id: [u8; 16],
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "each host constructs only its own rename-domain proof variant"
)]
pub(crate) enum FsLegacyRenameDomain {
    LinuxMountId(u64),
    MacMountedFileSystem([u8; 8]),
    WindowsMountedVolume(Vec<u16>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FsLegacyObjectIdentity {
    pub(crate) durable: FsLegacyDurableIdentity,
    pub(crate) invocation: FsLegacyInvocationIdentity,
}
