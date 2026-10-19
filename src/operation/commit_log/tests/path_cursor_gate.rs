//! The path cursor's `git rev-list` spawns under its request's host gate (WH2 contract C9; operator decision D25).
//!
//! A pathspec log walks each member through a `git` child. On Windows a child created while another thread's launch
//! has pipe handles inheritable inherits them, so the spawn must wait for the host's gate like every other gwz spawn
//! that names no handle list. The row holds an excluding gate itself, runs a pathspec `handle_log` on another thread
//! against a backend on that gate, and sees that no child starts and the log does not finish until the hold is
//! dropped.
use super::*;
use std::{sync::mpsc, thread, time::Duration};

/// How long a log that must be blocked is given to prove it is not.
const BLOCKED: Duration = Duration::from_millis(500);

#[test]
fn a_pathspec_log_spawns_its_path_cursor_under_the_backends_gate() {
    let fixture = Fixture::new("path-cursor-gate");
    commit_file(&fixture.root, "root.txt", b"root\n", "root", 100, &[]);
    let repository = Repository::init(fixture.path().join("app")).unwrap();
    commit_file(&repository, "tracked.txt", b"app\n", "app", 200, &[]);
    fixture.write_manifest(&[member("mem_gate_app", "app", true)]);

    let gate = SpawnGate::with_exclusion(true);
    let mut backend = crate::git::Git2Backend::new();
    backend.spawn_gate = gate.clone();
    let registry = crate::operation::CommitLogOutputRegistry::new();
    let workspace = fixture.path();

    watch_path_readers("mem_gate_");
    let hold = gate.hold();
    let (finished, done) = mpsc::channel();
    let response = thread::scope(|scope| {
        let log = scope.spawn(|| {
            let response = crate::operation::handle_log(
                &backend,
                workspace,
                log_request(&[], &["."], false),
                "op-log-gate",
                &registry,
            );
            finished.send(()).unwrap();
            response
        });
        assert!(
            done.recv_timeout(BLOCKED).is_err(),
            "a pathspec log finished while another hold on its backend's gate existed"
        );
        assert_eq!(
            path_readers_seen("mem_gate_"),
            0,
            "a path cursor's git child ran while another hold on its backend's gate existed"
        );
        drop(hold);
        done.recv_timeout(Duration::from_secs(60))
            .expect("the pathspec log did not go on once the gate was released");
        log.join().unwrap()
    })
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert!(
        finish_watching_path_readers("mem_gate_") >= 1,
        "the member's path cursor never ran its git child"
    );
    registry.release(&response.output.log_id);
}
