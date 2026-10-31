//! A registered launch takes precedence over blocking holds, but only within a bound (plan Phase 1 remediation step R5;
//! `dev-docs/GwzPhase1-ReviewState.md` finding P3-1): at most [`SpawnGate::LAUNCHES_AHEAD`] launches are admitted while a
//! blocking hold waits, and the next admission is the blocking hold's. The count is of launches, not of time, so it does
//! not depend on the machine's speed or load.
//!
//! The rows run on every platform, on a gate that excludes in process (the process gate's kernel object is entered below
//! this rule, after the in-process lock is taken).
use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

/// The launch's poll (`https_auth::GATE_POLL`).
const POLL: Duration = Duration::from_millis(1);
/// Concurrent launchers in the saturation row: the reviewer's probe.
const LAUNCHERS: usize = 8;
/// Blocking holds the saturation row makes, one after the other, each against the saturated gate.
const BLOCKING_HOLDS: usize = 20;
/// The most launches that may be admitted ahead of a blocking hold that the saturation row sees. The gate admits
/// `LAUNCHES_AHEAD` after the hold begins to wait, and the row's count may include the one launch that held the gate as the
/// hold was registered.
const MOST_AHEAD: usize = SpawnGate::LAUNCHES_AHEAD + 1;

/// Waits (by yielding, with no clock) until `count` blocking holds of `gate` are waiting.
fn wait_for_waiting_holds(gate: &SpawnGate, count: usize) {
    while gate.shared.lock().waiting_holds != count {
        thread::yield_now();
    }
}

/// Makes a blocking hold while a launch is registered, and admits the launch whenever the gate takes it, until the blocking
/// hold is admitted. Returns the launches admitted ahead of it. A gate that never admits the hold is stopped at twenty times
/// the bound by dropping the launch, so that it fails the row instead of hanging it.
fn launches_ahead_of_a_blocking_hold(gate: &SpawnGate) -> usize {
    let launching = gate.launching();
    let (sender, receiver) = mpsc::channel();
    let blocking = {
        let gate = gate.clone();
        thread::spawn(move || {
            let hold = gate.hold();
            sender.send(()).unwrap();
            drop(hold);
        })
    };
    wait_for_waiting_holds(gate, 1);
    let mut ahead = 0;
    let mut admitted = false;
    while !admitted && ahead <= 20 * SpawnGate::LAUNCHES_AHEAD {
        if let Some(hold) = launching.try_hold() {
            // The blocking hold sends before it drops its hold, so a launch the gate admits after that is behind it.
            if receiver.try_recv().is_ok() {
                admitted = true;
            } else {
                ahead += 1;
            }
            drop(hold);
        } else {
            admitted = receiver.try_recv().is_ok();
            thread::yield_now();
        }
    }
    drop(launching);
    blocking.join().unwrap();
    ahead
}

/// A blocking hold is admitted after `LAUNCHES_AHEAD` launches, with the launch still registered and the gate free for it
/// each time.
#[test]
fn a_blocking_hold_is_admitted_after_the_launches_ahead_bound() {
    let gate = SpawnGate::with_exclusion(true);
    assert_eq!(
        launches_ahead_of_a_blocking_hold(&gate),
        SpawnGate::LAUNCHES_AHEAD,
        "a blocking hold is admitted after exactly the launches the gate allows ahead of it"
    );
}

/// The bound is per blocking hold: one that was served does not use up the next one's precedence.
#[test]
fn each_blocking_hold_is_admitted_after_the_launches_ahead_bound() {
    let gate = SpawnGate::with_exclusion(true);
    for _ in 0..3 {
        assert_eq!(
            launches_ahead_of_a_blocking_hold(&gate),
            SpawnGate::LAUNCHES_AHEAD
        );
    }
}

/// A launch is admitted at once while no blocking hold waits, however many launches went before: the bound is against
/// blocking holds, not against launches.
#[test]
fn launches_are_not_limited_while_no_blocking_hold_waits() {
    let gate = SpawnGate::with_exclusion(true);
    let launching = gate.launching();
    for _ in 0..(10 * SpawnGate::LAUNCHES_AHEAD) {
        drop(
            launching
                .try_hold()
                .expect("a launch takes a free gate while no blocking hold waits"),
        );
    }
}

/// `LAUNCHERS` launchers that never stop (each registers, polls like the helper's launch, holds the gate for one
/// yield and registers again). Dropping stops and joins them.
///
/// `ahead` counts the launches that ran while a blocking hold was waiting, since a blocking hold last ran: each launcher
/// adds one while it holds the gate if a blocking hold is waiting then, and the blocking hold reads and clears it when it
/// is admitted. Both happen under the gate, so the count is exact and needs no timing. A launch that held the gate as the
/// hold began to wait is counted though the gate did not count it, which is the one launch `MOST_AHEAD` allows beyond the bound.
struct Launchers {
    ahead: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handles: Vec<thread::JoinHandle<()>>,
}

/// The count at which the launchers give up, so that a gate that starves a blocking hold fails the row (the hold is
/// admitted once they stop, with a count over the bound) instead of hanging it.
const GIVE_UP: usize = 20 * MOST_AHEAD;

impl Launchers {
    fn start(gate: &SpawnGate) -> Self {
        let ahead = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let handles = (0..LAUNCHERS)
            .map(|_| {
                let (gate, ahead, stop) = (gate.clone(), ahead.clone(), stop.clone());
                thread::spawn(move || {
                    let done =
                        || stop.load(Ordering::Acquire) || ahead.load(Ordering::Acquire) > GIVE_UP;
                    while !done() {
                        let launching = gate.launching();
                        while !done() {
                            if let Some(hold) = launching.try_hold() {
                                if gate.shared.lock().waiting_holds > 0 {
                                    ahead.fetch_add(1, Ordering::AcqRel);
                                }
                                thread::yield_now();
                                drop(hold);
                                break;
                            }
                            thread::sleep(POLL);
                        }
                    }
                })
            })
            .collect();
        // Saturated before the first blocking hold is made: a launch is registered whenever the gate is released (all
        // but the one that was just admitted, which registers again at once), by yielding, with no clock.
        while gate.shared.lock().launches < LAUNCHERS - 1 {
            thread::yield_now();
        }
        Self {
            ahead,
            stop,
            handles,
        }
    }
}

impl Drop for Launchers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

/// The reviewer's saturation probe (State P3-1): `LAUNCHERS` launchers keep a launch registered at all times, so that
/// none is ever absent when the gate is released. Each of `BLOCKING_HOLDS` blocking holds, made in turn, must be admitted
/// with at most `MOST_AHEAD` launches run while it waited. Against a gate whose launches have unbounded precedence none is
/// admitted until the launchers give up.
#[test]
fn a_blocking_hold_is_admitted_within_the_bound_while_launchers_saturate_the_gate() {
    let gate = SpawnGate::with_exclusion(true);
    let launchers = Launchers::start(&gate);
    for blocking in 0..BLOCKING_HOLDS {
        let hold = gate.hold();
        let ahead = launchers.ahead.swap(0, Ordering::AcqRel);
        drop(hold);
        assert!(
            ahead <= MOST_AHEAD,
            "blocking hold {blocking} was admitted after {ahead} launches; the gate allows {MOST_AHEAD} at the most"
        );
    }
    drop(launchers);
    assert!(gate.try_hold().is_some(), "a launcher left the gate held");
}
