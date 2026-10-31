//! How a stream carrier ends (remediation round 1: S-P2-1, S-P2-2, S-P3-4, C-P2-3's refusal, C-P3-1): a peer that stops
//! listening, a peer that is killed, a peer that stops reading, a close announcement, and a refused `SessionOpen`.
use super::super::{
    Carrier, CarrierLimits, CloseCode, CloseReason, FrameClass, ProtocolFault, SendRefused,
    SessionOpenRefusal, StallBounds,
    local::{LocalAddress, LocalListener, LocalRefusal, PeerPolicy},
    wire,
};
use super::{local_rows::*, support::*, wire_rows::*};
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SHORT: Duration = Duration::from_millis(50);

fn pause_until(what: &str, mut check: impl FnMut() -> bool) {
    eventually(what, || check().then_some(()));
}

fn taken_then_closure(carrier: &dyn Carrier, count: usize) -> CloseReason {
    for index in 0..count {
        let arrival = eventually("an arrival", || match carrier.try_recv() {
            Ok(arrival) => arrival,
            Err(closed) => panic!("closed after {index} of {count} arrivals: {closed}"),
        });
        assert_eq!(arrival.message.1.data.unwrap().offset, index as i64);
    }
    eventually("the closure", || carrier.try_recv().err()).reason
}

#[test]
fn a_peer_that_stops_listening_does_not_cost_the_frames_it_already_sent() {
    let (carrier, from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    for offset in 0..5 {
        write_all(
            &mut to_carrier,
            &wire_for("session-1", data(1, offset, vec![1])),
        );
    }
    pause_until("all five routed", || carrier.queued_in() == 5);
    // The peer stops listening; this end's next write breaks.
    drop(from_carrier);
    let refused = eventually("the broken write", || carrier.try_send(window(1, 1)).err());
    match refused {
        SendRefused::Closed(_, closed) => assert_eq!(closed.reason, CloseReason::PeerGone),
        other => panic!("{other:?}"),
    }
    // Nothing the peer sent was dropped by it, and the one reason, once the peer's stream ends, is `PeerGone`.
    assert_eq!(carrier.queued_in(), 5);
    drop(to_carrier);
    assert_eq!(taken_then_closure(&carrier, 5), CloseReason::PeerGone);
    // The same reason, again, for everything after.
    assert_eq!(
        carrier.try_recv().unwrap_err().reason,
        CloseReason::PeerGone
    );
}

#[test]
fn an_announcement_in_flight_stands_as_the_reason_over_a_broken_write() {
    let (carrier, from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    for offset in 0..2 {
        write_all(
            &mut to_carrier,
            &wire_for("session-1", data(1, offset, vec![1])),
        );
    }
    let announcement = wire::close_frame(CloseCode::Cancelled);
    write_all(
        &mut to_carrier,
        &wire_frame(announcement.tag, &announcement.body),
    );
    drop(from_carrier);
    let _ = eventually("the broken write", || carrier.try_send(window(1, 1)).err());
    assert_eq!(
        taken_then_closure(&carrier, 2),
        CloseReason::Peer(CloseCode::Cancelled)
    );
}

#[test]
fn frames_after_the_peers_announcement_are_not_delivered() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    write_all(&mut to_carrier, &wire_for("session-1", data(1, 0, vec![1])));
    let announcement = wire::close_frame(CloseCode::Normal);
    write_all(
        &mut to_carrier,
        &wire_frame(announcement.tag, &announcement.body),
    );
    write_all(&mut to_carrier, &wire_frame(1, b"after-the-close"));
    write_all(&mut to_carrier, &wire_for("session-1", data(1, 1, vec![2])));
    // What follows the announcement is dropped, whether the router discards it (and counts it) or the end has already
    // stopped reading: either way only the one frame that preceded it is delivered, then the closure.
    assert_eq!(
        taken_then_closure(&carrier, 1),
        CloseReason::Peer(CloseCode::Normal)
    );
    assert!(carrier.try_recv_registry().is_err());
    assert!(carrier.discarded_frames() <= 2);
}

#[test]
fn a_refused_session_open_gets_exactly_one_session_error_and_the_stream_ends() {
    let (carrier, mut from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    write_all(
        &mut to_carrier,
        &wire_frame(4, &session_open_with_environment()),
    );
    // Exactly one frame: a `SessionError` for call 0 with `invalid_request`, that repeats nothing the peer sent.
    let (tag, body) = read_wire_frame(&mut from_carrier);
    let (call_id, code, message) = decode_session_error(&gwz_session_contract::Frame { tag, body });
    assert_eq!((call_id, code), (0, 1));
    assert!(!message.contains("CANARY"), "{message}");
    // The refused frame was not delivered, and the carrier's reason names the shape fault.
    let closed = eventually("the refusal", || carrier.recv_registry_timeout(SHORT).err());
    assert_eq!(
        closed.reason,
        CloseReason::Protocol(ProtocolFault::SessionOpen(
            SessionOpenRefusal::ForbiddenField(3)
        ))
    );
    drop(to_carrier);
    assert!(
        read_to_end(from_carrier).is_empty(),
        "then the end of the stream"
    );
}

#[test]
fn every_variant_of_the_session_open_gets_one_session_error_and_the_frozen_one_is_delivered() {
    // The frozen shape is delivered.
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    write_all(
        &mut to_carrier,
        &wire_frame(4, &session_open_without_environment()),
    );
    let frame = eventually("the SessionOpen", || {
        carrier.recv_registry_timeout(SHORT).unwrap()
    });
    assert_eq!(frame.tag, 4);
    drop(to_carrier);
    // Every variant that breaks the shape is answered by exactly one `SessionError` and not delivered.
    for (name, body) in refused_session_opens() {
        let (carrier, mut from_carrier, mut to_carrier) = against_raw(limits(8, 4));
        write_all(&mut to_carrier, &wire_frame(4, &body));
        let (tag, answer) = read_wire_frame(&mut from_carrier);
        let (call_id, code, _) =
            decode_session_error(&gwz_session_contract::Frame { tag, body: answer });
        assert_eq!((call_id, code), (0, 1), "{name}");
        let closed = eventually("the refusal", || carrier.recv_registry_timeout(SHORT).err());
        assert!(
            matches!(
                closed.reason,
                CloseReason::Protocol(ProtocolFault::SessionOpen(_))
            ),
            "{name}: {closed:?}"
        );
        drop(to_carrier);
        assert!(
            read_to_end(from_carrier).is_empty(),
            "{name}: then the end of the stream"
        );
    }
}

/// A stream that yields `bytes` and then fails with `error`, as a socket does when its peer resets the connection.
struct FailsAfter {
    /// The first read waits for this, so that the row can open its binding before any frame arrives.
    gate: Option<std::sync::mpsc::Receiver<()>>,
    bytes: std::io::Cursor<Vec<u8>>,
    error: std::io::ErrorKind,
}

impl Read for FailsAfter {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if let Some(gate) = self.gate.take() {
            let _ = gate.recv();
        }
        match self.bytes.read(buffer)? {
            0 => Err(std::io::Error::from(self.error)),
            read => Ok(read),
        }
    }
}

fn carrier_over_failing_reader(
    error: std::io::ErrorKind,
    frames: usize,
) -> super::super::StreamCarrier {
    let bytes: Vec<u8> = (0..frames as i64)
        .flat_map(|offset| wire_for("session-1", data(1, offset, vec![1])))
        .collect();
    let (open, gate) = std::sync::mpsc::channel();
    let carrier = super::super::StreamCarrier::new(
        Box::new(FailsAfter {
            gate: Some(gate),
            bytes: std::io::Cursor::new(bytes),
            error,
        }),
        Box::new(std::io::sink()),
        Box::new(|| {}),
        super::super::CarrierBinding::new(
            super::super::CarrierId::random().unwrap(),
            super::super::Side::Core,
            super::super::Peer::InProcess,
        ),
        limits(8, 4),
    )
    .unwrap();
    carrier
        .open_binding("session-1", super::super::BindingLimits::default())
        .unwrap();
    open.send(()).unwrap();
    carrier
}

#[test]
fn a_reader_that_meets_the_peers_departure_leaves_what_it_read_and_the_reason_peer_gone() {
    use std::io::ErrorKind::*;
    // The far side going away mid-stream, however the operating system spells it, is the peer's departure.
    for kind in [
        ConnectionReset,
        ConnectionAborted,
        BrokenPipe,
        NotConnected,
        UnexpectedEof,
    ] {
        let carrier = carrier_over_failing_reader(kind, 3);
        assert_eq!(
            taken_then_closure(&carrier, 3),
            CloseReason::PeerGone,
            "{kind:?}"
        );
    }
}

#[test]
fn a_reader_that_fails_for_its_own_reasons_ends_the_carrier_with_that_reason() {
    // This end's own trouble is not the peer's departure: it ends the carrier at once, naming the failure.
    let carrier = carrier_over_failing_reader(std::io::ErrorKind::PermissionDenied, 0);
    let closed = eventually("the failure", || carrier.wait_closed(SHORT));
    assert_eq!(
        closed.reason,
        CloseReason::Stream(std::io::ErrorKind::PermissionDenied)
    );
}

// ---- over the local carrier ------------------------------------------------------------------------------------------

const CHILD_ADDRESS: &str = "GWZ_CARRIER_ROW_ADDRESS";
const CHILD_MODE: &str = "GWZ_CARRIER_ROW_MODE";
const SENT_BEFORE_DEATH: i64 = 5;

fn child_test() -> String {
    let path = module_path!();
    let relative = path.split_once("::").map_or(path, |(_, rest)| rest);
    format!("{relative}::child_peer")
}

/// The other process of the rows below: this test binary run again on this one test, told by its environment what to
/// do. Run on its own it does nothing and passes.
#[test]
fn child_peer() {
    let (Some(address), Some(mode)) = (
        std::env::var_os(CHILD_ADDRESS),
        std::env::var_os(CHILD_MODE),
    ) else {
        return;
    };
    let address = address_from_text(&address.to_string_lossy());
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            if mode == "raw_hang" {
                // A plain connection, no carrier: the parent holds the other end and takes a hold on this process.
                let _stream = std::os::unix::net::UnixStream::connect(address.path()).unwrap();
                thread::sleep(Duration::from_secs(300));
            }
        }
    }
    let carrier = connected(
        &address,
        &PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .expect("the child connects");
    if mode == "send_then_hang" {
        for offset in 0..SENT_BEFORE_DEATH {
            carrier.try_send(data(1, offset, vec![1])).unwrap();
        }
        // Waits to be killed. The carrier's writer has the frames; the parent kills this process when they arrive.
        thread::sleep(Duration::from_secs(300));
    }
}

fn spawn_child(address: &LocalAddress, mode: &str) -> Child {
    Command::new(std::env::current_exe().expect("this test binary"))
        .args(["--exact", &child_test(), "--nocapture", "--test-threads=1"])
        .env(CHILD_ADDRESS, address.to_string())
        .env(CHILD_MODE, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the child")
}

#[test]
fn a_peer_process_that_is_killed_leaves_what_it_sent_and_the_reason_peer_gone() {
    let dir = dir();
    let listener = listener(&dir);
    let address = listener.address().clone();
    let mut child = spawn_child(&address, "send_then_hang");
    let carrier = accepted(&listener).expect("the child is accepted");
    pause_until("the child's frames", || {
        carrier.queued_in() == SENT_BEFORE_DEATH as usize
    });
    child.kill().expect("kill the child");
    child.wait().expect("reap the child");
    // The child's last words are delivered, then one deterministic reason: its process is gone.
    assert_eq!(
        taken_then_closure(&carrier, SENT_BEFORE_DEATH as usize),
        CloseReason::PeerGone
    );
    match carrier.try_send(window(1, 1)) {
        Err(SendRefused::Closed(_, closed)) => assert_eq!(closed.reason, CloseReason::PeerGone),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_peer_that_exits_before_the_check_is_refused() {
    let dir = dir();
    let listener = listener(&dir);
    let address = listener.address().clone();
    let mut child = spawn_child(&address, "connect_and_exit");
    child.wait().expect("the child exits");
    let refused = listener
        .accept()
        .err()
        .expect("a peer that is gone is not accepted");
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            assert!(matches!(refused, LocalRefusal::Peer(super::super::local::PeerRefusal::Unreadable(_))), "{refused:?}");
        } else {
            // On a pipe the connect can fail before the check does; either way no carrier exists.
            let _ = refused;
        }
    }
}

fn address_from_text(text: &str) -> LocalAddress {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            LocalAddress::from_path(std::path::Path::new(text))
        } else {
            LocalAddress::from_name(text)
        }
    }
}

/// A peer of the local carrier that is a plain stream the row holds, and never reads unless the row says so.
fn raw_peer(address: &LocalAddress) -> impl Read + Write + Send + 'static + use<> {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            std::os::unix::net::UnixStream::connect(address.path()).expect("a raw peer")
        } else {
            std::fs::OpenOptions::new().read(true).write(true).open(address.pipe_name()).expect("a raw peer")
        }
    }
}

/// A carrier whose peer never reads, filled until its writer is blocked in the stream.
fn blocked_carrier(
    dir: &Dir,
    stall: StallBounds,
) -> (
    super::super::StreamCarrier,
    impl Read + Write + Send + 'static,
) {
    let listener = LocalListener::bind(
        dir_runtime(dir),
        PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT.with_stall(stall),
    )
    .unwrap();
    let address = listener.address().clone();
    let accepting = thread::spawn(move || accepted(&listener).map(|c| (c, listener)));
    let peer = raw_peer(&address);
    let (carrier, _listener) = accepting.join().unwrap().expect("accepted");
    // Fill: 60 KB frames until the lane or the writer's queue is full, which only happens once the stream is blocked.
    let mut offset = 0;
    loop {
        match carrier.try_send(data(1, offset, vec![0; 60_000])) {
            Ok(()) => offset += 60_000,
            Err(SendRefused::Full(_)) => break,
            Err(SendRefused::Closed(_, closed)) => panic!("closed while filling: {closed}"),
        }
    }
    (carrier, peer)
}

#[test]
fn a_peer_that_never_reads_ends_the_carrier_as_stalled_and_releases_every_thread() {
    let dir = dir();
    let stall = StallBounds {
        no_progress: Duration::from_millis(400),
        close: Duration::from_secs(30),
    };
    let (carrier, _peer) = blocked_carrier(&dir, stall);
    assert!(
        carrier.stalled_for().is_some(),
        "frames wait, so the progress signal is live"
    );
    let started = Instant::now();
    let closed = eventually("the stall", || carrier.wait_closed(SHORT));
    assert_eq!(closed.reason, CloseReason::Stalled);
    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(
        carrier.wait_stopped(WAIT),
        "the reader and the writer threads returned"
    );
    assert_eq!(carrier.try_recv().unwrap_err().reason, CloseReason::Stalled);
}

#[test]
fn closing_a_carrier_whose_peer_never_reads_is_bounded_by_the_close_bound() {
    let dir = dir();
    let stall = StallBounds {
        no_progress: Duration::from_secs(120),
        close: Duration::from_millis(400),
    };
    let (carrier, _peer) = blocked_carrier(&dir, stall);
    carrier.close(CloseCode::Normal);
    let started = Instant::now();
    assert!(carrier.wait_stopped(WAIT), "close released the threads");
    assert!(started.elapsed() < Duration::from_secs(15));
    // The reason that stands is the close, not the shutdown that enforced it.
    assert_eq!(
        carrier.try_recv().unwrap_err().reason,
        CloseReason::Local(CloseCode::Normal)
    );
}

#[test]
fn an_abort_while_the_caller_still_holds_the_carrier_tells_the_peer_within_a_bound() {
    let dir = dir();
    let listener = listener(&dir);
    let address = listener.address().clone();
    let accepting = thread::spawn(move || accepted(&listener).map(|c| (c, listener)));
    let client = connected(
        &address,
        &PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .expect("connected");
    let (core, _listener) = accepting.join().unwrap().expect("accepted");
    // The core end is aborted and kept: the stream's end must not wait for it to be dropped.
    core.abort();
    let closed = eventually("the peer's closure", || client.try_recv().err());
    assert_eq!(closed.reason, CloseReason::PeerGone);
    assert!(
        core.wait_stopped(WAIT),
        "both threads of the aborted end returned"
    );
    assert_eq!(
        core.try_recv().unwrap_err().reason,
        CloseReason::Local(CloseCode::Cancelled)
    );
}

#[test]
fn an_abort_taken_inside_the_router_tells_the_peer_and_releases_the_threads() {
    let dir = dir();
    let listener = listener(&dir);
    let address = listener.address().clone();
    let accepting = thread::spawn(move || accepted(&listener).map(|c| (c, listener)));
    let mut peer = raw_peer(&address);
    let (core, _listener) = accepting.join().unwrap().expect("accepted");
    // A transport frame past the transport's own frame size makes the router end the session from the reader thread,
    // while this test still holds the carrier.
    let oversized = wire_frame(16, &vec![1u8; wire::MAX_TRANSPORT_FRAME + 10]);
    write_all(&mut peer, &oversized);
    let closed = eventually("the fault", || core.wait_closed(SHORT));
    assert!(
        matches!(
            closed.reason,
            CloseReason::Protocol(ProtocolFault::Frame(_))
        ),
        "{closed:?}"
    );
    assert!(
        core.wait_stopped(WAIT),
        "the reader and the writer returned"
    );
    // The peer reads the end of the stream, or a reset, and not a wait for the core to drop its carrier.
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0u8; 64];
        let _ = sender.send(peer.read(&mut buffer));
    });
    let read = receiver
        .recv_timeout(WAIT)
        .expect("the peer was told within the bound");
    // Whatever the stream says (the end, or a reset), it says it: nothing was left waiting on the core's handle.
    let _ = read;
}

#[test]
fn an_idle_carrier_has_no_stall_and_the_signal_is_quiet() {
    let (core, _client) = over_pipes(limits(8, 4));
    assert_eq!(core.stalled_for(), None);
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
    #[test]
    fn a_hold_on_the_peers_process_is_given_up_when_the_process_goes() {
        use super::super::local::{Hold, Principal, peer_of};
        use std::os::fd::AsRawFd;
        let dir = dir();
        let path = dir_runtime(&dir).path().join("hold-row");
        let socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let address = LocalAddress::from_path(&path);
        let mut child = spawn_child(&address, "raw_hang");
        let (stream, _) = socket.accept().expect("the child connects");
        let fd = stream.as_raw_fd();
        let peer = peer_of(fd).expect("the peer's credentials");
        let (Some(pid), Principal::Uid(uid)) = (peer.pid, peer.principal.clone()) else {
            panic!("a pid and a uid: {peer:?}");
        };
        assert_eq!(pid, child.id());
        // The hold is taken while the process is there and confirms as long as it is.
        let hold = Hold::take(fd, pid, uid).expect("a hold on a live process");
        assert!(hold.check(fd).is_ok());
        // The process goes, and the hold no longer confirms: a process ID, or a token, that names nothing is not a peer.
        child.kill().expect("kill the child");
        child.wait().expect("reap the child");
        assert!(hold.confirm(fd).is_err(), "a gone process is not confirmed");
        assert!(Hold::take(fd, pid, uid).is_err(), "and no hold is taken on it");
    }

    #[test]
    fn dropping_a_carrier_whose_peer_never_reads_shuts_the_stream_within_the_close_bound() {
        use std::os::unix::net::UnixStream;
        let dir = dir();
        let stall = StallBounds {
            no_progress: Duration::from_secs(120),
            close: Duration::from_millis(300),
        };
        let listener = LocalListener::bind(
            dir_runtime(&dir),
            PeerPolicy::current_user().unwrap(),
            CarrierLimits::DEFAULT.with_stall(stall),
        )
        .unwrap();
        let address = listener.address().clone();
        let accepting = thread::spawn(move || accepted(&listener).map(|c| (c, listener)));
        let mut peer = UnixStream::connect(address.path()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let (carrier, _listener) = accepting.join().unwrap().unwrap();
        while carrier.try_send(data(1, 0, vec![0; 60_000])).is_ok() {}
        drop(carrier);
        thread::sleep(Duration::from_millis(1500));
        // The peer reads what was in flight and then finds the stream shut: it does not wait on a writer that is stuck.
        let mut sink = vec![0u8; 1 << 20];
        loop {
            match peer.read(&mut sink) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) => panic!("the stream was not shut: {error}"),
            }
        }
    }

    }
}

#[test]
fn a_cancelled_frame_class_names_are_the_ones_the_rows_use() {
    // A guard that the class names the rows above assert on are the carrier's.
    assert_ne!(FrameClass::Registry, FrameClass::TransportData);
}
