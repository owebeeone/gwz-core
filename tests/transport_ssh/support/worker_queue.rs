use super::*;
use gwz_transport::protocol::{Effect, ErrorCode, Failure};

struct Refuse(Arc<AtomicUsize>);
struct Unused;
impl Connector for Refuse {
    type Resource = Unused;
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Unused, Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Failure {
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
    let (pool, mut host) = PoolHost::new(config, Refuse(calls.clone()), 0).unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let permits = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let clock = Arc::new(AtomicU64::new(10));
    let enqueue = |deadline, cancelled| {
        let (reply, result) = mpsc::sync_channel(1);
        permits.fetch_add(1, Ordering::SeqCst);
        sender
            .send(OpenRequest {
                progress: Default::default(),
                key: Key::ssh("git", "host", 22),
                identity: Identity::Ambient,
                service: GitService::UploadPack,
                path: "repo".into(),
                deadline,
                cancelled: Arc::new(AtomicBool::new(cancelled)),
                reply: Some(reply),
                selected: None,
                authority: None,
                permit: Permit(permits.clone()),
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
    let check_expired = |result: Receiver<io::Result<(BlockingStream, Opened)>>| {
        let error = result
            .recv_timeout(Duration::from_secs(5))
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
            .recv_timeout(Duration::from_secs(5))
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
