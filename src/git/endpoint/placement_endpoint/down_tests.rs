//! The SSH endpoint over a Down key (adaptive concurrency design §5.5, cases
//! 29 and 44): the members queued when the budget is spent finish with its
//! failure, a later member parks for the next retest, and after two retests in
//! a row have failed an arrival finishes at once.
use super::{
    retry_tests::{OPERATION, endpoint, key, open, stall, step},
    *,
};

fn retry_attempt(terminal: &Envelope) -> Option<i64> {
    terminal
        .open_failed
        .as_ref()
        .and_then(|failure| failure.detail.as_ref())
        .and_then(|detail| detail.retry_attempt.as_ref())
        .map(|count| count.attempt)
}

/// Steps at `now` until `want` terminals have arrived, for at most 10 s.
fn until_terminals(
    endpoint: &mut PlacementEndpoint,
    now: u64,
    terminals: &mut Vec<Envelope>,
    want: usize,
) {
    let begun = Instant::now();
    while terminals.len() < want {
        assert!(begun.elapsed() < Duration::from_secs(10), "never finished");
        step(endpoint, now, terminals);
    }
}

#[test]
fn queued_members_finish_when_the_budget_is_spent_and_later_ones_park_for_the_retests() {
    // One setup at a time, one retry: attempts 1 and 2 spend the budget.
    let (mut endpoint, starts) = endpoint(stall(), Some(Duration::from_millis(30)), 1, 1);
    let mut terminals = Vec::new();
    for id in 1..=4 {
        endpoint.accept(OPERATION.into(), open(id, 30_000)).unwrap();
    }
    let begun = Instant::now();
    let wake = loop {
        step(&mut endpoint, 0, &mut terminals);
        if endpoint.opens.is_empty()
            && let Some(wake) = endpoint.retries.machine(OPERATION, &key()).wake_at()
        {
            break wake;
        }
        assert!(
            begun.elapsed() < Duration::from_secs(10),
            "the key never waited: {} starts, {} terminals",
            starts.lock().unwrap(),
            terminals.len()
        );
    };
    // The probe at the wake spends the budget; the members queued on the key
    // all finish with its failure, as they did when it closed.
    let mut now = wake;
    until_terminals(&mut endpoint, now, &mut terminals, 4);
    assert_eq!(*starts.lock().unwrap(), 2);
    assert!(terminals.iter().all(|t| t.kind == MessageKind::OpenFailed));
    // The limit machine was told the key left Healthy: N_good is frozen.
    let site = Key::ssh("git", "host", 22);
    let outage = |endpoint: &PlacementEndpoint| {
        let scoped = endpoint.endpoint.governor().scoped(OPERATION);
        scoped
            .view(&site, endpoint.endpoint.pool_now())
            .map(|v| v.outage)
    };
    assert_eq!(outage(&endpoint), Some(true));
    // A member that arrives now parks: the key is Down, not closed.
    endpoint.accept(OPERATION.into(), open(5, 30_000)).unwrap();
    let retest_at = endpoint
        .retries
        .machine(OPERATION, &key())
        .wake_at()
        .unwrap();
    assert_eq!(retest_at, now + 30_000);
    now = retest_at - 1;
    for _ in 0..20 {
        step(&mut endpoint, now, &mut terminals);
    }
    assert_eq!(*starts.lock().unwrap(), 2, "no handshake before the retest");
    assert_eq!(terminals.len(), 4, "the parked member has not failed");
    // It carries the retest, and shares its failure: no attempt count.
    now = retest_at;
    until_terminals(&mut endpoint, now, &mut terminals, 5);
    assert_eq!(*starts.lock().unwrap(), 3);
    assert_eq!(retry_attempt(terminals.last().unwrap()), None);
    // The second retest, 30 s on.
    endpoint.accept(OPERATION.into(), open(6, 30_000)).unwrap();
    now = endpoint
        .retries
        .machine(OPERATION, &key())
        .wake_at()
        .unwrap();
    until_terminals(&mut endpoint, now, &mut terminals, 6);
    assert_eq!(*starts.lock().unwrap(), 4);
    // Two in a row have failed: an arrival before the third is due finishes
    // at once, with no handshake.
    endpoint.accept(OPERATION.into(), open(7, 30_000)).unwrap();
    until_terminals(&mut endpoint, now, &mut terminals, 7);
    assert_eq!(*starts.lock().unwrap(), 4);
    assert_eq!(retry_attempt(terminals.last().unwrap()), None);
    endpoint.shutdown();
}

#[test]
fn a_key_going_down_gives_the_notes_reader_one_line_for_the_episode() {
    // Four members, one setup at a time and one retry: the second attempt
    // spends the budget and the key is Down. The note is made then, once,
    // and the retests' failures that follow are the same episode.
    let (mut endpoint, _) = endpoint(stall(), Some(Duration::from_millis(30)), 1, 1);
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let kept = lines.clone();
    endpoint.set_notes(setup_retry::NoteSink::new(move |line| {
        kept.lock().unwrap().push(line.to_owned())
    }));
    let mut terminals = Vec::new();
    for id in 1..=4 {
        endpoint.accept(OPERATION.into(), open(id, 30_000)).unwrap();
    }
    let begun = Instant::now();
    let wake = loop {
        step(&mut endpoint, 0, &mut terminals);
        if endpoint.opens.is_empty()
            && let Some(wake) = endpoint.retries.machine(OPERATION, &key()).wake_at()
        {
            break wake;
        }
        assert!(begun.elapsed() < Duration::from_secs(10), "never waited");
    };
    assert!(
        lines.lock().unwrap().is_empty(),
        "retrying is not Down, and nothing is said"
    );
    until_terminals(&mut endpoint, wake, &mut terminals, 4);
    step(&mut endpoint, wake, &mut terminals);
    assert_eq!(
        *lines.lock().unwrap(),
        ["note: host:22 is not answering; waiting up to 30 s to retry"]
    );
    // The first retest fails: the same episode.
    endpoint.accept(OPERATION.into(), open(5, 30_000)).unwrap();
    let retest_at = endpoint
        .retries
        .machine(OPERATION, &key())
        .wake_at()
        .unwrap();
    until_terminals(&mut endpoint, retest_at, &mut terminals, 5);
    step(&mut endpoint, retest_at, &mut terminals);
    assert_eq!(lines.lock().unwrap().len(), 1);
    endpoint.shutdown();
}
