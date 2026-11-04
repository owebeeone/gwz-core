//! The re-authentication of a connection-bound key before its POST's first byte (HTTPS model draft r5 section 3.1;
//! sub-step H3b, rows H3b-1..5): a POST of a `ConnectionBound` key whose leased connection is not the logon session's
//! sends the key's own advertisement GET there first, answered only by the operation's logon session (RA1); a refusal
//! (RA4) or an unchallenged answer for a key learned `Challenged` (RA2) ends the exchange before any POST byte and
//! rejects the key; an unchallenged answer for a key learned `OnBound` (RA2a) sends the POST as an unrecorded key's.
use super::{
    Bounds, Evidence, Ntlm, Recorded, advertised, bound_by, bound_case, bound_case_with, finish,
    logon_context, need, push, refused, settle,
};
use crate::{
    git::endpoint::{https_adapter::Binding, https_fixture::Behaviour},
    transport_host::connection_host::{
        https_tests::{Case, body, operation_of},
        tests::block_on,
    },
};
use gwz_connection::{GitService, HostPhase, Operation};
use gwz_transport::protocol::{Disposition, Effect};
use std::{sync::atomic::Ordering, time::Duration};

/// The requests of one re-authenticated POST, in order, on one connection: the advertisement GET, its token, the POST.
#[track_caller]
pub(super) fn re_authenticated_then_posted(server: &Ntlm, before: usize, path: &str) -> usize {
    let sent = server.since(before);
    assert_eq!(sent.len(), 3, "the GET, its token and the POST: {sent:?}");
    let on = sent[0].connection;
    assert!(
        sent.iter().all(|seen| seen.connection == on),
        "the re-authentication and the POST were not on one connection: {sent:?}"
    );
    assert!(
        sent[0].get && sent[0].scheme.is_none() && sent[0].path == format!("{path}/info/refs"),
        "the re-authentication did not start with the key's own advertisement GET: {sent:?}"
    );
    assert!(
        sent[1].get && sent[1].token(),
        "no logon-session token: {sent:?}"
    );
    assert!(
        !sent[2].get
            && sent[2].scheme.is_none()
            && sent[2].path == format!("{path}/git-receive-pack"),
        "the POST: {sent:?}"
    );
    on
}
/// A POST of `path` that ends `Authentication` before any POST byte: `Effect::None`, `Discarded`, no POST at the server,
/// the host `Open`. `held` connections stay leased by the row's holds.
#[track_caller]
pub(super) fn refused_before_the_post(
    case: &Case,
    server: &Ntlm,
    operation: &Operation,
    path: &str,
    held: usize,
) {
    let before = server.count();
    let (exchange, read) = push(case, operation, path);
    assert!(refused(&read), "{path}'s POST read {read:?}");
    let end = block_on(exchange.finish());
    assert_eq!(end.disposition, Disposition::Discarded);
    assert!(
        matches!(end.effect, Effect::None),
        "{path}'s POST had an effect"
    );
    assert!(
        server.since(before).iter().all(|seen| seen.get),
        "a POST reached the server"
    );
    case.wait("only the holds are leased", |counts| counts.leased == held);
    assert_eq!(case.handle.snapshot().phase, HostPhase::Open);
}
/// After the key was rejected: its next POST ends `Authentication` before any request, with no GET. `held` connections
/// stay leased by the row's holds.
#[track_caller]
pub(super) fn sends_nothing_more(
    case: &Case,
    server: &Ntlm,
    operation: &Operation,
    path: &str,
    held: usize,
) {
    let before = server.count();
    let (exchange, read) = push(case, operation, path);
    assert!(refused(&read), "the next POST read {read:?}");
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    assert!(
        server.since(before).is_empty(),
        "the next POST of a rejected key sent a request"
    );
    case.wait("only the holds are leased", |counts| counts.leased == held);
}

#[test]
fn h3b_1_a_post_whose_preferred_connection_is_busy_re_authenticates_another() {
    // 3.1: `/private` is learned on c1, which the binding then holds with another exchange (reading, not finishing), so
    // the POST's hint misses and it lands on a new connection: the key's advertisement GET is challenged there and the
    // logon session answers, then the POST is sent once and succeeds. The entry stays `Challenged`, the new connection
    // is the logon session's, and the POST may have had an effect.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-busy", context.clone());
    settle(&case, advertised(&case, &operation, "/private"));
    let held = advertised(&case, &operation, "/hold");
    let c1 = held_connection(&server);
    let before = server.count();
    let (exchange, read) = push(&case, &operation, "/private");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "the POST was refused"
    );
    let end = block_on(exchange.finish());
    assert_eq!(end.disposition, Disposition::Reusable);
    assert!(
        matches!(end.effect, Effect::Possible),
        "the POST had no effect"
    );
    settle(&case, held);
    let on = re_authenticated_then_posted(&server, before, "/private");
    assert_ne!(on, c1, "the POST went on the busy connection");
    assert!(bound_by(
        &need(&case, &context, "/private"),
        Evidence::Challenged
    ));
    let logon = case
        .scopes()
        .iter()
        .filter(|(_, scope)| scope.binding == Binding::LogonSession)
        .count();
    assert_eq!(
        logon, 2,
        "the re-authenticated connection is not the logon session's"
    );
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3b_2_a_post_whose_preferred_connection_is_gone_re_authenticates_another() {
    // 3.1: `/private` is learned on c1, which the server closes once it has been idle for a second; the POST's hint names a
    // connection that no longer exists, so it lands on a new one and re-authenticates it, then succeeds once.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case_with(
        &server,
        &bounds,
        Behaviour {
            idle: Some(Duration::from_millis(1_000)),
            ..Behaviour::default()
        },
    );
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-gone", context.clone());
    settle(&case, advertised(&case, &operation, "/private"));
    let c1 = server.requests()[1].connection;
    case.wait("the idle connection is gone", |counts| counts.total() == 0);
    let before = server.count();
    let (exchange, read) = push(&case, &operation, "/private");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "the POST was refused"
    );
    finish(exchange);
    let on = re_authenticated_then_posted(&server, before, "/private");
    assert_ne!(on, c1);
    assert_eq!(case.accepted(), 2);
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3b_3_a_refused_re_authentication_rejects_the_key_before_any_post_byte() {
    // Consistency P2-1, RA4: the logon session's token is refused during the re-authentication. No POST byte reaches the
    // server, the exchange ends `Authentication` with `Effect::None`, its connection is discarded and the host stays
    // `Open`. The key is `Rejected`: its next POST sends nothing and no GET (P7), and a later challenge to its
    // advertisement is not answered by the logon session (G11).
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-ra4", context.clone());
    settle(&case, advertised(&case, &operation, "/private"));
    let held = advertised(&case, &operation, "/hold");
    server.mode.refuse_tokens.store(true, Ordering::SeqCst);
    refused_before_the_post(&case, &server, &operation, "/private", 1);
    assert!(matches!(
        need(&case, &context, "/private"),
        Recorded::Rejected
    ));
    sends_nothing_more(&case, &server, &operation, "/private", 1);
    // G11: with c1 still held, the advertisement lands on a new connection and is challenged: no token answers it.
    server.mode.refuse_tokens.store(false, Ordering::SeqCst);
    let before = server.count();
    let mut exchange =
        case.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/private");
    assert!(refused(&body(&mut exchange)), "the challenge was answered");
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    let sent = server.since(before);
    assert!(
        sent.len() == 1 && sent[0].scheme.is_none(),
        "a rejected key's challenge was answered: {sent:?}"
    );
    settle(&case, held);
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3b_4_an_unchallenged_re_authentication_of_a_challenged_key_rejects_it() {
    // Safety P3-2(a), RA2 (re-scoped to `Challenged`, RemPlan-2 R2-P2-1): `/private` was learned by answering its
    // challenge, but its re-authentication GET on another connection is served with no challenge: server misbehaviour.
    // The exchange ends as in RA4: no POST byte, `Authentication`, `Effect::None`; the key is `Rejected` and its next
    // POST sends nothing and no GET.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-ra2", context.clone());
    settle(&case, advertised(&case, &operation, "/private"));
    let held = advertised(&case, &operation, "/hold");
    server.mode.open.store(true, Ordering::SeqCst);
    refused_before_the_post(&case, &server, &operation, "/private", 1);
    assert!(matches!(
        need(&case, &context, "/private"),
        Recorded::Rejected
    ));
    sends_nothing_more(&case, &server, &operation, "/private", 1);
    settle(&case, held);
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3b_5_a_public_key_learned_on_a_bound_connection_posts_anonymously_after_an_unchallenged_re_authentication()
 {
    // U7, R2-P2-1 = N-P2-1, R3-P3-1, N3-P3-1: `/priv` (challenged) authenticates c1 and `/pub` (served to anyone) is
    // learned `OnBound` on c1. Its POST, forced onto c2, re-authenticates with one GET that is served unchallenged
    // (RA2a): the POST goes anonymously and succeeds; the entry is unchanged; c2 stays `Anonymous`, the POST does not say
    // its connection is bound, and no hint names c2 (the next `/pub` exchange prefers c1). After a re-authentication that
    // the logon session answers (RA1, `/pub` challenged for a while), the entry is `Challenged`, and a later unchallenged
    // re-authentication is RA2: `Rejected`, zero POST bytes.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-u7", context.clone());
    settle(&case, advertised(&case, &operation, "/priv"));
    settle(&case, advertised(&case, &operation, "/pub"));
    let c1 = server.requests()[1].connection;
    assert!(bound_by(&need(&case, &context, "/pub"), Evidence::OnBound));
    // RA2a.
    let held = advertised(&case, &operation, "/hold-1");
    let before = server.count();
    let (exchange, read) = push(&case, &operation, "/pub");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "RA2a's POST was refused"
    );
    finish(exchange);
    settle(&case, held);
    let sent = server.since(before);
    assert!(
        sent.len() == 2
            && sent
                .iter()
                .all(|seen| seen.scheme.is_none() && seen.connection == sent[0].connection)
            && sent[0].get
            && !sent[1].get,
        "not one unchallenged GET, then an anonymous POST, on one connection: {sent:?}"
    );
    let c2 = sent[0].connection;
    assert_ne!(c2, c1);
    assert!(
        bound_by(&need(&case, &context, "/pub"), Evidence::OnBound),
        "the entry changed"
    );
    assert_eq!(bounds.of(GitService::ReceivePackExchange, "/pub"), [false]);
    let scopes = case.scopes();
    assert_eq!(
        scopes
            .iter()
            .filter(|(_, scope)| scope.binding == Binding::Anonymous)
            .count(),
        1,
        "c2 is not Anonymous: {scopes:?}"
    );
    let before = server.count();
    let (exchange, read) = push(&case, &operation, "/pub");
    assert!(read.is_ok(), "the next /pub exchange");
    settle(&case, exchange);
    let next = server.since(before);
    assert!(
        next.len() == 1 && next[0].connection == c1,
        "the next /pub POST did not prefer c1: {next:?}"
    );
    // RA1 on the `OnBound` key: `/pub` is challenged for a while, and its POST is forced onto a third connection.
    server.mode.protect_pub.store(true, Ordering::SeqCst);
    let holds = [
        advertised(&case, &operation, "/hold-2"),
        advertised(&case, &operation, "/hold-3"),
    ];
    let before = server.count();
    let (exchange, read) = push(&case, &operation, "/pub");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "RA1's POST was refused"
    );
    settle_all(&case, exchange, holds);
    re_authenticated_then_posted(&server, before, "/pub");
    assert!(
        bound_by(&need(&case, &context, "/pub"), Evidence::Challenged),
        "RA1 did not upgrade"
    );
    // RA2: served unchallenged again, on a fourth connection.
    server.mode.protect_pub.store(false, Ordering::SeqCst);
    let holds = [
        advertised(&case, &operation, "/hold-4"),
        advertised(&case, &operation, "/hold-5"),
        advertised(&case, &operation, "/hold-6"),
    ];
    refused_before_the_post(&case, &server, &operation, "/pub", 3);
    for hold in holds {
        finish(hold);
    }
    case.wait("no connection is leased", |counts| counts.leased == 0);
    assert!(matches!(need(&case, &context, "/pub"), Recorded::Rejected));
    block_on(operation.finish());
    assert!(case.close());
}

/// The server's connection number of the last request: the one a just-opened hold was served on.
fn held_connection(server: &Ntlm) -> usize {
    server.requests().last().expect("a request").connection
}
/// Ends `exchange` and every hold, and waits until no connection is leased.
fn settle_all<const N: usize>(
    case: &Case,
    exchange: gwz_connection::Exchange,
    holds: [gwz_connection::Exchange; N],
) {
    finish(exchange);
    for hold in holds {
        finish(hold);
    }
    case.wait("no connection is leased", |counts| counts.leased == 0);
}
