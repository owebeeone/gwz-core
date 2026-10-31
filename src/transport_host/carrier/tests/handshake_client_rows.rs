//! The client's side of the handshake over a scripted wire and a clock the row sets (step 2.3d-1).
use super::super::{
    handshake::{
        Action, Bounds, Cause, ClientConfig, ClientFault, ClientOutcome, Control, FIRST_FRAME_MAX,
        Hello, Intent, OpenRequest, Opened, REPLY_FRAME_MAX, Refusal, SERVER_UNAVAILABLE,
        SERVER_VERSION_MISMATCH, Stage, State, Unavailable, accept_open, client_handshake,
    },
    validate_session_open,
    wire::{INVALID_REQUEST, SERVER_PEER_REFUSED},
};
use super::handshake_support::*;
use super::support::wire_frame;
use gwz_session_contract::{Frame, Tag};
use std::time::Duration;

fn config() -> ClientConfig {
    ClientConfig {
        supported: 1..=2,
        client: "gwz 1.1.0".into(),
        limits: None,
        peer_pid: Some(4242),
        bounds: Bounds::DEFAULT,
    }
}

fn hello_frame(hello: &Hello) -> Vec<u8> {
    wire_frame(Tag::SessionHello.byte(), &hello.encode())
}

fn opened_for(hello: &Hello, protocol: u32) -> Opened {
    Opened {
        protocol,
        server: hello.server.clone(),
        core: hello.core.clone(),
        server_pid: hello.server_pid,
        open_sessions: 1,
    }
}

fn run(wire: &mut ScriptedBytes, intent: &Intent) -> Result<ClientOutcome, ClientFault> {
    let started = wire.clock;
    client_handshake(wire, started, &config(), intent)
}

fn unavailable(fault: ClientFault) -> Unavailable {
    match fault {
        ClientFault::Unavailable(found) => found,
        other => panic!("expected server_unavailable, got {other}"),
    }
}

#[test]
fn a_client_sends_nothing_before_a_well_formed_hello() {
    let undecodable = wire_frame(Tag::SessionHello.byte(), &[0xa1, 1, 1]);
    let scripts: [(&str, Vec<u8>, After); 7] = [
        ("silent host", vec![], After::Silent),
        ("a banner", b"Welcome to a host\r\n".to_vec(), After::Silent),
        (
            "a well-formed hello body under SessionOpened's tag",
            wire_frame(Tag::SessionOpened.byte(), &hello().encode()),
            After::Silent,
        ),
        ("a hello that does not decode", undecodable, After::Silent),
        ("end at once", vec![], After::Ended),
        ("end inside the header", vec![9, 0], After::Ended),
        (
            "a second hello's tag but a call",
            wire_frame(Tag::SessionCall.byte(), &[0xa0]),
            After::Silent,
        ),
    ];
    for (name, script, after) in scripts {
        let mut wire = ScriptedBytes::new(script, after);
        let fault = run(&mut wire, &Intent::Open).unwrap_err();
        assert_eq!(fault.code(), Some(SERVER_UNAVAILABLE), "{name}: {fault}");
        assert!(wire.written.is_empty(), "{name}: the client wrote bytes");
        assert!(wire.aborted, "{name}: the stream was not aborted");
    }
}

#[test]
fn a_first_frame_that_is_empty_or_over_4_kib_fails_after_its_five_header_bytes() {
    for size in [0u32, FIRST_FRAME_MAX as u32 + 1, 1 << 20, u32::MAX] {
        let mut script = size.to_le_bytes().to_vec();
        script.push(Tag::SessionHello.byte());
        script.extend(vec![b'x'; 9000]);
        let mut wire = ScriptedBytes::new(script, After::Silent);
        let found = unavailable(run(&mut wire, &Intent::Open).unwrap_err());
        assert!(
            matches!(found.cause, Cause::NotAHello { .. }),
            "size {size}"
        );
        assert_eq!(wire.read, 5, "size {size}: the client read on");
        assert!(wire.written.is_empty());
    }
}

#[test]
fn the_first_frame_cap_is_4_kib_counting_the_tag() {
    // A hello whose frame is exactly 4096 bytes (tag and body) is read; one byte more is not.
    let mut hello = hello();
    hello.server = "s".repeat(10);
    let base = hello.encode().len() + 1;
    // The text head of `server` grows from one byte to three at 256 bytes.
    let pad = FIRST_FRAME_MAX - base - 2;
    hello.server = "s".repeat(10 + pad);
    assert_eq!(hello.encode().len() + 1, FIRST_FRAME_MAX);
    let mut script = hello_frame(&hello);
    script.extend(wire_frame(
        Tag::SessionOpened.byte(),
        &opened_for(&hello, 2).encode(),
    ));
    let mut wire = ScriptedBytes::new(script, After::Silent);
    assert!(run(&mut wire, &Intent::Open).is_ok());
    hello.server.push('s');
    let mut wire = ScriptedBytes::new(hello_frame(&hello), After::Silent);
    let found = unavailable(run(&mut wire, &Intent::Open).unwrap_err());
    assert!(matches!(found.cause, Cause::NotAHello { .. }));
    assert_eq!(wire.read, 5);
}

#[test]
fn a_reply_over_its_cap_is_a_protocol_error_after_five_bytes() {
    let hello = hello();
    let mut script = hello_frame(&hello);
    script.extend((REPLY_FRAME_MAX as u32 + 1).to_le_bytes());
    script.push(Tag::SessionOpened.byte());
    script.extend(vec![0u8; 20_000]);
    let mut wire = ScriptedBytes::new(script, After::Silent);
    assert!(matches!(
        run(&mut wire, &Intent::Open),
        Err(ClientFault::Protocol(_))
    ));
    assert_eq!(wire.read, hello_frame(&hello).len() + 5);
    assert!(wire.aborted);
}

#[test]
fn a_banner_is_shown_escaped_and_not_read_on() {
    let mut wire = ScriptedBytes::new(b"\x1b[31mWelcome\r\n".to_vec(), After::Silent);
    let fault = run(&mut wire, &Intent::Open).unwrap_err();
    let text = fault.to_string();
    assert!(text.contains("\\x1b[31m"), "{text}");
    assert!(!text.contains('\x1b') && !text.contains('\r'), "{text:?}");
    assert!(text.contains("not a SessionHello"), "{text}");
    assert_eq!(wire.read, 5);
}

#[test]
fn an_ended_stream_names_what_had_arrived() {
    let mut wire = ScriptedBytes::new(vec![b'a', b'\n', b'c'], After::Ended);
    let found = unavailable(run(&mut wire, &Intent::Open).unwrap_err());
    assert_eq!(found.stage, Stage::Hello);
    assert_eq!(
        found.cause,
        Cause::Ended {
            first: "a\\x0ac".into()
        }
    );
    // The host went away after its hello.
    let mut wire = ScriptedBytes::new(hello_frame(&hello()), After::Ended);
    let found = unavailable(run(&mut wire, &Intent::Open).unwrap_err());
    assert_eq!(found.stage, Stage::Reply);
    assert!(matches!(found.cause, Cause::Ended { .. }));
}

#[test]
fn a_silent_host_fails_at_30_s_on_the_injected_clock() {
    let mut wire = ScriptedBytes::new(vec![], After::Silent);
    let start = wire.clock;
    let fault = run(&mut wire, &Intent::Open).unwrap_err();
    let found = unavailable(fault);
    assert_eq!(found.stage, Stage::Hello);
    assert_eq!(
        found.cause,
        Cause::Silent {
            waited: Duration::from_secs(30),
            first: String::new()
        }
    );
    assert_eq!(found.peer_pid, Some(4242));
    assert_eq!(wire.clock - start, Duration::from_secs(30));
    assert!(wire.aborted);
    // Silent after the hello: the same bound, and the message names the stage and the process.
    let mut wire = ScriptedBytes::new(hello_frame(&hello()), After::Silent);
    let start = wire.clock;
    let fault = run(&mut wire, &Intent::Open).unwrap_err();
    assert_eq!(wire.clock - start, Duration::from_secs(30));
    let text = fault.to_string();
    assert!(text.contains("30s") && text.contains("reply"), "{text}");
    assert!(text.contains("process 4242"), "{text}");
}

#[test]
fn a_control_exchange_waits_5_s() {
    let control = Control {
        action: Action::Status,
        force: false,
    };
    let mut wire = ScriptedBytes::new(hello_frame(&hello()), After::Silent);
    let start = wire.clock;
    let found = unavailable(run(&mut wire, &Intent::Control(control)).unwrap_err());
    assert_eq!(found.stage, Stage::Reply);
    assert_eq!(wire.clock - start, Duration::from_secs(5));
}

#[test]
fn the_bound_counts_from_the_start_of_the_connect() {
    // The connect took 20 s: the hello may take 10 more, 30 s in all.
    let mut wire = ScriptedBytes::new(vec![], After::Silent);
    let started = wire.clock;
    wire.clock += Duration::from_secs(20);
    let fault = client_handshake(&mut wire, started, &config(), &Intent::Open).unwrap_err();
    assert_eq!(wire.clock - started, Duration::from_secs(30));
    assert!(matches!(
        unavailable(fault).cause,
        Cause::Silent { waited, .. } if waited == Duration::from_secs(30)
    ));
    // A connect that used the whole bound reads nothing at all.
    let mut wire = ScriptedBytes::new(hello_frame(&hello()), After::Silent);
    let started = wire.clock;
    wire.clock += Duration::from_secs(31);
    let fault = client_handshake(&mut wire, started, &config(), &Intent::Open).unwrap_err();
    assert_eq!(wire.reads, 0);
    assert_eq!(fault.code(), Some(SERVER_UNAVAILABLE));
    assert!(wire.aborted);
}

#[test]
fn a_range_outside_the_hosts_is_a_mismatch_and_nothing_is_sent() {
    for (low, high) in [(3, 4), (7, 9)] {
        let host = Hello {
            protocol_min: low,
            protocol_max: high,
            ..hello()
        };
        let mut wire = ScriptedBytes::new(hello_frame(&host), After::Silent);
        let fault = run(&mut wire, &Intent::Open).unwrap_err();
        assert_eq!(
            fault,
            ClientFault::VersionMismatch {
                host: (low, high),
                client: (1, 2)
            }
        );
        assert_eq!(fault.code(), Some(SERVER_VERSION_MISMATCH));
        assert!(fault.to_string().contains(&format!("{low}..={high}")));
        assert!(wire.written.is_empty() && wire.aborted);
    }
}

fn sent_protocol(wire: &ScriptedBytes) -> u32 {
    assert_eq!(wire.written[4], Tag::SessionOpen.byte());
    OpenRequest::decode(&wire.written[5..]).unwrap().protocol
}

#[test]
fn the_client_speaks_the_highest_version_both_do() {
    for ((low, high), expected) in [((1, 2), 2), ((1, 1), 1), ((2, 9), 2), ((1, 9), 2)] {
        let host = Hello {
            protocol_min: low,
            protocol_max: high,
            ..hello()
        };
        let mut script = hello_frame(&host);
        script.extend(wire_frame(
            Tag::SessionOpened.byte(),
            &opened_for(&host, expected).encode(),
        ));
        let mut wire = ScriptedBytes::new(script, After::Silent);
        assert!(run(&mut wire, &Intent::Open).is_ok(), "host {low}..={high}");
        assert_eq!(sent_protocol(&wire), expected, "host {low}..={high}");
    }
}

#[test]
fn an_open_is_the_frozen_shape_and_carries_no_environment() {
    for limits in [None, Some(vec![0xa1, 1, 5])] {
        let host = hello();
        let mut script = hello_frame(&host);
        script.extend(wire_frame(
            Tag::SessionOpened.byte(),
            &opened_for(&host, 2).encode(),
        ));
        let mut wire = ScriptedBytes::new(script, After::Silent);
        let mut config = config();
        config.limits = limits.clone();
        let started = wire.clock;
        client_handshake(&mut wire, started, &config, &Intent::Open).unwrap();
        // Exactly one frame: length, tag 4, the body.
        let size = u32::from_le_bytes(wire.written[..4].try_into().unwrap()) as usize;
        assert_eq!(wire.written.len(), 4 + size);
        assert_eq!(wire.written[4], Tag::SessionOpen.byte());
        let body = &wire.written[5..];
        assert_eq!(validate_session_open(body), Ok(()));
        // Four entries, or five with the limits: keys 1, 2, 5 and 6 and no other (no environment, no attributes).
        let entries = if limits.is_some() { 5 } else { 4 };
        let expected = OpenRequest {
            protocol: 2,
            client: "gwz 1.1.0".into(),
            limits,
        }
        .encode();
        assert_eq!(body, expected);
        assert_eq!(body[0], 0xa0 | entries);
    }
}

#[test]
fn a_complete_open_returns_the_hosts_answer() {
    let host = hello();
    let mut script = hello_frame(&host);
    script.extend(wire_frame(
        Tag::SessionOpened.byte(),
        &opened_for(&host, 2).encode(),
    ));
    let mut wire = ScriptedBytes::new(script, After::Silent);
    let outcome = run(&mut wire, &Intent::Open).unwrap();
    assert_eq!(
        outcome,
        ClientOutcome::Opened {
            hello: host.clone(),
            opened: opened_for(&host, 2)
        }
    );
    assert!(
        !wire.aborted,
        "a good handshake leaves the stream to the carrier"
    );
}

#[test]
fn a_session_opened_that_differs_from_the_hello_is_a_protocol_error() {
    let host = hello();
    let variants: [(&str, Opened); 4] = [
        ("protocol", opened_for(&host, 1)),
        (
            "pid",
            Opened {
                server_pid: 7,
                ..opened_for(&host, 2)
            },
        ),
        (
            "core",
            Opened {
                core: "another".into(),
                ..opened_for(&host, 2)
            },
        ),
        (
            "server",
            Opened {
                server: "another".into(),
                ..opened_for(&host, 2)
            },
        ),
    ];
    for (name, opened) in variants {
        let mut script = hello_frame(&host);
        script.extend(wire_frame(Tag::SessionOpened.byte(), &opened.encode()));
        let mut wire = ScriptedBytes::new(script, After::Silent);
        assert!(
            matches!(run(&mut wire, &Intent::Open), Err(ClientFault::Protocol(_))),
            "{name}"
        );
        assert!(wire.aborted, "{name}");
    }
}

#[test]
fn a_refusal_is_surfaced_and_a_nonzero_call_id_is_not_one() {
    let host = hello();
    let refusal = refusal_frame(1, "bad\r\x1b[2Jtext");
    let mut script = hello_frame(&host);
    script.extend(wire_bytes(&[refusal]));
    let mut wire = ScriptedBytes::new(script, After::Silent);
    let fault = run(&mut wire, &Intent::Open).unwrap_err();
    assert_eq!(
        fault,
        ClientFault::Refused {
            code: 1,
            message: "bad\\x0d\\x1b[2Jtext".into()
        }
    );
    assert!(wire.aborted);
    // A SessionError for call 7 belongs to a call, not to the handshake.
    let mut body = refusal_frame(1, "x").body;
    body[2] = 7;
    let mut script = hello_frame(&host);
    script.extend(wire_frame(Tag::SessionError.byte(), &body));
    let mut wire = ScriptedBytes::new(script, After::Silent);
    assert!(matches!(
        run(&mut wire, &Intent::Open),
        Err(ClientFault::Protocol(_))
    ));
}

#[test]
fn the_listeners_refusal_in_place_of_the_hello_is_reported_as_the_refusal_and_nothing_is_written() {
    let control = Intent::Control(Control {
        action: Action::Status,
        force: false,
    });
    for intent in [Intent::Open, control] {
        let refusal = refusal_frame(
            SERVER_PEER_REFUSED,
            "peer refused: the peer is another user\r\x1b[2J",
        );
        // The listener writes this one frame and closes.
        let mut wire = ScriptedBytes::new(wire_bytes(&[refusal]), After::Ended);
        let fault = run(&mut wire, &intent).unwrap_err();
        assert_eq!(
            fault,
            ClientFault::Refused {
                code: SERVER_PEER_REFUSED,
                message: "peer refused: the peer is another user\\x0d\\x1b[2J".into()
            }
        );
        assert_eq!(fault.code(), Some(SERVER_PEER_REFUSED));
        assert!(
            wire.written.is_empty(),
            "the client wrote to a listener that refused it"
        );
        assert!(wire.aborted);
    }
}

#[test]
fn a_first_frame_session_error_that_is_not_the_refusal_is_not_a_hello() {
    // For a call, not the handshake.
    let mut for_a_call = refusal_frame(SERVER_PEER_REFUSED, "x").body;
    for_a_call[2] = 7;
    // Not the shape of a refusal at all.
    let cases: [(&str, Vec<u8>); 3] = [
        ("call ID 7", for_a_call),
        ("no error map", vec![0xa1, 1, 0]),
        ("not CBOR", b"Welcome".to_vec()),
    ];
    for (name, body) in cases {
        let mut wire = ScriptedBytes::new(
            wire_bytes(&[Frame::new(Tag::SessionError, body)]),
            After::Silent,
        );
        let found = unavailable(run(&mut wire, &Intent::Open).unwrap_err());
        assert_eq!(found.stage, Stage::Hello, "{name}");
        assert!(matches!(found.cause, Cause::NotAHello { .. }), "{name}");
        assert!(wire.written.is_empty() && wire.aborted, "{name}");
    }
    // The 4 KiB cap holds for a refusal too: the body is not read.
    let mut script = (FIRST_FRAME_MAX as u32 + 1).to_le_bytes().to_vec();
    script.push(Tag::SessionError.byte());
    script.extend(refusal_frame(SERVER_PEER_REFUSED, "x").body);
    let mut wire = ScriptedBytes::new(script, After::Silent);
    let found = unavailable(run(&mut wire, &Intent::Open).unwrap_err());
    assert!(matches!(found.cause, Cause::NotAHello { .. }));
    assert_eq!(wire.read, 5);
}

#[test]
fn a_hook_refusal_that_holds_a_value_reaches_the_client_escaped() {
    let log = Log::default();
    let mut host_wire = ScriptedFrames::new(
        vec![Frame::new(Tag::SessionOpen, open_body(2))],
        log.clone(),
    );
    let mut hooks = RecordingHooks::new(log);
    let open_sessions = 64;
    hooks.open_answer = Err(Refusal {
        code: 77,
        message: format!("the server has {open_sessions} open sessions\r\x1b[2J").into(),
    });
    accept_open(&mut host_wire, &hello(), &Bounds::DEFAULT, &mut hooks).unwrap();
    // What the host wrote is what the client reads.
    let mut wire = ScriptedBytes::new(wire_bytes(&host_wire.sent), After::Silent);
    let fault = run(&mut wire, &Intent::Open).unwrap_err();
    assert_eq!(
        fault,
        ClientFault::Refused {
            code: 77,
            message: "the server has 64 open sessions\\x0d\\x1b[2J".into()
        }
    );
}

#[test]
fn a_host_that_sends_two_bytes_and_goes_silent_is_shown_what_it_sent() {
    let mut wire = ScriptedBytes::new(vec![b'$', 0x1b], After::Silent);
    let fault = run(&mut wire, &Intent::Open).unwrap_err();
    let text = fault.to_string();
    let found = unavailable(fault);
    assert_eq!(found.stage, Stage::Hello);
    assert_eq!(
        found.cause,
        Cause::Silent {
            waited: Duration::from_secs(30),
            first: "$\\x1b".into()
        }
    );
    assert!(text.contains("it had sent \"$\\x1b\""), "{text}");
    assert!(!text.contains('\x1b'), "{text:?}");
    assert!(wire.written.is_empty() && wire.aborted);
}

#[test]
fn a_request_longer_than_a_first_frame_is_refused_before_a_byte_is_written() {
    let frame_len = |n: usize| {
        OpenRequest {
            protocol: 2,
            client: "x".repeat(n),
            limits: None,
        }
        .encode()
        .len()
            + 1
    };
    let exact = (3900..FIRST_FRAME_MAX)
        .find(|n| frame_len(*n) == FIRST_FRAME_MAX)
        .expect("a client string that makes a request of exactly the cap");
    let host = hello();
    let mut script = hello_frame(&host);
    script.extend(wire_frame(
        Tag::SessionOpened.byte(),
        &opened_for(&host, 2).encode(),
    ));
    // Exactly the cap goes through.
    let mut config = config();
    config.client = "x".repeat(exact);
    let mut wire = ScriptedBytes::new(script.clone(), After::Silent);
    let started = wire.clock;
    assert!(client_handshake(&mut wire, started, &config, &Intent::Open).is_ok());
    // One byte more is the caller's mistake: nothing is written and the stream is aborted.
    config.client = "x".repeat(exact + 1);
    let mut wire = ScriptedBytes::new(script, After::Silent);
    let started = wire.clock;
    let fault = client_handshake(&mut wire, started, &config, &Intent::Open).unwrap_err();
    assert_eq!(
        fault,
        ClientFault::TooLarge {
            size: FIRST_FRAME_MAX + 1,
            cap: FIRST_FRAME_MAX
        }
    );
    assert_eq!(fault.code(), Some(INVALID_REQUEST));
    assert!(wire.written.is_empty() && wire.aborted);
}

/// A `ServerState` whose log is `length` bytes of ASCII.
fn state_with_log(length: usize) -> State {
    State {
        log: Some("l".repeat(length)),
        ..state()
    }
}

#[test]
fn the_reply_cap_is_128_kib_counting_the_tag() {
    assert_eq!(REPLY_FRAME_MAX, 128 * 1024);
    let length = (100_000..REPLY_FRAME_MAX)
        .find(|n| state_with_log(*n).encode().len() + 1 == REPLY_FRAME_MAX)
        .expect("a log that makes a reply of exactly the cap");
    let control = Intent::Control(Control {
        action: Action::Status,
        force: false,
    });
    let mut script = hello_frame(&hello());
    script.extend(wire_frame(
        Tag::ServerState.byte(),
        &state_with_log(length).encode(),
    ));
    let mut wire = ScriptedBytes::new(script, After::Silent);
    let outcome = run(&mut wire, &control).expect("a reply of exactly the cap is read");
    assert!(matches!(outcome, ClientOutcome::State { .. }));
    // One byte more is refused after its five header bytes.
    let mut script = hello_frame(&hello());
    script.extend(wire_frame(
        Tag::ServerState.byte(),
        &state_with_log(length + 1).encode(),
    ));
    let mut wire = ScriptedBytes::new(script, After::Silent);
    assert!(matches!(
        run(&mut wire, &control),
        Err(ClientFault::Protocol(_))
    ));
    assert_eq!(wire.read, hello_frame(&hello()).len() + 5);
}

#[test]
fn the_largest_legal_server_state_is_read_by_a_control_exchange() {
    // A log path of 32 767 UTF-16 units: up to 98 301 UTF-8 bytes (the Windows maximum).
    let state = State {
        log: Some("\u{20ac}".repeat(32_767)),
        ..state()
    };
    let control = Control {
        action: Action::Status,
        force: false,
    };
    let mut script = hello_frame(&hello());
    script.extend(wire_frame(Tag::ServerState.byte(), &state.encode()));
    let mut wire = ScriptedBytes::new(script, After::Silent);
    let outcome = run(&mut wire, &Intent::Control(control)).unwrap();
    assert_eq!(
        outcome,
        ClientOutcome::State {
            hello: hello(),
            state
        }
    );
}

#[test]
fn replies_out_of_order_are_protocol_errors() {
    let host = hello();
    let state = state().encode();
    let opened = opened_for(&host, 2).encode();
    let control = Intent::Control(Control {
        action: Action::Status,
        force: false,
    });
    let cases: [(&str, u8, &[u8], &Intent); 6] = [
        ("a second hello", 6, &host.encode(), &Intent::Open),
        ("a SessionOpen", 4, &open_body(1), &Intent::Open),
        ("a ServerState for an open", 8, &state, &Intent::Open),
        ("a SessionOpened for a control", 5, &opened, &control),
        ("a call", 1, &[0xa0], &Intent::Open),
        ("a transport frame", 16, &[0xa0], &Intent::Open),
    ];
    for (name, tag, body, intent) in cases {
        let mut script = hello_frame(&host);
        script.extend(wire_frame(tag, body));
        let mut wire = ScriptedBytes::new(script, After::Silent);
        assert!(
            matches!(run(&mut wire, intent), Err(ClientFault::Protocol(_))),
            "{name}"
        );
        assert!(wire.aborted, "{name}");
    }
}

#[test]
fn a_control_exchange_returns_the_state() {
    let host = hello();
    let control = Control {
        action: Action::Stop,
        force: true,
    };
    let mut script = hello_frame(&host);
    script.extend(wire_frame(Tag::ServerState.byte(), &state().encode()));
    let mut wire = ScriptedBytes::new(script, After::Silent);
    let outcome = run(&mut wire, &Intent::Control(control)).unwrap();
    assert_eq!(
        outcome,
        ClientOutcome::State {
            hello: host,
            state: state()
        }
    );
    assert_eq!(
        wire.written,
        wire_frame(Tag::ServerControl.byte(), &control.encode())
    );
}
