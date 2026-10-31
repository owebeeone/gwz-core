//! The local carrier over a real socket (Unix) or pipe (Windows), and the peer rule.
//!
//! Hosts here have one user. The rows that need a second one prove the rule on identities no single-user host can
//! produce (`PeerPolicy::verify` is a pure function of two identities), and prove the wiring with a policy that
//! expects a user the host does not have: the real operating-system read happens, and it disagrees.
use super::super::{
    BindingLimits, BindingRefused, Carrier, CarrierLimits, CloseCode, CloseReason, Lane, Peer,
    ProtocolFault, SessionOpenRefusal, Side, StreamCarrier,
    local::{
        Integrity, LocalAddress, LocalListener, LocalRefusal, PeerIdentity, PeerPolicy,
        PeerRefusal, Principal, Relation, RuntimeDir, RuntimeEnv, connect,
    },
};
use super::{support::*, wire_rows::*};
use gwz_session_contract::{Frame, FrameKey, Target};
use std::{thread, time::Duration};

const SHORT: Duration = Duration::from_millis(50);

/// A per-user directory for a row: a fresh private one on Unix, nothing on Windows.
pub(super) struct Dir {
    runtime: RuntimeDir,
    _guard: Option<tempfile::TempDir>,
}

pub(super) fn dir() -> Dir {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            let guard = tempfile::Builder::new().prefix("gwz").tempdir().expect("tempdir");
            std::fs::set_permissions(guard.path(), std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
            let runtime = RuntimeDir::at(guard.path()).expect("a private directory");
            Dir { runtime, _guard: Some(guard) }
        } else {
            Dir { runtime: RuntimeDir::for_current_user(&RuntimeEnv::default()).expect("runtime"), _guard: None }
        }
    }
}

/// Opens the rows' binding. A carrier whose peer refused it may be closed already, which is the row's point and not
/// a failure of the binding.
fn open_rows_binding(carrier: &StreamCarrier) {
    match carrier.open_binding("session-1", BindingLimits::default()) {
        Ok(()) | Err(BindingRefused::Closed(_)) => {}
        Err(other) => panic!("the rows' binding: {other:?}"),
    }
}

/// A carrier from the listener, with the rows' binding open on it, as the composition opens one per operation.
pub(super) fn accepted(listener: &LocalListener) -> Result<StreamCarrier, LocalRefusal> {
    let carrier = listener.accept()?;
    open_rows_binding(&carrier);
    Ok(carrier)
}

/// [`connect`], with the rows' binding open on the carrier.
pub(super) fn connected(
    address: &LocalAddress,
    policy: &PeerPolicy,
    limits: CarrierLimits,
) -> Result<StreamCarrier, LocalRefusal> {
    let carrier = connect(address, policy, limits)?;
    open_rows_binding(&carrier);
    Ok(carrier)
}

pub(super) fn dir_runtime(dir: &Dir) -> &RuntimeDir {
    &dir.runtime
}

pub(super) fn listener(dir: &Dir) -> LocalListener {
    LocalListener::bind(
        &dir.runtime,
        PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .expect("bind")
}

/// A policy for a user the host does not have: the accounts' names differ, nothing else does.
pub(super) fn other_user_policy() -> PeerPolicy {
    let mut own = PeerPolicy::current_user().unwrap().own().clone();
    own.principal = match &own.principal {
        Principal::Uid(uid) => Principal::Uid(uid.wrapping_add(1)),
        Principal::Sid(sid) => Principal::Sid(format!("{sid}-1")),
    };
    PeerPolicy::from_identity(own)
}

#[test]
fn a_frame_set_round_trips_over_a_real_local_carrier_between_two_threads() {
    let dir = dir();
    let listener = listener(&dir);
    let address = listener.address().clone();
    let server = thread::spawn(move || {
        let core = accepted(&listener).expect("accepted");
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(eventually("a frame", || {
                core.recv_registry_timeout(SHORT).unwrap()
            }));
        }
        for _ in 0..4 {
            seen.push(Frame {
                tag: 16,
                body: eventually("a message", || core.recv_timeout(SHORT).unwrap())
                    .message
                    .0
                    .into_bytes(),
            });
        }
        // Answer on the other lane and in the other direction.
        core.try_send(window(1, 4242)).unwrap();
        (seen, core)
    });
    let client = connected(
        &address,
        &PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .expect("connected");
    let frames = [
        Frame {
            tag: 1,
            body: b"call".to_vec(),
        },
        Frame {
            tag: 2,
            body: vec![7; 5000],
        },
        Frame {
            tag: 4,
            body: session_open_without_environment(),
        },
    ];
    for frame in frames.clone() {
        client
            .send_frame(frame, Lane::Data, FrameKey::ordinary(Target::new(0)))
            .unwrap();
    }
    for message in [
        data(1, 0, vec![1; 30_000]),
        window(1, 5),
        cancel(1),
        close(1, 30_000),
    ] {
        client.try_send(message).unwrap();
    }
    let (seen, core) = server.join().unwrap();
    assert_eq!(&seen[..3], &frames);
    let names: Vec<_> = seen[3..]
        .iter()
        .map(|f| String::from_utf8(f.body.clone()).unwrap())
        .collect();
    assert_eq!(names, ["request-1"; 4]);
    assert_eq!(next(&client).1.window.unwrap().max_offset, 4242);
    // Each end's binding names the connection, its own side, and the peer the check read.
    // Each end draws its own identity: the two processes share no memory to agree one in.
    assert_ne!(core.binding().id(), client.binding().id());
    assert_eq!(core.binding().side(), Side::Core);
    assert_eq!(client.binding().side(), Side::Client);
    let binding = client.binding();
    let Peer::Verified(peer) = binding.peer() else {
        panic!("a local peer is verified")
    };
    assert_eq!(peer.pid, Some(std::process::id()));
    assert_eq!(
        &peer.principal,
        &PeerPolicy::current_user().unwrap().own().principal
    );
}

#[test]
fn a_dropped_local_carrier_ends_both_with_the_reason_and_not_a_timeout() {
    let dir = dir();
    let listener = listener(&dir);
    let address = listener.address().clone();
    let accepting = thread::spawn(move || accepted(&listener).expect("accepted"));
    let client = connected(
        &address,
        &PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .unwrap();
    let core = accepting.join().unwrap();
    core.try_send(window(1, 1)).unwrap();
    drop(core);
    assert_eq!(tag(&next(&client)), tag(&window(1, 1)));
    let closed = eventually("the closure", || client.try_recv().err());
    assert_eq!(closed.reason, CloseReason::Peer(CloseCode::Dropped));

    // And the other way: the client's explicit close, with its code, reaches the core.
    let listener = listener_again(&dir);
    let address = listener.address().clone();
    let accepting = thread::spawn(move || accepted(&listener).expect("accepted"));
    let client = connected(
        &address,
        &PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .unwrap();
    let core = accepting.join().unwrap();
    client.close(CloseCode::Cancelled);
    let closed = eventually("the closure", || core.try_recv().err());
    assert_eq!(closed.reason, CloseReason::Peer(CloseCode::Cancelled));
}

fn listener_again(dir: &Dir) -> LocalListener {
    listener(dir)
}

#[test]
fn a_squatted_listener_owned_by_another_principal_receives_zero_bytes() {
    let dir = dir();
    // The squatter is a real listener that took the name; the client expects its own user's listener there, and the
    // operating system's answer about the squatter's account disagrees.
    let squatter = listener(&dir);
    let address = squatter.address().clone();
    let accepting = thread::spawn(move || accepted(&squatter));
    let refused = connected(&address, &other_user_policy(), CarrierLimits::DEFAULT)
        .err()
        .expect("the client must refuse a listener of another principal");
    assert_eq!(refused, LocalRefusal::Peer(PeerRefusal::OtherPrincipal));
    // The squatter got a connection and then nothing. On a pipe the client may have gone before the squatter reads its
    // credentials, in which case the squatter's accept fails without a carrier; either way no frame reached it.
    if let Ok(squatter_end) = accepting.join().unwrap() {
        let closed = eventually("the squatter's closure", || squatter_end.try_recv().err());
        assert_eq!(closed.reason, CloseReason::PeerGone);
    }
}

#[test]
fn an_other_user_client_is_refused_at_accept_and_gets_nothing() {
    let dir = dir();
    let strict =
        LocalListener::bind(&dir.runtime, other_user_policy(), CarrierLimits::DEFAULT).unwrap();
    let address = strict.address().clone();
    let accepting = thread::spawn(move || accepted(&strict));
    // The client's own check of the listener passes (the listener is this user); the listener's check of the client
    // fails.
    let client = connected(
        &address,
        &PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .unwrap();
    let refused = accepting.join().unwrap().err().expect("refused at accept");
    assert_eq!(refused, LocalRefusal::Peer(PeerRefusal::OtherPrincipal));
    // The refused client is owed exactly one answer: a `SessionError` for call 0 with `server_peer_refused` (82,
    // provisional), naming the cause, and then the end of the stream. Nothing else was written to it.
    let answer = eventually("the answer", || {
        client.recv_registry_timeout(SHORT).unwrap()
    });
    let (call_id, code, message) = decode_session_error(&answer);
    assert_eq!((call_id, code), (0, 82));
    assert!(message.contains("another user"), "{message}");
    let closed = eventually("the closure", || client.try_recv().err());
    assert_eq!(closed.reason, CloseReason::PeerGone);
    assert!(client.recv_registry_timeout(SHORT).is_err());
}

fn identity(principal: Principal, integrity: Option<Integrity>) -> PeerIdentity {
    PeerIdentity {
        principal,
        pid: Some(1),
        integrity,
        app_container: false,
        sandbox: Default::default(),
    }
}

#[test]
fn a_process_below_medium_integrity_reaches_nothing_and_is_reached_by_nothing() {
    // The sandbox rule's one platform check that is a comparison (parked server design §4): integrity below Medium is
    // sandboxed, absolutely, in both relations and whoever the other side is, so two Low processes of one user do not
    // reach each other either.
    let me = Principal::Sid("S-1-5-21-1".into());
    let levels = [
        Integrity::Untrusted,
        Integrity::Low,
        Integrity::Medium,
        Integrity::High,
        Integrity::System,
    ];
    for own in levels {
        let policy = PeerPolicy::from_identity(identity(me.clone(), Some(own)));
        for theirs in levels {
            for relation in [
                Relation::ListenerChecksClient,
                Relation::ClientChecksListener,
            ] {
                let verdict = policy.verify(&identity(me.clone(), Some(theirs)), relation);
                if own < Integrity::Medium || theirs < Integrity::Medium {
                    assert_eq!(
                        verdict,
                        Err(PeerRefusal::Integrity),
                        "{own:?} {theirs:?} {relation:?}"
                    );
                } else if (relation == Relation::ClientChecksListener && own != theirs)
                    || (relation == Relation::ListenerChecksClient && theirs < own)
                {
                    assert_eq!(verdict, Err(PeerRefusal::Integrity));
                } else {
                    assert_eq!(verdict, Ok(()), "{own:?} {theirs:?} {relation:?}");
                }
            }
        }
    }
}

#[test]
fn the_peer_rule_is_one_function_of_two_identities() {
    let me = Principal::Sid("S-1-5-21-1".into());
    let policy = PeerPolicy::from_identity(identity(me.clone(), Some(Integrity::Medium)));
    let listener_sees = |who: PeerIdentity| policy.verify(&who, Relation::ListenerChecksClient);
    let client_sees = |who: PeerIdentity| policy.verify(&who, Relation::ClientChecksListener);

    // Same user, same level: both directions accept.
    assert_eq!(
        listener_sees(identity(me.clone(), Some(Integrity::Medium))),
        Ok(())
    );
    assert_eq!(
        client_sees(identity(me.clone(), Some(Integrity::Medium))),
        Ok(())
    );
    // Another user is refused whatever its level.
    let other = Principal::Sid("S-1-5-21-2".into());
    for level in [
        Integrity::Low,
        Integrity::Medium,
        Integrity::High,
        Integrity::System,
    ] {
        assert_eq!(
            listener_sees(identity(other.clone(), Some(level))),
            Err(PeerRefusal::OtherPrincipal)
        );
        assert_eq!(
            client_sees(identity(other.clone(), Some(level))),
            Err(PeerRefusal::OtherPrincipal)
        );
    }
    // A same-user listener at lower integrity is refused by the client; a lower-integrity client by the listener.
    assert_eq!(
        client_sees(identity(me.clone(), Some(Integrity::Low))),
        Err(PeerRefusal::Integrity)
    );
    assert_eq!(
        listener_sees(identity(me.clone(), Some(Integrity::Low))),
        Err(PeerRefusal::Integrity)
    );
    // The client wants the listener at its own level exactly; the listener accepts a client at or above its own.
    assert_eq!(
        client_sees(identity(me.clone(), Some(Integrity::High))),
        Err(PeerRefusal::Integrity)
    );
    assert_eq!(
        listener_sees(identity(me.clone(), Some(Integrity::High))),
        Ok(())
    );
    // An AppContainer token is refused at any level.
    let mut boxed = identity(me.clone(), Some(Integrity::Medium));
    boxed.app_container = true;
    assert_eq!(listener_sees(boxed.clone()), Err(PeerRefusal::AppContainer));
    assert_eq!(client_sees(boxed), Err(PeerRefusal::AppContainer));
    // A platform that reported a level for one side and not the other: the check cannot be made, so it refuses.
    assert!(matches!(
        listener_sees(identity(me.clone(), None)),
        Err(PeerRefusal::Unreadable(_))
    ));
    // Unix identities carry no level and compare on the user alone.
    let unix = PeerPolicy::from_identity(identity(Principal::Uid(501), None));
    assert_eq!(
        unix.verify(
            &identity(Principal::Uid(501), None),
            Relation::ListenerChecksClient
        ),
        Ok(())
    );
    assert_eq!(
        unix.verify(
            &identity(Principal::Uid(502), None),
            Relation::ClientChecksListener
        ),
        Err(PeerRefusal::OtherPrincipal)
    );
    assert_eq!(Integrity::from_rid(0x2000), Integrity::Medium);
    assert_eq!(Integrity::from_rid(0x1000), Integrity::Low);
    assert_eq!(Integrity::from_rid(0x3000), Integrity::High);
    assert_eq!(Integrity::from_rid(0x4000), Integrity::System);
    assert_eq!(Integrity::from_rid(0x2100), Integrity::Medium);
}

#[test]
fn the_operating_system_names_the_peer_process_and_user() {
    let dir = dir();
    let listener = listener(&dir);
    let address = listener.address().clone();
    let accepting = thread::spawn(move || accepted(&listener).expect("accepted"));
    let client = connected(
        &address,
        &PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .unwrap();
    let core = accepting.join().unwrap();
    let own = PeerPolicy::current_user().unwrap();
    for end in [&core, &client] {
        let Peer::Verified(peer) = end.binding().peer().clone() else {
            panic!("verified")
        };
        assert_eq!(peer.principal, own.own().principal);
        assert_eq!(peer.pid, Some(std::process::id()));
    }
}

#[test]
fn a_session_open_with_an_environment_never_crosses_a_local_carrier() {
    let dir = dir();
    let listener = listener(&dir);
    let address = listener.address().clone();
    let accepting = thread::spawn(move || accepted(&listener).expect("accepted"));
    let client = connected(
        &address,
        &PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .unwrap();
    let core = accepting.join().unwrap();
    let refused = client.send_frame(
        Frame {
            tag: 4,
            body: session_open_with_environment(),
        },
        Lane::Data,
        FrameKey::ordinary(Target::new(0)),
    );
    assert!(refused.is_err());
    // The core sees the carrier end with the client's reason or its disappearance, and never a frame.
    let seen = eventually("the end", || match core.recv_registry_timeout(SHORT) {
        Ok(Some(frame)) => panic!("a frame crossed: tag {}", frame.tag),
        Ok(None) => None,
        Err(closed) => Some(closed),
    });
    assert!(
        matches!(
            seen.reason,
            CloseReason::Peer(CloseCode::ProtocolFault) | CloseReason::PeerGone
        ),
        "{seen}"
    );
    let _ = (
        SessionOpenRefusal::NotAMap,
        ProtocolFault::Message,
        Side::Core,
    );
}

#[cfg(unix)]
mod unix {
    use super::super::super::local::{
        DirectoryFault, SOCKET_NAME_BYTES, SUN_PATH_MAX, socket_path,
    };
    use super::*;
    use std::{
        io::Read,
        os::{fd::AsRawFd, unix::fs::PermissionsExt, unix::net::UnixListener},
        path::Path,
    };

    #[test]
    fn nothing_listens_on_tcp() {
        let dir = dir();
        let listener = listener(&dir);
        let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockname(listener.as_raw_fd(), (&raw mut storage).cast(), &mut length)
        };
        assert_eq!(result, 0);
        assert_eq!(i32::from(storage.ss_family), libc::AF_UNIX);
        assert!(listener.address().path().starts_with(dir.runtime.path()));
    }

    #[test]
    fn a_refused_client_that_trickles_bytes_holds_the_accept_thread_for_one_second_at_most() {
        use std::{io::Write, os::unix::net::UnixStream, time::Instant};
        let dir = dir();
        let strict =
            LocalListener::bind(&dir.runtime, other_user_policy(), CarrierLimits::DEFAULT).unwrap();
        let address = strict.address().clone();
        // A refused peer that sends one byte every 100 ms for four seconds: each read succeeds, so only a bound on
        // time ends the drain before the peer does.
        let mut trickler = UnixStream::connect(address.path()).unwrap();
        let trickling = thread::spawn(move || {
            for _ in 0..40 {
                if trickler.write_all(&[0]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        // The next client is already waiting behind it.
        let next = connected(
            &address,
            &PeerPolicy::current_user().unwrap(),
            CarrierLimits::DEFAULT,
        )
        .unwrap();
        let accepting = thread::spawn(move || {
            let started = Instant::now();
            let first = strict.accept().err().expect("the trickler is refused");
            let first_took = started.elapsed();
            let second = strict
                .accept()
                .err()
                .expect("the next client is refused too");
            (first, first_took, second, started.elapsed())
        });
        let (first, first_took, second, both_took) = accepting.join().unwrap();
        assert_eq!(first, LocalRefusal::Peer(PeerRefusal::OtherPrincipal));
        assert_eq!(second, LocalRefusal::Peer(PeerRefusal::OtherPrincipal));
        assert!(
            first_took < Duration::from_millis(1500),
            "accept was held {first_took:?} by a refused client"
        );
        assert!(
            both_took < Duration::from_millis(2500),
            "the next client waited {both_took:?}"
        );
        // And the next client got its one answer.
        let answer = eventually("the next client's answer", || {
            next.recv_registry_timeout(SHORT).unwrap()
        });
        assert_eq!(decode_session_error(&answer).1, 82);
        trickling.join().unwrap();
    }

    #[test]
    fn a_squatted_unix_socket_counts_zero_bytes_from_a_client_that_refuses_it() {
        let dir = dir();
        // A raw listener that is not ours to trust in the client's eyes: the client expects another user.
        let path = dir.runtime.path().join("s-0123456789abcdef");
        let squatter = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let address = super::super::super::local::LocalAddress::from_path(&path);
        let reading = thread::spawn(move || {
            let (mut stream, _) = squatter.accept().unwrap();
            let mut seen = Vec::new();
            stream.read_to_end(&mut seen).unwrap();
            seen
        });
        let refused = connected(&address, &other_user_policy(), CarrierLimits::DEFAULT)
            .err()
            .unwrap();
        assert_eq!(refused, LocalRefusal::Peer(PeerRefusal::OtherPrincipal));
        assert_eq!(
            reading.join().unwrap(),
            Vec::<u8>::new(),
            "the squatter received bytes"
        );
    }

    #[test]
    fn the_runtime_directory_is_created_private_and_refused_otherwise() {
        let base = tempfile::tempdir().unwrap();
        let wide = base.path().join("wide");
        std::fs::create_dir(&wide).unwrap();
        std::fs::set_permissions(&wide, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            RuntimeDir::at(&wide).err(),
            Some(LocalRefusal::Directory(DirectoryFault::GroupOrOtherAccess))
        ));
        let group = base.path().join("group");
        std::fs::create_dir(&group).unwrap();
        std::fs::set_permissions(&group, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(matches!(
            RuntimeDir::at(&group).err(),
            Some(LocalRefusal::Directory(DirectoryFault::GroupOrOtherAccess))
        ));
        let private = base.path().join("private");
        std::fs::create_dir(&private).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(RuntimeDir::at(&private).is_ok());
        assert!(matches!(
            RuntimeDir::at(&base.path().join("missing")).err(),
            Some(LocalRefusal::Directory(DirectoryFault::Missing))
        ));
        let file = base.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(matches!(
            RuntimeDir::at(&file).err(),
            Some(LocalRefusal::Directory(DirectoryFault::NotADirectory))
        ));
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&private, &link).unwrap();
        assert!(matches!(
            RuntimeDir::at(&link).err(),
            Some(LocalRefusal::Directory(DirectoryFault::Link))
        ));
        // `at` never creates; `for_current_user` makes the user's directory with mode 0700 and accepts it again.
        let own =
            RuntimeDir::for_current_user(&RuntimeEnv::default()).expect("the user's directory");
        let mode = std::fs::metadata(own.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        assert!(RuntimeDir::for_current_user(&RuntimeEnv::default()).is_ok());
    }

    #[test]
    fn a_socket_path_that_does_not_fit_is_refused_and_one_that_fits_exactly_is_not() {
        // The limit is the field less its NUL: the platform's `sun_path` size is what the constant reports.
        let sun = {
            let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
            address.sun_path.len()
        };
        assert_eq!(SUN_PATH_MAX, sun - 1);
        let name = "s-0123456789abcdef";
        assert_eq!(name.len(), SOCKET_NAME_BYTES);
        let fits = "/".to_string() + &"d".repeat(SUN_PATH_MAX - 1 - SOCKET_NAME_BYTES - 1);
        let path = socket_path(Path::new(&fits), name).unwrap();
        assert_eq!(path.as_os_str().len(), SUN_PATH_MAX);
        let over = fits + "d";
        assert_eq!(
            socket_path(Path::new(&over), name),
            Err(LocalRefusal::AddressTooLong {
                length: SUN_PATH_MAX + 1,
                limit: SUN_PATH_MAX
            })
        );
    }

    #[test]
    fn a_directory_too_deep_for_the_field_makes_bind_refuse_before_creating_anything() {
        let base = tempfile::tempdir().unwrap();
        let mut deep = base.path().to_path_buf();
        while deep.as_os_str().len() < SUN_PATH_MAX {
            deep.push("deepdir-xxxxxxxxxxxx");
            std::fs::create_dir(&deep).unwrap();
            std::fs::set_permissions(&deep, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let runtime = RuntimeDir::at(&deep).unwrap();
        let refused = LocalListener::bind(
            &runtime,
            PeerPolicy::current_user().unwrap(),
            CarrierLimits::DEFAULT,
        )
        .err()
        .unwrap();
        assert!(
            matches!(refused, LocalRefusal::AddressTooLong { .. }),
            "{refused:?}"
        );
        assert_eq!(
            std::fs::read_dir(&deep).unwrap().count(),
            0,
            "bind created something"
        );
    }

    #[test]
    fn an_address_that_is_not_a_socket_of_ours_is_refused() {
        let dir = dir();
        let path = dir.runtime.path().join("s-fedcba9876543210");
        std::fs::write(&path, b"not a socket").unwrap();
        let address = super::super::super::local::LocalAddress::from_path(&path);
        let refused = connected(
            &address,
            &PeerPolicy::current_user().unwrap(),
            CarrierLimits::DEFAULT,
        )
        .err()
        .unwrap();
        assert_eq!(refused, LocalRefusal::NotOurSocket);
    }

    #[test]
    fn the_listener_removes_its_socket_when_it_goes_and_the_socket_is_private() {
        let dir = dir();
        let listener = listener(&dir);
        let path = listener.address().path().to_path_buf();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        drop(listener);
        assert!(!path.exists());
    }
}
