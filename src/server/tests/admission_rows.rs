//! A server's admission of sessions (plan step 2.3g2; server design §3, §6 and §17; `GwzSplitCommandChannel.md` §4.3,
//! C20): `--max-sessions` and the client's `SessionLimits` (operator decision Q1 (a)), over a real local carrier and
//! at the decode.
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use gwz_session_host::{Ending, Limits};

use super::session_rows::{Admit, Host, WAIT, call_frame, fetch_call, hello, reply, send};
use crate::server::session::SessionPolicy;
use crate::session_host::HostContext;
use crate::session_host::served::stream::{ServeFault, Served, StreamSession, session_limits};
use crate::transport_host::carrier::{
    CarrierLimits, StreamCarrier,
    handshake::{
        Action, Bounds, ClientConfig, ClientFault, ClientOutcome, Control, DialFault, Hooks,
        HostOutcome, Intent, OpenRequest, OpenedInfo, Refusal, State, decode_session_limits, dial,
    },
    local::{PeerPolicy, row_gate, walk_to},
};

/// `invalid_request` and `transport_session_full` on the wire.
const INVALID_REQUEST: u64 = 1;
const SESSION_FULL: u64 = 77;

/// The `SessionLimits` of the contract's §1 defaults, as the wire carries them: a map of ten keys in ascending order,
/// each value in its shortest head. Pinned: a change here is a wire change.
const DEFAULT_LIMITS: [u8; 35] = [
    0xaa, // a map of 10
    0x01, 0x08, // running_operations 8
    0x02, 0x18, 0x40, // queued_operations 64
    0x03, 0x18, 0x80, // operation_table 128
    0x04, 0x08, // direct_workers 8
    0x05, 0x19, 0x10, 0x00, // event_log 4096
    0x06, 0x18, 0x40, // open_logs 64
    0x07, 0x19, 0x04, 0x00, // outstanding_calls 1024
    0x08, 0x18, 0x40, // control_reserve 64
    0x09, 0x1a, 0x00, 0x10, 0x00, 0x00, // read_bytes 1 MiB
    0x0a, 0x19, 0xea, 0x60, // close_wait_ms 60 000
];

/// The default value of each key, 1 to 10, in milliseconds for `close_wait_ms`.
const DEFAULTS: [u64; 10] = [8, 64, 128, 8, 4096, 64, 1024, 64, 1024 * 1024, 60_000];

/// A CBOR head of `major` and `argument`, shortest form.
fn head(major: u8, argument: u64) -> Vec<u8> {
    let major = major << 5;
    match argument {
        0..=23 => vec![major | argument as u8],
        24..=0xff => vec![major | 24, argument as u8],
        0x100..=0xffff => [vec![major | 25], (argument as u16).to_be_bytes().to_vec()].concat(),
        0x1_0000..=0xffff_ffff => {
            [vec![major | 26], (argument as u32).to_be_bytes().to_vec()].concat()
        }
        _ => [vec![major | 27], argument.to_be_bytes().to_vec()].concat(),
    }
}

/// A map of `entries` in the order given, each an unsigned key and an already encoded value.
fn map(entries: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut out = head(5, entries.len() as u64);
    for (key, value) in entries {
        out.extend(head(0, *key));
        out.extend_from_slice(value);
    }
    out
}

/// The ten keys at their defaults, with `change` applied: `(key, value)` replaces a key's unsigned value.
fn stated(change: &[(u64, u64)]) -> Vec<u8> {
    let entries: Vec<(u64, Vec<u8>)> = (1..=10u64)
        .map(|key| {
            let value = change
                .iter()
                .find(|(changed, _)| *changed == key)
                .map_or(DEFAULTS[key as usize - 1], |(_, value)| *value);
            (key, head(0, value))
        })
        .collect();
    map(&entries)
}

/// The session's limits for `stated` at a host of the §1 defaults: `Ok`, or the refusal's code and message.
fn judged(stated: &[u8]) -> Result<Limits, (u64, String)> {
    session_limits(Some(stated), &Limits::default())
        .map_err(|refusal| (refusal.code, refusal.message.into_owned()))
}

/// Each key's field of [`Limits`], by key number, and its value there in `DEFAULTS`' units.
#[allow(clippy::type_complexity, reason = "a table of one accessor per key")]
const FIELDS: [(u64, &str, fn(&Limits) -> u64); 10] = [
    (1, "running_operations", |l| l.running_operations as u64),
    (2, "queued_operations", |l| l.queued_operations as u64),
    (3, "operation_table", |l| l.operation_table as u64),
    (4, "direct_workers", |l| l.direct_workers as u64),
    (5, "event_log", |l| l.event_log as u64),
    (6, "open_logs", |l| l.open_logs as u64),
    (7, "outstanding_calls", |l| l.outstanding_calls as u64),
    (8, "control_reserve", |l| l.control_reserve as u64),
    (9, "read_bytes", |l| l.read_bytes as u64),
    (10, "close_wait_ms", |l| l.close_wait.as_millis() as u64),
];

/// Golden: the wire bytes of `SessionLimits` at the defaults are pinned, are the generated taut type's own encoding,
/// and decode by key to the defaults; at the host, they give the host's limits.
#[test]
fn session_limits_wire_bytes_are_pinned_and_are_the_generated_types() {
    let generated = crate::SessionLimits {
        running_operations: 8,
        queued_operations: 64,
        operation_table: 128,
        direct_workers: 8,
        event_log: 4096,
        open_logs: 64,
        outstanding_calls: 1024,
        control_reserve: 64,
        read_bytes: 1024 * 1024,
        close_wait_ms: 60_000,
    };
    assert_eq!(crate::encode(&generated.to_cbor()), DEFAULT_LIMITS);
    assert_eq!(stated(&[]), DEFAULT_LIMITS);
    assert_eq!(
        crate::SessionLimits::decode(&DEFAULT_LIMITS).unwrap(),
        generated
    );
    assert_eq!(decode_session_limits(&DEFAULT_LIMITS), Ok(DEFAULTS));
    assert_eq!(judged(&DEFAULT_LIMITS), Ok(Limits::default()));
    // The `SessionOpen` that carries them, key 4 between the client (2) and the attributes (5).
    let open = OpenRequest {
        protocol: 1,
        client: "c".into(),
        limits: Some(DEFAULT_LIMITS.to_vec()),
    };
    let expected = [
        &[0xa5, 1, 1, 2, 0x61, b'c', 4][..],
        &DEFAULT_LIMITS[..],
        &[5, 0xa0, 6, 0xf5][..],
    ]
    .concat();
    assert_eq!(open.encode(), expected);
}

/// Each `SessionLimits` field is decoded by its key number: a value below its default at key k changes the field the
/// contract's §1 table lists k-th, and no other.
#[test]
fn each_session_limit_is_decoded_by_its_key_number() {
    let defaults = Limits::default();
    for (key, name, field) in FIELDS {
        let lower = DEFAULTS[key as usize - 1] - 1;
        let limits = judged(&stated(&[(key, lower)]))
            .unwrap_or_else(|refused| panic!("key {key} ({name}) refused: {refused:?}"));
        for (other, other_name, other_field) in FIELDS {
            let expected = if other == key {
                lower
            } else {
                other_field(&defaults)
            };
            assert_eq!(
                other_field(&limits),
                expected,
                "key {key} ({name}) set {other_name}"
            );
        }
        assert_eq!(field(&limits), lower, "key {key} is {name}");
    }
}

/// A value above the host's default is `invalid_request`, for every key, naming the limit and both numbers; at the
/// default every key is accepted (the golden row); below it, `open`'s validation still applies.
#[test]
fn a_value_above_the_hosts_default_is_invalid_request() {
    for (key, name, _) in FIELDS {
        let above = DEFAULTS[key as usize - 1] + 1;
        let (code, message) = judged(&stated(&[(key, above)])).expect_err(name);
        assert_eq!(code, INVALID_REQUEST, "{name}");
        assert!(
            message.contains(name) && message.contains(&above.to_string()),
            "{name}: {message}"
        );
    }
    // At or below the default, the whole goes to `open`'s validation: no running slot, and a table smaller than the
    // running plus the queued operations, are refused with its texts.
    let (code, message) = judged(&stated(&[(1, 0)])).unwrap_err();
    assert_eq!(code, INVALID_REQUEST);
    assert!(
        message.contains("running_operations must be at least 1"),
        "{message}"
    );
    let (code, message) = judged(&stated(&[(3, 71)])).unwrap_err();
    assert_eq!(code, INVALID_REQUEST);
    assert!(message.contains("operation_table must hold"), "{message}");
}

/// An unknown key is `invalid_request`, as is every map the canonical rule refuses: a missing key, a negative, null or
/// text value, a key out of order or twice, a head not in its shortest form, a value that is not a map, bytes after it.
#[test]
fn an_unknown_key_and_a_malformed_map_are_invalid_request() {
    let every: Vec<(u64, Vec<u8>)> = (1..=10u64)
        .map(|key| (key, head(0, DEFAULTS[key as usize - 1])))
        .collect();
    let with = |key: u64, value: Vec<u8>| -> Vec<u8> {
        let mut entries = every.clone();
        entries.retain(|(found, _)| *found != key);
        entries.push((key, value));
        entries.sort_by_key(|(key, _)| *key);
        map(&entries)
    };
    let mut eleven = every.clone();
    eleven.push((11, head(0, 1)));
    let mut zero = vec![(0, head(0, 1))];
    zero.extend(every.clone());
    let mut unsorted = every.clone();
    unsorted.swap(0, 1);
    let mut twice = every.clone();
    twice[1] = (1, head(0, 8));
    let mut long_head = vec![0xaa, 0x01, 0x18, 0x08];
    long_head.extend_from_slice(&DEFAULT_LIMITS[3..]);
    let mut trailing = DEFAULT_LIMITS.to_vec();
    trailing.push(0x00);
    let cases: [(&str, Vec<u8>, &str); 11] = [
        ("key 11", map(&eleven), "key 11"),
        ("key 0", map(&zero), "key 0"),
        ("a missing key", map(&every[..9]), "key 10"),
        ("a negative value", with(4, head(1, 0)), "key 4"),
        ("null", with(6, vec![0xf6]), ""),
        ("text", with(2, vec![0x61, b'8']), "key 2"),
        ("keys out of order", map(&unsorted), ""),
        ("a key twice", map(&twice), ""),
        ("a long head", long_head, ""),
        (
            "an array",
            [vec![0x8a], DEFAULTS.map(|v| head(0, v)).concat()].concat(),
            "",
        ),
        ("trailing bytes", trailing, ""),
    ];
    for (what, bytes, names) in cases {
        let (code, message) = judged(&bytes).expect_err(what);
        assert_eq!(code, INVALID_REQUEST, "{what}");
        assert!(message.contains(names), "{what}: {message}");
    }
}

/// The composition's own hooks with a control answer: admits every open and answers a `ServerState`.
struct Answer;

impl Hooks for Answer {
    fn open(&mut self, _open: &OpenRequest) -> Result<OpenedInfo, Refusal> {
        Ok(OpenedInfo { open_sessions: 0 })
    }

    fn control(&mut self, _control: &Control) -> Result<State, Refusal> {
        Ok(State {
            server_pid: std::process::id(),
            open_sessions: 0,
            max_sessions: 0,
            idle_exit: None,
            log: None,
            stopping: false,
        })
    }
}

/// One connection: the host serves it with `outer` while a client dials with `limits` and `intent`.
fn connect(
    host: &Host,
    outer: &mut (dyn Hooks + Send),
    limits: Option<Vec<u8>>,
    intent: Intent,
) -> (
    Result<Served, ServeFault>,
    Result<(StreamCarrier, ClientOutcome), DialFault>,
) {
    thread::scope(|scope| {
        let server = scope.spawn(|| {
            let carrier: Arc<StreamCarrier> = host.accept();
            host.serve(carrier, outer)
        });
        let config = ClientConfig {
            supported: 1..=1,
            client: "gwz rows".into(),
            limits,
            peer_pid: None,
            bounds: Bounds::DEFAULT,
        };
        let gate = row_gate();
        let client = dial(
            walk_to(host.address(), &gate).expect("the walk"),
            &PeerPolicy::current_user().unwrap(),
            CarrierLimits::DEFAULT,
            &config,
            &intent,
        );
        (server.join().unwrap(), client)
    })
}

/// A session opened over a real carrier: the host's side, the client's carrier and the open sessions it was told of.
fn opened(host: &Host, limits: Option<Vec<u8>>) -> (StreamSession, StreamCarrier, u32) {
    let (served, client) = connect(host, &mut Admit::default(), limits, Intent::Open);
    let Ok(Served::Session(session)) = served else {
        panic!("no session: {:?}", served.err());
    };
    let (carrier, outcome) = client.expect("the dial");
    let ClientOutcome::Opened { opened, .. } = outcome else {
        panic!("not opened: {outcome:?}");
    };
    (session, carrier, opened.open_sessions)
}

/// A dial the host refuses: the code and the message the client read.
fn refused(host: &Host, limits: Option<Vec<u8>>) -> (u64, String) {
    refused_by(host, &mut Admit::default(), limits)
}

/// A dial the host refuses with `outer` as the composition's own hooks: the code and the message the client read.
fn refused_by(
    host: &Host,
    outer: &mut (dyn Hooks + Send),
    limits: Option<Vec<u8>>,
) -> (u64, String) {
    let (served, client) = connect(host, outer, limits, Intent::Open);
    let Ok(Served::Ended(HostOutcome::Refused { code: host_code })) = served else {
        panic!("not refused: {:?}", served.err());
    };
    let Err(DialFault::Handshake(ClientFault::Refused { code, message })) = client else {
        panic!(
            "the client was not refused: {:?}",
            client.map(|(_, outcome)| outcome)
        );
    };
    assert_eq!(code, host_code);
    (code, message)
}

/// Ends the session of `client` with `session.close`, its first call.
fn close(client: &StreamCarrier) {
    let close = crate::SessionCloseRequest {};
    send(
        client,
        call_frame(1, "session.close", crate::encode(&close.to_cbor())),
        1,
    );
    reply(client, 1).expect("the close report");
}

/// (C20) A session beyond `--max-sessions` N gets one `transport_session_full` naming N, before any session is built;
/// a `ServerControl` is still answered at the cap; once one session has ended, the next is served. `SessionOpened`
/// counts the host's other sessions.
#[test]
fn a_session_beyond_max_sessions_is_refused_naming_n_and_the_next_after_one_ends_is_served() {
    let policy = SessionPolicy::new(hello()).with_max_sessions(2).unwrap();
    let host = Host::new(policy);
    let (first, first_client, others) = opened(&host, None);
    assert_eq!(others, 0);
    let (_second, _second_client, others) = opened(&host, None);
    assert_eq!(others, 1);
    assert_eq!(host.policy.sessions().open(), 2);

    let (code, message) = refused(&host, None);
    assert_eq!(code, SESSION_FULL, "{message}");
    assert!(
        message.contains("2 open sessions") && message.contains("--max-sessions"),
        "{message}"
    );
    assert_eq!(host.script.seen.lock().unwrap().started, 0);
    assert_eq!(host.policy.sessions().open(), 2, "the refusal took no slot");
    // The stated limits are judged before the count: at the cap, limits above the default are still
    // `invalid_request`.
    let (code, message) = refused(&host, Some(stated(&[(1, 9)])));
    assert_eq!(code, INVALID_REQUEST, "{message}");

    let control = Intent::Control(Control {
        action: Action::Status,
        force: false,
    });
    let (served, client) = connect(&host, &mut Answer, None, control);
    assert!(
        matches!(served, Ok(Served::Ended(HostOutcome::Controlled(_)))),
        "{:?}",
        served.err()
    );
    assert!(matches!(client, Ok((_, ClientOutcome::State { .. }))));

    close(&first_client);
    // The slot is given back before the session's end is logged.
    host.log.wait_end();
    assert!(first.service().wait_ended(WAIT));
    assert_eq!(host.policy.sessions().open(), 1);
    let (_third, _third_client, others) = opened(&host, None);
    assert_eq!(others, 1);
    assert_eq!(host.policy.sessions().open(), 2);
}

/// `--max-sessions` is 64 unless given, and is 1 to 1024.
#[test]
fn max_sessions_is_64_by_default_and_1_to_1024() {
    assert_eq!(SessionPolicy::new(hello()).sessions().max(), 64);
    for (max, taken) in [(0, false), (1, true), (1024, true), (1025, false)] {
        assert_eq!(
            SessionPolicy::new(hello())
                .with_max_sessions(max)
                .map(|policy| policy.sessions().max()),
            taken.then_some(max),
            "{max}"
        );
    }
}

/// Over a real carrier: `SessionLimits` at the defaults are served; a lower value is applied to the session (one open
/// log: the second is `transport_session_full`); a value above the default and an unknown key are refused with
/// `invalid_request` before any session is built, and the refusal holds no slot.
#[test]
fn stated_limits_are_judged_at_the_open_and_applied_to_the_session() {
    let host = Host::new(SessionPolicy::new(hello()));
    let (session, client, _) = opened(&host, Some(DEFAULT_LIMITS.to_vec()));
    let spools = session.service().session().spools();
    Host::diff_log_of(&session, 1, 1);
    let second = spools.reserve().expect("a second log at the default");
    drop(second);
    close(&client);
    // Its slot is given back before its end is logged.
    host.log.wait_end();
    assert!(session.service().wait_ended(WAIT));
    assert_eq!(host.policy.sessions().open(), 0);

    let (session, _client, _) = opened(&host, Some(stated(&[(6, 1)])));
    let spools = session.service().session().spools();
    Host::diff_log_of(&session, 1, 1);
    let Err(full) = spools.reserve() else {
        panic!("a second log under open_logs 1");
    };
    assert_eq!(full.code, crate::model::ErrorCode::TransportSessionFull);
    assert!(full.message.contains("has 1 diff"), "{}", full.message);

    let (code, message) = refused(&host, Some(stated(&[(7, 1025)])));
    assert_eq!(code, INVALID_REQUEST);
    assert!(message.contains("outstanding_calls"), "{message}");
    let mut eleven = DEFAULT_LIMITS.to_vec();
    eleven[0] = 0xab;
    eleven.extend([0x0b, 0x01]);
    let (code, message) = refused(&host, Some(eleven));
    assert_eq!(code, INVALID_REQUEST);
    assert!(message.contains("key 11"), "{message}");
    // Only the open session holds a slot: each refusal gave its slot back before the host's side returned.
    assert_eq!(host.policy.sessions().open(), 1);
}

/// A session holds its slot until it has ended, however it ends. After its carrier is lost, while an operation that
/// ignores its cancel still runs, the session has not ended, so a dial at `--max-sessions` 1 is refused naming 1; once
/// the operation returns and the session's close has settled, its end is logged with its slot given back, and the next
/// session is served.
#[test]
fn a_session_holds_its_slot_until_it_has_ended_after_carrier_loss() {
    let host = Host::new(SessionPolicy::new(hello()).with_max_sessions(1).unwrap());
    let (first, client, _) = opened(&host, None);
    send(&client, fetch_call(&host.root, 1, "deaf-1"), 1);
    host.script.wait("the operation", |seen| seen.started == 1);
    client.abort();
    // The close the carrier's loss started has cancelled the operation, which runs on.
    host.script.wait("its cancel", |seen| seen.cancel_seen);
    assert!(!first.service().wait_ended(Duration::ZERO));

    let (code, message) = refused(&host, None);
    assert_eq!(code, SESSION_FULL, "{message}");
    assert!(message.contains("1 open sessions"), "{message}");
    assert!(
        !first.service().wait_ended(Duration::ZERO),
        "the session ended before its operation returned"
    );
    assert_eq!(host.policy.sessions().open(), 1);

    host.script.release();
    let end = host.log.wait_end();
    assert_eq!(end.ending, Ending::Channel, "{end}");
    assert!(first.service().wait_ended(Duration::ZERO));
    assert_eq!(host.policy.sessions().open(), 0);
    let (_next, _next_client, others) = opened(&host, None);
    assert_eq!(others, 0);
}

/// The composition's own hooks that refuse every open, as a server's own admission may.
struct Refuse;

impl Hooks for Refuse {
    fn open(&mut self, _open: &OpenRequest) -> Result<OpenedInfo, Refusal> {
        Err(Refusal {
            code: 2,
            message: "the composition refuses".into(),
        })
    }

    fn control(&mut self, _control: &Control) -> Result<State, Refusal> {
        Err(Refusal {
            code: 1,
            message: "the rows answer no control".into(),
        })
    }
}

/// A refused session holds no slot, whoever refuses: when the composition's own `open` refuses after the count took
/// a slot, its refusal reaches the client, the count is back to 0, and at `--max-sessions` 1 the next session is served.
#[test]
fn a_composition_refusal_gives_its_slot_back() {
    let host = Host::new(SessionPolicy::new(hello()).with_max_sessions(1).unwrap());
    let (code, message) = refused_by(&host, &mut Refuse, None);
    assert_eq!(code, 2, "{message}");
    assert!(message.contains("the composition refuses"), "{message}");
    assert_eq!(host.policy.sessions().open(), 0);
    let (_session, _client, others) = opened(&host, None);
    assert_eq!(others, 0);
    assert_eq!(host.policy.sessions().open(), 1);
}

/// The composition's own hooks that admit an open after shutting the host context down, so the session's service
/// cannot be built after the count has taken its slot.
struct ShutsDown<'a>(&'a HostContext);

impl Hooks for ShutsDown<'_> {
    fn open(&mut self, _open: &OpenRequest) -> Result<OpenedInfo, Refusal> {
        self.0.shutdown();
        Ok(OpenedInfo { open_sessions: 0 })
    }

    fn control(&mut self, _control: &Control) -> Result<State, Refusal> {
        Err(Refusal {
            code: 1,
            message: "the rows answer no control".into(),
        })
    }
}

/// A session admitted by the count and the composition that the host then cannot build is refused, and gives its slot
/// back by the time the host's side returns.
#[test]
fn a_session_the_host_cannot_build_gives_its_slot_back() {
    let host = Host::new(SessionPolicy::new(hello()).with_max_sessions(1).unwrap());
    let (code, message) = refused_by(&host, &mut ShutsDown(&host.context), None);
    assert_eq!(code, INVALID_REQUEST, "{message}");
    assert!(message.contains("shut down"), "{message}");
    assert_eq!(host.policy.sessions().open(), 0);
}
