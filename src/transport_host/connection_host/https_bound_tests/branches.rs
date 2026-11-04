//! The branches of a connection-bound key's re-authentication and its POST (HTTPS model draft r5 sections 3(d), 3(e) and
//! 3.1; sub-step H3c, rows H3c-1..6): a challenge only the helper could answer (RA3), a 401 to the POST after a
//! successful re-authentication (P6), a connection that ends during the re-authentication's body or a redirect in
//! answer to it (RA5, RA7), the deadline D (RA5), and the scope record the re-authentication writes (W1, W3).
use super::{
    Bounds, Evidence, Ntlm, Recorded, advertised, bound_by, bound_case, finish, logon_context,
    need, push,
    reauth::{refused_before_the_post, sends_nothing_more},
    refused, settle,
};
use crate::{
    git::endpoint::{
        helper_script, https_adapter::Binding, https_fixture::Behaviour, https_worker::native,
    },
    transport_host::connection_host::{
        https_tests::{Case, Parts, context_of, operation_of, settings},
        tests::block_on,
    },
};
use gwz_connection::{AuthContext, StreamError};
use gwz_transport::protocol::{Disposition, Effect};
use std::{io, path::Path, sync::atomic::Ordering};

/// The context of an operation with a logon session and a helper that counts its runs in `directory`'s `count` file.
fn counted_context(directory: &Path) -> AuthContext {
    let count = helper_script::script_path(&directory.join("count"));
    let helper = helper_script::git_fixture(
        directory,
        &format!(
            "printf 'lookup\\n' >> '{count}'\nprintf 'username=fixture\\npassword=fixture-token\\n\\n'"
        ),
    );
    context_of(Some(helper), Some(native::publication_fixture()))
}
/// How often the helper of `directory` ran.
fn lookups(directory: &Path) -> usize {
    std::fs::read_to_string(directory.join("count")).map_or(0, |text| text.lines().count())
}

#[test]
fn h3c_1_a_re_authentication_challenge_offering_only_basic_is_not_answered() {
    // Safety P3-2(b), RA3: the re-authentication GET's challenge offers Basic only. Neither the helper nor the record
    // answers it (the key is connection-bound, never `Basic`): the exchange ends `Authentication` before any POST byte,
    // the helper never runs, no Basic header is sent, and the key is `Rejected`.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let directory = tempfile::tempdir().unwrap();
    let context = counted_context(directory.path());
    let operation = operation_of(&case.handle, "op-ra3", context.clone());
    settle(&case, advertised(&case, &operation, "/private"));
    let held = advertised(&case, &operation, "/hold");
    server.mode.basic_only.store(true, Ordering::SeqCst);
    let before = server.count();
    refused_before_the_post(&case, &server, &operation, "/private", 1);
    assert!(
        server
            .since(before)
            .iter()
            .all(|seen| seen.scheme.is_none()),
        "the re-authentication's challenge was answered"
    );
    assert_eq!(lookups(directory.path()), 0, "the helper ran");
    assert!(matches!(
        need(&case, &context, "/private"),
        Recorded::Rejected
    ));
    settle(&case, held);
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3c_2_a_401_to_the_post_after_a_re_authentication_rejects_the_key() {
    // Safety P3-2(c), P6, U4: the re-authentication succeeds, then the POST itself is challenged. That POST ends
    // `Authentication` and may have had an effect (its body was on the wire; it is never replayed); the key is
    // `Rejected`, so the next POST sends nothing and no GET.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let context = logon_context();
    let operation = operation_of(&case.handle, "op-p6", context.clone());
    settle(&case, advertised(&case, &operation, "/private"));
    let held = advertised(&case, &operation, "/hold");
    server.mode.refuse_posts.store(true, Ordering::SeqCst);
    let before = server.count();
    let (exchange, read) = push(&case, &operation, "/private");
    assert!(refused(&read), "the POST read {read:?}");
    let end = block_on(exchange.finish());
    assert_eq!(end.disposition, Disposition::Discarded);
    assert!(
        matches!(end.effect, Effect::Possible),
        "the refused POST had no effect"
    );
    let sent = server.since(before);
    assert_eq!(
        sent.iter().filter(|seen| !seen.get).count(),
        1,
        "the POST was not sent once: {sent:?}"
    );
    assert!(
        sent.iter().any(|seen| seen.token()),
        "no re-authentication: {sent:?}"
    );
    case.wait("no connection is leased", |counts| counts.leased == 1);
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
fn h3c_3_a_re_authentication_cut_short_or_redirected_ends_before_the_post() {
    // Consistency P2-1, RA5 and RA7: the re-authentication's 200 ends with its connection before its body does, or its
    // GET is answered with a redirect. Either ends the exchange before any POST byte with `Effect::None`, with no second
    // attempt, and the entry is unchanged.
    for cut in [true, false] {
        let (server, bounds) = (Ntlm::default(), Bounds::default());
        let case = bound_case(&server, &bounds);
        let context = logon_context();
        let operation = operation_of(&case.handle, "op-ra5-ra7", context.clone());
        settle(&case, advertised(&case, &operation, "/private"));
        let held = advertised(&case, &operation, "/hold");
        let flag = if cut {
            &server.mode.truncate
        } else {
            &server.mode.redirect
        };
        flag.store(true, Ordering::SeqCst);
        let before = server.count();
        let (exchange, read) = push(&case, &operation, "/private");
        assert!(read.is_err(), "cut {cut}: the POST read {read:?}");
        let end = block_on(exchange.finish());
        assert_eq!(end.disposition, Disposition::Discarded, "cut {cut}");
        assert!(matches!(end.effect, Effect::None), "cut {cut}: an effect");
        let sent = server.since(before);
        assert!(
            sent.iter().all(|seen| seen.get),
            "cut {cut}: a POST was sent: {sent:?}"
        );
        let expected = if cut { 2 } else { 1 };
        assert_eq!(
            sent.len(),
            expected,
            "cut {cut}: a second attempt: {sent:?}"
        );
        assert!(
            bound_by(&need(&case, &context, "/private"), Evidence::Challenged),
            "cut {cut}: the entry changed"
        );
        case.wait("no connection is leased", |counts| counts.leased == 1);
        settle(&case, held);
        block_on(operation.finish());
        assert!(case.close());
    }
}

#[test]
fn h3c_4_re_authentication_rounds_past_the_deadline_end_in_timeout() {
    // Consistency P3-11, RA5: D is minted for the re-authentication's conversation from `connect_timeout_ms` (2 s here).
    // The logon session's round never completes, so the conversation passes D: the exchange ends `Timeout` with
    // `Effect::None` and no POST byte, and the entry is unchanged.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let (caller, stall) = native::stalling_fixture();
    let handler = server.handler();
    let case = Case::with_parts(
        |_| handler,
        Behaviour::default(),
        |_| (None, None),
        None,
        Parts {
            settings: crate::git::endpoint::https_adapter::HttpsSettings {
                connect_timeout_ms: 2_000,
                ..settings()
            },
            wrap: Some(bounds.wrap()),
        },
    );
    let context = context_of(None, Some(caller));
    let operation = operation_of(&case.handle, "op-deadline", context.clone());
    settle(&case, advertised(&case, &operation, "/private"));
    let held = advertised(&case, &operation, "/hold");
    stall.store(true, Ordering::SeqCst);
    let before = server.count();
    let started = std::time::Instant::now();
    let (exchange, read) = push(&case, &operation, "/private");
    assert!(
        matches!(&read, Err(StreamError::Io(error)) if error.kind() == io::ErrorKind::TimedOut),
        "the POST read {read:?}"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_millis(2_000)
            && elapsed < std::time::Duration::from_secs(10),
        "ended at {elapsed:?}, not at D"
    );
    let end = block_on(exchange.finish());
    assert!(matches!(end.effect, Effect::None));
    assert!(
        server.since(before).iter().all(|seen| seen.get),
        "a POST was sent"
    );
    assert!(bound_by(
        &need(&case, &context, "/private"),
        Evidence::Challenged
    ));
    case.wait("no connection is leased", |counts| counts.leased == 1);
    settle(&case, held);
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h3c_5_a_re_authentication_on_a_connection_another_binding_used_raises_its_scope() {
    // Safety P2-1, W1: B, an operation of equal scope, leaves c2 idle and `Anonymous`. A's POST, its preferred c1
    // held, is given c2 (an anonymous connection serves any binding at once), re-authenticates it and posts. The
    // re-authentication reports `LogonSession`, so c2's scope rises before it idles, and B's next exchange is not given
    // c2 (its lease is `Unprovable`): it goes on a new connection.
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let (a_context, b_context) = (logon_context(), logon_context());
    assert!(a_context.scope() == b_context.scope(), "equal scopes");
    let a = operation_of(&case.handle, "op-a", a_context);
    let b = operation_of(&case.handle, "op-b", b_context);
    settle(&case, advertised(&case, &a, "/private"));
    settle(&case, advertised(&case, &b, "/pub"));
    let c2 = server.requests().last().unwrap().connection;
    let held = advertised(&case, &a, "/hold");
    let before = server.count();
    let (exchange, read) = push(&case, &a, "/private");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "A's POST was refused"
    );
    finish(exchange);
    settle(&case, held);
    let sent = server.since(before);
    assert!(
        sent.iter().all(|seen| seen.connection == c2) && sent.iter().any(|seen| seen.token()),
        "A's POST did not re-authenticate c2: {sent:?}"
    );
    let scopes = case.scopes();
    assert!(
        scopes
            .iter()
            .all(|(_, scope)| scope.binding == Binding::LogonSession),
        "c2's scope did not rise: {scopes:?}"
    );
    let before = server.count();
    settle(&case, advertised(&case, &b, "/pub"));
    let next = server.since(before);
    assert!(
        next.len() == 1 && next[0].connection != c2,
        "B was given the connection A's logon session authenticated: {next:?}"
    );
    block_on(b.finish());
    block_on(a.finish());
    assert!(case.close());
}

#[test]
fn h3c_6_a_basic_post_on_a_logon_session_connection_leaves_it_the_logon_sessions() {
    // Safety P2-1, W3: `/basic` is learned `Basic` on c2 while c1, which the logon session authenticated, is held. Its
    // POST, with no hint, goes on the lowest idle connection, c1, carrying the record's credential: c1's scope stays
    // `LogonSession` (a helper token does not lower it).
    let (server, bounds) = (Ntlm::default(), Bounds::default());
    let case = bound_case(&server, &bounds);
    let directory = tempfile::tempdir().unwrap();
    let operation = operation_of(&case.handle, "op-w3", counted_context(directory.path()));
    settle(&case, advertised(&case, &operation, "/private"));
    let c1 = server.requests()[1].connection;
    let held = advertised(&case, &operation, "/hold");
    finish(advertised(&case, &operation, "/basic"));
    settle(&case, held);
    let before = server.count();
    let (exchange, read) = push(&case, &operation, "/basic");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "the POST was refused"
    );
    settle(&case, exchange);
    let sent = server.since(before);
    assert!(
        sent.len() == 1 && sent[0].connection == c1 && sent[0].scheme.as_deref() == Some("Basic"),
        "the Basic POST did not go on c1: {sent:?}"
    );
    let logon = case
        .scopes()
        .iter()
        .filter(|(_, scope)| scope.binding == Binding::LogonSession)
        .count();
    assert_eq!(logon, 1, "c1's scope was lowered");
    assert_eq!(lookups(directory.path()), 1);
    block_on(operation.finish());
    assert!(case.close());
}
