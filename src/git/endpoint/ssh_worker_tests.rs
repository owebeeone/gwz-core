use super::*;
use gwz_transport::protocol::{Effect, ErrorCode, Failure};

struct Refuse(Arc<AtomicUsize>);
struct Unused;
impl Connector for Refuse {
    type Resource = Unused;
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Unused, Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Failure {
            detail: None,
            setup_cause: None,
            facts: None,
            code: ErrorCode::Authentication,
            effect: Effect::None,
        })
    }
}
impl Resource for Unused {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        unreachable!()
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, _: bool) -> Poll<io::Result<()>> {
        unreachable!()
    }
    fn reusable(&self) -> bool {
        false
    }
}
impl ChannelResource for Unused {
    fn start_exchange(
        &mut self,
        _: Stream,
        _: MessageEndpoint,
        _: GitService,
        _: &str,
    ) -> io::Result<()> {
        unreachable!()
    }
    fn pump(&mut self) -> Option<&mut SshPump<SshChannel>> {
        None
    }
    fn reclaim(&mut self) -> bool {
        false
    }
}
#[test]
fn queued_expiry_releases_admission_without_stopping_worker() {
    let config = PoolConfig::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let (pool, mut host) = PoolHost::new(config.clone(), Refuse(calls.clone()), 0).unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let permits = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let clock = Arc::new(AtomicU64::new(10));
    let enqueue = |deadline, cancelled| {
        let (reply, result) = mpsc::sync_channel(1);
        permits.fetch_add(1, Ordering::SeqCst);
        sender
            .send(OpenRequest {
                tag: None,
                limits: None,
                request: None,
                carrier: false,
                progress: Default::default(),
                setup_slot: Arc::default(),
                key: Key::ssh("git", "host", 22),
                identity: Identity::Ambient,
                service: GitService::UploadPack,
                path: "repo".into(),
                deadline,
                cancelled: Arc::new(AtomicBool::new(cancelled)),
                reply: Some(reply),
                selected: None,
                authority: None,
                url: None,
                producer: None,
                permit: Permit(permits.clone()),
                context: BridgeContext {
                    session_id: "session".into(),
                    stream_id: 1,
                    version: 2,
                    limits: gwz_transport::binding::default_limits(),
                    deadlines: Deadlines {
                        allocation_ms: config.allocation_timeout_ms as i64,
                        connect_ms: config.connect_timeout_ms as i64,
                        io_ms: 100,
                        interaction_ms: config.interaction_timeout_ms as i64,
                        cleanup_ms: config.cleanup_timeout_ms as i64,
                    },
                    waker: None,
                },
            })
            .unwrap();
        result
    };
    // Enqueue before the worker starts: no scheduling race can hide this branch.
    let exact = enqueue(Some(10), false);
    let worker_stop = stop.clone();
    let worker_clock = clock.clone();
    let mut admissions = Admissions::new(
        Registry::new(),
        Arc::new(Registry::start),
        Instant::now(),
        100,
    );
    let join = thread::spawn(move || {
        run(
            receiver,
            pool,
            &mut host,
            &mut admissions,
            100,
            10,
            worker_stop,
            move || worker_clock.load(Ordering::SeqCst),
            999,
            &Status::default(),
        )
    });
    let check_expired = |result: Receiver<OpenOutcome>| {
        let error = result
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(permits.load(Ordering::SeqCst), 0);
        assert!(!stop.load(Ordering::Acquire));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    };
    // Always stop/join even if an assertion fails.
    let assertions = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_expired(exact);
        clock.store(11, Ordering::SeqCst);
        let past = enqueue(Some(10), false);
        join.thread().unpark();
        check_expired(past);
        let cancelled = enqueue(None, true);
        join.thread().unpark();
        check_expired(cancelled);
        let next = enqueue(None, false);
        join.thread().unpark();
        let error = next
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::Other); // connector was reached
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(permits.load(Ordering::SeqCst), 0);
        assert!(!stop.load(Ordering::Acquire));
    }));
    stop.store(true, Ordering::Release);
    join.thread().unpark();
    join.join().unwrap();
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
}
/// Cancels its open from inside the connect, so that the worker's second expiry
/// check of the pass is the first to see the flag.
struct CancelsOnStart(Arc<AtomicBool>);
struct Connecting;
impl Connector for CancelsOnStart {
    type Resource = Connecting;
    fn setup_clock_source(&self) -> Option<(Instant, Arc<dyn Fn() -> u64 + Send + Sync>)> {
        let origin = Instant::now();
        Some((
            origin,
            Arc::new(move || origin.elapsed().as_millis() as u64),
        ))
    }
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Connecting, Failure> {
        self.0.store(true, Ordering::Release);
        Ok(Connecting)
    }
}
impl Resource for Connecting {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        Poll::Pending
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, _: bool) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn reusable(&self) -> bool {
        false
    }
}
impl ChannelResource for Connecting {
    fn start_exchange(
        &mut self,
        _: Stream,
        _: MessageEndpoint,
        _: GitService,
        _: &str,
    ) -> io::Result<()> {
        unreachable!()
    }
    fn pump(&mut self) -> Option<&mut SshPump<SshChannel>> {
        None
    }
    fn reclaim(&mut self) -> bool {
        false
    }
}
#[test]
fn an_open_cancelled_while_connecting_fails_cancelled_whichever_pass_sees_it() {
    let config = PoolConfig::default();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (pool, mut host) =
        PoolHost::new(config.clone(), CancelsOnStart(cancelled.clone()), 0).unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let (reply, result) = mpsc::sync_channel(1);
    sender
        .send(OpenRequest {
            tag: None,
            limits: None,
            request: None,
            carrier: false,
            progress: Default::default(),
            setup_slot: Arc::default(),
            key: Key::ssh("git", "host", 22),
            identity: Identity::Ambient,
            service: GitService::UploadPack,
            path: "repo".into(),
            deadline: None,
            cancelled: cancelled.clone(),
            reply: Some(reply),
            selected: None,
            authority: None,
            url: None,
            producer: None,
            permit: Permit(Arc::new(AtomicUsize::new(1))),
            context: BridgeContext {
                session_id: "session".into(),
                stream_id: 1,
                version: 2,
                limits: gwz_transport::binding::default_limits(),
                deadlines: Deadlines {
                    allocation_ms: config.allocation_timeout_ms as i64,
                    connect_ms: config.connect_timeout_ms as i64,
                    io_ms: 100,
                    interaction_ms: config.interaction_timeout_ms as i64,
                    cleanup_ms: config.cleanup_timeout_ms as i64,
                },
                waker: None,
            },
        })
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let mut admissions = Admissions::new(
        Registry::new(),
        Arc::new(Registry::start),
        Instant::now(),
        100,
    );
    let origin = Instant::now();
    let join = thread::spawn(move || {
        run(
            receiver,
            pool,
            &mut host,
            &mut admissions,
            100,
            10,
            worker_stop,
            move || origin.elapsed().as_millis() as u64,
            999,
            &Status::default(),
        )
    });
    let outcome = result.recv_timeout(Duration::from_secs(30));
    stop.store(true, Ordering::Release);
    join.thread().unpark();
    join.join().unwrap();
    let error = outcome.unwrap().err().unwrap();
    let failure = &error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<EndpointOpenFailure>())
        .expect("typed failure")
        .failure;
    assert_eq!(failure.code, ErrorCode::Cancelled);
}

fn bridge_context() -> BridgeContext {
    BridgeContext {
        session_id: "session".into(),
        stream_id: 1,
        version: 2,
        limits: gwz_transport::binding::default_limits(),
        deadlines: Deadlines {
            allocation_ms: 1_000,
            connect_ms: 1_000,
            io_ms: 1_000,
            interaction_ms: 1_000,
            cleanup_ms: 1_000,
        },
        waker: None,
    }
}

/// The pump relays the server's output into the endpoint end of the bridged
/// stream and reads from it only to take the client's input, which a fetch's
/// advertisement never is: nothing forces a send, so what the pump writes must
/// not wait out a coalescing window to reach Git (TR8.1: 100 ms per command).
#[test]
fn the_endpoint_end_sends_what_the_pump_writes_without_a_coalescing_window() {
    let (stream, endpoint) = Stream::new(stream_config(&bridge_context(), Side::Endpoint)).unwrap();
    endpoint.advance(0);
    let mut cx = Context::from_waker(Waker::noop());
    let advertisement = b"001e# service=git-upload-pack\n";
    assert!(matches!(
        pin!(stream.write(advertisement)).poll(&mut cx),
        Poll::Ready(Ok(count)) if count == advertisement.len()
    ));
    // No time passes: the message is there at once, not at a batch deadline.
    match pin!(endpoint.next_message()).poll(&mut cx) {
        Poll::Ready(Ok(Some(message))) => {
            assert_eq!(message.kind, gwz_transport::protocol::MessageKind::Data);
            assert_eq!(message.data.unwrap().payload, advertisement);
        }
        other => panic!("the written bytes were held back: {other:?}"),
    }
}

/// Counts the times it is woken.
#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// A placement session parks between passes and learns that the worker has
/// ended, and its cleanup with it, only by being woken or by its next timer
/// tick, which is up to 5 ms of each command's shutdown (TR8.1).
#[test]
fn the_worker_wakes_its_watcher_when_its_shutdown_has_settled() {
    let endpoint = Endpoint::with_registry(
        PoolConfig::default(),
        Registry::new(),
        |_, _| Refuse(Arc::default()),
        100,
    )
    .unwrap();
    let wakes = Arc::new(Wakes::default());
    endpoint.watch_shutdown(&Waker::from(wakes.clone()));
    let status = endpoint.shutdown_watch();
    endpoint.shutdown();
    let deadline = Instant::now() + Duration::from_secs(30);
    while wakes.0.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "the worker ended and did not wake its watcher"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        status.status().cleanup_complete,
        "the watcher was woken before the worker's cleanup was complete"
    );
}
/// One group of opens for `opening_by_request`: the transport request they belong to, its limits, and how many.
type Group<'a> = (
    Option<&'a str>,
    Option<gwz_transport::pool::Capacity>,
    usize,
);
/// Opens to one host over a connector whose connects never finish, sent group by group to ONE worker (one runtime):
/// how many connections the pool has opening once each group is in the pool.
fn opening_by_request(config: PoolConfig, groups: &[Group<'_>]) -> Vec<usize> {
    let total: usize = groups.iter().map(|group| group.2).sum();
    let (pool, mut host) = PoolHost::new(
        config.clone(),
        CancelsOnStart(Arc::new(AtomicBool::new(false))),
        0,
    )
    .unwrap();
    let (sender, receiver) = mpsc::sync_channel(total);
    let permits = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let mut admissions = Admissions::new(
        Registry::new(),
        Arc::new(Registry::start),
        Instant::now(),
        100,
    );
    let observed = pool.clone();
    let worker_stop = stop.clone();
    let join = thread::spawn(move || {
        run(
            receiver,
            pool,
            &mut host,
            &mut admissions,
            100,
            10,
            worker_stop,
            || 0,
            999,
            &Status::default(),
        )
    });
    let mut replies = Vec::new();
    let mut sent = 0;
    let mut opening = Vec::new();
    for (request, limits, count) in groups {
        for _ in 0..*count {
            sent += 1;
            let (reply, result) = mpsc::sync_channel(1);
            replies.push(result);
            permits.fetch_add(1, Ordering::SeqCst);
            sender
                .send(OpenRequest {
                    tag: None,
                    limits: *limits,
                    request: request.map(str::to_owned),
                    carrier: false,
                    progress: Default::default(),
                    setup_slot: Arc::default(),
                    key: Key::ssh("git", "host", 22),
                    identity: Identity::Ambient,
                    service: GitService::UploadPack,
                    path: "repo".into(),
                    deadline: None,
                    cancelled: Arc::new(AtomicBool::new(false)),
                    reply: Some(reply),
                    selected: None,
                    authority: None,
                    url: None,
                    producer: None,
                    permit: Permit(permits.clone()),
                    context: BridgeContext {
                        session_id: "session".into(),
                        stream_id: sent,
                        version: 2,
                        limits: gwz_transport::binding::default_limits(),
                        deadlines: Deadlines {
                            allocation_ms: config.allocation_timeout_ms as i64,
                            connect_ms: config.connect_timeout_ms as i64,
                            io_ms: 100,
                            interaction_ms: config.interaction_timeout_ms as i64,
                            cleanup_ms: config.cleanup_timeout_ms as i64,
                        },
                        waker: None,
                    },
                })
                .unwrap();
        }
        join.thread().unpark();
        let until = Instant::now() + Duration::from_secs(30);
        while observed.outstanding_requests() < sent as usize {
            assert!(Instant::now() < until, "the worker never took the opens");
            thread::sleep(Duration::from_millis(1));
        }
        opening.push(observed.counts().opening);
    }
    stop.store(true, Ordering::Release);
    join.thread().unpark();
    join.join().unwrap();
    drop(replies);
    opening
}
/// Two opens of one request (or of none) with `limits` on each: how many connections the pool has opening.
fn opening_with(limits: Option<gwz_transport::pool::Capacity>) -> usize {
    opening_by_request(PoolConfig::default(), &[(None, limits, 2)])[0]
}
#[test]
fn an_open_carries_its_operations_limits_to_the_pool_which_holds_the_worker_to_them() {
    use gwz_transport::pool::Capacity;
    // No limits: the pool's own capacity (32 a host) lets both connect at once.
    assert_eq!(opening_with(None), 2);
    // The operation's limit of one connection on the host: the second open waits in the pool, whatever the worker's
    // owner (the worker is one owner for all of its opens).
    let one = Capacity {
        per_user_host: 1,
        per_host: 1,
        total: 256,
        max_requests: 1024,
    };
    assert_eq!(opening_with(Some(one)), 1);
}
#[test]
fn overlapping_requests_on_one_worker_are_each_held_to_their_own_limit() {
    use gwz_transport::pool::Capacity;
    // State review P2-1: the worker is one runtime for every request, and the pool counts per owner session, so an
    // owner keyed by the worker alone would judge a request at 4 against the 10 held by a request at 32 (and starve it),
    // or let it eat into the larger request's share. One request is one owner.
    let limits = |per_host| Capacity {
        per_user_host: per_host,
        per_host,
        total: 256,
        max_requests: 1024,
    };
    let config = || PoolConfig {
        per_user_host: 64,
        per_host: 64,
        total: 64,
        ..PoolConfig::default()
    };
    // The request at 32 holds 10; the request at 4 then holds exactly 4.
    let opening = opening_by_request(
        config(),
        &[
            (Some("large"), Some(limits(32)), 10),
            (Some("small"), Some(limits(4)), 8),
        ],
    );
    assert_eq!(
        opening,
        vec![10, 14],
        "the request at 4 is granted exactly 4"
    );
    // Reverse order: the request at 4 holds 4, and the request at 32 still gets its 32.
    let opening = opening_by_request(
        config(),
        &[
            (Some("small"), Some(limits(4)), 8),
            (Some("large"), Some(limits(32)), 40),
        ],
    );
    assert_eq!(
        opening,
        vec![4, 36],
        "the request at 32 gets 32 beside the 4"
    );
    // A lone request at 32 is unchanged.
    let opening = opening_by_request(config(), &[(Some("large"), Some(limits(32)), 40)]);
    assert_eq!(opening, vec![32]);
}
