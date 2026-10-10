//! The worker's background close, on the endpoint production builds and a
//! loopback `sshd` whose channels end slowly, badly or never
//! (`dev-docs/GwzTransportSshBackgroundCloseDesign.md` §4 to §6, §10).
//!
//! A fetch's member is done when libgit2 closes, and a push's when the server's
//! output ends. The connection stays leased, and counted, until its graceful
//! close ends, and is then reused or discarded.
use super::attachment;
use crate::git::endpoint::{
    shared_reservation::Authority,
    ssh_channel::GitService,
    ssh_close_fixture::{
        delayed_close_fixture, delayed_eof_fixture, gated_close_fixture, read_advertisement,
        silent_fixture, stuck_close_fixture,
    },
    ssh_fixture::SshdFixture,
    ssh_local,
    ssh_worker::Endpoint,
    stream_io::BlockingStream,
};
use gwz_transport::{
    pool::{Config, Counts, Key},
    protocol::Opened,
};
use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const PATIENCE: Duration = Duration::from_secs(10);

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// What an open that waited out one deferral and then connected may take: the deferral bound and a connect.
        const DEFERRED_ONCE_AT_MOST: Duration = Duration::from_millis(450);
        /// What an open whose deferral the close's own bound cut short may take: the bound and a connect, well
        /// under the 250 ms wait it would otherwise have made.
        const BOUNDED_WAIT_AT_MOST: Duration = Duration::from_millis(250);
    } else {
        /// The same bound where a connect through Windows' `sshd.exe` takes about 200 ms more (measured on dabeest,
        /// step 1.5); an open deferred twice would take the deferral bound again on top.
        const DEFERRED_ONCE_AT_MOST: Duration = Duration::from_millis(650);
        /// The same with the extra connect time: halfway between the cut-short wait and the full 250 ms one.
        const BOUNDED_WAIT_AT_MOST: Duration = Duration::from_millis(360);
    }
}

fn config(per_host: usize, cleanup_ms: u64) -> Config {
    Config {
        total: per_host,
        per_host,
        per_user_host: per_host,
        cleanup_timeout_ms: cleanup_ms,
        ..Config::default()
    }
}

/// The endpoint as the transport host builds it, over `fixture`.
fn endpoint(fixture: &SshdFixture, per_host: usize, cleanup_ms: u64, io_ms: u64) -> Endpoint {
    let config = config(per_host, cleanup_ms);
    let authority = Authority::new(config.total, config.per_host);
    ssh_local::connect_with_authority(
        config,
        fixture.known_hosts.clone(),
        None,
        io_ms,
        authority,
        Default::default(),
    )
    .unwrap()
}

fn key(fixture: &SshdFixture) -> Key {
    Key::ssh(&fixture.user, "127.0.0.1", fixture.port)
}

fn identity(fixture: &SshdFixture) -> PathBuf {
    fixture.temp.path().join("client_ed25519")
}

/// One server, one endpoint over it, and the identity that opens use.
struct Rig {
    fixture: SshdFixture,
    endpoint: Endpoint,
    cleanup_ms: u64,
    io_ms: u64,
    per_host: usize,
}

impl Rig {
    fn new(fixture: SshdFixture, per_host: usize, cleanup_ms: u64) -> Self {
        Self::with_io(fixture, per_host, cleanup_ms, 3_000)
    }

    fn with_io(fixture: SshdFixture, per_host: usize, cleanup_ms: u64, io_ms: u64) -> Self {
        let endpoint = endpoint(&fixture, per_host, cleanup_ms, io_ms);
        Self {
            fixture,
            endpoint,
            cleanup_ms,
            io_ms,
            per_host,
        }
    }

    fn open_path(
        &self,
        service: GitService,
        identity: PathBuf,
        path: &str,
    ) -> io::Result<(BlockingStream, Opened)> {
        attachment::open(
            &self.endpoint,
            key(&self.fixture),
            Some(identity),
            service,
            path,
            attachment::deadlines(&config(self.per_host, self.cleanup_ms), self.io_ms),
        )
    }

    fn open_as(&self, service: GitService, identity: PathBuf) -> (BlockingStream, Opened) {
        let path = self.fixture.repository.to_str().unwrap().to_owned();
        self.open_path(service, identity, &path).unwrap()
    }

    fn open(&self, service: GitService) -> (BlockingStream, Opened) {
        self.open_as(service, identity(&self.fixture))
    }

    fn counts(&self) -> Counts {
        self.endpoint.pool().counts()
    }

    /// Waits until `condition` holds of the pool's counts, or fails.
    fn wait(&self, what: &str, condition: impl Fn(Counts) -> bool) -> Duration {
        let started = Instant::now();
        while !condition(self.counts()) {
            assert!(
                started.elapsed() < PATIENCE,
                "never {what}: {:?}",
                self.counts()
            );
            thread::sleep(Duration::from_millis(2));
        }
        started.elapsed()
    }

    /// A fetch's last turn, as libgit2 takes it: read the advertisement, send
    /// the closing flush-pkt, and close. How long the close took.
    fn fetch(&self, stream: BlockingStream) -> Duration {
        self.fetch_result(stream).0
    }

    fn fetch_result(&self, mut stream: BlockingStream) -> (Duration, io::Result<()>) {
        read_advertisement(&mut stream);
        close_after_flush(stream)
    }

    /// A push with nothing to send: the advertisement, the closing flush-pkt,
    /// the server's output to its end, and the close.
    fn push(&self, mut stream: BlockingStream) -> (Duration, io::Result<()>) {
        read_advertisement(&mut stream);
        stream.write_all(b"0000").unwrap();
        stream.end_write().unwrap();
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).unwrap();
        timed_close(&stream)
    }
}

fn close_after_flush(mut stream: BlockingStream) -> (Duration, io::Result<()>) {
    stream.write_all(b"0000").unwrap();
    timed_close(&stream)
}

fn timed_close(stream: &BlockingStream) -> (Duration, io::Result<()>) {
    let started = Instant::now();
    let result = stream.close().map(|_| ());
    (started.elapsed(), result)
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

#[test]
fn upload_pack_close_does_not_wait_for_server_eof() {
    let delay = ms(1500);
    let rig = Rig::new(delayed_eof_fixture(delay), 1, 5_000);
    let (stream, opened) = rig.open(GitService::UploadPack);
    assert!(!opened.reused);
    let (closed_in, result) = {
        let mut stream = stream;
        read_advertisement(&mut stream);
        close_after_flush(stream)
    };
    result.expect("the close is clean");
    assert!(
        closed_in < delay / 3,
        "the member waited {closed_in:?} for a server that ends in {delay:?}"
    );
    // The close ends off the member's path, and the connection is reused.
    rig.wait("idle after the close", |counts| counts.idle == 1);
    let (stream, again) = rig.open(GitService::UploadPack);
    assert!(again.reused, "an early-closed connection is not reused");
    rig.fetch(stream);
}

#[test]
fn receive_pack_close_returns_before_the_servers_close() {
    let delay = ms(1500);
    let rig = Rig::new(delayed_close_fixture(delay), 1, 5_000);
    let (stream, _) = rig.open(GitService::ReceivePack);
    let (closed_in, result) = rig.push(stream);
    result.expect("the close is clean");
    assert!(
        closed_in < delay / 3,
        "the member waited {closed_in:?} for the server's close, {delay:?} after its output"
    );
    rig.wait("idle after the close", |counts| counts.idle == 1);
    let (stream, again) = rig.open(GitService::ReceivePack);
    assert!(again.reused);
    rig.push(stream).1.unwrap();
}

#[test]
fn connection_is_reusable_only_after_the_close_finishes() {
    let delay = ms(1000);
    let rig = Rig::new(delayed_close_fixture(delay), 1, 5_000);
    let (stream, _) = rig.open(GitService::UploadPack);
    let closed_in = rig.fetch(stream);
    assert!(closed_in < delay / 2, "close() took {closed_in:?}");
    let counts = rig.counts();
    assert_eq!(
        (counts.leased, counts.idle),
        (1, 0),
        "the connection is the close's until it ends: {counts:?}"
    );
    let ended = rig.wait("idle after the close", |counts| counts.idle == 1);
    assert!(ended < delay + ms(2_000), "the close took {ended:?} to end");
    let (stream, again) = rig.open(GitService::UploadPack);
    assert!(again.reused);
    rig.fetch(stream);
}

#[test]
fn a_connection_in_background_close_counts_against_the_cap() {
    let delay = ms(500);
    let rig = Rig::new(delayed_close_fixture(delay), 1, 5_000);
    let (stream, _) = rig.open(GitService::UploadPack);
    let closed_in = rig.fetch(stream);
    assert!(closed_in < delay / 2, "close() took {closed_in:?}");
    let busiest = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let pool = rig.endpoint.pool().clone();
    let sampler = {
        let (busiest, stop) = (busiest.clone(), stop.clone());
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                busiest.fetch_max(pool.counts().total(), Ordering::AcqRel);
                thread::sleep(ms(2));
            }
        })
    };
    let started = Instant::now();
    let (stream, second) = rig.open(GitService::UploadPack);
    let waited = started.elapsed();
    stop.store(true, Ordering::Release);
    sampler.join().unwrap();
    assert!(second.reused, "a second connection was opened past the cap");
    assert_eq!(busiest.load(Ordering::Acquire), 1);
    assert!(
        waited >= delay / 3,
        "the second open took {waited:?}, so it did not wait for the close"
    );
    rig.fetch(stream);
}

// The fixture ends the server's own session process with `ps` and `kill -9`, which have no Windows twin here.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use crate::git::endpoint::ssh_close_fixture::dropped_close_fixture;

        #[test]
        fn a_background_close_that_drops_the_connection_is_never_reused() {
            let rig = Rig::new(dropped_close_fixture(), 2, 5_000);
            let (stream, _) = rig.open(GitService::UploadPack);
            // libgit2 ignores the close's result: the fetch's stands whatever it is.
            let _ = rig.fetch_result(stream);
            rig.wait("discarded", |counts| counts.total() == 0);
            let (stream, next) = rig.open(GitService::UploadPack);
            assert!(!next.reused, "a connection whose close failed was reused");
            drop(stream);
        }
    }
}

#[test]
fn a_refused_open_releases_its_connection_at_once() {
    let rig = Rig::new(delayed_close_fixture(ms(2_000)), 1, 5_000);
    let refused = format!("{}/refused", rig.fixture.repository.display());
    let (mut stream, _) = rig
        .open_path(GitService::UploadPack, identity(&rig.fixture), &refused)
        .unwrap();
    let mut byte = [0_u8; 1];
    let error = stream
        .read(&mut byte)
        .expect_err("the server refused the repository");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    // The refusal is the terminal: nothing remains of the exchange to wait for.
    rig.wait("released", |counts| counts.leased == 0);
    let started = Instant::now();
    let (stream, valid) = rig.open(GitService::UploadPack);
    assert!(!valid.reused);
    assert!(
        started.elapsed() < ms(1_500),
        "the next open waited {:?} for the refused exchange",
        started.elapsed()
    );
    drop(stream);
}

#[test]
fn a_stream_timeout_terminal_is_released_at_once() {
    let cleanup = 1_200;
    let rig = Rig::new(silent_fixture(), 1, cleanup);
    let (mut stream, _) = rig.open(GitService::ReceivePack);
    stream.write_all(b"0000").unwrap();
    stream.end_write().unwrap();
    // The server never ends its output, so the endpoint cannot complete the
    // close: the stream times out `cleanup` after the member's Close.
    let (closed_in, result) = timed_close(&stream);
    assert!(
        result.is_err(),
        "the close completed against a silent server"
    );
    assert!(
        closed_in >= ms(cleanup - 300),
        "gave up after {closed_in:?}"
    );
    // The terminal ends the exchange in the pass that hands it over. A second
    // bound, from the handoff, is no part of it.
    let released = rig.wait("released", |counts| counts.total() == 0);
    assert!(
        released < ms(cleanup / 2),
        "the connection was held {released:?} past the stream's own timeout"
    );
}

// A shell script cannot end a channel's output and keep its process on Windows: MSYS's `exec` leaves a wrapper
// process holding the channel's pipes, so the server never sends EOF and the row has nothing to wait for.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn a_closing_exchange_is_released_at_its_deadline() {
            let cleanup = 900;
            let rig = Rig::new(stuck_close_fixture(), 1, cleanup);
            let (stream, _) = rig.open(GitService::ReceivePack);
            let (closed_in, result) = rig.push(stream);
            result.expect("the push's result is final at the server's EOF");
            assert!(closed_in < ms(cleanup / 3), "close() took {closed_in:?}");
            thread::sleep(ms(cleanup / 3));
            assert_eq!(
                rig.counts().leased,
                1,
                "released before its bound: {:?}",
                rig.counts()
            );
            // No worker stop: the exchange's own bound, from the handoff, ends it.
            let ended = rig.wait("released at its bound", |counts| counts.total() == 0);
            assert!(
                ended < ms(cleanup) + ms(1_500),
                "the bound was {cleanup} ms and the close ended after {ended:?} more"
            );
        }
    }
}

#[test]
fn a_background_close_that_times_out_is_discarded() {
    // The close's bound is the exchange's cleanup, whatever the stall clock.
    for io_ms in [0, 9_000] {
        let cleanup = 400;
        let rig = Rig::with_io(stuck_close_fixture(), 1, cleanup, io_ms);
        let (stream, _) = rig.open(GitService::UploadPack);
        let closed_in = rig.fetch(stream);
        assert!(
            closed_in < ms(cleanup / 2),
            "io {io_ms}: close() took {closed_in:?}"
        );
        let ended = rig.wait("discarded", |counts| counts.total() == 0);
        assert!(
            ended < ms(cleanup) + ms(2_000),
            "io {io_ms}: the discard took {ended:?} more"
        );
        let (stream, next) = rig.open(GitService::UploadPack);
        assert!(!next.reused, "io {io_ms}: a timed-out close was reused");
        drop(stream);
    }
}

// A shell script cannot end a channel's output and keep its process on Windows: MSYS's `exec` leaves a wrapper
// process holding the channel's pipes, so the server never sends EOF and the row has nothing to wait for.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn shutdown_discards_a_close_in_flight() {
            let rig = Rig::new(stuck_close_fixture(), 1, 5_000);
            let (stream, _) = rig.open(GitService::ReceivePack);
            let (closed_in, result) = rig.push(stream);
            result.expect("the push's result is final at the server's EOF");
            assert!(closed_in < ms(2_000), "close() took {closed_in:?}");
            assert_eq!(rig.counts().leased, 1);
            let watch = rig.endpoint.shutdown_watch();
            let started = Instant::now();
            rig.endpoint.shutdown();
            while !watch.status().cleanup_complete {
                assert!(
                    started.elapsed() < ms(2_500),
                    "shutdown waited for the server: {:?}",
                    watch.status()
                );
                thread::sleep(ms(2));
            }
            assert!(
                started.elapsed() < ms(500),
                "cleanup took {:?} against a close that is 5 s from its bound",
                started.elapsed()
            );
            assert_eq!(watch.status().pending_connections, 0);
        }
    }
}

#[test]
fn discard_after_use_does_not_wait_for_the_server() {
    let delay = ms(1_500);
    let rig = Rig::new(delayed_close_fixture(delay), 1, 5_000);
    let path = rig.fixture.repository.to_str().unwrap().to_owned();
    let context = attachment::deadlines(&config(1, rig.cleanup_ms), rig.io_ms);
    let pending = attachment::start(
        &rig.endpoint,
        key(&rig.fixture),
        Some(identity(&rig.fixture)),
        GitService::ReceivePack,
        &path,
        context.clone(),
    )
    .unwrap();
    let (exchange, _) = attachment::finish(&pending).unwrap();
    exchange.discard_after_use();
    let mut stream = attachment::drive(exchange, &attachment::context(context));
    read_advertisement(&mut stream);
    stream.write_all(b"0000").unwrap();
    stream.end_write().unwrap();
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).unwrap();
    let (closed_in, result) = timed_close(&stream);
    result.expect("the close is clean");
    assert!(closed_in < delay / 3, "close() took {closed_in:?}");
    // Not reused, and not kept for the server's close either.
    let gone = rig.wait("discarded", |counts| counts.total() == 0);
    assert!(
        gone < delay / 2,
        "the discard waited {gone:?} for the server"
    );
}

#[test]
fn late_cancel_and_attachment_drop_do_not_discard_a_clean_close() {
    let delay = ms(600);
    let rig = Rig::new(delayed_close_fixture(delay), 1, 5_000);
    let path = rig.fixture.repository.to_str().unwrap().to_owned();
    let deadlines = attachment::deadlines(&config(1, rig.cleanup_ms), rig.io_ms);
    let pending = attachment::start(
        &rig.endpoint,
        key(&rig.fixture),
        Some(identity(&rig.fixture)),
        GitService::ReceivePack,
        &path,
        deadlines.clone(),
    )
    .unwrap();
    let (exchange, _) = attachment::finish(&pending).unwrap();
    // The member has its Closed, and then cancels and lets go, as a command
    // that is finishing does.
    let mut stream =
        attachment::drive_then(exchange, &attachment::context(deadlines), |exchange| {
            exchange.cancel();
            drop(exchange);
        });
    read_advertisement(&mut stream);
    stream.write_all(b"0000").unwrap();
    stream.end_write().unwrap();
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).unwrap();
    let (closed_in, result) = timed_close(&stream);
    result.expect("the close is clean");
    assert!(closed_in < delay / 2, "close() took {closed_in:?}");
    rig.wait("idle after the close", |counts| counts.idle == 1);
    let (stream, again) = rig.open(GitService::UploadPack);
    assert!(again.reused, "a late cancel discarded a clean close");
    rig.fetch(stream);
}

/// An ls-remote on the rig's URL and identity, which has its result and is
/// closing, and the open of the next exchange on it.
fn closing_fetch(rig: &Rig) {
    let (stream, _) = rig.open(GitService::UploadPack);
    let closed_in = rig.fetch(stream);
    assert!(closed_in < ms(500), "close() took {closed_in:?}");
    let counts = rig.counts();
    assert_eq!(
        (counts.leased, counts.idle),
        (1, 0),
        "the fetch's close is not in the background: {counts:?}"
    );
}

#[test]
fn ls_remote_then_push_reuses_the_closing_connection() {
    let rig = Rig::new(delayed_close_fixture(ms(150)), 32, 5_000);
    closing_fetch(&rig);
    let (stream, push) = rig.open(GitService::ReceivePack);
    assert!(push.reused, "the push opened a second connection");
    assert_eq!(rig.counts().total(), 1);
    rig.push(stream).1.unwrap();
}

#[test]
fn a_close_slower_than_the_wait_bound_opens_a_new_connection() {
    let rig = Rig::new(delayed_close_fixture(ms(600)), 32, 5_000);
    closing_fetch(&rig);
    let started = Instant::now();
    let (stream, push) = rig.open(GitService::ReceivePack);
    let waited = started.elapsed();
    assert!(!push.reused);
    assert_eq!(rig.counts().total(), 2);
    assert!(
        waited >= ms(200),
        "the push opened after {waited:?}, without waiting for the close"
    );
    assert!(waited < ms(2_000), "the push waited {waited:?}");
    rig.push(stream).1.unwrap();
    rig.wait("both idle", |counts| counts.idle == 2);
}

#[test]
fn the_wait_is_bounded_by_the_closes_remaining_cleanup() {
    // The design's 100 ms, less: a bound far below the 250 ms wait leaves the
    // test its margin when a loaded machine is slow to set up a connection.
    let cleanup = 40;
    let rig = Rig::new(stuck_close_fixture(), 32, cleanup);
    closing_fetch(&rig);
    let started = Instant::now();
    let (stream, push) = rig.open(GitService::ReceivePack);
    let waited = started.elapsed();
    assert!(!push.reused);
    assert!(
        waited >= ms(cleanup - 10),
        "the push opened after {waited:?}, without waiting for the close"
    );
    assert!(
        waited < BOUNDED_WAIT_AT_MOST,
        "the push waited {waited:?}, past the close's own bound of {cleanup} ms"
    );
    drop(stream);
}

#[test]
fn one_closing_connection_defers_one_open() {
    // The close ends when the test says so, and the pool's counts say when the
    // second connection began: neither is a duration, so a slow connect on a
    // loaded machine cannot reorder what the test compares. The only clock left
    // is the deferral's own bound, which the test is well inside: it releases
    // the close as soon as the second connection has begun.
    let (fixture, release) = gated_close_fixture();
    let rig = Rig::new(fixture, 32, 5_000);
    closing_fetch(&rig);
    let released = AtomicBool::new(false);
    let opens: Vec<_> = thread::scope(|scope| {
        let jobs: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    let (stream, opened) = rig.open(GitService::ReceivePack);
                    // Read at once, after the open, before anything else can
                    // order against it.
                    (stream, opened, released.load(Ordering::SeqCst))
                })
            })
            .collect();
        // The open that is not deferred starts its connection at once, while
        // the close is held; the deferred one starts none.
        let started = Instant::now();
        while rig.counts().total() != 2 {
            assert!(started.elapsed() < PATIENCE, "no second connection began");
            thread::sleep(ms(1));
        }
        released.store(true, Ordering::SeqCst);
        fs::write(&release, b"").unwrap();
        jobs.into_iter().map(|job| job.join().unwrap()).collect()
    });
    let reused: Vec<_> = opens
        .iter()
        .filter(|(_, opened, _)| opened.reused)
        .collect();
    let fresh = opens.len() - reused.len();
    assert_eq!((reused.len(), fresh), (1, 1), "one reuses, one opens");
    // The deferred open is the one that took the closing connection, and it
    // could not have it while the close was held.
    assert!(reused[0].2, "an open was served before the close ended");
    assert_eq!(rig.counts().total(), 2);
}

#[test]
fn an_open_is_deferred_at_most_once() {
    let rig = Rig::new(delayed_close_fixture(ms(600)), 32, 5_000);
    // Three exchanges, none of them waiting on another, then closed one after
    // another: three connections closing, each for about 600 ms.
    let streams: Vec<_> = (0..3)
        .map(|_| {
            let (mut stream, opened) = rig.open(GitService::UploadPack);
            assert!(!opened.reused);
            read_advertisement(&mut stream);
            stream
        })
        .collect();
    for mut stream in streams {
        stream.write_all(b"0000").unwrap();
        let (closed_in, result) = timed_close(&stream);
        result.expect("a clean close");
        assert!(closed_in < ms(300), "close() took {closed_in:?}");
        thread::sleep(ms(80));
    }
    let started = Instant::now();
    let (stream, _) = rig.open(GitService::ReceivePack);
    let waited = started.elapsed();
    assert!(
        waited >= ms(200),
        "the open took {waited:?}, so it did not wait for a closing connection"
    );
    assert!(
        waited < DEFERRED_ONCE_AT_MOST,
        "the open took {waited:?}: it was deferred more than once"
    );
    drop(stream);
}

#[test]
fn a_different_identity_is_not_deferred() {
    let rig = Rig::new(delayed_close_fixture(ms(600)), 32, 5_000);
    let other = rig.fixture.second_identity();
    closing_fetch(&rig);
    let started = Instant::now();
    let (stream, opened) = rig.open_as(GitService::UploadPack, other);
    let waited = started.elapsed();
    assert!(!opened.reused);
    assert!(
        waited < ms(250),
        "an open for another identity took {waited:?}, as if it waited for the close"
    );
    drop(stream);
}
