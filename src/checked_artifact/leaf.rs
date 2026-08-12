//! Bounded ordinary and same-handle durable leaf-observation contracts.

use std::io::{self, Read};

use super::capability::{AsciiComponent, CheckedFsError};
use super::namespace::{NamespaceProtocol, RetainedDirectory};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LeafOther {
    WrongKind,
    Executable,
    Substituted,
    LengthMismatch,
    ContentMismatch,
    ParentChanged,
    NameChanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LeafProof<Identity> {
    Missing,
    Exact {
        identity: Identity,
        length: u64,
        sha256: [u8; 32],
        bytes: Vec<u8>,
    },
    Other(LeafOther),
}

impl<Identity> LeafProof<Identity> {
    pub(super) fn is_exact(&self) -> bool {
        matches!(self, Self::Exact { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DurableLeafProof<Identity> {
    MissingDurable,
    ExactDurable {
        identity: Identity,
        length: u64,
        sha256: [u8; 32],
    },
    Other(LeafOther),
}

impl<Identity> DurableLeafProof<Identity> {
    pub(super) fn is_exact_durable(&self) -> bool {
        matches!(self, Self::ExactDurable { .. })
    }
}

/// A value that can be opened twice for exact streaming comparison before and
/// after the namespace barrier. The observer must not assume one reader can be
/// rewound or retained between those two comparisons.
pub(super) trait ExpectedLeafContent {
    type Reader: Read;

    fn len(&self) -> u64;
    fn sha256(&self) -> [u8; 32];
    fn open(&self) -> io::Result<Self::Reader>;
}

pub(super) enum DurableLeafExpectation<'a, Content: ExpectedLeafContent + ?Sized> {
    Missing,
    Exact(&'a Content),
}

/// Platform-independent observation interface. Implementations must use a
/// bounded/fallible allocation for `observe_bounded`. `observe_durable` owns
/// the single retained leaf handle across exact proof, flush, namespace
/// barrier, and exact reobservation.
pub(super) trait LeafObserver {
    type DirectoryHandle;
    type Identity: Clone + Eq;
    type PathProfile;

    fn observe_bounded(
        &self,
        parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        leaf: &AsciiComponent,
        max_bytes: usize,
    ) -> Result<LeafProof<Self::Identity>, CheckedFsError>;

    fn observe_durable<Content, Protocol>(
        &self,
        parent: &RetainedDirectory<Self::DirectoryHandle, Self::Identity, Self::PathProfile>,
        leaf: &AsciiComponent,
        expected: DurableLeafExpectation<'_, Content>,
        namespace: &mut Protocol,
        barrier_ordinal: Protocol::BarrierOrdinal,
    ) -> Result<DurableLeafProof<Self::Identity>, CheckedFsError>
    where
        Content: ExpectedLeafContent + ?Sized,
        Protocol: NamespaceProtocol<
                DirectoryHandle = Self::DirectoryHandle,
                Identity = Self::Identity,
                PathProfile = Self::PathProfile,
            >;
}
