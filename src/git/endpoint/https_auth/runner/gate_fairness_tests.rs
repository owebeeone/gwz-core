//! The spawn-gate fairness probe (plan step 1.8; `dev-docs/GwzLaneCloseout-2026-10-19.md` section 6, decision 3;
//! `dev-docs/GwzLogPathCursorGate-ReviewState.md` finding P3-1).
//!
//! A pathspec `log` takes the host's gate once per entry from up to `DEFAULT_JOBS` threads
//! (`operation/commit_log/mod.rs`, the `PathWalk` arm: `gate.spawn(&mut command)` and then
//! `child.wait_with_output()`), and the HTTPS helper's launch (`Runner::hold_spawn_gate`, the real polling code with the
//! lookup's real deadline) must get the same gate meanwhile. The hypothesis was that an unfair gate lets the takers starve
//! the polling launch into `GateTimeout`.
//!
//! The takers here make exactly the log's call, `SpawnGate::spawn` then `wait_with_output`, of a real child, from the
//! worst case's thread count. Nothing in the probe sleeps, retries or skips: a taker loops until the launch is done, and
//! the launch is first started once every taker has made a spawn, so the gate is under load when it asks.
//!
//! The rows run on every platform. Where a spawn needs a creation window (Windows) the process gate excludes by itself
//! and is the gate of the first row; elsewhere that gate never waits, so the second row's gate, which excludes
//! everywhere and is the process gate's in-process lock, is the one that is contended.
use super::*;
use crate::git::spawn_gate::SpawnGate;
use std::{
    process::Command,
    sync::atomic::{AtomicBool, AtomicUsize},
    thread,
    time::Duration as StdDuration,
};

/// The worst case the closeout names: `DEFAULT_JOBS` log threads, each cycling the gate.
const TAKERS: usize = crate::operation::DEFAULT_JOBS;
/// Launches the probe makes against each gate. Each starts under load and must be admitted.
const LAUNCHES: usize = 5;
/// The launch's clock: the deadline the review's reproduction named (2 s), a sliver of `HELPER_INTERACTION` (120 s). A
/// fair gate admits the launch after the holds ahead of it, each one `CreateProcessW` (tens of milliseconds at worst
/// on Windows), so it is admitted in a few hundred milliseconds at the most; a gate that starves it runs the whole clock
/// out and answers `GateTimeout`.
const LAUNCH_CLOCK: StdDuration = StdDuration::from_secs(2);

/// The most spawns that may complete while a launch waits. A gate that admits in arrival order puts the launch behind the
/// holds queued when it arrived, at most one per taker, and a taker whose hold ended just before has yet to count it, so
/// `2 * TAKERS` counts them all; a launch that is passed more often is being overtaken by takers that arrived after it.
/// This bound does not depend on the machine's speed or load, as the clock does.
const MOST_AHEAD: usize = 2 * TAKERS;

/// A host whose helper launches use `gate`.
fn owner(gate: &SpawnGate) -> AuthOwner {
    AuthOwner::new(HelperSlots::with_gate(gate.clone()))
}

/// `TAKERS` threads making the log's call, `SpawnGate::spawn` of a real child and then `wait_with_output` with the gate
/// released, until dropped. Dropping stops and joins them, so a failed row leaves no thread spawning.
struct Takers {
    spawned: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handles: Vec<thread::JoinHandle<()>>,
}

impl Takers {
    fn start(gate: &SpawnGate) -> Self {
        let spawned = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let handles = (0..TAKERS)
            .map(|_| {
                let (gate, spawned, stop) = (gate.clone(), spawned.clone(), stop.clone());
                thread::spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        let mut command = probe_command();
                        let output = gate
                            .spawn(&mut command)
                            .and_then(|child| child.wait_with_output());
                        assert!(output.is_ok_and(|output| output.status.success()));
                        spawned.fetch_add(1, Ordering::AcqRel);
                    }
                })
            })
            .collect();
        Self {
            spawned,
            stop,
            handles,
        }
    }

    fn completed(&self) -> usize {
        self.spawned.load(Ordering::Acquire)
    }
}

impl Drop for Takers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

/// The child the log runs, `git`, at its cheapest: the same program through the same `PATH` on every platform.
fn probe_command() -> Command {
    let mut command = Command::new("git");
    command
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    command
}

/// `LAUNCHES` helper launches against `TAKERS` takers on `gate`. Each must hold the gate within `LAUNCH_CLOCK`. Returns the
/// waits.
async fn launches_are_admitted(name: &str, gate: SpawnGate) -> Vec<StdDuration> {
    let owner = owner(&gate);
    let config = Config {
        executable: PathBuf::from("unused"),
        environment: Vec::new(),
    };
    let cancelled = CancellationToken::new();
    let takers = Takers::start(&gate);
    let mut waits = Vec::new();
    for launch in 0..LAUNCHES {
        // Under load: every taker has made a spawn since the last launch (no pause, only the takers' own progress).
        let target = takers.completed() + TAKERS;
        while takers.completed() < target {
            tokio::task::yield_now().await;
        }
        let runner = Runner {
            owner: &owner,
            config: &config,
            executable: &config.executable,
            permits: Arc::new(owner::AdmissionPermits {
                _helper_slot: owner
                    .inner
                    .helper_slots
                    .0
                    .clone()
                    .acquire_owned()
                    .await
                    .unwrap(),
                _endpoint_slot: None,
            }),
            cancelled: &cancelled,
            deadline: Instant::now() + LAUNCH_CLOCK,
            setup: None,
        };
        let before = takers.completed();
        let started = std::time::Instant::now();
        let held = runner.hold_spawn_gate().await;
        let waited = started.elapsed();
        let passed_ahead = takers.completed() - before;
        eprintln!(
            "gate-fairness {name} launch {launch}: waited {waited:?} of {LAUNCH_CLOCK:?}, {passed_ahead} spawns ahead of it, against {TAKERS} takers"
        );
        assert!(
            held.is_ok(),
            "{name}: launch {launch} was not admitted within its clock ({LAUNCH_CLOCK:?}): {:?}",
            held.err()
        );
        assert!(
            passed_ahead <= MOST_AHEAD,
            "{name}: launch {launch} waited behind {passed_ahead} spawns; a fair gate admits it after {MOST_AHEAD} at the most"
        );
        waits.push(waited);
        drop(held);
    }
    drop(takers);
    assert!(
        gate.try_hold().is_some(),
        "{name}: a taker left the gate held"
    );
    waits
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_helper_launch_is_admitted_while_the_process_gate_is_saturated_by_log_takers() {
    launches_are_admitted(
        "process gate",
        SpawnGate::process_named(&format!(
            "Local\\gwz-spawn-gate-fairness-{}",
            std::process::id()
        )),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_helper_launch_is_admitted_while_an_excluding_gate_is_saturated_by_log_takers() {
    launches_are_admitted("excluding gate", SpawnGate::with_exclusion(true)).await;
}
