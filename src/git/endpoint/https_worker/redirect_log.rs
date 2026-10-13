//! What an endpoint remembers of its discovery redirects, so that the words around a request can name them
//! (parity design section 14 item 13, U9 and text T5).
//!
//! The failure a refused redirect ends in is a fixed protocol failure that has no place for an address, and the
//! address is the server's own text. The endpoint therefore keeps it here, by the open that met it (its operation and the
//! canonical address it was asked to open), and the transport host asks for it by the same two when it words that open's
//! failure; one member's refusal is never another's. The log belongs to one endpoint (no global), holds a bounded number of entries,
//! and stores only what a message may show: addresses without credentials, cut to a bounded length.
use std::{collections::HashMap, sync::Mutex};

/// Followed redirects not yet told, and refusals not yet worded, at most.
const LIMIT: usize = 64;
/// The longest address a message shows. A longer one is cut and ends in `...`.
pub(crate) const SHOWN: usize = 256;

/// A discovery redirect the endpoint followed: from the address it asked to the address the server sent it to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Followed {
    pub(crate) from: String,
    pub(crate) to: String,
}

/// A redirect from https to http, refused: the address that redirected, and the one the server named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Refused {
    pub(crate) from: String,
    pub(crate) to: String,
}

#[derive(Default)]
struct Entries {
    followed: Vec<Followed>,
    /// By the open: its operation, and the canonical address (as the member's URL parses) it was asked to open.
    refused: HashMap<(String, String), Refused>,
}

#[derive(Default)]
pub(crate) struct RedirectLog {
    entries: Mutex<Entries>,
}

impl RedirectLog {
    fn entries(&self) -> std::sync::MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Remembers a redirect that was followed. The oldest goes when the log is full.
    pub(crate) fn followed(&self, from: &str, to: &str) {
        let mut entries = self.entries();
        if entries.followed.len() == LIMIT {
            entries.followed.remove(0);
        }
        entries.followed.push(Followed {
            from: shown(from),
            to: shown(to),
        });
    }

    /// Remembers that the redirect from `from` to `to` was refused for the open of `open` (the canonical address the
    /// member was asked to open) in `operation`. One per open: the first stands, because it is the one that ended it.
    pub(crate) fn refused(&self, operation: &str, open: &str, from: &str, to: &str) {
        let mut entries = self.entries();
        let key = (operation.to_owned(), open.to_owned());
        if entries.refused.len() == LIMIT && !entries.refused.contains_key(&key) {
            return;
        }
        entries.refused.entry(key).or_insert_with(|| Refused {
            from: shown(from),
            to: shown(to),
        });
    }

    /// The followed redirects since the last call, oldest first.
    pub(crate) fn drain_followed(&self) -> Vec<Followed> {
        std::mem::take(&mut self.entries().followed)
    }

    /// The redirect the open of `open` in `operation` was refused at, once.
    pub(crate) fn take_refused(&self, operation: &str, open: &str) -> Option<Refused> {
        self.entries()
            .refused
            .remove(&(operation.to_owned(), open.to_owned()))
    }
}

/// `address` cut to what a message shows.
fn shown(address: &str) -> String {
    if address.len() <= SHOWN {
        return address.to_owned();
    }
    let mut end = SHOWN;
    while !address.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &address[..end])
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;

            #[test]
            fn followed_redirects_are_told_once_oldest_first() {
                let log = RedirectLog::default();
                log.followed("https://a.example/one", "https://b.example/one");
                log.followed("https://b.example/one", "https://c.example/one");
                let told = log.drain_followed();
                assert_eq!(told.len(), 2);
                assert_eq!(told[0].from, "https://a.example/one");
                assert_eq!(told[1].to, "https://c.example/one");
                assert!(log.drain_followed().is_empty());
            }

            #[test]
            fn the_followed_log_is_bounded_and_keeps_the_newest() {
                let log = RedirectLog::default();
                for hop in 0..(LIMIT + 10) {
                    log.followed("https://a.example/", &format!("https://b.example/{hop}"));
                }
                let told = log.drain_followed();
                assert_eq!(told.len(), LIMIT);
                assert_eq!(told[0].to, "https://b.example/10");
                assert_eq!(told.last().unwrap().to, format!("https://b.example/{}", LIMIT + 9));
            }

            const OPEN: &str = "https://a.example/repo";

            #[test]
            fn a_refusal_belongs_to_its_open_and_is_told_once() {
                let log = RedirectLog::default();
                log.refused("op-1", OPEN, "https://a.example/", "http://b.example/");
                // The first refusal of an open is the one that ended it.
                log.refused("op-1", OPEN, "https://a.example/", "http://other.example/");
                assert!(log.take_refused("op-2", OPEN).is_none());
                assert!(log.take_refused("op-1", "https://c.example/repo").is_none());
                let refused = log.take_refused("op-1", OPEN).unwrap();
                assert_eq!(refused.to, "http://b.example/");
                assert!(log.take_refused("op-1", OPEN).is_none());
            }

            #[test]
            fn two_opens_of_one_operation_keep_a_refusal_each() {
                let log = RedirectLog::default();
                log.refused("op", "https://a.example/one", "https://a.example/one", "http://x.example/");
                log.refused("op", "https://a.example/two", "https://a.example/two", "http://y.example/");
                // Taken in either order, each is its own; neither is lost and neither is swapped.
                assert_eq!(log.take_refused("op", "https://a.example/two").unwrap().to, "http://y.example/");
                assert_eq!(log.take_refused("op", "https://a.example/one").unwrap().to, "http://x.example/");
            }

            #[test]
            fn the_refusals_are_bounded() {
                let log = RedirectLog::default();
                for operation in 0..(LIMIT + 5) {
                    log.refused(&format!("op-{operation}"), OPEN, "https://a.example/", "http://b.example/");
                }
                assert!(log.take_refused("op-0", OPEN).is_some());
                assert!(log.take_refused(&format!("op-{}", LIMIT + 1), OPEN).is_none());
            }

            #[test]
            fn a_long_address_is_cut_on_a_character_boundary() {
                let log = RedirectLog::default();
                let long = format!("http://a.example/{}", "\u{e9}".repeat(300));
                log.refused("op", OPEN, "https://a.example/", &long);
                let refused = log.take_refused("op", OPEN).unwrap();
                assert!(refused.to.ends_with("..."));
                assert!(refused.to.len() <= SHOWN + 3);
                assert!(std::str::from_utf8(refused.to.as_bytes()).is_ok());
            }
        }
    }
}
