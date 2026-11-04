//! The operation's credential record at its bounds, driving the connection host's HTTPS adapter (HTTPS model draft r5
//! sub-step H1d, rows H1d-1..5; sections 3(b) bounds and 3(d) G14, P8, X1-X4). A row brings the record near one bound
//! with fillers at an origin of their own (`OperationAuth::crowd`, the draft's small test bounds), then reaches it with
//! real exchanges. An exchange the record cannot serve ends with the budget code, `Capacity`, which the client's stream
//! reads as `QuotaExceeded` with a message that names the record and no pool limit. Credentials are compared by the
//! digest of their header; none is printed.
use super::{
    ADVERTISEMENT, Case, Refused, Requests, advertised, basic_digest, block_on, body, case,
    context_of, counting_helper, counts, discarded, lookups, operation_of, posted, push, refused,
    refusing, routing_helper, settle, since,
};
use crate::git::endpoint::https_auth::{
    Bound, OperationAuth,
    needs::{CREDENTIALS, FINGERPRINTS, KEYS, RETAINED_BYTES},
};
use bytes::Bytes;
use gwz_connection::{AuthContext, GitService, HostPhase, Operation, StreamError};
use gwz_transport::protocol::{Disposition, Effect};
use std::io;

/// What the record's budget code reads as on the client's stream (draft r5 3(b), A9).
const BUDGET: &str = "operation credential record budget exceeded";

/// Whether `read` failed with the record's budget code: `QuotaExceeded`, its message the record's.
fn over_budget<T>(read: &Result<T, StreamError>) -> bool {
    matches!(read, Err(StreamError::Io(error))
        if error.kind() == io::ErrorKind::QuotaExceeded && error.to_string() == BUDGET)
}
/// The record of the operation whose context is `context`.
fn record_of(context: &AuthContext) -> &OperationAuth {
    OperationAuth::of(context).expect("core's own provider")
}
/// Opens a push advertisement of `path`, whose first read fails with the budget code: no byte of its body reached the
/// client, so no POST follows it.
#[track_caller]
fn advertisement_over_budget(case: &Case, operation: &Operation, path: &str) {
    let mut exchange = case.exchange_at(operation, GitService::ReceivePackAdvertisement, path);
    let mut first = [0u8; 1];
    let read = block_on(exchange.stream().read(&mut first));
    assert!(over_budget(&read), "{path}'s advertisement read {read:?}");
    discarded(exchange);
}
/// A push's POST to `path` that ends with the budget code before its request: `Effect::None`, `Discarded`, and nothing
/// at the server.
#[track_caller]
fn post_over_budget(case: &Case, operation: &Operation, requests: &Requests, path: &str) {
    let before = requests.lock().unwrap().len();
    let (exchange, read) = push(case, operation, path);
    assert!(over_budget(&read), "{path}'s POST read {read:?}");
    let end = block_on(exchange.finish());
    assert_eq!(end.disposition, Disposition::Discarded);
    assert!(
        matches!(end.effect, Effect::None),
        "{path}'s POST had an effect"
    );
    assert!(
        since(requests, before).is_empty(),
        "{path}'s POST reached the server"
    );
}

#[test]
fn h1d_1_a_new_key_past_the_key_table_ends_its_advertisement_with_the_budget_code() {
    // G14, Safety P2-4, N-P3-6: with the key table one entry from full, `/a` is learned (the 512th key). `/b`'s challenge
    // is answered and served, but its key cannot be stored: the advertisement ends with the budget code before any body
    // byte, so no POST follows it, and nothing is stored. The earlier key still works.
    let requests = Requests::default();
    let case = case(&requests, Bytes::new());
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let context = context_of(Some(helper), None);
    record_of(&context).crowd(Bound::Keys, 1);
    let operation = operation_of(&case.handle, "op-keys-full", context.clone());
    settle(&case, advertised(&case, &operation, "/a"));
    assert_eq!(counts(&context).keys, KEYS);
    let before = requests.lock().unwrap().len();
    advertisement_over_budget(&case, &operation, "/b");
    let sent = since(&requests, before);
    assert_eq!(sent.len(), 2, "the GET and its answer");
    assert!(sent.iter().all(|s| !s.post), "a POST followed");
    let after = counts(&context);
    assert_eq!((after.keys, after.credentials), (KEYS, 1), "/b was stored");
    let (exchange, read) = push(&case, &operation, "/a");
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "the earlier key's POST"
    );
    settle(&case, exchange);
    assert_eq!(lookups(directory.path()), 2);
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h1d_2_a_credential_past_the_count_or_byte_bound_leaves_its_key_unretained() {
    // Consistency P2-3, G14: the record one credential from its count bound, then one credential's length from its byte
    // bound. `/a`'s credential T fills it; `/other`'s, U, is past the bound: its advertisement ends with the budget code
    // before a body byte, and its key is stored `Unretained` with no credential, so its POST sends nothing (P8). `/a`'s
    // POST still carries T.
    let t_bytes = "fixture".len() + "fixture-token-a".len();
    for (bound, room) in [(Bound::Credentials, 1), (Bound::Bytes, t_bytes)] {
        let requests = Requests::default();
        let case = case(&requests, Bytes::new());
        let directory = tempfile::tempdir().unwrap();
        let helper = routing_helper(directory.path(), "fixture-token-a", "fixture-token-u");
        let context = context_of(Some(helper), None);
        record_of(&context).crowd(bound, room);
        let operation = operation_of(&case.handle, "op-unretained", context.clone());
        settle(&case, advertised(&case, &operation, "/a"));
        let full = counts(&context);
        match bound {
            Bound::Credentials => assert_eq!(full.credentials, CREDENTIALS),
            _ => assert_eq!(full.retained, RETAINED_BYTES),
        }
        advertisement_over_budget(&case, &operation, "/other");
        let after = counts(&context);
        assert_eq!(
            (after.keys, after.credentials, after.retained),
            (full.keys + 1, full.credentials, full.retained),
            "{bound:?}: /other is not stored Unretained, with no credential"
        );
        post_over_budget(&case, &operation, &requests, "/other");
        let (exchange, read) = push(&case, &operation, "/a");
        assert_eq!(
            read.ok().as_deref(),
            Some(&b"0000"[..]),
            "{bound:?}: /a's POST"
        );
        settle(&case, exchange);
        let last = requests.lock().unwrap().last().cloned().unwrap();
        assert!(last.post && last.digest == Some(basic_digest("fixture-token-a")));
        assert_eq!(lookups(directory.path()), 2, "{bound:?}");
        block_on(operation.finish());
        assert!(case.close());
    }
}

#[test]
fn h1d_3_a_five_kib_credential_is_retained_and_carried_by_its_push() {
    // Safety P2-4: the bounds keep a large real credential. A 5 KiB token, whose header is about 6.8 KiB, is learned, and
    // the push's POST carries it from its first byte and succeeds.
    let token = "t".repeat(5 * 1024);
    let header = "Basic ".len() + ("fixture:".len() + token.len()).div_ceil(3) * 4;
    assert!(
        (6_800..7_000).contains(&header),
        "a header of {header} bytes"
    );
    let requests = Requests::default();
    let case = case(&requests, Bytes::new());
    let directory = tempfile::tempdir().unwrap();
    let context = context_of(Some(counting_helper(directory.path(), &token)), None);
    let operation = operation_of(&case.handle, "op-large", context.clone());
    settle(&case, advertised(&case, &operation, "/repo"));
    let learned = counts(&context);
    assert_eq!(
        (learned.keys, learned.credentials, learned.retained),
        (1, 1, "fixture".len() + token.len())
    );
    let (exchange, read) = push(&case, &operation, "/repo");
    assert_eq!(read.ok().as_deref(), Some(&b"0000"[..]), "the push's POST");
    settle(&case, exchange);
    let seen = requests.lock().unwrap().clone();
    let posts: Vec<_> = seen.iter().filter(|s| s.post).collect();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0].digest == Some(basic_digest(&token)),
        "the POST went without the credential"
    );
    assert_eq!(lookups(directory.path()), 1);
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h1d_4_an_unretained_keys_post_sends_no_request() {
    // P8: the record at its count bound, so `/a`'s advertisement stores its key `Unretained`. The key's POST ends with the
    // budget code before any byte: `Effect::None`, `Discarded`, no request at the server, no helper run; the host stays
    // `Open`.
    let requests = Requests::default();
    let case = case(&requests, Bytes::new());
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let context = context_of(Some(helper), None);
    record_of(&context).crowd(Bound::Credentials, 0);
    let operation = operation_of(&case.handle, "op-p8", context.clone());
    advertisement_over_budget(&case, &operation, "/a");
    assert_eq!(counts(&context).keys, CREDENTIALS + 1);
    post_over_budget(&case, &operation, &requests, "/a");
    assert_eq!(lookups(directory.path()), 1, "a helper ran for the POST");
    block_on(operation.finish());
    assert_eq!(case.handle.snapshot().phase, HostPhase::Open);
    assert!(case.close());
}

#[test]
fn h1d_5_the_65th_rejected_fingerprint_exhausts_the_record() {
    // X1-X4, P3: `/a` is learned with T. With 64 fingerprints kept, U answers `/other`'s challenge and is refused: the
    // 65th fingerprint is not stored, the record becomes `Exhausted` (credentials wiped, entries tombstones) and that
    // exchange ends `Authentication` (X1). From then on no helper runs and nothing is answered from the record: the
    // challenged advertisement of `/a` (a tombstone) and of a key never learned each end with the budget code after one
    // anonymous GET, and write no entry (X2, X4); `/a`'s POST sends no request (X3); a POST of a key never learned goes
    // out anonymously (P3).
    let (requests, refused_set) = (Requests::default(), Refused::default());
    refused_set
        .lock()
        .unwrap()
        .push(basic_digest("fixture-token-u"));
    let case = refusing(&requests, Bytes::new(), &refused_set);
    let directory = tempfile::tempdir().unwrap();
    let helper = routing_helper(directory.path(), "fixture-token-a", "fixture-token-u");
    let context = context_of(Some(helper), None);
    let operation = operation_of(&case.handle, "op-x1", context.clone());
    settle(&case, advertised(&case, &operation, "/a"));
    record_of(&context).crowd(Bound::Fingerprints, 0);
    assert_eq!(counts(&context).fingerprints, FINGERPRINTS);
    let mut exchange = case.exchange_at(&operation, GitService::ReceivePackAdvertisement, "/other");
    assert!(
        refused(&body(&mut exchange)),
        "the 401 that produced the bound"
    );
    discarded(exchange);
    let exhausted = counts(&context);
    assert!(exhausted.exhausted, "the record is not Exhausted");
    assert_eq!(
        (exhausted.credentials, exhausted.fingerprints),
        (0, FINGERPRINTS)
    );
    assert_eq!(lookups(directory.path()), 2);
    for path in ["/a", "/new"] {
        let before = requests.lock().unwrap().len();
        advertisement_over_budget(&case, &operation, path);
        let sent = since(&requests, before);
        assert!(
            sent.len() == 1 && sent[0].digest.is_none(),
            "{path}: the challenge was answered"
        );
    }
    assert_eq!(
        lookups(directory.path()),
        2,
        "a helper ran for an Exhausted record"
    );
    assert_eq!(
        counts(&context).keys,
        exhausted.keys,
        "an entry was written"
    );
    post_over_budget(&case, &operation, &requests, "/a");
    let before = requests.lock().unwrap().len();
    let (exchange, read) = push(&case, &operation, "/public");
    assert_eq!(read.ok().as_deref(), Some(&b"0000"[..]), "P3's POST");
    settle(&case, exchange);
    let sent = since(&requests, before);
    assert!(sent.len() == 1 && sent[0].post && sent[0].digest.is_none());
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h1d_5_a_poisoned_lock_exhausts_the_record() {
    // X1 for a poisoned lock: `/a` is learned with T, then the record's lock is poisoned. The next exchange to consult the
    // record, `/b`'s challenged advertisement, ends with the budget code after one anonymous GET and runs no helper; the
    // record is `Exhausted`, T wiped, and `/a`'s POST sends no request (X3).
    let requests = Requests::default();
    let case = case(&requests, Bytes::new());
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let context = context_of(Some(helper), None);
    let operation = operation_of(&case.handle, "op-poisoned", context.clone());
    settle(&case, advertised(&case, &operation, "/a"));
    record_of(&context).poison();
    let before = requests.lock().unwrap().len();
    advertisement_over_budget(&case, &operation, "/b");
    let sent = since(&requests, before);
    assert!(
        sent.len() == 1 && sent[0].digest.is_none(),
        "/b was answered"
    );
    assert_eq!(lookups(directory.path()), 1, "a helper ran");
    let exhausted = counts(&context);
    assert!(exhausted.exhausted && exhausted.credentials == 0);
    post_over_budget(&case, &operation, &requests, "/a");
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn h1d_5_a_record_exhausted_from_creation_keeps_public_traffic_anonymous() {
    // X1 for failing randomness, P3: the record is `Exhausted` from its creation. A public fetch's advertisement and its
    // POST succeed, anonymously; a challenged advertisement ends with the budget code after one anonymous GET, runs no
    // helper (X2) and records nothing.
    let requests = Requests::default();
    let case = case(&requests, Bytes::new());
    let directory = tempfile::tempdir().unwrap();
    let helper = counting_helper(directory.path(), "fixture-token-a");
    let context = OperationAuth::context_without_randomness(helper, None);
    assert!(counts(&context).exhausted);
    let operation = operation_of(&case.handle, "op-no-randomness", context.clone());
    let mut exchange = case.exchange_at(&operation, GitService::UploadPackAdvertisement, "/public");
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    settle(&case, exchange);
    let (exchange, read) =
        posted(case.exchange_at(&operation, GitService::UploadPackExchange, "/public"));
    assert_eq!(
        read.ok().as_deref(),
        Some(&b"0000"[..]),
        "the public fetch's POST"
    );
    settle(&case, exchange);
    assert!(
        requests.lock().unwrap().iter().all(|s| s.digest.is_none()),
        "a public request carried a credential"
    );
    let before = requests.lock().unwrap().len();
    advertisement_over_budget(&case, &operation, "/private");
    assert_eq!(since(&requests, before).len(), 1, "/private was answered");
    assert_eq!(lookups(directory.path()), 0, "a helper ran");
    assert_eq!(counts(&context).keys, 0);
    block_on(operation.finish());
    assert!(case.close());
}
