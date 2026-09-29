#![cfg(test)]
//! Tests of cancellation tokens, operation gates and the handler's context
//! (session plan CS1.4), moved from gwz-core's `session_host/gate/tests.rs`
//! to the same place here, and of the crate map's nesting design
//! (`GwzCoreSessionCrateMap.md` §2), which replaces CS1.4's thread-local
//! rule. They use only the crate's public items, and live in the library so
//! that the Tier A command, `cargo test -p gwz-session-host --lib`, runs them.
//!
//! Of the moved tests, three change shape:
//! - `a_crossing_inside_a_crossing_of_the_same_gate_panics_instead_of_deadlocking`
//!   cannot be written any more: a crossing takes the gate mutably, so its
//!   closure cannot cross the same gate. The `compile_fail` examples on
//!   `OperationGate` and `GateScope` stand in for it.
//! - `a_crossing_of_another_gate_inside_a_crossing_panics` is gone. Crossing
//!   another operation's gate needs that operation's gate, which only
//!   smuggling can bring into a closure: the path the map states stays
//!   undetected, and that review checks.
//! - `a_cancel_callback_that_crosses_a_gate_is_stopped_and_the_canceller_goes_on`
//!   becomes `a_cancel_callback_that_revokes_inside_a_crossing_is_stopped_and_the_canceller_goes_on`:
//!   a callback holds no crossing capability, and a revoke is the re-entry
//!   the per-gate backstop stops.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::{
    CallControls, CancelRegistration, CancellationToken, GateScope, GateState, GateView,
    HandlerContext, Limits, OperationGate, Refused,
};

/// Stands for core's per-session data, which the crate passes through
/// without looking inside.
#[derive(Default)]
struct Session {
    limits: Limits,
}

/// The words every nesting panic carries.
const RULE: &str = "never crosses a gate";

fn call() -> (Arc<Session>, CallControls<Session>, OperationGate<Session>) {
    let session = Arc::new(Session::default());
    let (controls, gate) = CallControls::new(&session);
    (session, controls, gate)
}

/// Runs `work` on its own thread and waits for it at most five seconds, so
/// a regression to a deadlock fails the test instead of hanging it.
fn bounded<R: Send + 'static>(work: impl FnOnce() -> R + Send + 'static) -> thread::Result<R> {
    let worker = thread::spawn(work);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !worker.is_finished() {
        assert!(Instant::now() < deadline, "the work deadlocked");
        thread::sleep(Duration::from_millis(5));
    }
    worker.join()
}

/// The message of the panic that ended `outcome`'s thread.
fn panic_message<R>(outcome: thread::Result<R>) -> String {
    let Err(payload) = outcome else {
        panic!("the work did not panic");
    };
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => payload
            .downcast::<&str>()
            .map(|message| message.to_string())
            .unwrap_or_default(),
    }
}

/// Compiles only while `T` has no `Clone`: with one, both impls apply and
/// `check` is ambiguous.
trait NotClone<A> {
    fn check() {}
}

impl<T> NotClone<()> for T {}

impl<T: Clone> NotClone<u8> for T {}

#[test]
fn a_live_gate_lets_every_crossing_through() {
    let (_session, _controls, mut gate) = call();
    assert_eq!(gate.state(), GateState::Live);
    let running = gate.effect(|scope| scope.session().limits.running_operations);
    assert_eq!(running.unwrap(), 8);
    assert_eq!(gate.append(|_| "appended").unwrap(), "appended");
    assert_eq!(gate.report(|_| "event"), Some("event"));
}

#[test]
fn after_cancellation_the_gate_refuses_effects_and_log_appends_with_cancelled() {
    let (_session, controls, mut gate) = call();
    controls.cancel();
    assert_eq!(gate.state(), GateState::Cancelled);
    let ran = AtomicBool::new(false);
    let effect = gate.effect(|_| ran.store(true, SeqCst)).unwrap_err();
    let append = gate.append(|_| ran.store(true, SeqCst)).unwrap_err();
    assert_eq!(effect, Refused::Cancelled);
    assert_eq!(append, Refused::Cancelled);
    assert!(!ran.load(SeqCst), "a refused crossing runs nothing");
    // Events and the terminal still land: a handler that has succeeded
    // keeps its Completed terminal (§5.3).
    assert_eq!(gate.report(|_| "event"), Some("event"));
    assert_eq!(gate.report(|_| "terminal"), Some("terminal"));
}

#[test]
fn after_revocation_the_gate_also_ignores_events_and_terminals() {
    let (_session, controls, mut gate) = call();
    controls.revoke();
    assert_eq!(gate.state(), GateState::Revoked);
    assert!(
        gate.token().is_cancelled(),
        "close cancels a token before it revokes its gate (§8)"
    );
    let ran = AtomicBool::new(false);
    assert_eq!(gate.report(|_| ran.store(true, SeqCst)), None, "event");
    assert_eq!(gate.report(|_| ran.store(true, SeqCst)), None, "terminal");
    let effect = gate.effect(|_| ran.store(true, SeqCst)).unwrap_err();
    let append = gate.append(|_| ran.store(true, SeqCst)).unwrap_err();
    assert_eq!(effect, Refused::Revoked);
    assert_eq!(append, Refused::Revoked);
    assert!(!ran.load(SeqCst), "a revoked gate touches no session state");
}

#[test]
fn nothing_but_the_token_cancels() {
    let (_session, controls, gate) = call();
    let mut handler = HandlerContext::new(gate);
    let view = handler.view();
    // Crossings of every kind, a failed effect and dropped views and tokens
    // leave the call live.
    assert_eq!(handler.gate().report(|_| ()), Some(()));
    handler.gate().append(|_| ()).unwrap();
    let failed = handler
        .gate()
        .effect(|_| Err::<(), _>("the effect failed"))
        .unwrap();
    assert_eq!(failed, Err("the effect failed"));
    drop(view.clone());
    drop(handler.view());
    drop(handler.token().clone());
    assert_eq!(view.state(), GateState::Live);
    assert!(!view.token().is_cancelled());
    // Controls discarded without a cancel cancel nothing, and neither does
    // a dropped gate.
    drop(controls);
    assert_eq!(view.state(), GateState::Live);
    drop(handler);
    assert_eq!(view.state(), GateState::Live);
    assert!(!view.token().is_cancelled());

    // The token's own cancel reaches every holder.
    let (_session, controls, gate) = call();
    let handler = HandlerContext::new(gate);
    let observer = handler.token().clone();
    let view = controls.view();
    controls.cancel();
    assert!(observer.is_cancelled());
    assert!(handler.token().is_cancelled());
    assert_eq!(handler.view().state(), GateState::Cancelled);
    assert_eq!(view.state(), GateState::Cancelled);
}

#[test]
fn a_registration_attaches_to_the_token_and_is_refused_once_it_is_cancelled() {
    let (_session, controls, gate) = call();
    let token = gate.token().clone();
    let fired = Arc::new(AtomicUsize::new(0));
    let counter = fired.clone();
    let kept = token
        .on_cancel(move || {
            counter.fetch_add(1, SeqCst);
        })
        .unwrap();
    let counter = fired.clone();
    let detached = token
        .on_cancel(move || {
            counter.fetch_add(100, SeqCst);
        })
        .unwrap();
    drop(detached);
    // A callback that panics does not unwind into the canceller.
    let _panics = token
        .on_cancel(|| panic!("a cancel callback panicked"))
        .unwrap();
    controls.cancel();
    controls.cancel();
    assert_eq!(fired.load(SeqCst), 1, "the kept callback ran once");
    let refused = token.on_cancel(|| unreachable!()).err().unwrap();
    assert_eq!(refused, Refused::Cancelled);
    drop(kept);
}

#[test]
fn revocation_waits_for_a_crossing_in_flight_and_nothing_lands_after_it() {
    let (_session, controls, mut gate) = call();
    let (entered, crossing) = mpsc::channel();
    let (release, latch) = mpsc::channel::<()>();
    let worker = thread::spawn(move || {
        let landed = gate.report(|_| {
            entered.send(()).unwrap();
            latch.recv().unwrap();
            "landed"
        });
        (landed, gate)
    });
    crossing.recv().unwrap();
    let revoked = Arc::new(AtomicBool::new(false));
    let flag = revoked.clone();
    let revoker = thread::spawn(move || {
        controls.revoke();
        flag.store(true, SeqCst);
        controls
    });
    thread::sleep(Duration::from_millis(100));
    assert!(!revoked.load(SeqCst), "revoke waits for the crossing");
    release.send(()).unwrap();
    let (landed, mut gate) = worker.join().unwrap();
    assert_eq!(landed, Some("landed"));
    let _controls = revoker.join().unwrap();
    assert!(revoked.load(SeqCst));
    assert_eq!(gate.report(|_| "late"), None);
}

#[test]
fn a_gate_does_not_keep_its_session_alive() {
    let session = Arc::new(Session::default());
    let (_controls, mut gate) = CallControls::new(&session);
    let weak = Arc::downgrade(&session);
    drop(session);
    assert!(
        weak.upgrade().is_none(),
        "the session's data drops with the session"
    );
    assert_eq!(gate.state(), GateState::Revoked);
    assert_eq!(gate.report(|_| ()), None);
    assert_eq!(gate.effect(|_| ()).unwrap_err(), Refused::Revoked);
}

#[test]
fn a_revoke_inside_a_crossing_panics() {
    // The per-gate backstop: the thread running the gate's closure reaches
    // the record's controls, which a closure never should, and revokes.
    let (_session, controls, mut gate) = call();
    let controls = Arc::new(controls);
    let inside = controls.clone();
    let outcome = bounded(move || gate.append(|_| inside.revoke()));
    assert!(panic_message(outcome).contains(RULE));
    // The panic came before the revoke's cancel, and unwound the crossing,
    // so its lock is free again.
    assert_eq!(controls.view().state(), GateState::Live);
    let revoker = controls.clone();
    bounded(move || revoker.revoke()).expect("revoke returns");
    assert_eq!(controls.view().state(), GateState::Revoked);
}

#[test]
fn state_inside_a_crossing_returns_the_gates_state() {
    let (_session, controls, mut gate) = call();
    let view = gate.view();
    let (live, mut gate) =
        bounded(move || (gate.effect(|_| view.state()), gate)).expect("no panic");
    assert_eq!(live, Ok(GateState::Live));
    controls.cancel();
    let view = gate.view();
    let cancelled = bounded(move || gate.report(|_| view.state())).expect("no panic");
    assert_eq!(cancelled, Some(GateState::Cancelled));
}

#[test]
fn a_cancel_callback_that_revokes_inside_a_crossing_is_stopped_and_the_canceller_goes_on() {
    let (_session, controls, mut gate) = call();
    let controls = Arc::new(controls);
    let reached = Arc::new(AtomicUsize::new(0));
    let (progress, revoker) = (reached.clone(), controls.clone());
    let _registration = gate
        .token()
        .on_cancel(move || {
            progress.fetch_add(1, SeqCst);
            revoker.revoke();
            progress.fetch_add(1, SeqCst);
        })
        .unwrap();
    // The closure cancels through the record's controls, which it should
    // never hold, so the callback runs on the thread inside the crossing.
    let canceller = controls.clone();
    let outcome = bounded(move || gate.report(|_| canceller.cancel()));
    assert_eq!(outcome.expect("the canceller does not panic"), Some(()));
    assert_eq!(
        reached.load(SeqCst),
        1,
        "the callback stopped at its revoke"
    );
    assert_eq!(controls.view().state(), GateState::Cancelled);
    let later = controls.clone();
    bounded(move || later.revoke()).expect("revoke returns");
    assert_eq!(controls.view().state(), GateState::Revoked);
}

#[test]
fn the_handler_context_reaches_its_gate_and_carries_its_token() {
    let (_session, controls, gate) = call();
    let mut handler = HandlerContext::new(gate);
    let open_logs = handler
        .gate()
        .effect(|scope| scope.session().limits.open_logs);
    assert_eq!(open_logs.unwrap(), 64);
    assert!(!handler.token().is_cancelled());
    controls.cancel();
    assert!(handler.token().is_cancelled());
}

#[test]
fn the_controls_the_gate_the_handler_context_and_a_scope_are_not_clone() {
    // One holder for each capability: the record's controls cancel and
    // revoke, and the worker's gate crosses (crate map §2).
    <CallControls<Session> as NotClone<_>>::check();
    <OperationGate<Session> as NotClone<_>>::check();
    <HandlerContext<Session> as NotClone<_>>::check();
    <GateScope<'static, Session> as NotClone<_>>::check();
}

#[test]
fn the_cloneable_view_keeps_the_gates_state_and_its_token() {
    fn cloneable<T: Clone + Send + Sync>() {}
    cloneable::<GateView<Session>>();
    cloneable::<CancellationToken>();
    let (session, controls, gate) = call();
    let view = controls.view();
    let views = [view.clone(), gate.view(), view.clone()];
    let states =
        |views: &[GateView<Session>]| views.iter().map(GateView::state).collect::<Vec<_>>();
    assert_eq!(states(&views), [GateState::Live; 3]);
    controls.cancel();
    assert_eq!(states(&views), [GateState::Cancelled; 3]);
    assert!(views.iter().all(|view| view.token().is_cancelled()));
    controls.revoke();
    assert_eq!(states(&views), [GateState::Revoked; 3]);
    // A view outlives the gate and the controls, and keeps no session alive.
    drop((gate, controls));
    assert_eq!(states(&views), [GateState::Revoked; 3]);
    assert_eq!(Arc::strong_count(&session), 1);
}

#[test]
fn a_crossings_closure_gets_the_sessions_own_data_through_its_scope() {
    // The scope is all a closure receives: the session's data, passed
    // through untouched. It has no way to cross or revoke; the
    // `compile_fail` examples on `GateScope` show both refused.
    let (session, _controls, mut gate) = call();
    let same = gate
        .effect(|scope| std::ptr::eq(scope.session(), &*session))
        .unwrap();
    assert!(same);
}

#[test]
fn a_crossing_records_its_thread_only_while_its_closure_runs() {
    // No stale record: once a crossing has returned, or unwound, a revoke
    // from the same thread goes through.
    let (_session, controls, mut gate) = call();
    let controls = Arc::new(controls);
    let inside = controls.clone();
    let outcome = bounded(move || {
        gate.report(|_| ()).expect("a report lands");
        let unwound = catch_unwind(AssertUnwindSafe(|| {
            gate.report(|_| panic!("the closure panicked"))
        }));
        assert!(unwound.is_err());
        inside.revoke();
        gate.state()
    });
    assert_eq!(
        outcome.expect("the revoke went through"),
        GateState::Revoked
    );
}

#[test]
fn the_gate_types_cross_threads() {
    // A worker thread takes the gate, and the record's controls stay with
    // the host (§5.2).
    fn shareable<T: Send + Sync>() {}
    shareable::<CallControls<Session>>();
    shareable::<OperationGate<Session>>();
    shareable::<GateView<Session>>();
    shareable::<HandlerContext<Session>>();
    shareable::<CancellationToken>();
    shareable::<CancelRegistration>();
    shareable::<Refused>();
}
