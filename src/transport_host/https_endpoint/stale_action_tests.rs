//! State-2 P2-4: on an endpoint Session without an SSH engine, a stale admitted
//! action for a retired HTTPS stream is no work, as the SSH engine makes it,
//! never a reason to close the Session. Each case runs an HTTPS-only endpoint
//! Session (`ssh: None`, the Windows qualification route) through its own pump
//! and mux. The test plays the driver with an initiator mux bound with small
//! queues, so it can fill the endpoint's outbound queue, and it reads every
//! message the endpoint sends.
use super::https_cancel_mux_tests::{open_for, small_limits};
use super::*;
use crate::git::endpoint::{
    https_destination::Destination as HttpsDestination,
    https_fixture::{Server, response},
    https_worker::native,
    ssh_handoff::Handoff,
};
use crate::transport_host::{
    ClientRequest, EndpointSettings,
    session::{Session, TransportPort},
};
use gwz_transport::mux;
use std::{pin::pin, sync::atomic::AtomicUsize, task::Waker};
use tokio::sync::{Notify, Semaphore};

const ADVERTISEMENT: GitService = GitService::UploadPackAdvertisement;

fn run(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test);
}

struct Fixture {
    _server: Server,
    /// Native authentication guards it: 401 NTLM until authorized.
    repo: HttpsDestination,
    /// Served without authentication.
    open_repo: HttpsDestination,
    endpoint: Arc<Session>,
    port: TransportPort,
    initiator: mux::Mux,
    owner: mux::Owner,
    /// The first authorized response waits for a permit; later ones pass.
    release: Arc<Semaphore>,
    reached: Arc<Notify>,
    _bootstrap: ClientRequest,
}
impl Fixture {
    async fn start() -> Self {
        let release = Arc::new(Semaphore::new(0));
        let reached = Arc::new(Notify::new());
        let (gate, signal) = (release.clone(), reached.clone());
        let server = Server::start(Arc::new(move |request| {
            let (gate, signal) = (gate.clone(), signal.clone());
            Box::pin(async move {
                let open = request.uri().path().starts_with("/anon/");
                let authorized = request.headers().contains_key("authorization");
                if authorized {
                    signal.notify_one();
                    drop(gate.acquire().await);
                }
                let status = if open || authorized { 200 } else { 401 };
                let mut reply = response(status, ADVERTISEMENT, bytes::Bytes::from_static(b"ok"));
                if status == 401 {
                    reply
                        .headers_mut()
                        .insert("www-authenticate", "NTLM".parse().unwrap());
                }
                reply
            })
        }))
        .await;
        let repo = HttpsDestination::parse(&server.url).unwrap();
        let open_repo = server.url.replace("/repo", "/anon/repo");
        let mut pool_config = pool::Config::default();
        pool_config.total = 1;
        pool_config.per_host = 1;
        let (endpoint, port) = Session::endpoint_with_https_native(
            EndpointSettings {
                ssh: None,
                pool: pool_config,
                io_timeout_ms: 3000,
            },
            Some((
                HttpsEndpointConfig {
                    tls: server.config(),
                    auth: None,
                },
                HelperSlots::new(),
            )),
            Handoff::default(),
            Some(native::publication_fixture()),
        )
        .unwrap();
        let bootstrap = ClientRequest::new(endpoint.clone(), "bootstrap").unwrap();
        let config = mux::Config {
            limits: small_limits(),
            ..Default::default()
        };
        let mut initiator = mux::Mux::initiator("stale", config).unwrap();
        initiator
            .register("bootstrap", Some("operation".into()))
            .unwrap();
        initiator.begin("bootstrap").unwrap();
        port.deliver(initiator.next_message().unwrap())
            .await
            .unwrap();
        initiator
            .receive(&port.next_message().await.unwrap().unwrap())
            .unwrap();
        Self {
            _server: server,
            repo,
            open_repo: HttpsDestination::parse(&open_repo).unwrap(),
            owner: endpoint.mux_owner_for_test(),
            endpoint,
            port,
            initiator,
            release,
            reached,
            _bootstrap: bootstrap,
        }
    }
    /// Registers `name` with the initiator and, as the Local placement does,
    /// with the endpoint Session; dropping the guard drops it there.
    fn request(&mut self, name: &str) -> ClientRequest {
        self.initiator
            .register(name, Some("operation".into()))
            .unwrap();
        ClientRequest::new(self.endpoint.clone(), name).unwrap()
    }
    /// Opens `service` natively on the guarded repository, or anonymously on
    /// the open one.
    async fn open(&mut self, request: &str, native: bool, service: GitService) -> i64 {
        let binding = self.initiator.binding().unwrap();
        let destination = if native { &self.repo } else { &self.open_repo };
        let mut open = open_for(destination, binding.limits());
        open.endpoint_id = binding.endpoint_id().into();
        open.service = service;
        if native {
            open.policy = AuthPolicy::WindowsDefault;
            open.identity.mode = IdentityMode::Ambient;
        }
        let id = self.initiator.open(request, open).unwrap();
        self.port
            .deliver(self.initiator.next_message().unwrap())
            .await
            .unwrap();
        id
    }
    /// Fills the endpoint's outbound queue, four frames under small_limits,
    /// with Opened for four filler streams. The pump is held while the test
    /// takes their Open actions itself, so they start no HTTPS work.
    async fn fill(&mut self) -> ClientRequest {
        let filler = self.request("filler");
        self.endpoint.hold_pump_for_test(true);
        for _ in 0..4 {
            let id = self.open("filler", false, ADVERTISEMENT).await;
            let (request, action) = self.owner.next_action().await.unwrap().unwrap();
            assert_eq!((request.as_str(), action.stream_id), ("filler", id));
            let binding = self.initiator.binding().unwrap();
            let opened = Envelope {
                version: 2,
                session_id: binding.session_id().into(),
                stream_id: id,
                kind: MessageKind::Opened,
                opened: Some(Opened {
                    connection_id: format!("filler-{id}"),
                    reused: false,
                    endpoint_id: binding.endpoint_id().into(),
                    trust_owner: binding.trust_owner().into(),
                    facts: Facts::default(),
                    receive_limits: binding.limits().clone(),
                }),
                ..Default::default()
            };
            self.owner.send("filler", &opened).unwrap();
        }
        self.endpoint.hold_pump_for_test(false);
        filler
    }
    /// A native target whose real attempt, done before D, the test holds
    /// while it fills the outbound queue: the server keeps the authorized
    /// response until the attempt is taken, so no pass can collect it first.
    async fn native_target(&mut self) -> (ClientRequest, Key, Attempt, ClientRequest) {
        let target = self.request("target");
        let id = self.open("target", true, ADVERTISEMENT).await;
        let key = (String::from("target"), id);
        self.reached.notified().await;
        let task = self.endpoint.with_https_for_test(|https| {
            https
                .entries
                .get_mut(&key)
                .unwrap()
                .preparing
                .take()
                .unwrap()
        });
        let filler = self.fill().await;
        self.release.add_permits(1);
        (target, key, task.await.unwrap(), filler)
    }
    /// Hands the attempt back for the next pass to collect.
    fn resume(&self, key: &Key, attempt: Attempt, clock: Option<Clock>) {
        self.endpoint.with_https_for_test(|https| {
            if let Some(clock) = clock {
                https.clock = clock;
            }
            let preparing = https.runtime.spawn(async move { attempt });
            https.entries.get_mut(key).unwrap().preparing = Some(preparing);
        });
    }
    /// The endpoint's next message, which the initiator also takes.
    async fn next(&mut self) -> (String, Envelope) {
        let item = self.port.next_message().await.unwrap().unwrap();
        self.initiator.receive(&item).unwrap();
        while self.initiator.next_action().is_some() {}
        item
    }
    async fn opened(&mut self, id: i64) {
        let (_, opened) = self.next().await;
        assert_eq!((opened.kind, opened.stream_id), (MessageKind::Opened, id));
    }
    /// The endpoint's messages until stream `id` has one: four filler
    /// Opened and that one, in whatever order the mux sends them.
    async fn after_fillers(&mut self, id: i64) -> Envelope {
        let (mut fillers, mut found) = (0, None);
        while fillers < 4 || found.is_none() {
            let (request, message) = self.next().await;
            if request == "filler" {
                assert_eq!(message.kind, MessageKind::Opened);
                fillers += 1;
            } else {
                assert_eq!(message.stream_id, id);
                assert!(found.replace(message).is_none(), "a second message");
            }
        }
        found.unwrap()
    }
    /// The initiator's EndWrite and Close for advertisement stream `id`, then
    /// the endpoint's messages for it through its terminal, which must close
    /// it cleanly.
    async fn complete(&mut self, request: &str, id: i64) {
        for kind in [MessageKind::EndWrite, MessageKind::Close] {
            let mut message = self.initiator.message(id, kind).unwrap();
            message.end_write =
                (kind == MessageKind::EndWrite).then_some(EndWrite { final_offset: 0 });
            message.close = (kind == MessageKind::Close).then_some(Close { final_offset: 0 });
            self.initiator.send(request, &message).unwrap();
            self.port
                .deliver(self.initiator.next_message().unwrap())
                .await
                .unwrap();
        }
        loop {
            let (_, message) = self.next().await;
            assert_eq!(message.stream_id, id);
            if matches!(message.kind, MessageKind::Closed | MessageKind::Failed) {
                assert!(message.closed.is_some_and(|c| c.failure.is_none()));
                return;
            }
        }
    }
    /// Waits, at most two seconds, until `done` or until the Session closes.
    async fn until(&self, done: impl Fn() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !done() && !self.endpoint.is_closed() {
            assert!(tokio::time::Instant::now() < deadline, "no progress");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
    fn entry(&self, key: &Key, test: impl FnOnce(Option<&Entry>) -> bool) -> bool {
        self.endpoint
            .with_https_for_test(|https| test(https.entries.get(key)))
    }
    /// Once a pass has removed the retired entry, lets the pump run three
    /// more, which take the stale action admitted for it.
    async fn settles_open(&self, key: &Key) {
        self.until(|| self.entry(key, |entry| entry.is_none()))
            .await;
        let start = self.endpoint.pumps_for_test();
        self.until(|| self.endpoint.pumps_for_test() >= start + 3)
            .await;
        assert!(
            !self.endpoint.is_closed(),
            "a stale action closed the HTTPS-only Session"
        );
    }
    /// After two more passes the endpoint has nothing to send.
    async fn idle(&self) -> bool {
        let start = self.endpoint.pumps_for_test();
        self.until(|| self.endpoint.pumps_for_test() >= start + 2)
            .await;
        pin!(self.port.next_message())
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    }
}

/// State-2 case (a). The native Opened is refused under the lock and its
/// OpenFailed waits behind a full outbound queue. The initiator's Cancel for
/// that stream is admitted while the route is live, and a later pass takes it
/// after removing the retired entry. The Session stays open, the refusal is
/// the stream's one terminal, the real connection's charge lasts until its
/// disposal, and a second request on the same Session opens and completes.
#[test]
fn a_cancel_racing_a_refused_native_open_leaves_the_https_only_session_open() {
    run(async {
        let mut f = Fixture::start().await;
        let (target, key, attempt, filler) = f.native_target().await;
        let prepared = attempt.0.as_ref().expect("native preparation before D");
        let (route, connection) = prepared.publication_resources_for_test();
        let until = attempt.2.budget.publication_deadline().unwrap();
        let script = [
            until - Duration::from_nanos(1),
            until + Duration::from_millis(1),
        ];
        let readings = Arc::new(AtomicUsize::new(0));
        let count = readings.clone();
        let clock = Clock(Some(Arc::new(move || {
            script[count.fetch_add(1, Ordering::SeqCst).min(1)]
        })));
        f.resume(&key, attempt, Some(clock));
        f.until(|| f.entry(&key, |entry| entry.is_none_or(|e| e.retired)))
            .await;
        f.initiator.cancel("target").unwrap();
        let cancel = f.initiator.next_message().unwrap();
        assert_eq!(
            (cancel.1.kind, cancel.1.stream_id),
            (MessageKind::Cancel, key.1)
        );
        f.port.deliver(cancel).await.unwrap();
        f.settles_open(&key).await;

        let terminal = f.after_fillers(key.1).await;
        let failure = terminal
            .open_failed
            .expect("late native Open must never publish");
        assert_eq!(
            (failure.code, failure.effect),
            (ErrorCode::Timeout, Effect::None)
        );
        assert_eq!(failure.facts.unwrap().authenticated, Some(true));
        assert_eq!(readings.load(Ordering::SeqCst), 2);
        assert!(route.revoked_for_test());
        let authority = f.endpoint.authority_for_test();
        assert_eq!(authority.counts(f.repo.host()), (1, 1));
        assert!(authority.try_reserve(f.repo.host()).is_none());
        assert!(f.idle().await, "the stream has one terminal");
        drop(connection);
        f.until(|| authority.counts(f.repo.host()) == (0, 0)).await;
        assert_eq!(authority.counts(f.repo.host()), (0, 0));

        let second = f.request("second");
        let id = f.open("second", false, ADVERTISEMENT).await;
        f.opened(id).await;
        f.complete("second", id).await;
        assert!(!f.endpoint.is_closed());
        drop((second, target, filler));
        f.endpoint.close();
    });
}

/// State-2 case (b), a pending native Opened: admitted under the lock, it waits
/// behind a full queue, so its HTTPS entry is idle on a live route when the
/// endpoint side drops the request. The mux's own Cancelled terminal is the
/// stream's one terminal; the Opened never publishes.
#[test]
fn dropping_a_request_with_a_pending_native_opened_leaves_the_https_only_session_open() {
    run(async {
        let mut f = Fixture::start().await;
        let (target, key, attempt, filler) = f.native_target().await;
        let (_, connection) = attempt
            .0
            .as_ref()
            .expect("native preparation before D")
            .publication_resources_for_test();
        f.resume(&key, attempt, None);
        f.until(|| {
            f.entry(&key, |entry| {
                entry.is_some_and(|e| {
                    e.preparing.is_none() && e.output.is_none() && e.prepared.is_some()
                })
            })
        })
        .await;
        assert!(!f.endpoint.is_closed());
        drop(target);
        f.settles_open(&key).await;

        let terminal = f.after_fillers(key.1).await;
        assert_eq!(terminal.kind, MessageKind::OpenFailed);
        assert_eq!(terminal.open_failed.unwrap().code, ErrorCode::Cancelled);
        assert!(f.idle().await, "the stream has one terminal");
        let authority = f.endpoint.authority_for_test();
        assert_eq!(authority.counts(f.repo.host()), (1, 1));
        drop(connection);
        f.until(|| authority.counts(f.repo.host()) == (0, 0)).await;
        assert!(!f.endpoint.is_closed());
        drop(filler);
        f.endpoint.close();
    });
}

/// State-2 case (b), a published POST: the exchange's Opened is published and
/// its HTTPS entry waits for the first Data when the endpoint side drops the
/// request. The mux's own Cancelled terminal is the stream's one terminal.
#[test]
fn dropping_a_request_with_a_published_post_leaves_the_https_only_session_open() {
    run(async {
        let mut f = Fixture::start().await;
        let post = f.request("post");
        // An advertisement pins the route its POST uses.
        let get = f.open("post", false, ADVERTISEMENT).await;
        f.opened(get).await;
        f.complete("post", get).await;
        let id = f.open("post", false, GitService::UploadPackExchange).await;
        f.opened(id).await;
        let key = (String::from("post"), id);
        f.until(|| {
            f.entry(&key, |entry| {
                entry.is_some_and(|e| {
                    e.prepared.is_some() && e.preparing.is_none() && e.serving.is_none()
                })
            })
        })
        .await;
        drop(post);
        f.settles_open(&key).await;

        let (_, terminal) = f.next().await;
        assert_eq!(
            (terminal.kind, terminal.stream_id),
            (MessageKind::Failed, id)
        );
        assert_eq!(terminal.failed.unwrap().code, ErrorCode::Cancelled);
        assert!(f.idle().await, "the stream has one terminal");
        assert!(!f.endpoint.is_closed());
        f.endpoint.close();
    });
}

/// Work the HTTPS endpoint cannot do stays fatal: an identity check still goes
/// to the absent SSH engine and closes the Session, as before, rather than
/// vanishing at HTTPS as no work.
#[test]
fn an_identity_check_on_the_https_only_session_stays_fatal() {
    run(async {
        let mut f = Fixture::start().await;
        let _check = f.request("check");
        let identity = Identity {
            mode: IdentityMode::ExplicitKey,
            key_path: Some("key".into()),
            path_base: Some("/endpoint".into()),
        };
        f.initiator
            .check_identity("check", identity, 1_000)
            .unwrap();
        f.port
            .deliver(f.initiator.next_message().unwrap())
            .await
            .unwrap();
        f.until(|| f.endpoint.is_closed()).await;
        assert!(f.endpoint.is_closed());
    });
}
