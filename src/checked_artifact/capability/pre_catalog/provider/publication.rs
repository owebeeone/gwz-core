//! Sealed source-associated catalog namespace publication.

use std::ffi::OsStr;
use std::io::Read;

use cap_std::fs::Dir;

use super::platform::HostPlatform;
use super::retained::encode_identity;
use crate::checked_artifact::capability::{CheckedFsError, DurableIdentityProvider};
use crate::model::ErrorCode;

pub(super) enum PublicationSourceV1<'a> {
    RegularFile {
        expected_identity: &'a [u8],
        expected_bytes: &'a [u8],
    },
    Directory {
        expected_identity: &'a [u8],
    },
}

impl<'a> PublicationSourceV1<'a> {
    pub(super) const fn regular_file(
        expected_identity: &'a [u8],
        expected_bytes: &'a [u8],
    ) -> Self {
        Self::RegularFile {
            expected_identity,
            expected_bytes,
        }
    }

    pub(super) const fn directory(expected_identity: &'a [u8]) -> Self {
        Self::Directory { expected_identity }
    }

    const fn expected_identity(&self) -> &[u8] {
        match self {
            Self::RegularFile {
                expected_identity, ..
            }
            | Self::Directory { expected_identity } => expected_identity,
        }
    }
}

pub(super) fn publish_verified_no_replace(
    source_dir: &Dir,
    source: &OsStr,
    destination_dir: &Dir,
    destination: &OsStr,
    expected: PublicationSourceV1<'_>,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let mut source_handle = crate::checked_artifact::platform::open_rename_source(
        source_dir,
        source,
        ErrorCode::IoError,
        label,
    )
    .map_err(|source| CheckedFsError::ambiguous(label, source.message))?;
    if encode_identity(&HostPlatform.file_identity(source_handle.file())?)
        != expected.expected_identity()
    {
        return Err(CheckedFsError::ambiguous(
            label,
            "publication source identity changed",
        ));
    }
    if let PublicationSourceV1::RegularFile { expected_bytes, .. } = expected {
        let mut bytes = Vec::with_capacity(expected_bytes.len() + 1);
        source_handle
            .file_mut()
            .by_ref()
            .take((expected_bytes.len() + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|source| CheckedFsError::io("read publication source", source))?;
        if bytes != expected_bytes {
            return Err(CheckedFsError::ambiguous(
                label,
                "publication source bytes changed",
            ));
        }
    }
    crate::checked_artifact::platform::rename_open_source(
        &source_handle,
        destination_dir,
        destination,
        false,
        ErrorCode::IoError,
        label,
    )
    .map_err(|source| CheckedFsError::ambiguous(label, source.message))
}
