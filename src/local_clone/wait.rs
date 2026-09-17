//! `--wait <secs>` on the family lock (GwzLaneCleanFixes R21).
//!
//! The family lock is a try-lock and nothing else: `FamilyStore::try_lock`
//! takes one OS advisory lock without waiting and reports
//! [`StoreError::Busy`] the moment another family operation holds it
//! (`gwz-family-store`'s `lock` module). That is exactly right for a person
//! at a terminal and exactly wrong for two hooks that were fired for one
//! request, so `--wait <secs>` is added *here*, above the store contract,
//! rather than inside it.
//!
//! The wait is a poll, deliberately:
//!
//! - **No blocking acquisition.** Nothing calls a blocking `flock`, so the
//!   wait is portable to every platform the try-lock already supports, and
//!   a stuck holder costs the caller its own deadline and nothing more.
//! - **A fixed short interval** ([`POLL_INTERVAL`]), so the loop is boring
//!   and predictable rather than a backoff schedule with its own behaviour
//!   to reason about.
//! - **Only `Busy` is retried.** Every other refusal -- a malformed index,
//!   an unsupported platform lock, an I/O failure -- is the answer, and
//!   repeating it until a deadline would only delay it.
//! - **The deadline is checked before sleeping**, so `--wait 0` is exactly
//!   today's behaviour: one attempt, then `Busy`.
//!
//! Nothing here reads or writes the index. Rereading after a wait is not
//! this module's job and does not need to be: every caller takes the
//! session it is handed and rereads through it under the lock
//! (`FamilySession::reread`, and `FamilySession::apply`'s own reread), so a
//! create that waited behind another create of the same name is answered by
//! the index that create left behind -- the standing row, its owner and
//! all -- and never by the view it observed before the wait.

use std::thread::sleep;
use std::time::{Duration, Instant};

use gwz_family_store_contract::{FamilyLocation, FamilyStore, StoreError};

/// How often a wait retries the try-lock. Short enough that a wait ends
/// promptly after the holder leaves, long enough that a long wait is not a
/// spin.
pub const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Take the family lock, retrying a busy lock until `wait` is spent.
///
/// `wait` of `None` is the unchanged behaviour: one attempt, and `Busy` is
/// immediate. A wait that succeeds returns an ordinary session, and the
/// caller rereads through it exactly as it would have without a wait.
pub(crate) fn lock_family<S: FamilyStore>(
    store: &S,
    location: &FamilyLocation,
    wait: Option<Duration>,
) -> Result<S::Session, StoreError> {
    let deadline = wait.map(|wait| Instant::now() + wait);
    loop {
        match store.try_lock(location) {
            Err(StoreError::Busy { lock_path }) => {
                let Some(deadline) = deadline else {
                    return Err(StoreError::Busy { lock_path });
                };
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(StoreError::Busy { lock_path });
                }
                sleep(POLL_INTERVAL.min(remaining));
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwz_family_store::YamlFamilyStore;

    #[test]
    fn a_wait_of_none_reports_busy_at_once() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".gwz")).unwrap();
        let store = YamlFamilyStore::new();
        let location = FamilyLocation::new(&root);
        let held = lock_family(&store, &location, None).expect("the first lock");
        let started = Instant::now();
        let busy = lock_family(&store, &location, None).expect_err("the second is busy");
        assert!(matches!(busy, StoreError::Busy { .. }), "{busy:?}");
        assert!(
            started.elapsed() < POLL_INTERVAL,
            "no wait means no sleeping"
        );
        drop(held);
    }

    #[test]
    fn a_wait_that_expires_still_reports_busy_and_spends_the_deadline() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".gwz")).unwrap();
        let store = YamlFamilyStore::new();
        let location = FamilyLocation::new(&root);
        let held = lock_family(&store, &location, None).expect("the first lock");
        let wait = POLL_INTERVAL * 4;
        let started = Instant::now();
        let busy = lock_family(&store, &location, Some(wait)).expect_err("still busy");
        assert!(matches!(busy, StoreError::Busy { .. }), "{busy:?}");
        assert!(
            started.elapsed() >= wait,
            "the whole deadline is spent before Busy is reported"
        );
        drop(held);
    }

    #[test]
    fn a_wait_wins_the_lock_once_the_holder_leaves() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let root = temp.path().join("root");
        std::fs::create_dir_all(root.join(".gwz")).unwrap();
        let store = YamlFamilyStore::new();
        let location = FamilyLocation::new(&root);
        let held = lock_family(&store, &location, None).expect("the first lock");
        std::thread::scope(|scope| {
            scope.spawn(|| {
                sleep(POLL_INTERVAL * 2);
                drop(held);
            });
            let won = lock_family(&store, &location, Some(Duration::from_secs(30)))
                .expect("the wait wins the lock the holder released");
            drop(won);
        });
    }
}
