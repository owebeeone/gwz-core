//! The connection host the profile rows run through: the composition's real process gate and inert ports, and one
//! adapter that connects at once and records whose token each connect and exchange was given (step 1.5).
use super::*;

struct Still;
impl Clock for Still {
    fn now(&self) -> Tick {
        Tick::new(0)
    }
    fn sleep_until(&self, _: Tick) -> Sleep {
        Box::pin(std::future::pending())
    }
    fn epoch(&self) -> u64 {
        0
    }
}
struct Seed;
impl OwnerSeed for Seed {
    fn namespace(&self) -> HostNamespace {
        HostNamespace::new(15)
    }
}
/// An executor that keeps the host's work and never runs it: this adapter's connects complete inside the submit's
/// own pump, so no row here needs the driver to run.
struct Held(Mutex<Vec<gwz_connection::DriverWork>>);
impl Progress for Held {
    fn check(&self) -> Result<(), ProgressUnavailable> {
        Ok(())
    }
    fn spawn(&self, work: gwz_connection::DriverWork) -> Result<(), ProgressUnavailable> {
        self.0.lock().unwrap().push(work);
        Ok(())
    }
}
struct Keep(Mutex<Vec<RetainedWork>>);
impl RetentionPort for Keep {
    fn retain(&self, work: RetainedWork) -> Result<(), RetentionRefused> {
        self.0.lock().unwrap().push(work);
        Ok(())
    }
}
pub(super) type Seen = Arc<Mutex<Vec<(&'static str, Option<OsString>)>>>;
struct Factory {
    seen: Seen,
    made: Arc<Mutex<usize>>,
}
impl EndpointFactory for Factory {
    fn create(
        &self,
        spec: &EndpointSpec,
        _: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        // The factory gets the profile's inputs and nothing of any operation's helper.
        assert!(spec.profile.inputs().is_some());
        *self.made.lock().unwrap() += 1;
        Ok(Box::new(Adapter {
            seen: self.seen.clone(),
            completions: VecDeque::new(),
        }))
    }
}
struct Adapter {
    seen: Seen,
    completions: VecDeque<Completion>,
}
fn token(auth: &AuthContext) -> Option<OsString> {
    let auth = OperationAuth::of(auth)?;
    auth.helper()
        .environment
        .iter()
        .find(|(name, _)| name == "GH_TOKEN")
        .map(|(_, value)| value.clone())
}
struct Loopback(VecDeque<u8>);
impl ExchangeIo for Loopback {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.extend(data);
        Ok(data.len())
    }
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = out.len().min(self.0.len());
        for (slot, byte) in out.iter_mut().zip(self.0.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }
    fn poll_finish(&mut self, _: &mut Context<'_>) -> Poll<Disposition> {
        Poll::Ready(Disposition::Reusable)
    }
    fn abort(&mut self) {}
}
impl EndpointAdapter for Adapter {
    fn connect(&mut self, command: ConnectCommand) {
        let given = command.auth.as_ref().and_then(token);
        self.seen.lock().unwrap().push(("connect", given));
        self.completions.push_back(Completion::Connected {
            connection: command.connection,
            result: Ok(Some(command.identity)),
        });
    }
    fn cancel_connect(&mut self, _: ConnectionId) {}
    fn close(&mut self, connection: ConnectionId) {
        self.completions
            .push_back(Completion::Closed { connection });
    }
    fn start_exchange(
        &mut self,
        _: ConnectionId,
        _: &ExchangeSpec,
        auth: &AuthContext,
    ) -> io::Result<Box<dyn ExchangeIo>> {
        self.seen.lock().unwrap().push(("exchange", token(auth)));
        Ok(Box::new(Loopback(VecDeque::new())))
    }
    /// The inert test port proves every cross-binding reuse at once (step 1.6's `authorize_reuse`; its own rows are
    /// the crate's), so these rows see the host's profile and scope partition alone.
    fn authorize_reuse(&mut self, command: gwz_connection::ReuseCommand) -> bool {
        self.completions.push_back(Completion::Reuse {
            connection: command.connection,
            verdict: gwz_connection::ReuseVerdict::Authorized,
        });
        true
    }
    fn poll_completion(&mut self, _: &mut Context<'_>) -> Poll<Completion> {
        match self.completions.pop_front() {
            Some(completion) => Poll::Ready(completion),
            None => Poll::Pending,
        }
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}
/// A future that the inert ports leave ready on its first poll.
pub(super) fn ready<F: Future>(future: F) -> F::Output {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the future was not ready"),
    }
}
/// One operation over `capture`, one exchange (HTTPS when `https`, else SSH): whether it reused a connection.
pub(super) fn one_operation_on(
    handle: &HostHandle,
    name: &str,
    capture: OperationCapture,
    https: bool,
) -> bool {
    let input = PreparedOperation::new(
        name,
        EndpointRoute::ssh_and_https(),
        capture.profile,
        capture.auth,
        OperationPolicy::new(4, 32, 3).unwrap(),
    )
    .unwrap();
    let operation = ready(handle.admit(input).unwrap().acquire())
        .unwrap()
        .register();
    let (key, identity) = if https {
        (Key::https("example.invalid", 443), Identity::Https)
    } else {
        (Key::ssh("git", "example.invalid", 22), Identity::Ambient)
    };
    let spec = ExchangeSpec::git(
        key,
        identity,
        GitService::UploadPackExchange,
        "/owner/repo.git",
    )
    .unwrap();
    let exchange = ready(operation.open(spec).unwrap().wait()).unwrap();
    let reused = exchange.reused();
    assert_eq!(ready(exchange.finish()).disposition, Disposition::Reusable);
    ready(operation.finish());
    reused
}
/// One SSH operation over `capture`.
pub(super) fn one_operation(handle: &HostHandle, name: &str, capture: OperationCapture) -> bool {
    one_operation_on(handle, name, capture, false)
}
/// A connection host of the composition's real gate and these inert ports, and what its adapters saw and made.
pub(super) fn test_host() -> (
    gwz_connection::CleanupDomainOwner,
    gwz_connection::HostOwner,
    HostHandle,
    Seen,
    Arc<Mutex<usize>>,
) {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let made = Arc::new(Mutex::new(0));
    let (domain_owner, domain) = gwz_connection::cleanup_domain(
        DomainConfig::default(),
        Arc::new(Keep(Mutex::new(Vec::new()))),
    )
    .unwrap();
    let config = HostConfig {
        domain,
        safety: Arc::new(crate::transport_host::composition::process_safety()),
        limits: HostLimits::default(),
    };
    let ports = HostPorts::new(
        Arc::new(Still),
        Arc::new(Seed),
        Arc::new(Held(Mutex::new(Vec::new()))),
        Arc::new(Factory {
            seen: seen.clone(),
            made: made.clone(),
        }),
    );
    let (owner, handle) = gwz_connection::build(config, ports).unwrap();
    (domain_owner, owner, handle, seen, made)
}
