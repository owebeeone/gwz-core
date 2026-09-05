//! `gwz-history-check`: bounded, read-only, in-memory history verification
//! (lane H).
//!
//! [`check_history`] decides whether every protected root of a deletion
//! target is reachable, with a complete locally available object graph, from
//! the surviving witnesses' retained roots (gwz-dev
//! `dev-docs/GwzLocalCloneImplementationArchitecture.md` §5, design §5.1).
//! It reads through an injected `gwz_repo_contract::ObjectReader`, memoizes
//! visits within one invocation, accounts for its own bookkeeping against
//! [`Limits`], polls its own [`Cancellation`] port between bounded units,
//! and persists nothing. A read failure, cancellation or exceeded limit is
//! [`HistoryOutcome::Unknown`], never `Verified`. Nothing here can delete.
//!
//! LCM1.0c checkpoint state: the vocabulary is frozen; `check_history`
//! returns `Unknown` with an `Unimplemented` reason and performs no read.

#![forbid(unsafe_code)]

use gwz_repo_contract::{
    ObjectId, ObjectReader, ProtectedRoot, ProtectedRoots, RepoKey, UnknownReason,
};

/// Cooperative cancellation port owned by this crate.
pub trait Cancellation {
    fn is_cancelled(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NeverCancelled;

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// One surviving family repository eligible to preserve history, paired
/// with the target repository by identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Witness {
    pub repository: RepoKey,
    /// Human-readable location for diagnostics only.
    pub label: String,
}

/// Explicit resource bounds. Exceeding either returns `Unknown` before any
/// disposing row is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_roots: u64,
    pub max_bookkeeping_bytes: u64,
}

impl Default for Limits {
    /// The architecture's initial cap: 100,000 roots and 256 MiB of
    /// verifier bookkeeping.
    fn default() -> Self {
        Self {
            max_roots: 100_000,
            max_bookkeeping_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub roots_checked: u64,
    pub objects_visited: u64,
    pub witnesses_used: Vec<RepoKey>,
    /// Bookkeeping bytes accounted at completion.
    pub bookkeeping_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnpreservedItem {
    pub root: ProtectedRoot,
    /// The first object found missing from every witness, when the root
    /// itself was present somewhere.
    pub missing: Option<ObjectId>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryOutcome {
    Verified(Coverage),
    Unpreserved(Vec<UnpreservedItem>),
    Unknown(Vec<UnknownReason>),
}

impl HistoryOutcome {
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified(_))
    }
}

/// Check that every root in `protected` is preserved by `witnesses` through
/// `reader`.
pub fn check_history(
    _protected: &ProtectedRoots,
    _witnesses: &[Witness],
    _reader: &dyn ObjectReader,
    _limits: Limits,
    _cancellation: &dyn Cancellation,
) -> HistoryOutcome {
    HistoryOutcome::Unknown(vec![UnknownReason::unimplemented(
        "gwz-history-check: check_history",
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_repo_contract::contract_tests::GraphFixture;
    use gwz_repo_contract::{ObjectFormat, UnknownKind};

    #[test]
    fn checkpoint_check_is_unknown_and_reads_nothing() {
        let (fixture, reader) = GraphFixture::small(ObjectFormat::Sha1);
        let witnesses = [Witness {
            repository: RepoKey::Root,
            label: "root".to_owned(),
        }];
        let outcome = check_history(
            &fixture.roots,
            &witnesses,
            &reader,
            Limits::default(),
            &NeverCancelled,
        );
        let HistoryOutcome::Unknown(reasons) = outcome else {
            panic!("the checkpoint verifier never verifies");
        };
        assert_eq!(reasons[0].kind, UnknownKind::Unimplemented);
        assert!(reader.reads().is_empty(), "no object was read");
        assert!(!HistoryOutcome::Unknown(Vec::new()).is_verified());
        assert_eq!(Limits::default().max_roots, 100_000);
    }
}
