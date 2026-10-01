//! 1.1.0 S6.1's cancellable entry (gwz-core `dev-docs/GwzV110PlanAmendment.md`
//! §3.4): a cancel before the start and a cancel while running, the cleanup
//! report each returns, and a fault-injected panic in finish after a panic in
//! the operation, with the process staying alive and the next operation
//! succeeding.

use super::cancellable::{Step, run};
use super::driver_tests::{block_on, commit, common, endpoint_home, fixture_url, local_meta};
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
    time::{Duration, Instant},
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

/// How long after a cancel while running the entry may take to return. The
/// cancel fails the operation's I/O at once, and finish and shutdown then
/// wait only for in-process hand-offs: the endpoint's answer to the cancel,
/// and the disposal of a connection whose channel the client has aborted.
/// Those take milliseconds. 500 ms leaves ten times that for a loaded run,
/// and is a tenth of the cleanup bound, so a request left to that bound
/// fails.
const RETIRED: Duration = Duration::from_millis(500);

/// The transport host's bound on a cleanup wait (`session.rs`'s `CLEANUP`).
const CLEANUP: Duration = Duration::from_secs(5);

/// What one cancel while running observed, each time from the cancel.
pub(super) struct Cancelled<T> {
    /// The action's value.
    pub(super) value: T,
    cleanup: CleanupReport,
    /// When the action's I/O failed.
    failed: Duration,
    finish: Option<Duration>,
    shutdown: Option<Duration>,
    returned: Duration,
}

/// Runs `action` through the entry on a thread of its own, and cancels the
/// token once `held` finds the exchange held by its server. Should the cancel
/// not reach the I/O, `release` ends the server's hold 20 s later, so that
/// the action fails, and the assertions with it, instead of the test waiting
/// out the I/O's own deadline.
pub(super) fn cancel_while_running<T: Send>(
    meta: &RequestMeta,
    environment: &EnvironmentSnapshot,
    held: impl Fn() -> bool,
    release: impl FnOnce(),
    action: impl FnOnce(&Git2Backend) -> T + Send,
) -> Cancelled<T> {
    let (controls, token) = caller_token();
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        let token = &token;
        scope.spawn(move || {
            let steps = RefCell::new(Vec::new());
            let outcome = run(
                meta.clone(),
                "clone".into(),
                environment,
                token,
                |backend| (action(backend), Instant::now()),
                &|step| steps.borrow_mut().push((step, Instant::now())),
            );
            sender
                .send((outcome, steps.into_inner(), Instant::now()))
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !held() {
            assert!(
                Instant::now() < deadline,
                "the exchange never reached its server"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let cancelled = Instant::now();
        controls.cancel();
        let ((result, cleanup), steps, returned) = receiver
            .recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|_| {
                release();
                receiver
                    .recv()
                    .expect("the entry returns once its server lets go")
            });
        let (value, failed) = result.expect("the action ran");
        let since = |step| {
            steps
                .iter()
                .find(|(seen, _)| *seen == step)
                .map(|(_, at)| at.duration_since(cancelled))
        };
        Cancelled {
            value,
            cleanup,
            failed: failed.duration_since(cancelled),
            finish: since(Step::Finish),
            shutdown: since(Step::Shutdown),
            returned: returned.duration_since(cancelled),
        }
    })
}

/// The cancel failed the I/O at once, and the entry returned within
/// `RETIRED` of it, with no local work left.
pub(super) fn assert_prompt<T>(scheme: &str, cancelled: &Cancelled<T>) {
    eprintln!(
        "{scheme}, from the cancel: the read failed after {:?}; finish began after {:?} \
         and shutdown after {:?}; the entry returned after {:?}",
        cancelled.failed, cancelled.finish, cancelled.shutdown, cancelled.returned
    );
    assert!(
        cancelled.failed < RETIRED,
        "{scheme}: the cancelled read failed only after {:?}",
        cancelled.failed
    );
    assert!(
        cancelled.returned < RETIRED,
        "{scheme}: the entry returned {:?} after the cancel; finish began after {:?} \
         and shutdown after {:?}",
        cancelled.returned,
        cancelled.finish,
        cancelled.shutdown
    );
    assert_eq!(cancelled.cleanup.pending_local_work, 0);
    assert!(!cancelled.cleanup.peer_cleanup_confirmed);
}

/// A cancel while running: the token's callback cancels the request, so the
/// operation's blocked read fails at once, not at its timeout, and the entry
/// returns the action's own failure with the operation's cleanup report. The
/// endpoint answers the cancel, so finish and shutdown wait on nothing
/// remote, and the entry returns within `RETIRED` as well.
#[test]
fn a_cancel_while_running_fails_its_io_and_returns_its_cleanup_report() {
    let fixture = stalling_fixture();
    let home = endpoint_home(&fixture);
    let (url, target) = (fixture_url(&fixture), fixture.temp.path().join("clone"));
    let started = fixture.temp.path().join("stalled");
    // The fixture's command writes its PID once it holds the clone's read of
    // the advertisement.
    let stalled = || {
        std::fs::read_to_string(&started)
            .ok()
            .filter(|pid| pid.ends_with('\n'))
    };
    let meta = local_meta("cancel-while-running", &home);
    let cancelled = cancel_while_running(
        &meta,
        &environment(&home),
        || stalled().is_some(),
        || {
            if let Some(pid) = stalled() {
                common::run(Command::new("kill").args(["-9", pid.trim()]));
            }
        },
        |backend| {
            backend
                .with_transport(&home, meta.transport.as_ref())
                .unwrap()
                .unwrap()
                .clone_repo(&url, &target)
        },
    );
    assert!(cancelled.value.is_err(), "the cancelled clone fails");
    assert_prompt("SSH", &cancelled);
}

/// A cancel the peer never answers: the endpoint session stops taking the
/// driver's messages, as a peer that has stopped answering does, and its
/// exchange's I/O timeout outlasts the cleanup bound, so no terminal retires
/// the cancelled stream. Finish waits for the peer only up to the host's
/// cleanup bound, then returns with cleanup unconfirmed.
#[test]
fn a_cancel_the_peer_never_answers_waits_out_the_cleanup_bound() {
    let fixture = stalling_fixture();
    let home = endpoint_home(&fixture);
    let started = fixture.temp.path().join("stalled");
    let config = SshEndpointConfig::fixture(home.clone(), None).with_io_timeout_ms(60_000);
    let runtime = TransportRuntime::new(config).unwrap();
    let meta = local_meta("unanswered-cancel", &home);
    let request = block_on(runtime.request(meta, "fetch".into())).unwrap();
    let identity = home.join("client_ed25519").to_string_lossy().into_owned();
    let stream = request
        .context
        .open(
            &fixture_url(&fixture),
            GitService::UploadPack,
            Some(identity),
            Arc::new(|_, _| {}),
            Arc::new(|_| {}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(&started).is_ok_and(|pid| pid.ends_with('\n')) {
        assert!(
            Instant::now() < deadline,
            "the open never reached the fixture"
        );
        thread::sleep(Duration::from_millis(5));
    }
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    endpoint.hold_pump_for_test(true);
    let cancelled = Instant::now();
    let cleanup = block_on(request.finish());
    let returned = cancelled.elapsed();
    endpoint.hold_pump_for_test(false);
    eprintln!("a cancel the peer never answers: finish returned after {returned:?}");
    // The bound runs from the cancel, on the mux's millisecond clock.
    assert!(
        returned > CLEANUP - Duration::from_millis(250),
        "finish gave up on the peer after only {returned:?}"
    );
    assert!(
        returned < CLEANUP + RETIRED,
        "finish waited {returned:?} for the peer, past the cleanup bound"
    );
    assert!(!cleanup.peer_cleanup_confirmed);
    drop(stream);
    block_on(runtime.shutdown());
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
