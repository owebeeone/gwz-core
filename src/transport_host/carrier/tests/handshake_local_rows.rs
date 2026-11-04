//! The client's open of a local server over a real socket or pipe: connect, handshake and carrier together, and the
//! connect's bound when the listener cannot take the connection (step 2.3d-2). The 30 s and 5 s bounds themselves are
//! rows of `handshake_client_rows` on an injected clock; the rows here pass the bound in `Bounds`, short, because a
//! blocked read on a real stream has no clock to inject.
//!
//! A squatter, a listener that is not a gwz server, is played by `squatted` on each platform (a socket on Unix, a pipe
//! on Windows), so that the rows about what a squatter sends are one row for both.
use super::super::{
    CarrierLimits, Lane,
    handshake::{
        Action, Bounds, CarrierWire, Cause, ClientConfig, ClientFault, ClientOutcome, Control,
        DialFault, HostOutcome, Intent, SERVER_LISTENER_REFUSED, SERVER_UNAVAILABLE,
        SERVER_VERSION_MISMATCH, Stage, WireEnd, accept_open, dial,
    },
    local::{
        LocalAddress, LocalListener, LocalRefusal, PeerPolicy, PeerRefusal, row_gate, walk_to,
    },
    wire::{INVALID_REQUEST, SERVER_PEER_REFUSED},
};
use super::handshake_support::*;
use super::local_rows::{Dir, dir, dir_runtime, listener, other_user_policy};
use super::support::eventually;
use gwz_session_contract::{Frame, FrameKey, Tag, Target};
use std::{
    io,
    process::{Command, Stdio},
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

/// A dial that must fail: its fault and how long it took.
fn dial_fault(
    address: &LocalAddress,
    config: &ClientConfig,
    intent: &Intent,
) -> (DialFault, Duration) {
    let started = Instant::now();
    let policy = PeerPolicy::current_user().unwrap();
    let gate = row_gate();
    let target = match walk_to(address, &gate) {
        Ok(target) => target,
        Err(refusal) => return (DialFault::Connect(refusal), started.elapsed()),
    };
    match dial(target, &policy, CarrierLimits::DEFAULT, config, intent) {
        Err(fault) => (fault, started.elapsed()),
        Ok(_) => panic!("the dial succeeded"),
    }
}

fn dial_err(listener: &LocalListener, bounds: Bounds, intent: &Intent) -> (DialFault, Duration) {
    dial_fault(listener.address(), &config(bounds), intent)
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
            walk_to(listener.address(), &row_gate()).expect("the walk"),
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
            walk_to(listener.address(), &row_gate()).expect("the walk"),
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
        assert!(matches!(found.cause, Cause::Silent { waited, .. } if waited >= BOUND));
        // The listener's process, read by the connect's peer check, is named.
        assert_eq!(found.peer_pid, Some(std::process::id()));
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use super::*;
            use super::super::super::local::{connect_once, connect_until};
            use socket2::{Domain, SockAddr, Socket, Type};
            use std::{
                io::Write,
                os::unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}},
                path::{Path, PathBuf},
                sync::mpsc,
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

            /// A squatter: a listening socket that is not a gwz server. It serves `count` clients, sends each `banner`,
            /// and holds every stream open, unanswered, until `run` (the dials) has finished. `run` is given the address.
            pub(super) fn squatted<T>(dir: &Dir, count: usize, banner: &[u8], run: impl FnOnce(&LocalAddress) -> T) -> T {
                let path = dir_runtime(dir).path().join("squat");
                let squatter = UnixListener::bind(&path).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                let address = LocalAddress::from_path(&path);
                thread::scope(|scope| {
                    let (release, released) = mpsc::channel::<()>();
                    let squatter = &squatter;
                    let serving = scope.spawn(move || {
                        let held: Vec<UnixStream> = (0..count)
                            .map(|_| {
                                let (mut stream, _) = squatter.accept().unwrap();
                                let _ = stream.write_all(banner);
                                stream
                            })
                            .collect();
                        let _ = released.recv();
                        held
                    });
                    let found = run(&address);
                    drop(release);
                    drop(serving.join().unwrap());
                    found
                })
            }

            /// An address with nothing at it.
            pub(super) fn missing(dir: &Dir) -> LocalAddress {
                LocalAddress::from_path(&dir_runtime(dir).path().join("missing"))
            }

            cfg_if::cfg_if! {
                if #[cfg(any(target_os = "linux", target_os = "android"))] {
                    /// The threads of this process: the `Threads:` line of `/proc/self/status`.
                    pub(super) fn threads() -> usize {
                        let status = std::fs::read_to_string("/proc/self/status").unwrap();
                        let line = status.lines().find_map(|line| line.strip_prefix("Threads:"));
                        line.expect("a Threads: line").trim().parse().unwrap()
                    }
                } else if #[cfg(target_vendor = "apple")] {
                    /// The threads of this process: `proc_pidinfo` with `PROC_PIDTASKINFO`.
                    pub(super) fn threads() -> usize {
                        // SAFETY: a plain read of this process's task info into a zeroed struct of the size passed.
                        let (got, size, info) = unsafe {
                            let mut info: libc::proc_taskinfo = std::mem::zeroed();
                            let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
                            let got = libc::proc_pidinfo(
                                libc::getpid(),
                                libc::PROC_PIDTASKINFO,
                                0,
                                (&mut info as *mut libc::proc_taskinfo).cast(),
                                size,
                            );
                            (got, size, info)
                        };
                        assert_eq!(got, size, "proc_pidinfo");
                        usize::try_from(info.pti_threadnum).unwrap()
                    }
                }
            }

            #[test]
            fn a_socket_file_with_no_listener_is_refused_and_is_server_unavailable() {
                let dir = dir();
                let (socket, path) = bound(dir_runtime(&dir).path(), "stale");
                // The file stays; nothing listens at it.
                drop(socket);
                let (fault, _) = dial_fault(&LocalAddress::from_path(&path), &config(Bounds::DEFAULT), &Intent::Open);
                assert!(
                    matches!(fault, DialFault::Connect(LocalRefusal::Os { kind: io::ErrorKind::ConnectionRefused, .. })),
                    "{fault:?}"
                );
                assert_eq!(fault.code(), Some(SERVER_UNAVAILABLE));
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
                        // A dial reports it as an unavailable server, at once.
                        let started = Instant::now();
                        let (fault, _) = dial_fault(&LocalAddress::from_path(&path), &config(Bounds::DEFAULT), &Intent::Open);
                        assert!(matches!(fault, DialFault::Connect(LocalRefusal::Os { kind: io::ErrorKind::ConnectionRefused, .. })), "{fault:?}");
                        assert_eq!(fault.code(), Some(SERVER_UNAVAILABLE));
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
                            let fault = match dial(walk_to(&address, &row_gate()).expect("the walk"), &policy, CarrierLimits::DEFAULT, &config(bounds), &intent) {
                                Err(fault) => fault,
                                Ok(_) => panic!("the dial succeeded"),
                            };
                            assert_eq!(fault, DialFault::Connect(LocalRefusal::Timeout));
                            assert_eq!(fault.code(), Some(SERVER_UNAVAILABLE));
                            assert!(started.elapsed() >= BOUND && started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
                        }
                    }
                }
            }
        }
        use unix::{missing, squatted, threads};
    } else if #[cfg(windows)] {
        mod windows {
            use super::*;
            use super::super::super::local::{connect_stream, create_first_instance, squat};
            use std::sync::mpsc;
            use windows_sys::Win32::{
                Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
                System::{
                    Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next},
                    Threading::GetCurrentProcessId,
                },
            };

            /// A pipe name no other row or process uses.
            fn unique_name(kind: &str) -> String {
                let mut random = [0u8; 8];
                getrandom::fill(&mut random).expect("random bytes");
                let hex: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
                format!(r"\\.\pipe\gwz-{kind}-{}-{hex}", std::process::id())
            }

            /// A squatter: a pipe that is not a gwz server. It serves `count` clients, sends each `banner`, and holds
            /// every one open, unanswered, until `run` (the dials) has finished. `run` is given the address.
            pub(super) fn squatted<T>(_: &Dir, count: usize, banner: &[u8], run: impl FnOnce(&LocalAddress) -> T) -> T {
                let name = unique_name("squat");
                let first = create_first_instance(&name).expect("the squatter takes the name");
                let address = LocalAddress::from_name(&name);
                thread::scope(|scope| {
                    let (release, released) = mpsc::channel::<()>();
                    let name = &name;
                    let serving = scope.spawn(move || {
                        let held = squat(first, name, count, banner).expect("the squatter serves");
                        let _ = released.recv();
                        held
                    });
                    let found = run(&address);
                    drop(release);
                    drop(serving.join().unwrap());
                    found
                })
            }

            /// An address with nothing at it.
            pub(super) fn missing(_: &Dir) -> LocalAddress {
                LocalAddress::from_name(&unique_name("missing"))
            }

            /// The threads of this process: a Toolhelp snapshot of the system's threads, counted by owner.
            pub(super) fn threads() -> usize {
                // SAFETY: a snapshot read through its own handle, which is closed; the entry is sized as the API asks.
                unsafe {
                    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
                    assert_ne!(snapshot, INVALID_HANDLE_VALUE, "CreateToolhelp32Snapshot");
                    let own = GetCurrentProcessId();
                    let mut entry = THREADENTRY32 {
                        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                        ..Default::default()
                    };
                    let mut count = 0;
                    let mut more = Thread32First(snapshot, &mut entry) != 0;
                    while more {
                        if entry.th32OwnerProcessID == own {
                            count += 1;
                        }
                        more = Thread32Next(snapshot, &mut entry) != 0;
                    }
                    CloseHandle(snapshot);
                    count
                }
            }

            #[test]
            fn a_busy_pipe_is_waited_for_until_the_bound_and_an_instance_that_frees_is_taken() {
                let dir = dir();
                let listener = listener(&dir);
                let policy = PeerPolicy::current_user().unwrap();
                // The listener's one instance is taken by the first client; until the listener accepts it and makes
                // another, the pipe is busy (ERROR_PIPE_BUSY) for the second.
                let first = connect_stream(walk_to(listener.address(), &row_gate()).unwrap(), &policy, Instant::now() + Duration::from_secs(5))
                    .expect("the first client");
                let started = Instant::now();
                let timed_out = connect_stream(walk_to(listener.address(), &row_gate()).unwrap(), &policy, started + BOUND);
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
                    connect_stream(walk_to(listener.address(), &row_gate()).unwrap(), &policy, started + Duration::from_secs(5))
                });
                assert!(second.is_ok(), "{:?}", second.err());
                assert!(started.elapsed() >= Duration::from_millis(100));
                drop(first);
            }
        }
        use windows::{missing, squatted, threads};
    }
}

#[test]
fn a_banner_from_a_listener_that_is_not_a_server_fails_at_once_and_is_not_read_on() {
    let dir = dir();
    let banner = b"\x1b[31mWelcome to a shell\r\n";
    let (fault, elapsed) = squatted(&dir, 1, banner, |address| {
        dial_fault(address, &config(Bounds::DEFAULT), &Intent::Open)
    });
    assert!(
        elapsed < Duration::from_secs(5),
        "waited for the 30 s bound: {elapsed:?}"
    );
    assert_eq!(fault.code(), Some(SERVER_UNAVAILABLE));
    let DialFault::Handshake(fault @ ClientFault::Unavailable(_)) = fault else {
        panic!("expected server_unavailable");
    };
    let text = fault.to_string();
    assert!(
        text.contains("not a SessionHello") && text.contains("\\x1b[31"),
        "{text}"
    );
}

#[test]
fn a_listener_that_sends_two_bytes_and_goes_silent_is_reported_with_them() {
    let dir = dir();
    let bounds = Bounds {
        open_wait: BOUND,
        ..Bounds::DEFAULT
    };
    let (fault, elapsed) = squatted(&dir, 1, b"$ ", |address| {
        dial_fault(address, &config(bounds), &Intent::Open)
    });
    assert!(
        elapsed >= BOUND && elapsed < Duration::from_secs(5),
        "{elapsed:?}"
    );
    let DialFault::Handshake(fault @ ClientFault::Unavailable(_)) = fault else {
        panic!("expected server_unavailable");
    };
    let text = fault.to_string();
    let ClientFault::Unavailable(found) = &fault else {
        unreachable!();
    };
    assert_eq!(found.stage, Stage::Hello);
    assert!(
        matches!(&found.cause, Cause::Silent { first, .. } if first == "$ "),
        "{text}"
    );
    assert!(text.contains("it had sent \"$ \""), "{text}");
}

#[test]
fn a_listener_that_refused_this_peer_is_reported_as_the_refusal_with_its_cause() {
    let dir = dir();
    // A listener for another user: it refuses this process, answers it with one SessionError, and closes.
    let strict = crate::transport_host::carrier::tests::support::bind_listener(
        dir_runtime(&dir),
        other_user_policy(),
        CarrierLimits::DEFAULT,
    )
    .unwrap();
    let (fault, elapsed) = thread::scope(|scope| {
        let host = scope.spawn(|| strict.accept().err());
        let found = dial_fault(strict.address(), &config(Bounds::DEFAULT), &Intent::Open);
        assert_eq!(
            host.join().unwrap(),
            Some(LocalRefusal::Peer(PeerRefusal::OtherPrincipal))
        );
        found
    });
    assert!(
        elapsed < Duration::from_secs(5),
        "waited for the 30 s bound: {elapsed:?}"
    );
    let DialFault::Handshake(ClientFault::Refused { code, message }) = &fault else {
        panic!("expected the listener's refusal, got {fault:?}");
    };
    assert_eq!(*code, SERVER_PEER_REFUSED);
    assert!(message.contains("another user"), "{message}");
    assert_eq!(fault.code(), Some(SERVER_PEER_REFUSED));
}

#[test]
fn each_failure_of_a_dial_has_the_code_the_server_design_gives_it() {
    let os = |kind| {
        DialFault::Connect(LocalRefusal::Os {
            call: "connect",
            kind,
            code: None,
        })
    };
    // server_unavailable: a connect that never completes, a socket that refuses or is missing, a host that did not
    // act as a gwz server.
    let unavailable = Some(SERVER_UNAVAILABLE);
    assert_eq!(
        DialFault::Connect(LocalRefusal::Timeout).code(),
        unavailable
    );
    assert_eq!(os(io::ErrorKind::ConnectionRefused).code(), unavailable);
    assert_eq!(os(io::ErrorKind::NotFound).code(), unavailable);
    assert_eq!(
        DialFault::Handshake(ClientFault::Protocol("a frame out of order")).code(),
        unavailable
    );
    assert_eq!(
        DialFault::Handshake(ClientFault::Wire(WireEnd::Io(io::ErrorKind::BrokenPipe))).code(),
        unavailable
    );
    // server_listener_refused: this process refused the listener.
    assert_eq!(
        DialFault::Connect(LocalRefusal::Peer(PeerRefusal::OtherPrincipal)).code(),
        Some(SERVER_LISTENER_REFUSED)
    );
    // Otherwise the code of the ClientFault, when it has one, and none when the connect failed for another reason.
    let ended = ClientFault::Unavailable(super::super::handshake::Unavailable {
        stage: Stage::Hello,
        cause: Cause::Ended {
            first: String::new(),
        },
        peer_pid: None,
    });
    assert_eq!(DialFault::Handshake(ended).code(), unavailable);
    let mismatch = ClientFault::VersionMismatch {
        host: (3, 4),
        client: (1, 2),
    };
    assert_eq!(
        DialFault::Handshake(mismatch).code(),
        Some(SERVER_VERSION_MISMATCH)
    );
    let refused = ClientFault::Refused {
        code: 77,
        message: String::new(),
    };
    assert_eq!(DialFault::Handshake(refused).code(), Some(77));
    let too_large = ClientFault::TooLarge { size: 2, cap: 1 };
    assert_eq!(DialFault::Request(too_large).code(), Some(INVALID_REQUEST));
    assert_eq!(os(io::ErrorKind::PermissionDenied).code(), None);
    assert_eq!(DialFault::Connect(LocalRefusal::NotOurSocket).code(), None);
}

#[test]
fn a_dial_to_an_address_with_nothing_at_it_is_server_unavailable() {
    let dir = dir();
    let (fault, elapsed) = dial_fault(&missing(&dir), &config(Bounds::DEFAULT), &Intent::Open);
    assert!(matches!(fault, DialFault::Connect(_)), "{fault:?}");
    assert_eq!(fault.code(), Some(SERVER_UNAVAILABLE));
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
}

#[test]
fn a_dial_that_refuses_the_listener_is_server_listener_refused_and_sends_nothing() {
    let dir = dir();
    let listener = listener(&dir);
    let started = Instant::now();
    // This client expects another user's listener: its own check of the listener disagrees.
    let fault = match dial(
        walk_to(listener.address(), &row_gate()).expect("the walk"),
        &other_user_policy(),
        CarrierLimits::DEFAULT,
        &config(Bounds::DEFAULT),
        &Intent::Open,
    ) {
        Err(fault) => fault,
        Ok(_) => panic!("the dial succeeded"),
    };
    assert_eq!(
        fault,
        DialFault::Connect(LocalRefusal::Peer(PeerRefusal::OtherPrincipal))
    );
    assert_eq!(fault.code(), Some(SERVER_LISTENER_REFUSED));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_request_longer_than_a_first_frame_is_refused_before_anything_is_connected() {
    let dir = dir();
    let listener = listener(&dir);
    // The shape of the State review's rv11: a 4 MiB client string, which a host that never reads would have held the
    // write of past any bound.
    let mut config = config(Bounds {
        open_wait: BOUND,
        ..Bounds::DEFAULT
    });
    config.client = "x".repeat(4 << 20);
    let (fault, elapsed) = dial_fault(listener.address(), &config, &Intent::Open);
    assert!(
        matches!(fault, DialFault::Request(ClientFault::TooLarge { .. })),
        "{fault:?}"
    );
    assert_eq!(fault.code(), Some(INVALID_REQUEST));
    assert!(elapsed < BOUND, "{elapsed:?}");
}

const DIALS: usize = 20;
const CHILD_MARK: &str = "GWZ_HANDSHAKE_ROW_THREADS";

fn child_test() -> String {
    let path = module_path!();
    let relative = path.split_once("::").map_or(path, |(_, rest)| rest);
    format!("{relative}::child_counts_threads_across_failed_dials")
}

/// The thread count is a property of a whole process, and the rows run beside each other in one: so the count is taken in a
/// process of its own, this test binary run again on the one row below (as `ending_rows` does for its peer process).
#[test]
fn failed_dials_leave_no_helper_thread_behind() {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args(["--exact", &child_test(), "--nocapture", "--test-threads=1"])
        .env(CHILD_MARK, "1")
        .stdin(Stdio::null())
        .output()
        .expect("run the child");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "the child failed:\n{text}");
    assert!(
        text.contains("helper threads: before"),
        "the child measured nothing:\n{text}"
    );
}

/// The other process of the row above. Run on its own it does nothing and passes: only the parent's environment makes
/// it count: 20 dials fail against a squatter that holds every stream open, so only the client's abort can free its
/// helper threads, and the process's thread count must come back to where it was.
#[test]
fn child_counts_threads_across_failed_dials() {
    if std::env::var_os(CHILD_MARK).is_none() {
        return;
    }
    let dir = dir();
    let bounds = Bounds {
        open_wait: SHORT,
        ..Bounds::DEFAULT
    };
    let (before, after) = squatted(&dir, DIALS, b"", |address| {
        let before = threads();
        for _ in 0..DIALS {
            let (fault, _) = dial_fault(address, &config(bounds), &Intent::Open);
            assert!(
                matches!(fault, DialFault::Handshake(ClientFault::Unavailable(_))),
                "{fault:?}"
            );
        }
        let after = eventually("the helper threads to end", || {
            let now = threads();
            (now <= before).then_some(now)
        });
        (before, after)
    });
    println!("helper threads: before {before}, after {after}");
}
