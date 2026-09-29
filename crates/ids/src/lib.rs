//! Unique numbers from a source each context owns (GwzCoreSessionCrateMap §2).
//!
//! An [`IdSource`] pairs a 64-bit prefix, fixed when the source is made, with
//! a counter inside the instance. [`IdSource::next`] is unique within the
//! source; [`IdSource::unique`] adds the prefix, so it is unique across
//! sources, contexts and processes unless two prefixes are equal. Core draws
//! each context's prefix from the operating system's random source; tests pass
//! fixed ones. The counter is never a static: no crate keeps global mutable
//! state (the map's §1).

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// A prefix and a counter: the unique numbers of one context.
#[derive(Debug)]
pub struct IdSource {
    prefix: u64,
    next: AtomicU64,
}

impl IdSource {
    /// A source whose numbers carry `prefix` and whose counter starts at zero.
    pub fn new(prefix: u64) -> Self {
        Self {
            prefix,
            next: AtomicU64::new(0),
        }
    }

    /// The prefix every [`unique`](Self::unique) value of this source carries.
    pub fn prefix(&self) -> u64 {
        self.prefix
    }

    /// The next number, unique within this source: 0, 1, 2 and so on.
    ///
    /// # Panics
    ///
    /// After `u64::MAX` draws, rather than repeat a number.
    pub fn next(&self) -> u64 {
        self.next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("IdSource exhausted: every u64 has been drawn")
    }

    /// The next number paired with this source's prefix.
    pub fn unique(&self) -> UniqueId {
        UniqueId {
            prefix: self.prefix,
            sequence: self.next(),
        }
    }
}

/// A prefix and a sequence number. It displays as `{prefix:016x}-{sequence:x}`,
/// a name part made of lowercase hexadecimal digits and one `-`, so it is safe
/// in a file name on every platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UniqueId {
    prefix: u64,
    sequence: u64,
}

impl UniqueId {
    pub fn prefix(self) -> u64 {
        self.prefix
    }

    pub fn sequence(self) -> u64 {
        self.sequence
    }
}

impl fmt::Display for UniqueId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:016x}-{:x}", self.prefix, self.sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[test]
    fn next_counts_from_zero_and_never_repeats() {
        let ids = IdSource::new(7);
        let drawn: Vec<u64> = (0..1000).map(|_| ids.next()).collect();
        assert_eq!(drawn, (0..1000).collect::<Vec<u64>>());
    }

    #[test]
    fn two_sources_count_independently() {
        let first = IdSource::new(1);
        let second = IdSource::new(1);
        assert_eq!((first.next(), first.next()), (0, 1));
        assert_eq!(second.next(), 0, "the counter lives in the instance");
    }

    #[test]
    fn unique_pairs_the_prefix_with_the_counter() {
        let ids = IdSource::new(0xfeed);
        assert_eq!(ids.prefix(), 0xfeed);
        let first = ids.unique();
        let second = ids.unique();
        assert_eq!((first.prefix(), first.sequence()), (0xfeed, 0));
        assert_eq!((second.prefix(), second.sequence()), (0xfeed, 1));
        assert_eq!(ids.next(), 2, "unique and next draw from one counter");
        let other = IdSource::new(0xbeef).unique();
        assert_ne!(first, other, "equal counters, different prefixes");
    }

    #[test]
    fn unique_displays_as_a_file_name_part() {
        let ids = IdSource::new(0x0123_4567_89ab_cdef);
        assert_eq!(ids.unique().to_string(), "0123456789abcdef-0");
        for _ in 0..30 {
            ids.next();
        }
        assert_eq!(ids.unique().to_string(), "0123456789abcdef-1f");
        let small = IdSource::new(0x2a);
        assert_eq!(
            small.unique().to_string(),
            "000000000000002a-0",
            "the prefix keeps 16 digits"
        );
    }

    #[test]
    fn a_shared_source_is_unique_across_threads() {
        fn send_and_sync<T: Send + Sync>() {}
        send_and_sync::<IdSource>();
        let ids = Arc::new(IdSource::new(3));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let ids = Arc::clone(&ids);
                std::thread::spawn(move || (0..1000).map(|_| ids.next()).collect::<Vec<_>>())
            })
            .collect();
        let mut seen = HashSet::new();
        for handle in handles {
            for number in handle.join().unwrap() {
                assert!(seen.insert(number), "{number} drawn twice");
            }
        }
        assert_eq!(seen.len(), 8000);
    }

    #[test]
    #[should_panic(expected = "exhausted")]
    fn an_exhausted_source_panics_rather_than_repeat() {
        let ids = IdSource {
            prefix: 0,
            next: AtomicU64::new(u64::MAX - 1),
        };
        assert_eq!(ids.next(), u64::MAX - 1);
        ids.next();
    }
}
