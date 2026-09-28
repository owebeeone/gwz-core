//! Tests of cancellation tokens, operation gates and the handler's context (session plan CS1.4).

use super::*;
use crate::model::{ErrorCode, ModelError};
use crate::session_host::ClientChannel;
use crate::session_host::context::test_session;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// The words every nesting panic carries.
const RULE: &str = "never crosses a gate";

fn call() -> (ClientChannel, CallControls) {
    let channel = test_session();
    let controls = CallControls::new(channel.session());
    (channel, controls)
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

#[test]
fn a_live_gate_lets_every_crossing_through() {
    let (_channel, controls) = call();
    let gate = controls.gate();
    assert_eq!(gate.state(), GateState::Live);
    let running = gate.effect(|scope| scope.session().limits().running_operations);
    assert_eq!(running.unwrap(), 8);
    assert_eq!(gate.append(|_| "appended").unwrap(), "appended");
    assert_eq!(gate.report(|_| "event"), Some("event"));
}

#[test]
fn after_cancellation_the_gate_refuses_effects_and_log_appends_with_cancelled() {
    let (_channel, controls) = call();
    let gate = controls.gate().clone();
    controls.cancel();
    assert_eq!(gate.state(), GateState::Cancelled);
    let ran = AtomicBool::new(false);
    let effect = gate.effect(|_| ran.store(true, SeqCst)).unwrap_err();
    let append = gate.append(|_| ran.store(true, SeqCst)).unwrap_err();
    assert_eq!(effect.code, ErrorCode::Cancelled);
    assert_eq!(append.code, ErrorCode::Cancelled);
    assert!(!ran.load(SeqCst), "a refused crossing runs nothing");
    // Events and the terminal still land: a handler that has succeeded
    // keeps its Completed terminal (§5.3).
    assert_eq!(gate.report(|_| "event"), Some("event"));
    assert_eq!(gate.report(|_| "terminal"), Some("terminal"));
}

#[test]
fn after_revocation_the_gate_also_ignores_events_and_terminals() {
    let (_channel, controls) = call();
    let gate = controls.gate().clone();
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
    assert_eq!(effect.code, ErrorCode::Cancelled);
    assert_eq!(append.code, ErrorCode::Cancelled);
    assert!(!ran.load(SeqCst), "a revoked gate touches no session state");
}

#[test]
fn nothing_but_the_token_cancels() {
    let (_channel, controls) = call();
    let gate = controls.gate().clone();
    let handler = HandlerContext::new(gate.clone());
    // Crossings of every kind, a failed effect and dropped clones leave the
    // call live.
    assert_eq!(gate.report(|_| ()), Some(()));
    gate.append(|_| ()).unwrap();
    let failed = gate
        .effect(|_| Err::<(), _>(ModelError::new(ErrorCode::IoError, "the effect failed")))
        .unwrap();
    assert_eq!(failed.unwrap_err().code, ErrorCode::IoError);
    drop(gate.clone());
    drop(handler.clone());
    drop(handler.token().clone());
    assert_eq!(gate.state(), GateState::Live);
    assert!(!handler.token().is_cancelled());
    // A record discarded without a cancel cancels nothing either.
    drop(controls);
    assert_eq!(gate.state(), GateState::Live);
    assert!(!handler.token().is_cancelled());

    // The token's own cancel reaches every holder.
    let (_channel, controls) = call();
    let handler = HandlerContext::new(controls.gate().clone());
    let observer = handler.token().clone();
    controls.cancel();
    assert!(observer.is_cancelled());
    assert!(handler.token().is_cancelled());
    assert_eq!(handler.gate().state(), GateState::Cancelled);
}

#[test]
fn a_registration_attaches_to_the_token_and_is_refused_once_it_is_cancelled() {
    let (_channel, controls) = call();
    let token = controls.gate().token().clone();
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
    assert_eq!(refused.code, ErrorCode::Cancelled);
    drop(kept);
}

#[test]
fn revocation_waits_for_a_crossing_in_flight_and_nothing_lands_after_it() {
    let (_channel, controls) = call();
    let gate = controls.gate().clone();
    let (entered, crossing) = mpsc::channel();
    let (release, latch) = mpsc::channel::<()>();
    let worker = thread::spawn(move || {
        gate.report(|_| {
            entered.send(()).unwrap();
            latch.recv().unwrap();
            "landed"
        })
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
    assert_eq!(worker.join().unwrap(), Some("landed"));
    let controls = revoker.join().unwrap();
    assert!(revoked.load(SeqCst));
    assert_eq!(controls.gate().report(|_| "late"), None);
}

#[test]
fn a_gate_does_not_keep_its_session_alive() {
    let channel = test_session();
    let controls = CallControls::new(channel.session());
    let session = Arc::downgrade(channel.session());
    drop(channel);
    assert!(
        session.upgrade().is_none(),
        "the session context, with its snapshot, drops with the session"
    );
    let gate = controls.gate();
    assert_eq!(gate.state(), GateState::Revoked);
    assert_eq!(gate.report(|_| ()), None);
    assert_eq!(gate.effect(|_| ()).unwrap_err().code, ErrorCode::Cancelled);
}

#[test]
fn a_crossing_inside_a_crossing_of_the_same_gate_panics_instead_of_deadlocking() {
    let (_channel, controls) = call();
    let controls = Arc::new(controls);
    let gate = controls.gate().clone();
    let outcome = bounded(move || gate.effect(|_| gate.effect(|_| ())));
    assert!(panic_message(outcome).contains(RULE));
    // The panic unwound the outer crossing, so its lock is free again.
    let revoker = controls.clone();
    bounded(move || revoker.revoke()).expect("revoke returns");
    assert_eq!(controls.gate().state(), GateState::Revoked);
}

#[test]
fn a_crossing_of_another_gate_inside_a_crossing_panics() {
    let (_first_channel, first) = call();
    let (_second_channel, second) = call();
    let first = Arc::new(first);
    let (outer, inner) = (first.gate().clone(), second.gate().clone());
    let outcome = bounded(move || outer.report(|_| inner.report(|_| ())));
    assert!(panic_message(outcome).contains(RULE));
    let revoker = first.clone();
    bounded(move || revoker.revoke()).expect("revoke returns");
    assert_eq!(first.gate().state(), GateState::Revoked);
}

#[test]
fn a_revoke_inside_a_crossing_panics() {
    let (_channel, controls) = call();
    let controls = Arc::new(controls);
    let (gate, inside) = (controls.gate().clone(), controls.clone());
    let outcome = bounded(move || gate.append(|_| inside.revoke()));
    assert!(panic_message(outcome).contains(RULE));
    let revoker = controls.clone();
    bounded(move || revoker.revoke()).expect("revoke returns");
    assert_eq!(controls.gate().state(), GateState::Revoked);
}

#[test]
fn state_inside_a_crossing_returns_the_gates_state() {
    let (_channel, controls) = call();
    let gate = controls.gate().clone();
    let live = bounded(move || gate.effect(|_| gate.state())).expect("no panic");
    assert_eq!(live.unwrap(), GateState::Live);
    controls.cancel();
    let gate = controls.gate().clone();
    let cancelled = bounded(move || gate.report(|_| gate.state())).expect("no panic");
    assert_eq!(cancelled, Some(GateState::Cancelled));
}

#[test]
fn a_cancel_callback_that_crosses_a_gate_is_stopped_and_the_canceller_goes_on() {
    let (_channel, controls) = call();
    let gate = controls.gate().clone();
    let reached = Arc::new(AtomicUsize::new(0));
    let progress = reached.clone();
    let _registration = controls
        .gate()
        .token()
        .on_cancel(move || {
            progress.fetch_add(1, SeqCst);
            let _ = gate.report(|_| ());
            progress.fetch_add(1, SeqCst);
        })
        .unwrap();
    let controls = Arc::new(controls);
    let canceller = controls.clone();
    bounded(move || canceller.cancel()).expect("the canceller does not panic");
    assert_eq!(
        reached.load(SeqCst),
        1,
        "the callback stopped at its crossing"
    );
    assert_eq!(controls.gate().state(), GateState::Cancelled);
}

#[test]
fn the_handler_context_reaches_its_gate_and_carries_its_token() {
    let (_channel, controls) = call();
    let handler = HandlerContext::new(controls.gate().clone());
    let open_logs = handler
        .gate()
        .effect(|scope| scope.session().limits().open_logs);
    assert_eq!(open_logs.unwrap(), 64);
    assert!(!handler.token().is_cancelled());
    controls.cancel();
    assert!(handler.token().is_cancelled());
}
