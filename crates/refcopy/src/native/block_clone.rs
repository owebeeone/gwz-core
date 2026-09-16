//! The arithmetic of a `FSCTL_DUPLICATE_EXTENTS_TO_FILE` request, kept off the
//! platform so it can be read and tested from any host -- the same reason the
//! failure tables above are.
//!
//! Block cloning duplicates a *range*, not a file, and the range has to obey
//! two rules the caller must satisfy itself: every offset and length is a
//! multiple of the volume's allocation unit, and one call moves at most
//! [`block_clone::MAX_DUPLICATE_BYTES`]. [`block_clone::next_range`] turns a
//! file length and a cluster size into the sequence of requests that obeys
//! both.
#![allow(dead_code)]

/// Most bytes one `FSCTL_DUPLICATE_EXTENTS_TO_FILE` may duplicate. Its
/// reference topic caps a single request at 4 GiB; 4 GiB is a whole number
/// of clusters for every cluster size a volume can be formatted with (all
/// are powers of two no larger than it), so chunking here never breaks the
/// alignment rule.
pub(crate) const MAX_DUPLICATE_BYTES: u64 = 4 << 30;

/// One duplicate-extents request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Range {
    /// Source and target file offset. Always cluster-aligned.
    pub(crate) offset: u64,
    /// `ByteCount`: cluster-aligned, and for the final range **rounded
    /// up** past the file's length.
    pub(crate) count: u64,
    /// Bytes of the file this request accounts for -- `count` except in
    /// the final range, where it is the unrounded remainder.
    pub(crate) advance: u64,
}

/// The request that continues a file of `length` bytes whose first
/// `offset` bytes are already duplicated, or `None` when there is nothing
/// left. An empty file yields nothing at all: it has no extents.
pub(crate) fn next_range(offset: u64, length: u64, cluster_bytes: u64) -> Option<Range> {
    if offset >= length {
        return None;
    }
    let advance = (length - offset).min(MAX_DUPLICATE_BYTES);
    Some(Range {
        offset,
        count: round_up_to_cluster(advance, cluster_bytes),
        advance,
    })
}

/// Round `bytes` up to a whole number of `cluster_bytes`.
///
/// A file's length is almost never a multiple of the allocation unit, and
/// the call refuses a `ByteCount` that is not. Rounding **up** rather than
/// down is the documented pattern and is what makes the last cluster of
/// the file arrive: the extra bytes lie inside the cluster the source has
/// already had allocated to it, and inside the one the pre-sized
/// destination has too, so the request stays within both allocations even
/// though it runs past the valid data length. Rounding down would silently
/// drop the tail.
pub(crate) fn round_up_to_cluster(bytes: u64, cluster_bytes: u64) -> u64 {
    // Not a volume geometry; answering `bytes` keeps this total, and the
    // wrapper has already declined a volume that describes itself so.
    if cluster_bytes == 0 {
        return bytes;
    }
    match bytes % cluster_bytes {
        0 => bytes,
        // Every call site bounds `bytes` by `MAX_DUPLICATE_BYTES`, so this
        // cannot overflow; saturating keeps the function total for a
        // caller that ignores the bound.
        remainder => bytes.saturating_add(cluster_bytes - remainder),
    }
}
