//! The network setup's waits and connects, which need no SSH server and run on every platform.
use super::*;
use crate::git::endpoint::agent_job::{self, Job};
use std::{
    fs,
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

fn finish<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
    finish_within(job, Duration::from_secs(5))
}

fn finish_within<T: Send + 'static>(job: &mut Job<T>, within: Duration) -> io::Result<T> {
    let deadline = Instant::now() + within;
    loop {
        if let Poll::Ready(r) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
            return r;
        }
        assert!(Instant::now() < deadline, "the job did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A loopback address nothing listens on.
fn closed_port() -> SocketAddr {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
}

#[test]
fn production_waits_complete_across_four_setup_stages() {
    use agent_job::ManualClock;
    let clock = ManualClock::new();
    let start = clock.now();
    let aggregate = start + Duration::from_secs(10);
    let advancing = clock.clone();
    let mut job = Job::start_timed(
        agent_job::Place::Take(&agent_job::Supervisor::new()),
        Some(aggregate),
        Duration::from_secs(1),
        Duration::from_secs(5),
        clock.clock(),
        move |control| {
            timed_resolution(&control, || {
                advancing.advance(Duration::from_millis(600));
                Ok(())
            })?;
            // TCP, handshake, and agent socket call this production
            // poll boundary. Readiness completes each wait.
            for _ in 0..3 {
                wait_step(&control, |_| {
                    advancing.advance(Duration::from_millis(600));
                    Ok(true)
                })?;
            }
            control.check()
        },
    )
    .unwrap();
    finish(&mut job).unwrap();
    assert!(clock.now().duration_since(start) > Duration::from_secs(1));
    assert!(clock.now() < aggregate);
}

#[test]
fn production_tcp_poll_without_readiness_expires_stall() {
    use agent_job::{ManualClock, TimeoutReason, timeout_reason};
    let clock = ManualClock::new();
    let aggregate = clock.now() + Duration::from_secs(10);
    let advancing = clock.clone();
    let mut job = Job::start_timed(
        agent_job::Place::Take(&agent_job::Supervisor::new()),
        Some(aggregate),
        Duration::from_secs(1),
        Duration::from_secs(5),
        clock.clock(),
        move |control| {
            wait_step(&control, |_| {
                advancing.advance(Duration::from_secs(1));
                Ok(false)
            })
        },
    )
    .unwrap();
    let error = finish(&mut job).unwrap_err();
    assert!(clock.now() < aggregate);
    assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
}

#[test]
fn production_dns_error_after_stall_keeps_timeout_reason() {
    use agent_job::{Control, ManualClock, TimeoutReason, timeout_reason};
    let clock = ManualClock::new();
    let aggregate = clock.now() + Duration::from_secs(10);
    let control = Control::scripted(
        Some(aggregate),
        Duration::from_secs(1),
        Duration::from_secs(5),
        clock.clock(),
    );
    let error = timed_resolution(&control, || {
        clock.advance(Duration::from_secs(1));
        Err::<(), _>(io::ErrorKind::NotFound.into())
    })
    .unwrap_err();
    assert!(clock.now() < aggregate);
    assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
}

#[test]
fn live_control_retries_socket_timeout_and_abort() {
    use std::sync::atomic::AtomicUsize;
    for kind in [io::ErrorKind::TimedOut, io::ErrorKind::ConnectionAborted] {
        let peer = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let live = peer.local_addr().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        let mut job = Job::start_isolated(
            Some(Instant::now() + Duration::from_secs(3)),
            Duration::from_secs(1),
            move |c| {
                connect_addresses(vec![live, live], &c, |address, _| {
                    if observed.fetch_add(1, Ordering::SeqCst) == 0 {
                        Err(kind.into())
                    } else {
                        TcpStream::connect(address)
                    }
                })
            },
        )
        .unwrap();
        let socket = finish(&mut job).expect("live Control must allow the next address");
        assert_eq!(socket.peer_addr().unwrap(), live);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn terminal_control_wins_over_socket_error_without_retry() {
    use std::sync::{atomic::AtomicUsize, mpsc};
    for cancel in [true, false] {
        let (entered_tx, entered) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (result_tx, result) = mpsc::channel();
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        let expiry = Instant::now() + Duration::from_millis(200);
        let mut job = Job::start_isolated(
            if cancel { None } else { Some(expiry) },
            Duration::from_secs(1),
            move |c| {
                let address = "127.0.0.1:1".parse().unwrap();
                let error = connect_addresses(vec![address, address], &c, |_, _| {
                    assert_eq!(observed.fetch_add(1, Ordering::SeqCst), 0);
                    entered_tx.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(3)).unwrap();
                    Err(if cancel {
                        io::ErrorKind::TimedOut
                    } else {
                        io::ErrorKind::ConnectionAborted
                    }
                    .into())
                })
                .unwrap_err();
                result_tx.send(error.kind()).unwrap();
                Err::<(), _>(error)
            },
        )
        .unwrap();
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        if cancel {
            job.cancel();
        } else {
            std::thread::sleep(expiry.saturating_duration_since(Instant::now()));
        }
        release.send(()).unwrap();
        let expected = if cancel {
            io::ErrorKind::ConnectionAborted
        } else {
            io::ErrorKind::TimedOut
        };
        assert_eq!(
            result.recv_timeout(Duration::from_secs(2)).unwrap(),
            expected
        );
        assert_eq!(finish(&mut job).unwrap_err().kind(), expected);
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn a_connect_to_a_closed_port_fails_inside_the_control_bound() {
    let address = closed_port();
    let started = Instant::now();
    let bound = Duration::from_secs(8);
    let mut job = Job::start_isolated(Some(started + bound), Duration::from_secs(1), move |c| {
        connect(address, &c)
    })
    .unwrap();
    // Windows retries a refused loopback connect for about two seconds before it reports the refusal.
    let error = finish_within(&mut job, bound + Duration::from_secs(4)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
    assert!(started.elapsed() < bound, "{:?}", started.elapsed());
}

#[test]
fn a_connect_to_a_listener_succeeds() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let mut job = Job::start_isolated(
        Some(Instant::now() + Duration::from_secs(5)),
        Duration::from_secs(1),
        move |c| connect(address, &c),
    )
    .unwrap();
    let socket = finish_within(&mut job, Duration::from_secs(8)).unwrap();
    assert_eq!(socket.peer_addr().unwrap(), address);
}

#[test]
fn a_cancelled_control_ends_a_wait_inside_one_slice() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (_server, _) = listener.accept().unwrap();
    let waiting = Arc::new(AtomicBool::new(false));
    let announce = waiting.clone();
    let mut job = Job::<()>::start_isolated(None, Duration::from_secs(1), move |c| {
        loop {
            announce.store(true, Ordering::SeqCst);
            // An idle socket is never readable; only the Control's cancellation ends the wait.
            wait_step(&c, |duration| {
                socket_wait::wait_readable(&client, duration).map_err(clean)
            })?;
        }
    })
    .unwrap();
    let begun = Instant::now();
    while !waiting.load(Ordering::SeqCst) {
        assert!(begun.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(100));
    let cancelled = Instant::now();
    job.cancel();
    let error: io::Error = finish_within(&mut job, Duration::from_secs(5)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
    assert!(
        cancelled.elapsed() < Duration::from_secs(2),
        "{:?}",
        cancelled.elapsed()
    );
}

fn read_in_a_job(path: std::path::PathBuf) -> io::Result<String> {
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |c| {
        read_regular(&path, &c)
    })
    .unwrap();
    finish_within(&mut job, Duration::from_secs(10))
}

#[test]
fn a_trust_file_reads_whatever_its_path_is_made_of() {
    let dir = tempfile::TempDir::new().unwrap();
    for name in ["known_hosts", "with spaces", "caf\u{e9} \u{65e5}\u{672c}"] {
        let path = dir.path().join(name);
        fs::write(&path, format!("# {name}\n")).unwrap();
        assert_eq!(read_in_a_job(path).unwrap(), format!("# {name}\n"));
    }
}

#[test]
fn a_trust_file_over_the_cap_a_directory_and_bad_text_are_refused() {
    let dir = tempfile::TempDir::new().unwrap();
    let at_cap = dir.path().join("at-cap");
    fs::write(&at_cap, vec![b'#'; FILE_CAP]).unwrap();
    assert_eq!(read_in_a_job(at_cap).unwrap().len(), FILE_CAP);
    let over = dir.path().join("over-cap");
    fs::write(&over, vec![b'#'; FILE_CAP + 1]).unwrap();
    assert_eq!(
        read_in_a_job(over).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        read_in_a_job(dir.path().to_owned()).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let binary = dir.path().join("binary");
    fs::write(&binary, [0xff, 0xfe, 0xfd]).unwrap();
    assert_eq!(
        read_in_a_job(binary).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        read_in_a_job(dir.path().join("absent")).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[test]
        fn a_trust_path_that_names_a_device_or_a_pipe_is_refused_without_blocking() {
            for name in ["CON", "NUL", "\\\\.\\pipe\\gwz-known-hosts-absent", "\\\\.\\NUL"] {
                let started = Instant::now();
                let error = read_in_a_job(name.into()).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name}");
                assert!(started.elapsed() < Duration::from_secs(5), "{name}");
            }
        }
    }
}
