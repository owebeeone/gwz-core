//! Real Windows SSPI through the connection host's HTTPS adapter (plan step 1.3c; contract 7.2 and 7.3): the product's
//! SSPI worker process and the logon session of the test, against the loopback SSPI acceptor fixture of plan step 4.8
//! (`native::tests::real_sspi`), which checks the connection's channel binding as an Extended Protection server does.
//! A Windows-only module, not a skip. No account is created and no password is used.
use super::{
    https_tests::{ADVERTISEMENT, Case, Unavailable, body, context_of, operation_of},
    tests::block_on,
};
use crate::git::endpoint::{
    https_adapter::Binding,
    https_fixture::{Behaviour, ConnectionId as Served, Handler, response},
    https_worker::native::tests::real_sspi::{
        Expects, acceptor_handler, real_caller, settle_worker,
    },
};
use bytes::Bytes;
use gwz_connection::GitService;
use gwz_transport::protocol::{self, Disposition};
use hyper::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Duration,
};

fn settle(supervisor: Arc<gwz_sspi::Supervisor>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(settle_worker(supervisor));
}

#[test]
fn the_logon_session_authenticates_through_the_host_and_its_connection_is_per_exchange_scoped() {
    let (caller, supervisor) = real_caller();
    for scheme in ["NTLM", "Negotiate"] {
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let (accepted, native) = (statuses.clone(), caller.clone());
        let case = Case::with(
            move |_| acceptor_handler(scheme, scheme, Expects::Right, accepted),
            Behaviour::default(),
            move |_| (None, Some(native)),
            None,
        );
        let a = case.operation("op-a");
        let mut exchange = case.exchange(&a, false);
        body(&mut exchange).unwrap_or_else(|error| panic!("{scheme}: {error:?}"));
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Reusable,
            "{scheme}"
        );
        block_on(a.finish());
        // The enforcing server accepted the client's binding: its last status is success.
        assert_eq!(statuses.lock().unwrap().last(), Some(&0), "{scheme}");
        let scopes = case.scopes();
        assert_eq!(scopes.len(), 1, "{scheme}");
        assert_eq!(scopes[0].1.binding, Binding::LogonSession, "{scheme}");
        assert!(scopes[0].1.binding.per_exchange(), "{scheme}");
        assert!(case.close(), "{scheme}");
    }
    settle(supervisor);
}

#[test]
fn the_logon_session_route_refuses_on_windows_while_the_process_gate_is_unavailable() {
    let (caller, supervisor) = real_caller();
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let accepted = statuses.clone();
    let case = Case::with(
        move |_| acceptor_handler("NTLM", "NTLM", Expects::Right, accepted),
        Behaviour::default(),
        move |_| (None, Some(caller)),
        Some(Arc::new(Unavailable)),
    );
    let a = case.operation("op-a");
    let mut exchange = case.exchange(&a, false);
    assert!(body(&mut exchange).is_err(), "the native route refuses");
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    block_on(a.finish());
    assert!(
        statuses.lock().unwrap().is_empty(),
        "the acceptor never saw a token"
    );
    assert!(case.close());
    settle(supervisor);
}

#[test]
fn the_logon_session_is_each_operations_own_through_the_host_on_windows() {
    // R3 (Code P2-2): one instance, two operations with equal profiles. A's context holds the real caller its entry
    // captured, and B's holds none: A's logon session authenticates, B's challenge goes unanswered, and the enforcing
    // server never sees a second token.
    let (caller, supervisor) = real_caller();
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let accepted = statuses.clone();
    let case = Case::with(
        move |_| acceptor_handler("NTLM", "NTLM", Expects::Right, accepted),
        Behaviour::default(),
        |_| (None, None),
        None,
    );
    let a = operation_of(&case.handle, "op-a", context_of(None, Some(caller)));
    let mut exchange = case.exchange(&a, false);
    body(&mut exchange).unwrap();
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(a.finish());
    let tokens = statuses.lock().unwrap().len();
    assert_eq!(statuses.lock().unwrap().last(), Some(&0), "A authenticated");
    let b = operation_of(&case.handle, "op-b", context_of(None, None));
    let mut exchange = case.exchange(&b, false);
    assert!(body(&mut exchange).is_err(), "B had no caller to answer with");
    block_on(exchange.finish());
    block_on(b.finish());
    assert_eq!(
        statuses.lock().unwrap().len(),
        tokens,
        "the acceptor saw a token from B"
    );
    assert!(case.close());
    settle(supervisor);
}

/// The acceptor fixture as an NTLM server is: it remembers each connection whose token it accepted and serves any later
/// request on it with no token, a push's POST included; every other request is the acceptor's to answer. Its answers
/// carry the push service's content type and the advertisement. `seen` records (GET, carried a token, connection).
fn remembering(statuses: Arc<Mutex<Vec<i32>>>, seen: Arc<Mutex<Vec<(bool, bool, usize)>>>) -> Handler {
    let acceptor = acceptor_handler("NTLM", "NTLM", Expects::Right, statuses);
    let authenticated = Arc::new(Mutex::new(BTreeSet::new()));
    Arc::new(move |request| {
        let connection = request.extensions().get::<Served>().map_or(0, |id| id.0);
        let get = request.method() == hyper::Method::GET;
        let service = if get {
            protocol::GitService::ReceivePackAdvertisement
        } else {
            protocol::GitService::ReceivePackExchange
        };
        let token = request.headers().contains_key(AUTHORIZATION);
        seen.lock().unwrap().push((get, token, connection));
        let served = if get {
            Bytes::from_static(ADVERTISEMENT)
        } else {
            Bytes::from_static(b"0000")
        };
        if !token && authenticated.lock().unwrap().contains(&connection) {
            let body = request.into_body();
            return Box::pin(async move {
                let _ = http_body_util::BodyExt::collect(body).await;
                response(200, service, served)
            });
        }
        let (acceptor, authenticated) = (acceptor.clone(), authenticated.clone());
        Box::pin(async move {
            let answer = acceptor(request).await;
            if answer.status() != 200 {
                return answer;
            }
            authenticated.lock().unwrap().insert(connection);
            let mut accepted = response(200, service, served);
            if let Some(last) = answer.headers().get(WWW_AUTHENTICATE) {
                accepted.headers_mut().insert(WWW_AUTHENTICATE, last.clone());
            }
            accepted
        })
    })
}

#[test]
fn h3d_5_a_logon_session_push_through_the_host_on_a_hint_hit_and_after_its_connection_is_gone() {
    // HTTPS model draft r5 rows 1 and 3, row H3d-5: real SSPI, the enforcing acceptor fixture. The push advertisement
    // authenticates c1 with the logon session and learns the key connection-bound. Hint hit: the POST goes on c1 with no
    // token and no second conversation. Connection gone: the server closes the idle c1, so the POST lands on c2, which
    // its re-authentication authenticates (a second conversation the acceptor accepts) before the POST goes, once.
    let (caller, supervisor) = real_caller();
    for gone in [false, true] {
        let (statuses, seen) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
        let (accepted, recorded) = (statuses.clone(), seen.clone());
        let behaviour = Behaviour {
            idle: gone.then_some(Duration::from_secs(1)),
            ..Behaviour::default()
        };
        let case = Case::with(
            move |_| remembering(accepted, recorded),
            behaviour,
            |_| (None, None),
            None,
        );
        let operation = operation_of(&case.handle, "op-push", context_of(None, Some(caller.clone())));
        let mut advertisement = case.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/repo");
        assert_eq!(body(&mut advertisement).unwrap(), ADVERTISEMENT, "gone {gone}");
        assert_eq!(block_on(advertisement.finish()).disposition, Disposition::Reusable);
        let c1 = seen.lock().unwrap().last().unwrap().2;
        let conversation = statuses.lock().unwrap().len();
        assert_eq!(statuses.lock().unwrap().last(), Some(&0), "gone {gone}: the acceptor accepted");
        if gone {
            case.wait("the idle connection is gone", |counts| counts.total() == 0);
        } else {
            case.wait("the connection is idle", |counts| counts.leased == 0);
        }
        let before = seen.lock().unwrap().len();
        let mut push = case.exchange_at(&operation, GitService::ReceivePackExchange, "/repo");
        let _ = block_on(push.stream().write_all(b"0000"));
        assert_eq!(body(&mut push).unwrap(), b"0000", "gone {gone}: the push");
        assert_eq!(block_on(push.finish()).disposition, Disposition::Reusable);
        let sent = seen.lock().unwrap()[before..].to_vec();
        let post = sent.iter().filter(|(get, _, _)| !*get).collect::<Vec<_>>();
        assert_eq!(post.len(), 1, "gone {gone}: the POST was not sent once: {sent:?}");
        assert!(!post[0].1, "gone {gone}: the POST carried a token");
        if gone {
            assert_ne!(post[0].2, c1, "the POST went on a closed connection");
            assert!(
                statuses.lock().unwrap().len() > conversation
                    && statuses.lock().unwrap().last() == Some(&0),
                "the re-authentication's conversation was not accepted"
            );
            assert!(sent.iter().all(|(_, _, on)| *on == post[0].2), "{sent:?}");
        } else {
            assert_eq!(post[0].2, c1, "the hint missed");
            assert_eq!(sent.len(), 1, "a re-authentication on a hint hit: {sent:?}");
            assert_eq!(statuses.lock().unwrap().len(), conversation);
        }
        assert!(
            case.scopes().iter().all(|(_, scope)| scope.binding == Binding::LogonSession),
            "gone {gone}"
        );
        block_on(operation.finish());
        assert!(case.close(), "gone {gone}");
    }
    settle(supervisor);
}
