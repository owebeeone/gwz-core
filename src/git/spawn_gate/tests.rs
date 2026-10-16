//! The gate's own rules, on every platform: an excluding gate admits one hold at a time, a hold is released when it
//! drops (a panic included), a gate that does not exclude never waits, and a spawn is held for the spawn only.
use super::*;
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// How long a thread that must stay blocked is given to prove it is not.
const BLOCKED: Duration = Duration::from_millis(200);

#[test]
fn an_excluding_gate_admits_one_hold_at_a_time() {
    let gate = SpawnGate::with_exclusion(true);
    let first = gate.hold();
    assert!(gate.try_hold().is_none(), "a second hold while one exists");
    let (sender, receiver) = mpsc::channel();
    let waiter = {
        let gate = gate.clone();
        thread::spawn(move || {
            let _second = gate.hold();
            sender.send(()).unwrap();
        })
    };
    assert!(
        receiver.recv_timeout(BLOCKED).is_err(),
        "a hold was admitted while another existed"
    );
    drop(first);
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the waiting hold is admitted once the first is dropped");
    waiter.join().unwrap();
    assert!(gate.try_hold().is_some(), "the gate is free again");
}

#[test]
fn a_gate_that_does_not_exclude_never_waits() {
    let gate = SpawnGate::with_exclusion(false);
    let (_first, _second) = (gate.hold(), gate.hold());
    assert!(gate.try_hold().is_some());
}

#[test]
fn the_platforms_gate_excludes_where_inheritance_needs_it() {
    let gate = SpawnGate::new();
    let _first = gate.hold();
    assert_eq!(gate.try_hold().is_none(), cfg!(windows));
}

#[test]
fn a_panic_under_a_hold_releases_it() {
    let gate = SpawnGate::with_exclusion(true);
    let panicking = {
        let gate = gate.clone();
        thread::spawn(move || {
            let _hold = gate.hold();
            panic!("a launch that panics");
        })
    };
    assert!(panicking.join().is_err());
    assert!(gate.try_hold().is_some(), "the gate stayed held");
}

#[test]
fn a_hold_can_move_to_the_thread_that_launches() {
    let gate = SpawnGate::with_exclusion(true);
    let hold = gate.hold();
    let launcher = thread::spawn(move || drop(hold));
    launcher.join().unwrap();
    assert!(gate.try_hold().is_some());
}

#[test]
fn a_spawn_waits_for_a_hold_and_is_held_for_the_spawn_alone() {
    let gate = SpawnGate::with_exclusion(true);
    let hold = gate.hold();
    let (sender, receiver) = mpsc::channel();
    let spawner = {
        let gate = gate.clone();
        thread::spawn(move || {
            let mut command = probe_command();
            let child = gate.spawn(&mut command).unwrap();
            // The spawn is over: the gate is free while the child still runs.
            sender.send(gate.try_hold().is_some()).unwrap();
            child.wait_with_output().unwrap();
        })
    };
    assert!(
        receiver.recv_timeout(BLOCKED).is_err(),
        "a spawn went ahead of a hold"
    );
    let released = Instant::now();
    drop(hold);
    assert!(
        receiver.recv_timeout(Duration::from_secs(30)).unwrap(),
        "the gate was still held after the spawn"
    );
    assert!(released.elapsed() < Duration::from_secs(30));
    spawner.join().unwrap();
}

#[test]
fn a_spawn_that_fails_releases_the_gate() {
    let gate = SpawnGate::with_exclusion(true);
    let mut command = Command::new("gwz-no-such-program-for-the-spawn-gate");
    assert!(gate.spawn(&mut command).is_err());
    assert!(gate.try_hold().is_some());
}

fn probe_command() -> Command {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/d", "/c", "exit", "0"]);
        command
    } else {
        Command::new("true")
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    command
}
