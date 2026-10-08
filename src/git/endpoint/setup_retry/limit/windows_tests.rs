//! §4.3's window: what each result proves, and the races of §4.4 that act on
//! the sets (R1, R3, R5, R6, and a background close).
use super::{states::*, windows::*};
use std::collections::BTreeSet;

const CLOCKED: ConnEvent = ConnEvent::Started { clocked: true };

fn id(n: u64) -> ConnId {
    ConnId(n)
}
fn ids(ns: &[u64]) -> BTreeSet<ConnId> {
    ns.iter().map(|n| ConnId(*n)).collect()
}

/// A key with the connection and window bookkeeping the facade will do.
struct Key {
    table: Table,
    windows: Windows,
    now: u64,
}

impl Key {
    fn new(connected: u64) -> Self {
        let mut key = Self {
            table: Table::new(),
            windows: Windows::new(),
            now: 0,
        };
        for n in 1..=connected {
            key.report(n, CLOCKED);
            key.report(n, ConnEvent::Connected);
        }
        key
    }
    fn report(&mut self, conn: u64, event: ConnEvent) {
        if let Some(change) = self.table.apply(id(conn), event, self.now) {
            self.windows.observe(&change);
        }
    }
    fn at(&mut self, now: u64) {
        self.now = now;
        self.table.advance(now);
    }
    /// An attempt on a new connection, started now.
    fn start(&mut self, attempt: u64, conn: u64, kind: AttemptKind) {
        assert!(
            self.windows
                .open(AttemptId(attempt), kind, Own::New(id(conn)), &self.table)
        );
        self.report(conn, CLOCKED);
    }
    fn result(&mut self, attempt: u64) -> Closed {
        self.windows
            .close(AttemptId(attempt))
            .expect("an open window")
    }
}

#[test]
fn a_new_connections_window_starts_with_the_connected_and_the_possible_others() {
    let mut key = Key::new(3);
    key.report(2, ConnEvent::Closing);
    key.report(3, ConnEvent::Closing);
    key.report(3, ConnEvent::Disposed);
    key.start(1, 10, AttemptKind::Ordinary);
    let closed = key.result(1);
    // 1 Connected; 1, 2, 3 are Possible (2 closing, 3 settling).
    assert_eq!((closed.lo, closed.hi), (1, 3));
    assert_eq!(closed.winding_down, ids(&[2, 3]));
}

#[test]
fn a_leased_exchanges_connection_is_in_neither_set() {
    let mut key = Key::new(3);
    assert!(key.windows.open(
        AttemptId(1),
        AttemptKind::Ordinary,
        Own::Leased(id(2)),
        &key.table
    ));
    key.report(2, ConnEvent::Closing);
    let closed = key.result(1);
    assert_eq!((closed.lo, closed.hi), (2, 2));
}

#[test]
fn a_connection_that_starts_in_the_window_is_in_hi_and_never_in_lo() {
    let mut key = Key::new(2);
    key.start(1, 10, AttemptKind::Ordinary);
    key.start(2, 11, AttemptKind::Ordinary);
    key.report(11, ConnEvent::Connected);
    let closed = key.result(1);
    assert_eq!((closed.lo, closed.hi), (2, 3), "11 joined hi only");
    // The second attempt saw 10 already setting up.
    let closed = key.result(2);
    assert_eq!((closed.lo, closed.hi), (2, 3));
}

#[test]
fn a_connection_that_leaves_connected_leaves_lo_and_stays_in_hi() {
    let mut key = Key::new(4);
    key.start(1, 10, AttemptKind::Ordinary);
    key.report(1, ConnEvent::Closing); // the client begins to close it
    key.report(2, ConnEvent::ServerClosed); // the server ends it
    key.report(1, ConnEvent::Disposed);
    key.at(5_000);
    let closed = key.result(1);
    assert_eq!(
        (closed.lo, closed.hi),
        (2, 4),
        "nothing is ever removed from hi"
    );
    assert_eq!(closed.winding_down, ids(&[1]));
}

#[test]
fn r1_a_connection_settling_at_the_start_is_in_hi_and_in_the_overlap() {
    let mut key = Key::new(2);
    key.report(1, ConnEvent::Closing);
    key.report(1, ConnEvent::Disposed);
    key.start(1, 10, AttemptKind::Ordinary);
    key.at(300); // Y settles during the window
    let closed = key.result(1);
    assert_eq!((closed.lo, closed.hi), (1, 2));
    assert_eq!(closed.winding_down, ids(&[1]));
    assert!(
        !key.table.any_winding_down(&closed.winding_down),
        "settled by the result"
    );
}

#[test]
fn r3_a_wave_of_32_refused_together_each_has_hi_31() {
    let mut key = Key::new(0);
    for attempt in 1..=32 {
        key.start(attempt, 100 + attempt, AttemptKind::Ordinary);
    }
    for attempt in 1..=32 {
        let closed = key.result(attempt);
        assert_eq!((closed.lo, closed.hi), (0, 31), "attempt {attempt}");
    }
}

#[test]
fn r5_churn_in_a_long_window_pushes_hi_to_n_and_lo_down() {
    // Case 33: N = 8, 7 others Possible; two close, settle and are replaced.
    let mut key = Key::new(7);
    key.start(1, 50, AttemptKind::Ordinary);
    for (old, new) in [(1, 60), (2, 61)] {
        key.report(old, ConnEvent::Closing);
        key.report(old, ConnEvent::Disposed);
        key.at(key.now + 300);
        key.report(new, CLOCKED);
    }
    let closed = key.result(1);
    assert_eq!((closed.lo, closed.hi), (5, 9));
    assert_eq!(closed.winding_down, ids(&[1, 2]));
}

#[test]
fn r6_an_abandoned_setup_is_in_hi_while_it_lingers_and_after() {
    let mut key = Key::new(2);
    key.report(10, CLOCKED); // stalled, abandoned: stays Setting up
    key.start(1, 11, AttemptKind::Ordinary);
    key.at(2_000);
    key.report(10, ConnEvent::Retired);
    let closed = key.result(1);
    assert_eq!((closed.lo, closed.hi), (2, 3));
    assert_eq!(closed.winding_down, ids(&[10]));
}

#[test]
fn a_background_close_leaves_the_window_fair_until_the_connection_is_discarded() {
    // Option A: the closing fetch's connection is Connected, so a probe at
    // N = 3 whose window overlaps its background close has lo = 3.
    let mut key = Key::new(3);
    key.start(1, 10, AttemptKind::Probe); // the background close runs: no event
    assert_eq!(key.result(1).lo, 3);
    // Discarded after the close failed: it leaves lo.
    key.start(2, 11, AttemptKind::Probe);
    key.report(3, ConnEvent::Closing);
    assert_eq!(key.result(2).lo, 2);
}

#[test]
fn a_test_carrier_is_known_while_its_window_is_open() {
    let mut key = Key::new(1);
    let kinds = [
        (AttemptKind::Ordinary, false),
        (AttemptKind::Probe, true),
        (AttemptKind::Confirming, true),
        (AttemptKind::Restore, false),
    ];
    for (n, (kind, test)) in kinds.into_iter().enumerate() {
        let n = n as u64 + 1;
        key.start(n, 100 + n, kind);
        assert_eq!(kind.is_test(), test);
        assert_eq!(key.windows.is_test_carrier(AttemptId(n)), test, "{kind:?}");
        key.result(n);
        assert!(!key.windows.is_test_carrier(AttemptId(n)), "closed");
    }
    assert!(!key.windows.is_test_carrier(AttemptId(99)), "unknown");
}

#[test]
fn an_attempt_has_one_window_and_closing_an_unknown_one_returns_nothing() {
    let mut key = Key::new(1);
    key.start(1, 10, AttemptKind::Ordinary);
    assert!(!key.windows.open(
        AttemptId(1),
        AttemptKind::Probe,
        Own::New(id(11)),
        &key.table
    ));
    assert_eq!(key.result(1).kind, AttemptKind::Ordinary);
    assert_eq!(key.windows.close(AttemptId(1)), None);
}
