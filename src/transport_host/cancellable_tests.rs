//! 1.1.0 S6.1's cancellable entry (gwz-core `dev-docs/GwzV110PlanAmendment.md`
//! §3.4): a cancel before the start and a cancel while running, the cleanup
//! report each returns, and a fault-injected panic in finish after a panic in
//! the operation, with the process staying alive and the next operation
//! succeeding.

use super::cancellable::{Step, run};
use super::driver_tests::{commit, common, endpoint_home, fixture_url, local_meta};
use super::*;
use crate::git::GitBackend;
use crate::git::endpoint::ssh_channel::GitService;
use crate::model::ErrorCode;
use crate::session_host::EnvironmentSnapshot;
use std::{
    cell::{Cell, RefCell},
    io::Read,
    path::Path,
    process::Command,
    sync::mpsc,
    thread,
    time::Instant,
};

/// A token and the controls that cancel it, as the entry's caller keeps them.
fn caller_token() -> (CallControls<()>, CancellationToken) {
    let (controls, gate) = CallControls::new(&Arc::new(()));
    (controls, gate.token().clone())
}

/// A snapshot that holds only `HOME`.
fn environment(home: &Path) -> EnvironmentSnapshot {
    EnvironmentSnapshot::from_os_pairs([("HOME".into(), home.as_os_str().to_owned())]).unwrap()
}

fn code<T>(result: &ModelResult<T>) -> Option<ErrorCode> {
    result.as_ref().err().map(|error| error.code)
}

/// A cancel before the start refuses with `Cancelled` and runs no action, at
/// each point: before the call, once the runtime is built and once the
/// request is registered. Each returns its cleanup report: with nothing
/// built, the contract's (0, false), since no peer cleanup occurred; with a
/// runtime built, after its request's finish and its shutdown, with no local
/// work left.
#[test]
fn a_cancel_before_the_start_refuses_and_returns_its_cleanup_report() {
    let (controls, token) = caller_token();
    controls.cancel();
    let ran = Cell::new(false);
    // No HOME: had the entry built a runtime first, it would have refused
    // with invalid_request instead.
    let nothing = EnvironmentSnapshot::from_os_pairs(Vec::new()).unwrap();
    let (result, cleanup) = with_cancellable_local_transport(
        local_meta("before-the-call", Path::new("/unused")),
        "fetch".into(),
        &nothing,
        &token,
        |_| ran.set(true),
    );
    assert_eq!(code(&result), Some(ErrorCode::Cancelled));
    assert!(!ran.get());
    assert_eq!(
        (cleanup.pending_local_work, cleanup.peer_cleanup_confirmed),
        (0, false)
    );

    let home = tempfile::TempDir::new().unwrap();
    let environment = environment(home.path());
    for (point, expected) in [
        (Step::Built, &[Step::Built, Step::Shutdown][..]),
        (
            Step::Registered,
            &[Step::Built, Step::Registered, Step::Finish, Step::Shutdown][..],
        ),
    ] {
        let (controls, token) = caller_token();
        let ran = Cell::new(false);
        let steps = RefCell::new(Vec::new());
        let (result, cleanup) = run(
            local_meta(&format!("cancelled-{point:?}"), home.path()),
            "fetch".into(),
            &environment,
            &token,
            |_| ran.set(true),
            &|step| {
                steps.borrow_mut().push(step);
                if step == point {
                    controls.cancel();
                }
            },
        );
        assert_eq!(code(&result), Some(ErrorCode::Cancelled), "{point:?}");
        assert!(!ran.get(), "{point:?}");
        assert_eq!(*steps.borrow(), expected, "{point:?}");
        assert_eq!(cleanup.pending_local_work, 0, "{point:?}");
        assert!(!cleanup.peer_cleanup_confirmed, "{point:?}");
    }
}

/// How long a cancelled operation's I/O may take to fail. The I/O itself
/// would wait out the transport's 9 s stall first.
const PROMPT: std::time::Duration = std::time::Duration::from_secs(3);

/// The transport host's bound on a cleanup wait (`session.rs`'s `CLEANUP`).
const CLEANUP: std::time::Duration = std::time::Duration::from_secs(5);

/// A cancel while running: the token's callback cancels the request, so the
/// operation's blocked read fails at once, not at its timeout, and the entry
/// returns the action's own failure with the operation's cleanup report.
#[test]
fn a_cancel_while_running_fails_its_io_and_returns_its_cleanup_report() {
    let fixture = stalling_fixture();
    let home = endpoint_home(&fixture);
    let (url, target) = (fixture_url(&fixture), fixture.temp.path().join("clone"));
    let started = fixture.temp.path().join("stalled");
    let environment = environment(&home);
    let meta = local_meta("cancel-while-running", &home);
    let (controls, token) = caller_token();
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        let (home, url, target, environment, meta, token) =
            (&home, &url, &target, &environment, &meta, &token);
        scope.spawn(move || {
            let steps = RefCell::new(Vec::new());
            let outcome = run(
                meta.clone(),
                "clone".into(),
                environment,
                token,
                |backend| {
                    let backend = backend
                        .with_transport(home, meta.transport.as_ref())
                        .unwrap()
                        .unwrap();
                    let result = backend.clone_repo(url, target);
                    (result, Instant::now())
                },
                &|step| steps.borrow_mut().push((step, Instant::now())),
            );
            sender
                .send((outcome, steps.into_inner(), Instant::now()))
                .unwrap();
        });
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        let stalled = loop {
            let pid = std::fs::read_to_string(&started).unwrap_or_default();
            if pid.ends_with('\n') {
                break pid.trim().to_owned();
            }
            assert!(
                Instant::now() < deadline,
                "the clone never reached the fixture"
            );
            thread::sleep(std::time::Duration::from_millis(5));
        };
        // The fixture's command now holds the clone's read of the advertisement.
        let cancelled = Instant::now();
        controls.cancel();
        let received = receiver.recv_timeout(std::time::Duration::from_secs(20));
        let ((result, cleanup), steps, returned) = received.unwrap_or_else(|_| {
            // The cancel did not reach the read. End the stalled command, so
            // the clone fails now, and the assertions below with it, instead
            // of waiting out the read's own deadline.
            common::run(Command::new("kill").args(["-9", &stalled]));
            receiver
                .recv()
                .expect("the entry returns once its read ends")
        });
        let (clone, failed) = result.expect("the action ran");
        let since = |step| {
            steps
                .iter()
                .find(|(seen, _)| *seen == step)
                .map(|(_, at)| at.duration_since(cancelled))
        };
        eprintln!(
            "from the cancel: the read failed after {:?}; finish began after {:?} and \
             shutdown after {:?}; the entry returned after {:?}",
            failed.duration_since(cancelled),
            since(Step::Finish),
            since(Step::Shutdown),
            returned.duration_since(cancelled)
        );
        assert!(clone.is_err(), "the cancelled clone fails");
        assert!(
            failed.duration_since(cancelled) < PROMPT,
            "the cancelled read failed only after {:?}",
            failed.duration_since(cancelled)
        );
        // Finish and shutdown each wait for the cancelled work within the
        // host's cleanup bound.
        assert!(returned.duration_since(failed) < 2 * CLEANUP + PROMPT);
        assert_eq!(cleanup.pending_local_work, 0);
        assert!(!cleanup.peer_cleanup_confirmed);
    });
}

/// An SSH fixture whose every exec writes its PID to `stalled` and then holds
/// its channel open without writing, until the client closes it.
fn stalling_fixture() -> common::SshdFixture {
    let fixture = common::SshdFixture::new();
    let script = fixture.temp.path().join("stall.sh");
    let started = fixture.temp.path().join("stalled");
    crate::git::endpoint::helper_script::write_helper_script(
        &script,
        &format!("echo $$ > '{}'\nexec cat >/dev/null\n", started.display()),
    );
    let public = std::fs::read_to_string(fixture.temp.path().join("client_ed25519.pub")).unwrap();
    std::fs::write(
        fixture.temp.path().join("authorized_keys"),
        format!("command=\"{}\" {}", script.display(), public),
    )
    .unwrap();
    fixture
}

/// A fault-injected panic in finish after a panic in the operation. The
/// entry catches the operation's panic first, then runs finish and shutdown,
/// each under its own guard and neither while unwinding, so the second panic
/// cannot abort the process. It reports failure with cleanup unconfirmed,
/// shutdown still drains the runtime, and the next operation succeeds.
#[test]
fn a_panic_in_finish_after_a_panic_in_the_operation_leaves_the_process_alive() {
    let fixture = common::SshdFixture::new();
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    commit(&server, "survivor");
    let home = endpoint_home(&fixture);
    let environment = environment(&home);
    let identity = home.join("client_ed25519").to_string_lossy().into_owned();
    let steps = RefCell::new(Vec::new());
    let (_controls, token) = caller_token();
    let (result, cleanup) = run(
        local_meta("panicking", &home),
        "fetch".into(),
        &environment,
        &token,
        |backend| {
            let context = backend.ssh.host_context().expect("the entry's request");
            let mut stream = context
                .open(
                    &fixture_url(&fixture),
                    GitService::UploadPack,
                    Some(identity.clone()),
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
                )
                .unwrap();
            let mut length = [0_u8; 4];
            stream.read_exact(&mut length).unwrap();
            panic!("injected operation fault, with a stream open");
        },
        &|step| {
            steps.borrow_mut().push((step, thread::panicking()));
            if step == Step::Finish {
                panic!("injected finish fault");
            }
        },
    );
    assert_eq!(code(&result), Some(ErrorCode::InternalError));
    assert!(!cleanup.peer_cleanup_confirmed);
    assert_eq!(
        cleanup.pending_local_work, 0,
        "shutdown ran under its own guard and drained the runtime"
    );
    // Finish ran after the operation's panic was caught, not while unwinding,
    // and shutdown ran after finish panicked.
    assert_eq!(
        *steps.borrow(),
        [
            (Step::Built, false),
            (Step::Registered, false),
            (Step::Finish, false),
            (Step::Shutdown, false),
        ]
    );

    let (_controls, token) = caller_token();
    let meta = local_meta("after-the-panics", &home);
    let target = fixture.temp.path().join("next");
    let (result, cleanup) = with_cancellable_local_transport(
        meta.clone(),
        "clone".into(),
        &environment,
        &token,
        |backend| {
            backend
                .with_transport(&home, meta.transport.as_ref())
                .unwrap()
                .unwrap()
                .clone_repo(&fixture_url(&fixture), &target)
        },
    );
    result
        .expect("the entry ran")
        .expect("the next operation succeeds");
    assert_eq!(cleanup.pending_local_work, 0);
    assert!(git2::Repository::open(&target).is_ok());
}
