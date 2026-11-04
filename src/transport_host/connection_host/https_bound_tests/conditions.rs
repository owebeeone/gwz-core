//! The conditions of a re-authentication (HTTPS model draft r5 section 3.1, "Conditions of the re-authentication" and
//! "Counter"; sub-step H3d, rows H3d-1..4; row H3d-5, real SSPI, is Windows-only in `https_sspi_tests.rs`): a key
//! learned `OnBound` re-authenticates when its challenge comes back; the GET repeats the original advertisement's
//! request line and headers; its body is drained under max(8 MiB, 1.25 x the advertisement the key's entry observed);
//! and every re-authentication is counted once, in the one counter the factory gives all its adapters.
use super::{
    Bounds, Evidence, Ntlm, Recorded, advertised, bound_by, bound_case, finish, logon_context,
    need, push, reauth::re_authenticated_then_posted, settle,
};
use crate::transport_host::connection_host::{
    https_tests::{ADVERTISEMENT, Case, operation_of},
    tests::block_on,
};
use bytes::Bytes;
use gwz_connection::{
    AuthContext, EndpointProfile, EndpointRoute, Operation, OperationPolicy, PreparedOperation,
    StreamError,
};
use gwz_transport::protocol::{Disposition, Effect};
use std::{io, sync::atomic::Ordering};

/// Posts `path` with the operation's idle connections held by its own exchanges, so that the POST lands on a new
/// connection and re-authenticates it: it succeeds. Returns the server's number of that connection.
fn forced_post(
    case: &Case,
    server: &Ntlm,
    operation: &Operation,
    path: &str,
    holds: &[&str],
) -> usize {
    let held: Vec<_> = holds
        .iter()
        .map(|hold| advertised(case, operation, hold))
        .collect();
    let before = server.count();
    let (exchange, read) = push(case, operation, path);
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "{path}'s POST was refused"
    );
    finish(exchange);
    for hold in held {
        finish(hold);
    }
    case.wait("no connection is leased", |counts| counts.leased == 0);
    re_authenticated_then_posted(server, before, path)
}

#[test]
fn h3d_1_a_key_learned_on_bound_re_authenticates_when_challenged_and_posts_once() {
    // Safety P2-2: `/a` authenticates c1 and `/b`, which needs authentication, is then served on c1 with no challenge:
    // it is learned `OnBound`. Its POST, forced onto c2 with c1 held, re-authenticates there (the GET is challenged and
    // the logon session answers it), then succeeds once; the entry becomes `Challenged` (RA1).
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-on-bound", context.clone());
    settle(&case, advertised(&case, &operation, "/a"));
    settle(&case, advertised(&case, &operation, "/b"));
    assert!(bound_by(&need(&case, &context, "/b"), Evidence::OnBound));
    let c1 = server.requests()[1].connection;
    let on = forced_post(&case, &server, &operation, "/b", &["/hold"]);
    assert_ne!(on, c1);
    let posts = server.requests().iter().filter(|seen| !seen.get).count();
    assert_eq!(posts, 1, "the POST was not sent once");
    assert!(bound_by(&need(&case, &context, "/b"), Evidence::Challenged));
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3d_2_the_re_authentication_repeats_the_original_advertisement_request() {
    // Safety P3-6, condition 1: the re-authentication's GET, and its token's, have the request line and headers of the
    // original advertisement (Authorization aside): one `request()` builds them all.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let operation = operation_of(&case.handle, "op-same-request", logon_context());
    settle(&case, advertised(&case, &operation, "/private"));
    let original = server.requests()[0].head.clone();
    assert!(original[0].starts_with("GET /private/info/refs?service=git-receive-pack"));
    forced_post(&case, &server, &operation, "/private", &["/hold"]);
    // The re-authentication's GET and its token's: the two requests before the POST.
    let all = server.requests();
    for seen in &all[all.len() - 3..all.len() - 1] {
        assert_eq!(
            seen.head, original,
            "the re-authentication's request differs"
        );
    }
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3d_3_an_advertisement_over_8_mib_is_drained_and_the_post_succeeds() {
    // Safety P3-6, condition 4: `/big`'s advertisement is 9 MiB, past the 8 MiB floor; the entry observed its length, so
    // its re-authentication drains it under 1.25 x 9 MiB and the POST succeeds after a hint miss.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let big = Bytes::from(vec![b'0'; 9 << 20]);
    *server.big.lock().unwrap() = big.clone();
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-big", context.clone());
    let mut advertisement = case.exchange_at(
        &operation,
        gwz_connection::GitService::ReceivePackAdvertisement,
        "/big",
    );
    assert_eq!(super::body(&mut advertisement).unwrap().len(), big.len());
    settle(&case, advertisement);
    assert!(matches!(
        need(&case, &context, "/big"),
        super::Recorded::ConnectionBound { observed, .. } if observed == big.len() as u64
    ));
    forced_post(&case, &server, &operation, "/big", &["/hold"]);
    block_on(operation.finish());
    assert!(case.close());
}

/// Posts `path`, whose re-authentication's 200 body is past its cap, with the operation's idle connection held so that
/// the POST lands on a new one: the exchange ends `Protocol` (RA6) before any POST byte, `Effect::None`, `Discarded`,
/// after `gets` requests, all GETs.
#[track_caller]
fn past_the_cap(case: &Case, server: &Ntlm, operation: &Operation, path: &str, gets: usize) {
    let held = advertised(case, operation, "/hold");
    let before = server.count();
    let (exchange, read) = push(case, operation, path);
    assert!(
        matches!(&read, Err(StreamError::Io(error)) if error.kind() == io::ErrorKind::InvalidData),
        "{path}'s POST read {read:?}"
    );
    let end = block_on(exchange.finish());
    assert_eq!(end.disposition, Disposition::Discarded);
    assert!(matches!(end.effect, Effect::None), "an effect");
    let sent = server.since(before);
    assert!(
        sent.iter().all(|seen| seen.get) && sent.len() == gets,
        "not {gets} GETs and no POST: {sent:?}"
    );
    case.wait("only the hold is leased", |counts| counts.leased == 1);
    settle(case, held);
}
/// Whether `recorded` is still the entry the 34-byte advertisement of `/big` left, learned with `evidence`.
fn unchanged(recorded: &Recorded, evidence: Evidence) -> bool {
    matches!(recorded, Recorded::ConnectionBound { observed, evidence: learned }
        if *observed == ADVERTISEMENT.len() as u64 && *learned == evidence)
}

#[test]
fn h3d_3b_a_re_authentication_answered_by_the_logon_session_past_its_cap_ends_before_the_post() {
    // RA6 after RA1 (R4w3 Code P3-3 (b); draft 3.1 condition 4): `/big` was learned `Challenged` from a 34-byte
    // advertisement, which now has grown to 9 MiB, past max(8 MiB, 1.25 x 34). The re-authentication's GET is challenged
    // and the logon session answers it; its 200 body passes the cap: `Protocol`, no POST byte, the entry unchanged.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    *server.big.lock().unwrap() = Bytes::from_static(ADVERTISEMENT);
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-ra6-ra1", context.clone());
    settle(&case, advertised(&case, &operation, "/big"));
    assert!(unchanged(
        &need(&case, &context, "/big"),
        Evidence::Challenged
    ));
    *server.big.lock().unwrap() = Bytes::from(vec![b'0'; 9 << 20]);
    past_the_cap(&case, &server, &operation, "/big", 2);
    assert!(
        unchanged(&need(&case, &context, "/big"), Evidence::Challenged),
        "the entry changed"
    );
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3d_3c_an_unchallenged_re_authentication_past_its_cap_ends_before_the_post() {
    // RA6 after RA2a (R4w3 Code P3-3 (a)): `/big` was learned `OnBound` from a 34-byte advertisement served on the
    // connection `/a` authenticated. The server then serves everything to anyone, and `/big` has grown to 9 MiB: the
    // re-authentication's GET is served unchallenged and its body is drained under the cap before any POST, so it ends
    // `Protocol` with no POST byte (a body left undrained would let the POST go), and the entry is unchanged.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    *server.big.lock().unwrap() = Bytes::from_static(ADVERTISEMENT);
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-ra6-ra2a", context.clone());
    settle(&case, advertised(&case, &operation, "/a"));
    settle(&case, advertised(&case, &operation, "/big"));
    assert!(unchanged(&need(&case, &context, "/big"), Evidence::OnBound));
    server.mode.open.store(true, Ordering::SeqCst);
    *server.big.lock().unwrap() = Bytes::from(vec![b'0'; 9 << 20]);
    past_the_cap(&case, &server, &operation, "/big", 1);
    assert!(
        unchanged(&need(&case, &context, "/big"), Evidence::OnBound),
        "the entry changed"
    );
    block_on(operation.finish());
    assert!(case.close());
}

/// An operation over `auth` under a profile of its own, so that its exchanges run on another endpoint instance (and so
/// another adapter) of the same host and factory.
fn other_instance(case: &Case, name: &str, auth: AuthContext) -> Operation {
    let input = PreparedOperation::new(
        name,
        EndpointRoute::ssh_and_https(),
        EndpointProfile::new(b"fixture-https-other".to_vec(), [8; 32]).unwrap(),
        auth,
        OperationPolicy::new(4, 32, 3).unwrap(),
    )
    .unwrap();
    block_on(case.handle.admit(input).unwrap().acquire())
        .unwrap()
        .register()
}

#[test]
fn h3d_4_re_authentications_are_counted_once_each_in_the_one_shared_counter() {
    // Consistency P3-8, R3-P3-4: two re-authentications on one instance's adapter and one on another instance's adapter
    // of the same factory: each adapter's counters read three, the one count they share.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let first = operation_of(&case.handle, "op-count-1", logon_context());
    settle(&case, advertised(&case, &first, "/a"));
    forced_post(&case, &server, &first, "/a", &["/hold-1"]);
    forced_post(&case, &server, &first, "/a", &["/hold-1", "/hold-2"]);
    let second = other_instance(&case, "op-count-2", logon_context());
    settle(&case, advertised(&case, &second, "/a"));
    forced_post(&case, &server, &second, "/a", &["/hold-1"]);
    let counters = case.counters();
    assert_eq!(counters.len(), 2, "two adapters");
    for counter in counters {
        assert_eq!(counter.reauthentications(), 3);
    }
    block_on(second.finish());
    block_on(first.finish());
    assert!(case.close());
}
