//! The client's open of a local server over a real socket or pipe: connect, handshake and carrier together, and the
//! connect's bound when the listener cannot take the connection (step 2.3d-2). The 30 s and 5 s bounds themselves are
//! rows of `handshake_client_rows` on an injected clock; the rows here pass the bound in `Bounds`, short, because a
//! blocked read on a real stream has no clock to inject.
use super::super::{
    CarrierLimits, Lane,
    handshake::{
        Action, Bounds, CarrierWire, Cause, ClientConfig, ClientFault, ClientOutcome, Control,
        DialFault, HostOutcome, Intent, Stage, accept_open, dial,
    },
    local::{LocalListener, PeerPolicy},
};
use super::handshake_support::*;
use super::local_rows::{dir, dir_runtime, listener};
use super::support::eventually;
use gwz_session_contract::{Frame, FrameKey, Tag, Target};
use std::{
    thread,
    time::{Duration, Instant},
};

const SHORT: Duration = Duration::from_millis(50);
const BOUND: Duration = Duration::from_millis(300);

fn config(bounds: Bounds) -> ClientConfig {
    ClientConfig {
        supported: 1..=2,
        client: "gwz 1.1.0".into(),
        limits: None,
        peer_pid: None,
        bounds,
    }
}

fn dial_err(listener: &LocalListener, bounds: Bounds, intent: &Intent) -> (DialFault, Duration) {
    let started = Instant::now();
    let policy = PeerPolicy::current_user().unwrap();
    match dial(
        listener.address(),
        &policy,
        CarrierLimits::DEFAULT,
        &config(bounds),
        intent,
    ) {
        Err(fault) => (fault, started.elapsed()),
        Ok(_) => panic!("the dial succeeded"),
    }
}

fn ordinary(call: u64) -> FrameKey {
    FrameKey::ordinary(Target::new(call))
}

#[test]
fn a_client_opens_a_session_over_a_real_local_carrier_and_loses_no_byte() {
    let dir = dir();
    let listener = listener(&dir);
    let policy = PeerPolicy::current_user().unwrap();
    let (host, client) = thread::scope(|scope| {
        let host = scope.spawn(|| {
            let carrier = listener.accept().unwrap();
            let mut hooks = RecordingHooks::new(Log::default());
            let outcome = accept_open(
                &mut CarrierWire(&carrier),
                &hello(),
                &Bounds::DEFAULT,
                &mut hooks,
            )
            .unwrap();
            // Written at once after SessionOpened: the client's reader must not have lost it in the handshake.
            let reply = Frame::new(Tag::SessionReply, b"first".to_vec());
            carrier
                .send_frame(reply, Lane::Control, ordinary(1))
                .unwrap();
            let call = eventually("the call", || carrier.try_recv_registry().unwrap());
            (outcome, call)
        });
        let (carrier, outcome) = dial(
            listener.address(),
            &policy,
            CarrierLimits::DEFAULT,
            &config(Bounds::DEFAULT),
            &Intent::Open,
        )
        .expect("the dial");
        let call = Frame::new(Tag::SessionCall, b"call".to_vec());
        carrier.send_frame(call, Lane::Data, ordinary(1)).unwrap();
        let reply = eventually("the reply", || carrier.try_recv_registry().unwrap());
        (host.join().unwrap(), (outcome, reply))
    });
    let (host_outcome, call) = host;
    assert!(matches!(host_outcome, HostOutcome::Opened(_)));
    assert_eq!(call.body, b"call");
    let (outcome, reply) = client;
    assert!(matches!(outcome, ClientOutcome::Opened { .. }));
    assert_eq!(reply.body, b"first");
}

#[test]
fn a_control_exchange_over_a_real_local_carrier_returns_the_state() {
    let dir = dir();
    let listener = listener(&dir);
    let policy = PeerPolicy::current_user().unwrap();
    let control = Control {
        action: Action::Status,
        force: false,
    };
    let outcome = thread::scope(|scope| {
        let host = scope.spawn(|| {
            let carrier = listener.accept().unwrap();
            let mut hooks = RecordingHooks::new(Log::default());
            accept_open(
                &mut CarrierWire(&carrier),
                &hello(),
                &Bounds::DEFAULT,
                &mut hooks,
            )
            .unwrap()
        });
        let outcome = dial(
            listener.address(),
            &policy,
            CarrierLimits::DEFAULT,
            &config(Bounds::DEFAULT),
            &Intent::Control(control),
        )
        .expect("the dial")
        .1;
        assert_eq!(host.join().unwrap(), HostOutcome::Controlled(control));
        outcome
    });
    assert_eq!(
        outcome,
        ClientOutcome::State {
            hello: hello(),
            state: state()
        }
    );
}

#[test]
fn a_silent_host_fails_at_the_bound_over_a_real_carrier_and_the_stream_is_aborted() {
    let dir = dir();
    let listener = listener(&dir);
    for (intent, bounds) in [
        (
            Intent::Open,
            Bounds {
                open_wait: BOUND,
                ..Bounds::DEFAULT
            },
        ),
        (
            Intent::Control(Control {
                action: Action::Status,
                force: false,
            }),
            Bounds {
                state_wait: BOUND,
                ..Bounds::DEFAULT
            },
        ),
    ] {
        let (fault, elapsed) = thread::scope(|scope| {
            // The host accepts and says nothing; it sees the stream end when the client gives up.
            let host = scope.spawn(|| {
                let carrier = listener.accept().unwrap();
                eventually("the stream to end", || {
                    carrier.recv_registry_timeout(SHORT).err()
                })
            });
            let found = dial_err(&listener, bounds, &intent);
            host.join().unwrap();
            found
        });
        assert!(
            elapsed >= BOUND && elapsed < Duration::from_secs(5),
            "{elapsed:?}"
        );
        let DialFault::Handshake(ClientFault::Unavailable(found)) = fault else {
            panic!("expected server_unavailable");
        };
        assert_eq!(found.stage, Stage::Hello);
        assert!(matches!(found.cause, Cause::Silent { waited } if waited >= BOUND));
        // The listener's process, read by the connect's peer check, is named.
        assert_eq!(found.peer_pid, Some(std::process::id()));
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use super::*;
            use super::super::super::local::{LocalAddress, LocalRefusal, connect_once, connect_until};
            use socket2::{Domain, SockAddr, Socket, Type};
            use std::{
                io::Write,
                os::unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}},
                path::{Path, PathBuf},
            };

            /// A listening socket in the row's private directory, mode 0600 as a server makes it.
            fn bound(dir: &Path, name: &str) -> (Socket, PathBuf) {
                let path = dir.join(name);
                let socket = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
                socket.bind(&SockAddr::unix(&path).unwrap()).unwrap();
                socket.listen(1).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                (socket, path)
            }

            /// Connects, holding each stream, until the listener will take no more: the streams and the answer.
            fn fill(path: &Path) -> (Vec<UnixStream>, std::io::Error) {
                let mut held = Vec::new();
                for _ in 0..64 {
                    match connect_once(path) {
                        Ok(stream) => held.push(stream),
                        Err(error) => return (held, error),
                    }
                }
                panic!("the accept queue never filled");
            }

            #[test]
            fn a_banner_from_a_listener_that_is_not_a_server_fails_at_once_and_is_not_read_on() {
                let dir = dir();
                let path = dir_runtime(&dir).path().join("squat");
                let squatter = UnixListener::bind(&path).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                let policy = PeerPolicy::current_user().unwrap();
                let address = LocalAddress::from_path(&path);
                let (fault, elapsed) = thread::scope(|scope| {
                    scope.spawn(|| {
                        let (mut stream, _) = squatter.accept().unwrap();
                        stream.write_all(b"\x1b[31mWelcome to a shell\r\n").unwrap();
                        // Hold the stream until the client has judged it.
                        let mut sink = [0u8; 16];
                        let _ = std::io::Read::read(&mut stream, &mut sink);
                    });
                    let started = Instant::now();
                    let fault = match dial(
                        &address,
                        &policy,
                        CarrierLimits::DEFAULT,
                        &config(Bounds::DEFAULT),
                        &Intent::Open,
                    ) {
                        Err(fault) => fault,
                        Ok(_) => panic!("the dial succeeded"),
                    };
                    (fault, started.elapsed())
                });
                assert!(elapsed < Duration::from_secs(5), "waited for the 30 s bound: {elapsed:?}");
                let DialFault::Handshake(fault @ ClientFault::Unavailable(_)) = fault else {
                    panic!("expected server_unavailable");
                };
                let text = fault.to_string();
                assert!(text.contains("not a SessionHello") && text.contains("\\x1b[31"), "{text}");
            }

            cfg_if::cfg_if! {
                if #[cfg(target_vendor = "apple")] {
                    #[test]
                    fn a_full_accept_queue_is_refused_and_not_retried() {
                        let dir = dir();
                        let (_listener, path) = bound(dir_runtime(&dir).path(), "queue");
                        let (_held, answer) = fill(&path);
                        assert_eq!(answer.kind(), std::io::ErrorKind::ConnectionRefused, "{answer}");
                        let started = Instant::now();
                        let refused = connect_until(&path, started + Duration::from_secs(5));
                        assert!(matches!(
                            refused,
                            Err(LocalRefusal::Os { call: "connect", kind: std::io::ErrorKind::ConnectionRefused, .. })
                        ));
                        assert!(started.elapsed() < Duration::from_secs(1), "retried: {:?}", started.elapsed());
                    }
                } else {
                    #[test]
                    fn a_full_accept_queue_answers_would_block_and_is_retried_to_the_bound() {
                        let dir = dir();
                        let (_listener, path) = bound(dir_runtime(&dir).path(), "queue");
                        let (_held, answer) = fill(&path);
                        assert_eq!(answer.kind(), std::io::ErrorKind::WouldBlock, "{answer}");
                        let started = Instant::now();
                        let timed_out = connect_until(&path, started + BOUND);
                        assert!(matches!(timed_out, Err(LocalRefusal::Timeout)));
                        assert!(started.elapsed() >= BOUND, "gave up early: {:?}", started.elapsed());
                        assert!(started.elapsed() < Duration::from_secs(5));
                    }

                    #[test]
                    fn a_queue_that_drains_within_the_bound_lets_the_connect_through() {
                        let dir = dir();
                        let (listener, path) = bound(dir_runtime(&dir).path(), "queue");
                        let (held, _answer) = fill(&path);
                        let started = Instant::now();
                        let (connected, took) = thread::scope(|scope| {
                            scope.spawn(|| {
                                thread::sleep(Duration::from_millis(150));
                                // Take what was queued; the queue has room again.
                                let taken: Vec<_> = (0..held.len()).map(|_| listener.accept().unwrap()).collect();
                                thread::sleep(Duration::from_millis(500));
                                drop(taken);
                            });
                            let connected = connect_until(&path, started + Duration::from_secs(5));
                            (connected, started.elapsed())
                        });
                        assert!(connected.is_ok(), "{:?}", connected.err());
                        // The retry is prompt once the queue has room: the pause between attempts is capped.
                        assert!(took >= Duration::from_millis(100) && took < Duration::from_secs(2), "{took:?}");
                    }

                    #[test]
                    fn a_dial_against_a_full_queue_fails_at_the_handshakes_bound() {
                        let dir = dir();
                        let (_listener, path) = bound(dir_runtime(&dir).path(), "queue");
                        let (_held, _answer) = fill(&path);
                        let policy = PeerPolicy::current_user().unwrap();
                        let address = LocalAddress::from_path(&path);
                        let control = Control { action: Action::Status, force: false };
                        for (intent, bounds) in [
                            (Intent::Open, Bounds { open_wait: BOUND, ..Bounds::DEFAULT }),
                            (Intent::Control(control), Bounds { state_wait: BOUND, ..Bounds::DEFAULT }),
                        ] {
                            let started = Instant::now();
                            let fault = match dial(&address, &policy, CarrierLimits::DEFAULT, &config(bounds), &intent) {
                                Err(fault) => fault,
                                Ok(_) => panic!("the dial succeeded"),
                            };
                            assert_eq!(fault, DialFault::Connect(LocalRefusal::Timeout));
                            assert!(started.elapsed() >= BOUND && started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
                        }
                    }
                }
            }
        }
    } else if #[cfg(windows)] {
        mod windows {
            use super::*;
            use super::super::super::local::{LocalRefusal, connect_stream};

            #[test]
            fn a_busy_pipe_is_waited_for_until_the_bound_and_an_instance_that_frees_is_taken() {
                let dir = dir();
                let listener = listener(&dir);
                let policy = PeerPolicy::current_user().unwrap();
                // The listener's one instance is taken by the first client; until the listener accepts it and makes
                // another, the pipe is busy (ERROR_PIPE_BUSY) for the second.
                let first = connect_stream(listener.address(), &policy, Instant::now() + Duration::from_secs(5))
                    .expect("the first client");
                let started = Instant::now();
                let timed_out = connect_stream(listener.address(), &policy, started + BOUND);
                assert!(matches!(timed_out, Err(LocalRefusal::Timeout)), "{:?}", timed_out.err());
                assert!(started.elapsed() >= BOUND, "gave up early: {:?}", started.elapsed());
                let started = Instant::now();
                let second = thread::scope(|scope| {
                    scope.spawn(|| {
                        thread::sleep(Duration::from_millis(150));
                        // Accepting the first client makes the next instance.
                        let accepted = listener.accept().unwrap();
                        thread::sleep(Duration::from_secs(2));
                        drop(accepted);
                    });
                    connect_stream(listener.address(), &policy, started + Duration::from_secs(5))
                });
                assert!(second.is_ok(), "{:?}", second.err());
                assert!(started.elapsed() >= Duration::from_millis(100));
                drop(first);
            }
        }
    }
}
