#![allow(dead_code)]
#[path = "../../../src/git/endpoint/agent_client.rs"]
mod agent_client;
#[path = "../../../src/git/endpoint/agent_job.rs"]
mod agent_job;
#[path = "../../../src/git/endpoint/agent_socket.rs"]
mod agent_socket;
use agent_client::{Agent, Channel};
use agent_job::{Control, Job};
use std::{
    io::{self, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
fn finish<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
            return result;
        }
        assert!(Instant::now() < limit, "helper failed to join");
        std::thread::sleep(Duration::from_millis(1));
    }
}
struct Frag {
    input: std::io::Cursor<Vec<u8>>,
    output: Arc<Mutex<Vec<u8>>>,
    closed: Arc<AtomicUsize>,
    seed: u64,
}
impl Drop for Frag {
    fn drop(&mut self) {
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
}
impl Frag {
    fn chunk(&mut self, len: usize) -> usize {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        len.min(1 + self.seed as usize % 17)
    }
}
impl Read for Frag {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.chunk(buf.len());
        self.input.read(&mut buf[..n])
    }
}
impl Write for Frag {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.chunk(buf.len());
        self.output.lock().unwrap().extend_from_slice(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Channel for Frag {
    fn wait(&mut self, _: bool, control: &Control) -> io::Result<()> {
        control.check()
    }
}
fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn frame(body: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    string(&mut out, &body);
    out
}
#[test]
fn seeded_fragmentation_preserves_identity_and_signature_frames() {
    let first = std::env::var("GWZ_AGENT_SEED")
        .ok()
        .map(|s| s.parse().unwrap())
        .unwrap_or(1_u64);
    for seed in first..first + 64 {
        eprintln!("GWZ_AGENT_SEED={seed}");
        let mut identities = vec![12];
        identities.extend_from_slice(&2_u32.to_be_bytes());
        string(&mut identities, b"key1");
        string(&mut identities, b"discard comment");
        string(&mut identities, b"key2");
        string(&mut identities, b"");
        let mut signature = Vec::new();
        string(&mut signature, b"ssh-ed25519");
        string(&mut signature, b"signed");
        let mut signed = vec![14];
        string(&mut signed, &signature);
        let mut replies = frame(identities);
        replies.extend(frame(signed));
        let mut job = Job::start(None, Duration::from_secs(1), move |control| {
            let output = Arc::new(Mutex::new(Vec::new()));
            let mut agent = Agent::new(
                Frag {
                    input: std::io::Cursor::new(replies),
                    output: output.clone(),
                    closed: Default::default(),
                    seed,
                },
                control,
            );
            assert_eq!(
                agent.identities()?,
                vec![b"key1".to_vec(), b"key2".to_vec()]
            );
            assert_eq!(agent.sign(b"key1", b"payload", "ssh-ed25519")?, b"signed");
            drop(agent);
            let mut expected = frame(vec![11]);
            let mut request = vec![13];
            string(&mut request, b"key1");
            string(&mut request, b"payload");
            request.extend_from_slice(&0_u32.to_be_bytes());
            expected.extend(frame(request));
            assert_eq!(*output.lock().unwrap(), expected);
            Ok(())
        })
        .unwrap();
        finish(&mut job).unwrap();
    }
}
#[test]
fn malformed_agent_frames_fail_without_payload_diagnostics() {
    for bytes in [
        vec![0, 16, 0, 1],
        frame(vec![12, 0, 0, 1, 1]),
        frame(vec![12, 0, 0, 0, 0, 99]),
        frame(vec![5]),
        frame(vec![99]),
        vec![0, 0, 0, 6, 12],
    ] {
        let mut job = Job::start(None, Duration::from_secs(1), move |control| {
            let output = Arc::new(Mutex::new(Vec::new()));
            Agent::new(
                Frag {
                    input: std::io::Cursor::new(bytes),
                    output: output.clone(),
                    closed: Default::default(),
                    seed: 42,
                },
                control,
            )
            .identities()
        })
        .unwrap();
        assert!(finish(&mut job).is_err());
    }
}
struct Owned(Arc<AtomicUsize>);
impl Drop for Owned {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn cancellation_discards_late_success_and_disposal_requires_join() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let observed = dropped.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let (started, running) = std::sync::mpsc::channel();
    let mut job = Job::start(None, Duration::from_millis(10), move |_| {
        started.send(()).unwrap();
        receive.recv().unwrap();
        Ok(Owned(observed))
    })
    .unwrap();
    running.recv_timeout(Duration::from_secs(3)).unwrap();
    job.cancel();
    job.cancel();
    std::thread::sleep(Duration::from_millis(30));
    assert!(matches!(
        job.poll_disposed(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(_))
    ));
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    send.send(()).unwrap();
    assert_eq!(
        finish(&mut job).err().unwrap().kind(),
        io::ErrorKind::ConnectionAborted
    );
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(matches!(
        job.poll_disposed(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(()))
    ));
}
#[test]
fn helper_panic_and_exact_deadline_fail_closed() {
    let mut panic_job = Job::<()>::start(None, Duration::from_secs(1), |_| {
        panic!("injected helper panic")
    })
    .unwrap();
    assert!(finish(&mut panic_job).is_err());
    let mut late = Job::start(Some(Instant::now()), Duration::from_secs(1), |control| {
        control.check()?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        finish(&mut late).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn cleanup_deadline_wakes_a_pending_disposal_waiter() {
    struct WakeCount(AtomicUsize);
    impl std::task::Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let wakes = Arc::new(WakeCount(AtomicUsize::new(0)));
    let (send, receive) = std::sync::mpsc::channel();
    let (started, running) = std::sync::mpsc::channel();
    let mut job = Job::start(None, Duration::from_millis(10), move |_| {
        started.send(()).unwrap();
        receive.recv().unwrap();
        Ok(())
    })
    .unwrap();
    running.recv_timeout(Duration::from_secs(3)).unwrap();
    let waker = Waker::from(wakes.clone());
    assert!(
        job.poll_disposed(&mut Context::from_waker(&waker))
            .is_pending()
    );
    std::thread::sleep(Duration::from_millis(80));
    let count = wakes.0.load(Ordering::SeqCst);
    send.send(()).unwrap();
    let _ = finish(&mut job);
    assert!(
        count > 0,
        "cleanup deadline must wake the waiter before helper exit"
    );
}

#[test]
fn wrong_signature_algorithm_trailing_failure_and_oversized_input_are_refused() {
    for response in [frame(vec![5, 1]), {
        let mut sig = vec![];
        string(&mut sig, b"ssh-rsa");
        string(&mut sig, b"secret-signature");
        let mut response = vec![14];
        string(&mut response, &sig);
        frame(response)
    }] {
        let mut job = Job::start(None, Duration::from_secs(1), move |control| {
            let output = Arc::new(Mutex::new(Vec::new()));
            let mut agent = Agent::new(
                Frag {
                    input: std::io::Cursor::new(response),
                    output: output.clone(),
                    closed: Default::default(),
                    seed: 33,
                },
                control,
            );
            let error = agent.sign(b"key", b"data", "rsa-sha2-512").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(!error.to_string().contains("secret"));
            assert!(agent.sign(b"key", b"data", "rsa-sha2-512").is_err());
            Ok(())
        })
        .unwrap();
        finish(&mut job).unwrap();
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod native {
            use super::*;
            use std::os::unix::net::UnixListener;
            #[test]
            fn cancellation_interrupts_every_partial_reply_and_closes_agent_socket() {
                for prefix in [vec![], vec![0, 0], vec![0, 0, 0, 5, 12]] {
                    let temp = tempfile::tempdir().unwrap(); let path = temp.path().join("agent.sock");
                    let server = UnixListener::bind(&path).unwrap();
                    let (ready, seen) = std::sync::mpsc::channel();
                    let peer = std::thread::spawn(move || {
                        let (mut socket, _) = server.accept().unwrap();
                        socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                        let mut request = [0; 5]; socket.read_exact(&mut request).unwrap(); assert_eq!(request, [0,0,0,1,11]);
                        socket.write_all(&prefix).unwrap(); ready.send(()).unwrap();
                        let mut byte = [0]; assert_eq!(socket.read(&mut byte).unwrap(), 0, "helper must close agent handle");
                    });
                    let mut job = Job::start(None, Duration::from_secs(1), move |control| agent_socket::connect(&path, control)?.identities()).unwrap();
                    seen.recv_timeout(Duration::from_secs(3)).unwrap(); job.cancel();
                    assert_eq!(finish(&mut job).unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
                    peer.join().unwrap();
                    assert!(matches!(job.poll_disposed(&mut Context::from_waker(Waker::noop())), Poll::Ready(Ok(()))));
                }
            }
            #[test]
            fn stalled_agent_reply_spends_absolute_deadline() {
                let temp = tempfile::tempdir().unwrap(); let path = temp.path().join("agent.sock");
                let server = UnixListener::bind(&path).unwrap();
                let peer = std::thread::spawn(move || {
                    let (mut socket, _) = server.accept().unwrap(); socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut request = [0; 5]; socket.read_exact(&mut request).unwrap();
                    let mut byte = [0]; assert_eq!(socket.read(&mut byte).unwrap(), 0);
                });
                let mut job = Job::start(Some(Instant::now() + Duration::from_millis(200)), Duration::from_secs(1), move |control| agent_socket::connect(&path, control)?.identities()).unwrap();
                assert_eq!(finish(&mut job).unwrap_err().kind(), io::ErrorKind::TimedOut); peer.join().unwrap();
            }
            #[test]
            fn unavailable_agent_is_redacted() {
                let temp = tempfile::tempdir().unwrap(); let path = temp.path().join("sentinel-agent-missing");
                let mut job = Job::start(None, Duration::from_secs(1), move |control| { let _ = agent_socket::connect(&path, control)?; Ok(()) }).unwrap();
                let error = finish(&mut job).unwrap_err(); assert!(!error.to_string().contains("sentinel"));
            }
            #[test]
            fn full_listen_backlog_connect_is_bounded() {
                use socket2::{Domain, Type, Socket, SockAddr};
                let temp = tempfile::tempdir().unwrap(); let path = temp.path().join("backlog.sock");
                let address = SockAddr::unix(&path).unwrap();
                let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap(); listener.bind(&address).unwrap(); listener.listen(0).unwrap();
                let mut blockers = Vec::new(); let mut filled = false;
                for _ in 0..256 {
                    let blocker = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap(); blocker.set_nonblocking(true).unwrap();
                    let result = blocker.connect(&address); blockers.push(blocker);
                    if let Err(error) = result {
                        assert!(error.kind() == io::ErrorKind::WouldBlock || error.kind() == io::ErrorKind::ConnectionRefused || error.raw_os_error() == Some(libc::EINPROGRESS), "backlog result: {error}");
                        filled = true; break;
                    }
                }
                assert!(filled, "fixture could not fill native backlog within its bound");
                let connected = Arc::new(AtomicUsize::new(0)); let flag = connected.clone();
                let mut job = Job::start(None, Duration::from_secs(1), move |control| {
                    let _agent = agent_socket::connect(&path, control)?; flag.store(1, Ordering::SeqCst); Ok(())
                }).unwrap();
                std::thread::sleep(Duration::from_millis(60));
                match job.poll_result(&mut Context::from_waker(Waker::noop())) {
                    Poll::Ready(result) => {
                        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionRefused);
                        eprintln!("native full backlog refuses immediately on this host");
                    }
                    Poll::Pending => {
                        job.cancel(); assert_eq!(finish(&mut job).unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
                        eprintln!("native full backlog pending connect cancelled and joined");
                    }
                }
                assert_eq!(connected.load(Ordering::SeqCst), 0, "fixture must stall at connect");
            }
        }
    }
}

#[test]
fn stalled_partial_request_write_is_cancelled_without_replay() {
    struct Blocked {
        accepted: Arc<AtomicUsize>,
    }
    impl Read for Blocked {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("request incomplete")
        }
    }
    impl Write for Blocked {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            if self.accepted.load(Ordering::SeqCst) == 0 {
                let n = input.len().min(3);
                self.accepted.store(n, Ordering::SeqCst);
                Ok(n)
            } else {
                Err(io::ErrorKind::WouldBlock.into())
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Channel for Blocked {
        fn wait(&mut self, _: bool, control: &Control) -> io::Result<()> {
            std::thread::sleep(control.quantum()?);
            control.check()
        }
    }
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = accepted.clone();
    let mut job = Job::start(None, Duration::from_secs(1), move |control| {
        Agent::new(Blocked { accepted: count }, control).identities()
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while accepted.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    job.cancel();
    assert_eq!(
        finish(&mut job).unwrap_err().kind(),
        io::ErrorKind::ConnectionAborted
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
}
#[test]
fn rsa_flags_are_explicit_and_oversized_sign_inputs_have_no_io() {
    for (method, flags) in [("rsa-sha2-256", 2_u32), ("rsa-sha2-512", 4_u32)] {
        let mut job = Job::start(None, Duration::from_secs(1), move |control| {
            let mut signature = vec![];
            string(&mut signature, method.as_bytes());
            string(&mut signature, b"sig");
            let mut response = vec![14];
            string(&mut response, &signature);
            let output = Arc::new(Mutex::new(Vec::new()));
            let mut agent = Agent::new(
                Frag {
                    input: std::io::Cursor::new(frame(response)),
                    output: output.clone(),
                    closed: Default::default(),
                    seed: 7,
                },
                control,
            );
            agent.sign(b"key", b"data", method)?;
            assert!(output.lock().unwrap().ends_with(&flags.to_be_bytes()));
            Ok(())
        })
        .unwrap();
        finish(&mut job).unwrap();
    }
    let mut job = Job::start(None, Duration::from_secs(1), |control| {
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut agent = Agent::new(
            Frag {
                input: std::io::Cursor::new(vec![]),
                output: output.clone(),
                closed: Default::default(),
                seed: 7,
            },
            control,
        );
        assert_eq!(
            agent
                .sign(b"key", &vec![0; 65537], "ssh-ed25519")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(output.lock().unwrap().is_empty());
        Ok(())
    })
    .unwrap();
    finish(&mut job).unwrap();
}

#[test]
fn expired_start_cannot_execute_setup_effects() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut job = Job::start(Some(Instant::now()), Duration::from_secs(1), move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        finish(&mut job).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn used_and_failed_agents_keep_channel_state_until_drop() {
    for reply in [frame(vec![12, 0, 0, 0, 0]), frame(vec![99])] {
        let mut job = Job::start(None, Duration::from_secs(1), move |control| {
            let output = Arc::new(Mutex::new(Vec::new()));
            let closed = Arc::new(AtomicUsize::new(0));
            let mut agent = Agent::new(
                Frag {
                    input: std::io::Cursor::new(reply),
                    output: output.clone(),
                    closed: closed.clone(),
                    seed: 7,
                },
                control,
            );
            let _ = agent.identities();
            assert!(agent.identities().is_err());
            assert_eq!(*output.lock().unwrap(), frame(vec![11]));
            assert!(agent.sign(b"key", b"data", "ssh-rsa").is_err());
            assert!(agent.sign(b"key", b"data", "ssh-ed25519").is_err());
            assert_eq!(*output.lock().unwrap(), frame(vec![11]));
            drop(agent);
            assert_eq!(closed.load(Ordering::SeqCst), 1);
            Ok(())
        })
        .unwrap();
        finish(&mut job).unwrap();
    }
}

#[test]
fn cancellation_at_publication_and_join_discards_once_but_claim_transfers_owner() {
    struct Notice(std::sync::mpsc::Sender<()>);
    impl std::task::Wake for Notice {
        fn wake(self: Arc<Self>) {
            let _ = self.0.send(());
        }
    }
    for after_join in [false, true] {
        let dropped = Arc::new(AtomicUsize::new(0));
        let observed = dropped.clone();
        let (published, publication) = std::sync::mpsc::channel();
        let (exit, allow_exit) = std::sync::mpsc::channel();
        let mut boundary = Some((published, allow_exit));
        let mut job = Job::start_with(
            None,
            Duration::from_secs(2),
            move |_| Ok(Owned(observed)),
            |name, body| {
                if name == "gwz-setup-reaper" {
                    return std::thread::Builder::new().name(name.into()).spawn(body);
                }
                let (published, allow_exit) = boundary.take().unwrap();
                Ok(std::thread::spawn(move || {
                    body(); // result published, but this thread cannot exit until released
                    published.send(()).unwrap();
                    allow_exit.recv_timeout(Duration::from_secs(3)).unwrap();
                }))
            },
        )
        .unwrap();
        publication.recv_timeout(Duration::from_secs(3)).unwrap();
        let (wake, joined) = std::sync::mpsc::channel();
        let waker = Waker::from(Arc::new(Notice(wake)));
        assert!(
            job.poll_result(&mut Context::from_waker(&waker))
                .is_pending()
        );
        if after_join {
            exit.send(()).unwrap();
            // No deadline/cancellation is set, so this wake can only follow join.
            joined.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        job.cancel();
        if !after_join {
            assert!(
                job.poll_result(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            assert_eq!(dropped.load(Ordering::SeqCst), 0);
            exit.send(()).unwrap();
        }
        assert_eq!(
            finish(&mut job).err().unwrap().kind(),
            io::ErrorKind::ConnectionAborted
        );
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(matches!(
            job.poll_disposed(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
        drop(job);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    let dropped = Arc::new(AtomicUsize::new(0));
    let observed = dropped.clone();
    let mut job = Job::start(None, Duration::from_secs(1), move |_| Ok(Owned(observed))).unwrap();
    let owned = finish(&mut job).unwrap();
    drop(job);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(owned);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}
