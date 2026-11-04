//! How a served stream session records its ends, and what `serve_stream` refuses (remediation of step 2.3g1: findings
//! F3/P3-1, F5 and F7/P3-2).
use std::sync::Arc;
use std::thread;

use gwz_session_contract::Tag;
use gwz_session_host::Ending;

use super::session_rows::{
    Admit, Host, Raw, call_frame, call_id_of, dir, fetch_call, hello, meta, send,
};
use crate::server::session::SessionPolicy;
use crate::session_host::served::stream::ServeFault;
use crate::transport_host::carrier::{
    CarrierLimits, CloseCode, CloseReason, StreamCarrier,
    local::{LocalListener, PeerPolicy},
};

/// F3/P3-1: a handler that panics is logged once, as panicked, with how long it ran: an operation's and a direct
/// call's alike. The client gets `internal_error` for each.
#[test]
fn a_panicking_handler_is_logged_once_as_panicked_with_its_duration() {
    let host = Host::new(SessionPolicy::new(hello()));
    let (_session, client) = host.dialled();
    send(&client, fetch_call(&host.root, 1, "panic-operation"), 1);
    let status = crate::FetchRequest {
        meta: meta(&host.root, "panic-direct"),
    };
    send(
        &client,
        call_frame(2, "status", crate::encode(&status.to_cbor())),
        2,
    );
    let mut answered = Vec::new();
    for _ in 0..2 {
        let frame = client
            .recv_registry_timeout(super::session_rows::WAIT)
            .unwrap()
            .expect("a reply");
        assert_eq!(frame.tag, Tag::SessionError.byte());
        let error = crate::SessionError::decode(&frame.body).unwrap().error;
        assert_eq!(error.code, crate::GwzErrorCode::InternalError);
        answered.push(call_id_of(&frame));
    }
    answered.sort_unstable();
    assert_eq!(answered, [1, 2]);
    let mut logged = host.log.wait_for(2);
    logged.sort_by_key(|entry| entry.call_id);
    assert_eq!(logged.len(), 2, "logged more than once: {logged:?}");
    for (entry, method) in logged.iter().zip(["fetch", "status"]) {
        assert_eq!(entry.method, method);
        assert!(entry.to_string().contains("outcome=panicked"), "{entry}");
        assert!(entry.duration.is_some(), "{entry}");
    }
    assert!(logged[0].operation_id.is_some() && logged[1].operation_id.is_none());
}

/// F7/P3-2: `serve_stream` serves only an end that holds its frames until the open
/// (`CarrierLimits::awaiting_open`); any other is refused before the hello, and closed.
#[test]
fn an_ungated_carrier_is_refused_before_the_hello() {
    let host = Host::new(SessionPolicy::new(hello()));
    let ungated_dir = dir();
    let listener = LocalListener::bind(
        &ungated_dir.runtime,
        PeerPolicy::current_user().unwrap(),
        CarrierLimits::DEFAULT,
    )
    .expect("bind");
    let (served, first) = thread::scope(|scope| {
        let server = scope.spawn(|| {
            let carrier: Arc<StreamCarrier> = Arc::new(listener.accept().expect("accepted"));
            host.serve(carrier, &mut Admit::default())
        });
        let mut raw = Raw::connect(listener.address());
        let first = raw.read().tag;
        drop(raw);
        (server.join().unwrap(), first)
    });
    assert_ne!(first, Tag::SessionHello.byte(), "the hello was written");
    assert!(
        matches!(served, Err(ServeFault::Setup(_))),
        "{:?}",
        served.as_ref().err()
    );
    assert_eq!(host.script.seen.lock().unwrap().started, 0);
}

/// F5: a session ended by a frame it cannot accept logs why, once: `Protocol` and the host-composed error naming the
/// frame's tag. The client reads the carrier's protocol-fault close.
#[test]
fn a_session_ended_by_a_protocol_error_logs_why() {
    let host = Host::new(SessionPolicy::new(hello()));
    let (session, mut raw) = host.raw_session();
    raw.write(&[Raw::session_open()]);
    let closed = session
        .carrier()
        .wait_closed(super::session_rows::WAIT)
        .expect("the session ends");
    assert_eq!(closed.reason, CloseReason::Local(CloseCode::ProtocolFault));
    let end = host.log.wait_end();
    assert_eq!(end.ending, Ending::Protocol, "{end}");
    assert!(end.detail.contains("tag 4"), "{end}");
    assert!(
        end.to_string().starts_with("session_end ending=protocol"),
        "{end}"
    );
    assert!(session.service().wait_ended(super::session_rows::WAIT));
}
