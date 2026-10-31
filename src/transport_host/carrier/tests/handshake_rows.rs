//! The host's side of the handshake: over a scripted wire with a clock the row sets, and over a real stream carrier
//! (step 2.3d-1).
use super::super::{
    CarrierLimits, CloseCode, CloseReason, Lane,
    handshake::{
        Action, Bounds, CarrierWire, Control, Hello, HostFault, HostOutcome, OpenRequest, Opened,
        OpenedInfo, Refusal, SERVER_VERSION_MISMATCH, WireEnd, accept_open,
    },
};
use super::handshake_support::*;
use super::support::{decode_session_error, eventually, over_pipes_unbound};
use gwz_session_contract::{Frame, FrameKey, Tag, Target};
use std::{
    thread,
    time::{Duration, Instant},
};

const SHAPE: &str = "the session open is not of the client placement's shape";

fn frame(tag: Tag, body: Vec<u8>) -> Frame {
    Frame::new(tag, body)
}

fn setup(inbox: Vec<Frame>) -> (ScriptedFrames, RecordingHooks) {
    let log = Log::default();
    (
        ScriptedFrames::new(inbox, log.clone()),
        RecordingHooks::new(log),
    )
}

fn accept(wire: &mut ScriptedFrames, hooks: &mut RecordingHooks) -> Result<HostOutcome, HostFault> {
    accept_open(wire, &hello(), &Bounds::DEFAULT, hooks)
}

/// The one refusal a row expects after the hello: (call id, code, text).
fn only_refusal(wire: &ScriptedFrames) -> (u64, u64, String) {
    assert_eq!(wire.sent.len(), 2, "the hello and exactly one refusal");
    decode_session_error(&wire.sent[1])
}

#[test]
fn the_host_speaks_first_and_writes_its_hello_before_it_reads() {
    let (mut wire, mut hooks) = setup(vec![frame(Tag::SessionOpen, open_body(1))]);
    accept(&mut wire, &mut hooks).unwrap();
    assert_eq!(wire.sent[0], frame(Tag::SessionHello, hello().encode()));
    assert_eq!(&wire.log.borrow()[..2], ["send 6", "recv"]);
}

#[test]
fn a_silent_client_is_closed_at_30_s_on_the_injected_clock() {
    let (mut wire, mut hooks) = setup(vec![]);
    let start = wire.clock;
    assert_eq!(accept(&mut wire, &mut hooks), Err(HostFault::Expired));
    assert_eq!(wire.clock - start, Duration::from_secs(30));
    assert!(wire.closed);
    assert_eq!(wire.sent.len(), 1, "only the hello was written");
    assert!(hooks.opened.is_empty() && hooks.controls.is_empty());
}

#[test]
fn the_hosts_bound_counts_from_the_start() {
    let (mut wire, mut hooks) = setup(vec![]);
    wire.send_cost = Duration::from_secs(10);
    let start = wire.clock;
    assert_eq!(accept(&mut wire, &mut hooks), Err(HostFault::Expired));
    // The hello took 10 s, so the wait was 20 and the whole 30.
    assert_eq!(wire.clock - start, Duration::from_secs(30));
}

#[test]
fn an_open_runs_the_hook_before_session_opened_is_written() {
    let (mut wire, mut hooks) = setup(vec![frame(Tag::SessionOpen, open_body(2))]);
    let outcome = accept(&mut wire, &mut hooks).unwrap();
    let expected = OpenRequest {
        protocol: 2,
        client: "gwz 1.1.0".into(),
        limits: None,
    };
    assert_eq!(outcome, HostOutcome::Opened(expected.clone()));
    assert_eq!(hooks.opened, [expected]);
    assert_eq!(
        *wire.log.borrow(),
        ["send 6", "recv", "hook open", "send 5"],
        "the hook runs after the hello and before SessionOpened"
    );
    let answer = Opened::decode(&wire.sent[1].body).unwrap();
    let host = hello();
    assert_eq!(answer.protocol, 2);
    assert_eq!(
        (answer.server, answer.core, answer.server_pid),
        (host.server, host.core, host.server_pid)
    );
    assert_eq!(answer.open_sessions, 3, "the hook's count");
    assert!(!wire.closed, "the carrier stays open for the session");
}

#[test]
fn a_refusal_from_the_hook_is_one_session_error_and_no_session_opened() {
    let (mut wire, mut hooks) = setup(vec![frame(Tag::SessionOpen, open_body(1))]);
    hooks.open_answer = Err(Refusal {
        code: 77,
        message: "the server is full".into(),
    });
    assert_eq!(
        accept(&mut wire, &mut hooks),
        Ok(HostOutcome::Refused { code: 77 })
    );
    assert_eq!(only_refusal(&wire), (0, 77, "the server is full".into()));
    assert!(wire.closed);
}

fn canary_environment() -> Vec<u8> {
    // [{1: h'CANARY-SECRET', 2: h'x'}]
    let mut value = vec![0x81, 0xa2, 0x01, 0x4d];
    value.extend_from_slice(b"CANARY-SECRET");
    value.extend_from_slice(&[0x02, 0x41, b'x']);
    value
}

fn canary_openssl() -> Vec<u8> {
    let mut value = vec![0xa1, 0x03, 0x6e];
    value.extend_from_slice(b"CANARY-OPENSSL");
    value
}

#[test]
fn opens_outside_the_client_placement_are_refused_exactly_once() {
    let environment = canary_environment();
    let openssl = canary_openssl();
    let without_host_environment = raw_map(&[(1, &[1]), (2, &[0x61, b'c']), (5, &[0xa0])]);
    let cases: [(&str, Vec<u8>); 8] = [
        ("an environment", open_with(&[(3, &environment)])),
        ("a umask", open_with(&[(5, &[0xa1, 1, 0x12])])),
        ("an openssl", open_with(&[(5, &openssl)])),
        ("transport_off set", open_with(&[(5, &[0xa1, 2, 0xf5])])),
        ("host_environment false", open_with(&[(6, &[0xf4])])),
        ("no host_environment", without_host_environment),
        ("an unknown key", open_with(&[(9, &[0])])),
        ("not a map", vec![0x80]),
    ];
    for (name, body) in cases {
        let (mut wire, mut hooks) = setup(vec![frame(Tag::SessionOpen, body)]);
        assert_eq!(
            accept(&mut wire, &mut hooks),
            Ok(HostOutcome::Refused { code: 1 }),
            "{name}"
        );
        assert_eq!(only_refusal(&wire), (0, 1, SHAPE.into()), "{name}");
        assert!(wire.closed, "{name}");
        assert!(hooks.opened.is_empty(), "{name}: the hook ran");
        for sent in &wire.sent {
            assert!(
                !sent.body.windows(6).any(|w| w == b"CANARY"),
                "{name}: the canary was written back"
            );
        }
    }
}

#[test]
fn a_protocol_outside_the_hosts_range_is_server_version_mismatch() {
    for protocol in [0u32, 3, 99] {
        let (mut wire, mut hooks) = setup(vec![frame(Tag::SessionOpen, open_body(protocol))]);
        let code = SERVER_VERSION_MISMATCH;
        assert_eq!(
            accept(&mut wire, &mut hooks),
            Ok(HostOutcome::Refused { code })
        );
        let (call, found, text) = only_refusal(&wire);
        assert_eq!((call, found), (0, code), "protocol {protocol}");
        // The refusal may name the number the peer sent, and the host's own range; it repeats no text of the peer's.
        assert_eq!(
            text,
            format!("protocol {protocol} is outside the 1..=2 this host serves")
        );
        assert!(hooks.opened.is_empty());
    }
    for protocol in [1u32, 2] {
        let (mut wire, mut hooks) = setup(vec![frame(Tag::SessionOpen, open_body(protocol))]);
        assert!(matches!(
            accept(&mut wire, &mut hooks),
            Ok(HostOutcome::Opened(_))
        ));
    }
}

#[test]
fn a_session_opened_that_cannot_be_written_is_a_wire_fault_after_the_hook_ran() {
    // The hook has opened the session when it returns Ok; if `SessionOpened` cannot be written the host reports
    // `Err(Wire)` and the session the hook opened is the caller's to roll back (`Hooks::open`).
    let (mut wire, mut hooks) = setup(vec![frame(Tag::SessionOpen, open_body(2))]);
    wire.fail_send = Some(Tag::SessionOpened.byte());
    let outcome = accept(&mut wire, &mut hooks);
    assert!(
        matches!(outcome, Err(HostFault::Wire(WireEnd::Io(_)))),
        "{outcome:?}"
    );
    assert_eq!(hooks.opened.len(), 1, "the hook ran, so a session is open");
    assert_eq!(
        *wire.log.borrow(),
        ["send 6", "recv", "hook open", "send 5"],
        "the hook ran before the write that failed"
    );
    assert!(wire.closed, "the wire is closed on a fault");
}

#[test]
fn a_hook_refusal_may_hold_a_value_the_host_knows() {
    let (mut wire, mut hooks) = setup(vec![frame(Tag::SessionOpen, open_body(1))]);
    let open_sessions = 64;
    hooks.open_answer = Err(Refusal {
        code: 77,
        message: format!("the server has {open_sessions} open sessions, its --max-sessions").into(),
    });
    assert_eq!(
        accept(&mut wire, &mut hooks),
        Ok(HostOutcome::Refused { code: 77 })
    );
    assert_eq!(
        only_refusal(&wire),
        (
            0,
            77,
            "the server has 64 open sessions, its --max-sessions".into()
        )
    );
}

#[test]
fn a_session_open_that_is_not_canonical_is_refused() {
    // Key 1 spelled 0x18 0x01: the shape is right and the encoding is not the one the value has.
    let body = [0xa4, 1, 0x18, 1, 2, 0x61, b'c', 5, 0xa0, 6, 0xf5].to_vec();
    let (mut wire, mut hooks) = setup(vec![frame(Tag::SessionOpen, body)]);
    assert_eq!(
        accept(&mut wire, &mut hooks),
        Ok(HostOutcome::Refused { code: 1 })
    );
    assert_eq!(only_refusal(&wire).2, "the request is not canonical");
    assert!(hooks.opened.is_empty());
}

#[test]
fn a_server_control_gets_state_and_a_close_and_is_not_a_session() {
    for (action, force) in [(Action::Status, false), (Action::Stop, true)] {
        let control = Control { action, force };
        let (mut wire, mut hooks) = setup(vec![frame(Tag::ServerControl, control.encode())]);
        assert_eq!(
            accept(&mut wire, &mut hooks),
            Ok(HostOutcome::Controlled(control))
        );
        assert_eq!(wire.sent.len(), 2);
        assert_eq!(wire.sent[1], frame(Tag::ServerState, state().encode()));
        assert!(wire.closed, "ServerState is followed by the close");
        assert_eq!(hooks.controls, [control]);
        assert!(
            hooks.opened.is_empty(),
            "a control exchange is not a session"
        );
    }
}

#[test]
fn a_control_the_host_refuses_is_one_session_error() {
    let control = Control {
        action: Action::Status,
        force: false,
    };
    let (mut wire, mut hooks) = setup(vec![frame(Tag::ServerControl, control.encode())]);
    hooks.control_answer = Err(Refusal {
        code: 1,
        message: "a stdio host has no server to control".into(),
    });
    assert_eq!(
        accept(&mut wire, &mut hooks),
        Ok(HostOutcome::Refused { code: 1 })
    );
    assert_eq!(only_refusal(&wire).1, 1);
    assert!(wire.closed);
    // A body that is not a ServerControl is refused before the hook is asked.
    let (mut wire, mut hooks) = setup(vec![frame(Tag::ServerControl, vec![0xa1, 1, 5])]);
    assert_eq!(
        accept(&mut wire, &mut hooks),
        Ok(HostOutcome::Refused { code: 1 })
    );
    assert!(hooks.controls.is_empty());
}

#[test]
fn any_other_first_frame_closes_without_a_reply() {
    for tag in [1u8, 2, 3, 5, 6, 8, 16, 17, 200] {
        let (mut wire, mut hooks) = setup(vec![Frame {
            tag,
            body: vec![0xa0],
        }]);
        assert_eq!(
            accept(&mut wire, &mut hooks),
            Err(HostFault::Unexpected(tag))
        );
        assert_eq!(wire.sent.len(), 1, "tag {tag}: only the hello was written");
        assert!(wire.closed, "tag {tag}");
        assert!(hooks.opened.is_empty() && hooks.controls.is_empty());
    }
}

#[test]
fn a_hello_the_host_could_not_send_is_not_sent() {
    for bad in [
        Hello {
            protocol_min: 0,
            ..hello()
        },
        Hello {
            protocol_min: 3,
            protocol_max: 2,
            ..hello()
        },
    ] {
        let (mut wire, mut hooks) = setup(vec![]);
        assert_eq!(
            accept_open(&mut wire, &bad, &Bounds::DEFAULT, &mut hooks),
            Err(HostFault::BadHello)
        );
        assert!(wire.sent.is_empty() && wire.closed);
    }
}

fn ordinary() -> FrameKey {
    FrameKey::ordinary(Target::new(0))
}

#[test]
fn a_host_over_a_real_carrier_serves_an_open() {
    let (core, client) = over_pipes_unbound(CarrierLimits::DEFAULT);
    let host = thread::scope(|scope| {
        let host = scope.spawn(|| {
            let mut hooks = RecordingHooks::new(Log::default());
            accept_open(
                &mut CarrierWire(&core),
                &hello(),
                &Bounds::DEFAULT,
                &mut hooks,
            )
        });
        let first = eventually("the hello", || client.try_recv_registry().unwrap());
        assert_eq!(first, frame(Tag::SessionHello, hello().encode()));
        client
            .send_frame(
                frame(Tag::SessionOpen, open_body(2)),
                Lane::Control,
                ordinary(),
            )
            .unwrap();
        let reply = eventually("SessionOpened", || client.try_recv_registry().unwrap());
        assert_eq!(reply.tag, Tag::SessionOpened.byte());
        host.join().unwrap()
    });
    assert!(matches!(host, Ok(HostOutcome::Opened(_))));
}

#[test]
fn a_silent_client_on_a_real_carrier_is_closed_at_a_short_bound() {
    let (core, client) = over_pipes_unbound(CarrierLimits::DEFAULT);
    let bounds = Bounds {
        host_wait: Duration::from_millis(150),
        ..Bounds::DEFAULT
    };
    let start = Instant::now();
    let mut hooks = RecordingHooks::new(Log::default());
    let outcome = accept_open(&mut CarrierWire(&core), &hello(), &bounds, &mut hooks);
    assert_eq!(outcome, Err(HostFault::Expired));
    assert!(start.elapsed() >= Duration::from_millis(150));
    // The client reads the hello the host wrote, and then sees the close the host announced.
    let first = eventually("the hello", || client.try_recv_registry().ok().flatten());
    assert_eq!(first.tag, Tag::SessionHello.byte());
    let closed = eventually("the client to see the close", || {
        client
            .recv_registry_timeout(Duration::from_millis(20))
            .err()
    });
    assert_eq!(closed.reason, CloseReason::Peer(CloseCode::Normal));
}
