//! Shared bounded record I/O and canonical-decoding helpers.

use std::io::{self, Read};

use super::cleanup::DurableLeafFingerprintV1;
use super::generated;
use crate::checked_artifact::capability::{
    AsciiComponent, DurableObjectIdentityV1, DurablePathV1, SupportedFilesystemProfile,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ProtocolRecordKindV1 {
    Authority,
    Capacity,
    Admission,
    BarrierIntent,
    BootstrapIntent,
    CatalogBootstrap,
    Infrastructure,
    Marker,
    CleanupWorklist,
    DurablePath,
}

impl ProtocolRecordKindV1 {
    pub(in crate::checked_artifact) const ALL: &'static [Self] = &[
        Self::Authority,
        Self::Capacity,
        Self::Admission,
        Self::BarrierIntent,
        Self::BootstrapIntent,
        Self::CatalogBootstrap,
        Self::Infrastructure,
        Self::Marker,
        Self::CleanupWorklist,
        Self::DurablePath,
    ];

    pub(in crate::checked_artifact) const fn stable_name(self) -> &'static str {
        match self {
            Self::Authority => "authority",
            Self::Capacity => "capacity",
            Self::Admission => "admission",
            Self::BarrierIntent => "barrier_intent",
            Self::BootstrapIntent => "bootstrap_intent",
            Self::CatalogBootstrap => "catalog_bootstrap",
            Self::Infrastructure => "infrastructure",
            Self::Marker => "marker",
            Self::CleanupWorklist => "cleanup_worklist",
            Self::DurablePath => "durable_path",
        }
    }

    pub(in crate::checked_artifact) const fn max_bytes(self) -> usize {
        match self {
            Self::Authority
            | Self::Capacity
            | Self::Admission
            | Self::BarrierIntent
            | Self::BootstrapIntent
            | Self::CatalogBootstrap
            | Self::CleanupWorklist => 16 * 1024,
            Self::Infrastructure => 8 * 1024,
            Self::Marker | Self::DurablePath => 4 * 1024,
        }
    }
}

#[derive(Debug)]
pub(in crate::checked_artifact) enum ProtocolCodecErrorV1 {
    Io(io::Error),
    Oversize { limit: usize },
    Invalid(&'static str),
}

impl From<io::Error> for ProtocolCodecErrorV1 {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(test)]
pub(in crate::checked_artifact) trait BoundedCanonicalRecordV1:
    Sized
{
    const KIND: ProtocolRecordKindV1;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1>;
    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1>;
}

#[cfg(not(test))]
pub(super) trait BoundedCanonicalRecordV1: Sized {
    const KIND: ProtocolRecordKindV1;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1>;
    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1>;
}

#[cfg(test)]
pub(in crate::checked_artifact) fn read_bounded_record<T: BoundedCanonicalRecordV1>(
    reader: impl Read,
) -> Result<T, ProtocolCodecErrorV1> {
    read_bounded_record_inner(reader)
}

pub(super) fn read_bounded_record_inner<T: BoundedCanonicalRecordV1>(
    reader: impl Read,
) -> Result<T, ProtocolCodecErrorV1> {
    let bytes = read_bounded_bytes(reader, T::KIND.max_bytes())?;
    let value = T::decode_record(&bytes)?;
    if value.encode_record()? != bytes {
        return Err(ProtocolCodecErrorV1::Invalid(
            "record is not canonically encoded",
        ));
    }
    Ok(value)
}

/// Runs one record owner's private canonical decoder only after the shared
/// `limit + 1` read has accepted the complete byte sequence. The closure keeps
/// raw semantic decoding at the owning module boundary.
pub(in crate::checked_artifact) fn read_bounded_value<T>(
    kind: ProtocolRecordKindV1,
    reader: impl Read,
    decode_canonical: impl FnOnce(&[u8]) -> Result<T, ProtocolCodecErrorV1>,
    encode_canonical: impl FnOnce(&T) -> Vec<u8>,
) -> Result<T, ProtocolCodecErrorV1> {
    let bytes = read_bounded_bytes(reader, kind.max_bytes())?;
    let value = decode_canonical(&bytes)?;
    if encode_canonical(&value) != bytes {
        return Err(ProtocolCodecErrorV1::Invalid(
            "record is not canonically encoded",
        ));
    }
    Ok(value)
}

pub(super) fn encode_bounded_record(
    kind: ProtocolRecordKindV1,
    bytes: Vec<u8>,
) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
    if bytes.len() > kind.max_bytes() {
        return Err(ProtocolCodecErrorV1::Oversize {
            limit: kind.max_bytes(),
        });
    }
    Ok(bytes)
}

#[cfg(test)]
pub(in crate::checked_artifact) fn read_bounded_bytes(
    reader: impl Read,
    limit: usize,
) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
    read_bounded_bytes_inner(reader, limit)
}

#[cfg(not(test))]
pub(super) fn read_bounded_bytes(
    reader: impl Read,
    limit: usize,
) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
    read_bounded_bytes_inner(reader, limit)
}

fn read_bounded_bytes_inner(
    mut reader: impl Read,
    limit: usize,
) -> Result<Vec<u8>, ProtocolCodecErrorV1> {
    let read_limit = u64::try_from(limit)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(ProtocolCodecErrorV1::Invalid("record limit overflow"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(limit.min(16 * 1024))
        .map_err(|_| ProtocolCodecErrorV1::Invalid("bounded record allocation failed"))?;
    reader.by_ref().take(read_limit).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(ProtocolCodecErrorV1::Oversize { limit });
    }
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ScratchBytesV1 {
    Missing,
    PartialExpectedPrefix,
    Exact,
    Other,
}

pub(in crate::checked_artifact) fn classify_expected_prefix(
    observed: &[u8],
    expected: &[u8],
) -> ScratchBytesV1 {
    if observed == expected {
        ScratchBytesV1::Exact
    } else if observed.len() < expected.len() && expected.starts_with(observed) {
        ScratchBytesV1::PartialExpectedPrefix
    } else {
        ScratchBytesV1::Other
    }
}

pub(super) fn decode_identity(
    value: generated::CheckedDurableObjectIdentityV1,
) -> Result<DurableObjectIdentityV1, ProtocolCodecErrorV1> {
    DurableObjectIdentityV1::decode_canonical(&crate::cbor::encode(&value.to_cbor()))
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid durable identity"))
}

pub(super) fn decode_path(
    value: generated::CheckedDurablePathV1,
) -> Result<DurablePathV1, ProtocolCodecErrorV1> {
    DurablePathV1::decode_canonical(&crate::cbor::encode(&value.to_cbor()))
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid durable path"))
}

pub(super) fn decode_ascii(value: &[u8]) -> Result<AsciiComponent, ProtocolCodecErrorV1> {
    AsciiComponent::parse(value)
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid ASCII protocol component"))
}

pub(super) fn path_matches_profile(
    path: &DurablePathV1,
    profile: SupportedFilesystemProfile,
) -> bool {
    path.components()
        .iter()
        .all(|component| component.parent_durable_identity().support_profile() == profile)
}

pub(super) fn encode_fingerprint(
    value: &DurableLeafFingerprintV1,
) -> generated::CheckedDurableLeafFingerprintV1 {
    generated::CheckedDurableLeafFingerprintV1 {
        identity: value.identity().to_generated(),
        length_u64le: value.length().to_le_bytes().to_vec(),
        sha256: value.sha256().to_vec(),
    }
}

pub(super) fn decode_fingerprint(
    value: generated::CheckedDurableLeafFingerprintV1,
) -> Result<DurableLeafFingerprintV1, ProtocolCodecErrorV1> {
    Ok(DurableLeafFingerprintV1::new(
        decode_identity(value.identity)?,
        u64::from_le_bytes(super::schedule::checked_array(value.length_u64le)?),
        super::schedule::checked_array(value.sha256)?,
    ))
}
