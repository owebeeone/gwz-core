//! Shared bounded record I/O and canonical-decoding helpers.

use std::io::{self, Read};

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
    CanonicalPathIdentity,
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
        Self::CanonicalPathIdentity,
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
            Self::CanonicalPathIdentity => "canonical_path_identity",
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
            Self::Marker | Self::CanonicalPathIdentity => 4 * 1024,
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

pub(in crate::checked_artifact) trait BoundedCanonicalRecordV1:
    Sized
{
    const KIND: ProtocolRecordKindV1;

    fn encode_record(&self) -> Result<Vec<u8>, ProtocolCodecErrorV1>;
    fn decode_record(bytes: &[u8]) -> Result<Self, ProtocolCodecErrorV1>;
}

pub(in crate::checked_artifact) fn read_bounded_record<T: BoundedCanonicalRecordV1>(
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

pub(in crate::checked_artifact) fn read_bounded_bytes(
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
