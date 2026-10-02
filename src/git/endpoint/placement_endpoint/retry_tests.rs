//! The retry plan's S3.1 sentences on the SSH endpoint, which drives the
//! per-key machine with the setup failures it owns (S3.3). A scripted
//! connector ends every setup with one outcome; the endpoint's clock is the
//! `now` each test passes to `step`, so no test waits out a real backoff.
use super::*;
use gwz_transport::{
    pool::Capacity,
    protocol::{Deadlines, SetupFailureCause},
};
use std::sync::{Arc, Mutex};

const JITTER: u64 = 7;
const OPERATION: &str = "operation";

fn failure(code: ErrorCode, setup_cause: Option<SetupFailureCause>) -> Failure {
    Failure {
        detail: None,
        setup_cause,
        code,
        effect: Effect::None,
        facts: None,
    }
}
fn stall() -> Failure {
    failure(ErrorCode::Timeout, Some(SetupFailureCause::Stall))
}

/// Setups that each end `after` their start with `outcome`, or never when
/// `after` is `None`; every start is recorded. With `offer_first` the first
/// setup reports an agent offer, as a setup that stalls after offering does.
struct Scripted {
    starts: Arc<Mutex<usize>>,
    after: Option<Duration>,
    outcome: Failure,
    offer_first: bool,
}
struct Setting {
    ends: Option<Instant>,
    outcome: Failure,
}
impl super::super::ssh_pool::Connector for Scripted {
    type Resource = Setting;
    fn start_reported(
        &mut self,
        key: &Key,
        identity: &gwz_transport::pool::Identity,
        deadline: Option<u64>,
        opening: super::super::ssh_pool::Opening,
    ) -> Result<Setting, Failure> {
        if self.offer_first && *self.starts.lock().unwrap() == 0 {
            let mut facts = opening.progress.lock().unwrap();
            facts.method = gwz_transport::protocol::AuthMethod::SshAgent;
            facts.credential_offered = true;
        }
        self.start(key, identity, deadline)
    }
    fn start(
        &mut self,
        _: &Key,
        _: &gwz_transport::pool::Identity,
        _: Option<u64>,
    ) -> Result<Setting, Failure> {
        *self.starts.lock().unwrap() += 1;
        Ok(Setting {
            ends: self.after.map(|after| Instant::now() + after),
            outcome: self.outcome.clone(),
        })
    }
}
impl super::super::ssh_pool::Resource for Setting {
    fn poll_connected(
        &mut self,
        _: &mut Context<'_>,
    ) -> Poll<Result<Option<gwz_transport::pool::Identity>, Failure>> {
        match self.ends {
            Some(ends) if Instant::now() >= ends => Poll::Ready(Err(self.outcome.clone())),
            _ => Poll::Pending,
        }
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, _: bool) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn reusable(&self) -> bool {
        false
    }
}
impl super::super::ssh_worker::ChannelResource for Setting {
    fn start_exchange(
        &mut self,
        _: gwz_transport::stream::Stream,
        _: gwz_transport::stream::MessageEndpoint,
        _: NativeService,
        _: &str,
    ) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn pump(
        &mut self,
    ) -> Option<&mut super::super::ssh_pump::SshPump<super::super::ssh_channel::SshChannel>> {
        None
    }
    fn reclaim(&mut self) -> bool {
        false
    }
}

/// An endpoint over the scripted connector, at an operation's per-host limit
/// and retry budget, with fixed jitter.
fn endpoint(
    outcome: Failure,
    after: Option<Duration>,
    per_host: usize,
    max_retries: u32,
) -> (PlacementEndpoint, Arc<Mutex<usize>>) {
    scripted_endpoint(
        Scripted {
            starts: Arc::new(Mutex::new(0)),
            after,
            outcome,
            offer_first: false,
        },
        per_host,
        max_retries,
    )
}
fn scripted_endpoint(
    connector: Scripted,
    per_host: usize,
    max_retries: u32,
) -> (PlacementEndpoint, Arc<Mutex<usize>>) {
    let starts = connector.starts.clone();
    let mut endpoint = PlacementEndpoint::new(
        Endpoint::with_registry(
            gwz_transport::pool::Config::default(),
            super::super::ssh_key_snapshot::Registry::new(),
            |_, _| connector,
            9_000,
        )
        .unwrap(),
        PathBuf::from("/tmp"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    endpoint
        .pool()
        .install_capacity(Capacity {
            per_user_host: per_host,
            per_host,
            total: 256,
            max_requests: 1024,
        })
        .unwrap();
    endpoint.set_jitter(setup_retry::Jitter::fixed(JITTER));
    endpoint.set_max_retries(OPERATION, max_retries);
    (endpoint, starts)
}
fn open(stream_id: i64, allocation_ms: i64) -> Envelope {
    let envelope = Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id,
        kind: MessageKind::Open,
        open: Some(gwz_transport::protocol::Open {
            endpoint_id: "endpoint".into(),
            operation_id: "operation".into(),
            destination: Destination {
                scheme: gwz_transport::protocol::Scheme::Ssh,
                host: "host".into(),
                port: 22,
                path: "/repo".into(),
                ssh_username: Some("git".into()),
                https_username: None,
            },
            service: GitService::UploadPackExchange,
            identity: Identity::default(),
            policy: gwz_transport::protocol::AuthPolicy::SshAmbient,
            deadlines: Deadlines {
                allocation_ms,
                connect_ms: 10_000,
                io_ms: 1_000,
                interaction_ms: 10_000,
                cleanup_ms: 1_000,
            },
            receive_limits: gwz_transport::binding::default_limits(),
        }),
        ..Default::default()
    };
    gwz_transport::codec::admit(&envelope).unwrap();
    envelope
}
/// The machine of every open here: one pool key, and the default identity
/// their Opens name.
fn key() -> RetryKey {
    (Key::ssh("git", "host", 22), Identity::default())
}
/// One pass at `now`, keeping the open terminals it hands out.
fn step(endpoint: &mut PlacementEndpoint, now: u64, terminals: &mut Vec<Envelope>) {
    let mut cx = Context::from_waker(std::task::Waker::noop());
    endpoint.step(now, &mut cx).unwrap();
    while let Some(item) = endpoint.take_outbound() {
        terminals.push(item.envelope);
    }
    std::thread::sleep(Duration::from_millis(1));
}
/// Steps at `now` until no attempt is in flight and the scripted setups that
/// started have ended, for at most 10 s of real time.
fn settle(endpoint: &mut PlacementEndpoint, now: u64, terminals: &mut Vec<Envelope>) {
    let begun = Instant::now();
    loop {
        step(endpoint, now, terminals);
        if endpoint.opens.is_empty() {
            return;
        }
        assert!(
            begun.elapsed() < Duration::from_secs(10),
            "attempts did not end"
        );
    }
}
/// Runs `opens` opens of `OPERATION` to their terminals, stepping at each of
/// the key's wakes in turn. Returns the terminals and the times of the wakes.
fn run(endpoint: &mut PlacementEndpoint, opens: i64) -> (Vec<Envelope>, Vec<u64>) {
    for stream_id in 1..=opens {
        endpoint
            .accept(OPERATION.into(), open(stream_id, 30_000))
            .unwrap();
    }
    let mut terminals = Vec::new();
    let mut wakes = Vec::new();
    let mut now = 0;
    while terminals.len() < opens as usize {
        settle(endpoint, now, &mut terminals);
        if terminals.len() == opens as usize {
            break;
        }
        let wake = endpoint
            .retries
            .machine(OPERATION, &key())
            .wake_at()
            .expect("members still waiting have a wake");
        assert!(wake > now, "the key woke and started nothing");
        now = wake;
        wakes.push(wake);
    }
    (terminals, wakes)
}
/// The code and origin of a terminal's open failure: the worker also
/// attaches the setup's facts, which these tests do not script.
fn open_failure(terminal: &Envelope) -> (ErrorCode, Option<SetupFailureCause>) {
    assert_eq!(terminal.kind, MessageKind::OpenFailed);
    let failure = terminal.open_failed.as_ref().unwrap();
    (failure.code, failure.setup_cause)
}
fn shape(failure: &Failure) -> (ErrorCode, Option<SetupFailureCause>) {
    (failure.code, failure.setup_cause)
}

#[test]
fn a_stall_holds_its_member_drops_its_connection_and_attempt_two_starts_at_the_wake() {
    let (mut endpoint, starts) = endpoint(stall(), Some(Duration::from_millis(20)), 32, 3);
    endpoint.accept(OPERATION.into(), open(1, 30_000)).unwrap();
    let mut terminals = Vec::new();
    settle(&mut endpoint, 0, &mut terminals);
    assert!(
        terminals.is_empty(),
        "attempt 1's stall does not finish the member"
    );
    assert_eq!(*starts.lock().unwrap(), 1);
    let counts = endpoint.pool().counts();
    assert_eq!(
        (counts.opening, counts.leased),
        (0, 0),
        "the failed connection is gone before the wait"
    );
    let wake = 1_000 + JITTER;
    for now in [1, wake - 1] {
        step(&mut endpoint, now, &mut terminals);
        assert_eq!(*starts.lock().unwrap(), 1, "nothing opens before the wake");
    }
    // The worker starts the probe's setup on its own thread.
    let begun = Instant::now();
    while *starts.lock().unwrap() < 2 {
        assert!(
            begun.elapsed() < Duration::from_secs(10),
            "attempt 2 never started"
        );
        step(&mut endpoint, wake, &mut terminals);
    }
    assert!(terminals.is_empty());
    endpoint.shutdown();
}

#[test]
fn attempt_four_finishes_the_member_with_its_stall_after_waits_of_one_two_and_four_seconds() {
    let (mut endpoint, starts) = endpoint(stall(), Some(Duration::from_millis(5)), 32, 3);
    let (terminals, wakes) = run(&mut endpoint, 1);
    assert_eq!(*starts.lock().unwrap(), 4);
    let gaps: Vec<_> = wakes.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert_eq!(wakes[0], 1_000 + JITTER);
    assert_eq!(gaps, [2_000 + JITTER, 4_000 + JITTER]);
    assert_eq!(open_failure(&terminals[0]), shape(&stall()));
}

#[test]
fn max_retries_zero_finishes_on_the_first_stall_and_one_allows_two_attempts() {
    for (max_retries, attempts) in [(0, 1), (1, 2)] {
        let (mut endpoint, starts) =
            endpoint(stall(), Some(Duration::from_millis(5)), 32, max_retries);
        let (terminals, _) = run(&mut endpoint, 1);
        assert_eq!(
            *starts.lock().unwrap(),
            attempts,
            "--max-retries {max_retries}"
        );
        assert_eq!(open_failure(&terminals[0]), shape(&stall()));
        let count = terminals[0]
            .open_failed
            .as_ref()
            .unwrap()
            .detail
            .as_ref()
            .and_then(|detail| detail.retry_attempt.as_ref())
            .unwrap();
        assert_eq!(
            (count.attempt, count.attempts),
            (attempts as i64, attempts as i64)
        );
    }
}

#[test]
fn thirty_two_cold_opens_on_a_dead_key_set_up_a_wave_at_the_limit_then_one_at_a_time() {
    for per_host in [32, 8] {
        let (mut endpoint, starts) =
            endpoint(stall(), Some(Duration::from_millis(50)), per_host, 3);
        let (terminals, _) = run(&mut endpoint, 32);
        assert_eq!(
            *starts.lock().unwrap(),
            per_host + 3,
            "per-host limit {per_host}"
        );
        assert!(
            terminals
                .iter()
                .all(|terminal| open_failure(terminal) == shape(&stall()))
        );
    }
}

#[test]
fn an_authentication_failure_waits_for_nothing_and_sets_up_nothing_beyond_the_wave() {
    let authentication = failure(ErrorCode::Authentication, None);
    let (mut endpoint, starts) = endpoint(
        authentication.clone(),
        Some(Duration::from_millis(50)),
        8,
        3,
    );
    let (terminals, wakes) = run(&mut endpoint, 32);
    assert!(wakes.is_empty(), "no wait");
    assert_eq!(*starts.lock().unwrap(), 8, "the wave, and nothing more");
    assert!(
        terminals
            .iter()
            .all(|terminal| open_failure(terminal) == shape(&authentication))
    );
}

#[test]
fn a_refused_connection_is_retried_and_a_missing_known_hosts_is_not() {
    for (cause, attempts) in [
        (SetupFailureCause::ConnectionRefused, 4),
        (SetupFailureCause::NotFound, 1),
    ] {
        let unavailable = failure(ErrorCode::Unavailable, Some(cause));
        let (mut endpoint, starts) =
            endpoint(unavailable.clone(), Some(Duration::from_millis(5)), 32, 3);
        let (terminals, _) = run(&mut endpoint, 1);
        assert_eq!(*starts.lock().unwrap(), attempts, "{cause:?}");
        assert_eq!(open_failure(&terminals[0]), shape(&unavailable));
    }
}

#[test]
fn an_interaction_timeout_is_returned_once_and_closes_the_key() {
    let interaction = failure(ErrorCode::Timeout, Some(SetupFailureCause::Interaction));
    let (mut endpoint, starts) =
        endpoint(interaction.clone(), Some(Duration::from_millis(5)), 32, 3);
    let (terminals, wakes) = run(&mut endpoint, 2);
    assert!(wakes.is_empty());
    assert_eq!(
        *starts.lock().unwrap(),
        2,
        "both opens were in the first wave"
    );
    assert!(
        terminals
            .iter()
            .all(|terminal| open_failure(terminal) == shape(&interaction))
    );
    // A later open of the operation finishes at once with the same failure.
    endpoint.accept(OPERATION.into(), open(3, 30_000)).unwrap();
    let mut late = Vec::new();
    step(&mut endpoint, 1, &mut late);
    assert_eq!(open_failure(&late[0]), shape(&interaction));
    assert_eq!(*starts.lock().unwrap(), 2);
}

#[test]
fn an_allocation_timeout_is_returned_once_and_moves_nothing() {
    // One setup that never ends holds the only connection the per-host limit
    // allows; the second open waits for it and runs out of allocation.
    let (mut endpoint, starts) = endpoint(stall(), None, 1, 3);
    endpoint.accept(OPERATION.into(), open(1, 30_000)).unwrap();
    endpoint.accept(OPERATION.into(), open(2, 100)).unwrap();
    let mut terminals = Vec::new();
    step(&mut endpoint, 0, &mut terminals);
    step(&mut endpoint, 100, &mut terminals);
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].stream_id, 2);
    assert_eq!(open_failure(&terminals[0]).0, ErrorCode::Timeout);
    assert_eq!(*starts.lock().unwrap(), 1, "the second open never set up");
    assert_eq!(
        endpoint.retries.machine(OPERATION, &key()).decide(100),
        setup_retry::Decision::Start,
        "the key did not move"
    );
    endpoint.shutdown();
}

#[test]
fn a_cancel_during_the_wait_opens_no_probe() {
    let (mut endpoint, starts) = endpoint(stall(), Some(Duration::from_millis(5)), 32, 3);
    endpoint.accept(OPERATION.into(), open(1, 30_000)).unwrap();
    endpoint.accept(OPERATION.into(), open(2, 30_000)).unwrap();
    let mut terminals = Vec::new();
    settle(&mut endpoint, 0, &mut terminals);
    assert!(terminals.is_empty());
    // One member's own cancel finishes it; the operation's cancel, the rest.
    let cancel = |stream_id| Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id,
        kind: MessageKind::Cancel,
        cancel: Some(gwz_transport::protocol::Cancel {
            reason: ErrorCode::Cancelled,
        }),
        ..Default::default()
    };
    endpoint.accept(OPERATION.into(), cancel(1)).unwrap();
    step(&mut endpoint, 1, &mut terminals);
    assert_eq!(terminals.len(), 1);
    assert_eq!(open_failure(&terminals[0]).0, ErrorCode::Cancelled);
    endpoint.cancel_request(OPERATION);
    for now in [2, 1_000 + JITTER, 60_000] {
        step(&mut endpoint, now, &mut terminals);
    }
    assert_eq!(*starts.lock().unwrap(), 2, "the wake opens no probe");
    assert_eq!(terminals.len(), 2);
    assert_eq!(open_failure(&terminals[1]).0, ErrorCode::Cancelled);
}

#[test]
fn each_attempts_facts_are_progress_on_the_members_one_row() {
    // Attempt 1 offers the agent's key and stalls; attempt 2 stalls before
    // any offer. The member's one reply carries both.
    let (mut endpoint, starts) = scripted_endpoint(
        Scripted {
            starts: Arc::new(Mutex::new(0)),
            after: Some(Duration::from_millis(5)),
            outcome: stall(),
            offer_first: true,
        },
        32,
        1,
    );
    let (terminals, _) = run(&mut endpoint, 1);
    assert_eq!(*starts.lock().unwrap(), 2);
    let facts = terminals[0]
        .open_failed
        .as_ref()
        .and_then(|failure| failure.facts.as_ref())
        .expect("the reply carries the member's facts");
    assert!(
        facts.credential_offered,
        "attempt 1's offer stays on the row"
    );
    assert_eq!(facts.method, gwz_transport::protocol::AuthMethod::SshAgent);
}
