//! TR2.9: streams that close together finish together.
//!
//! The fixture delays each channel's close by `CLOSE_DELAY`: after the
//! server's upload-pack exits, its forced command closes stdout and stderr and
//! waits before it exits, so the server's exit status and channel close
//! arrive that much later. N exchanges, each on its own connection, then
//! close at once, as a workspace fetch's members do. A member's result is
//! final when libgit2 closes, so each `close()` returns well under
//! `CLOSE_DELAY`; the connections' graceful closes overlap off the members'
//! path, and every connection is back in the pool, reusable, `CLOSE_DELAY`
//! after the exchange (GwzTransportSshBackgroundCloseDesign §10).
use super::driver_tests::{block_on, commit, endpoint_home, fixture_url, local_meta};
use super::*;
use crate::git::endpoint::{
    ssh_channel::GitService,
    ssh_close_fixture::{delayed_close_fixture, read_advertisement},
};
use gwz_transport::{protocol::Disposition, stream::CloseResult};
use std::{
    io::{self, Read, Write},
    sync::Barrier,
    thread,
    time::{Duration, Instant},
};

const CLOSE_DELAY: Duration = Duration::from_secs(1);

struct Closed {
    released: Instant,
    finished: Instant,
    result: io::Result<CloseResult>,
}

/// Opens `count` upload-pack exchanges, one connection each, reads each past
/// its advertisement, then closes them all at once as libgit2's smart
/// transport closes a fetch: a flush-pkt, then the stream's close.
fn assert_closes_overlap(count: usize) {
    let fixture = delayed_close_fixture(CLOSE_DELAY);
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    commit(&server, "advertised");
    let home = endpoint_home(&fixture);
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let meta = local_meta(&format!("close-together-{count}"), &home);
    let request = block_on(runtime.request(meta, "fetch".into())).unwrap();
    let identity = home.join("client_ed25519").to_string_lossy().into_owned();
    // Opened one at a time: this test times the closes, and the fixture's
    // sshd refuses some setups beyond ten unauthenticated at once.
    let streams: Vec<_> = (0..count)
        .map(|_| {
            let mut stream = request
                .context
                .open(
                    &fixture_url(&fixture),
                    GitService::UploadPack,
                    Some(identity.clone()),
                    Arc::new(|_, _| {}),
                    Arc::new(|_| {}),
                )
                .unwrap();
            read_advertisement(&mut stream);
            stream
        })
        .collect();
    let start = Barrier::new(count);
    let closed: Vec<Closed> = thread::scope(|threads| {
        let jobs: Vec<_> = streams
            .into_iter()
            .map(|mut stream| {
                let start = &start;
                threads.spawn(move || {
                    start.wait();
                    let released = Instant::now();
                    let result = stream.write_all(b"0000").and_then(|()| stream.close());
                    Closed {
                        released,
                        finished: Instant::now(),
                        result,
                    }
                })
            })
            .collect();
        jobs.into_iter().map(|job| job.join().unwrap()).collect()
    });
    let first = closed.iter().map(|close| close.released).min().unwrap();
    let mut ends: Vec<_> = closed
        .iter()
        .map(|close| close.finished.duration_since(first).as_millis())
        .collect();
    ends.sort_unstable();
    let elapsed = closed
        .iter()
        .map(|close| close.finished.duration_since(first))
        .max()
        .unwrap();
    let bound = CLOSE_DELAY / 2;
    eprintln!("{count} closes, each delayed {CLOSE_DELAY:?}: {elapsed:?}; ends in ms: {ends:?}");
    assert!(
        elapsed < bound,
        "{count} closes started together took {elapsed:?}, past {bound:?}: a member \
         waited for the server's close, delayed {CLOSE_DELAY:?}; their ends in ms: {ends:?}"
    );
    // Every member is done, and no connection is: each stays leased, and
    // counted, until its own close ends.
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    let counts = endpoint.ssh_counts_for_test().unwrap();
    assert_eq!((counts.leased, counts.idle), (count, 0), "{counts:?}");
    for close in &closed {
        let result = close.result.as_ref().map_err(io::Error::kind);
        assert_eq!(
            result.map(|result| result.disposition),
            Ok(Disposition::Reusable),
            "every exchange's close is clean; the connection's reuse is the lease's, below"
        );
    }
    // The endpoint's worker keeps each connection leased, and counted, until
    // its graceful close ends, and then returns it to the pool reusable.
    let deadline = Instant::now() + Duration::from_secs(5);
    let counts = loop {
        let counts = endpoint.ssh_counts_for_test().unwrap();
        if (counts.idle, counts.leased) == (count, 0) || Instant::now() >= deadline {
            break counts;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!((counts.idle, counts.leased), (count, 0), "{counts:?}");
    assert_eq!(block_on(request.finish()).pending_local_work, 0);
    block_on(runtime.shutdown());
    assert!(!fixture.marker.exists());
}

#[test]
fn eight_streams_closing_together_wait_out_one_delayed_close() {
    assert_closes_overlap(8);
}

#[test]
fn thirty_two_streams_closing_together_wait_out_one_delayed_close() {
    assert_closes_overlap(32);
}

/// A command that has all its results does not wait for a connection's close
/// to end: its request finishes, and its runtime shuts down, with a close in
/// flight, which is discarded.
#[test]
fn command_exit_does_not_wait_for_a_close_in_flight() {
    let delay = Duration::from_secs(3);
    let fixture = delayed_close_fixture(delay);
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    commit(&server, "advertised");
    let home = endpoint_home(&fixture);
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let meta = local_meta("command-exit", &home);
    let request = block_on(runtime.request(meta, "push".into())).unwrap();
    let identity = home.join("client_ed25519").to_string_lossy().into_owned();
    let mut stream = request
        .context
        .open(
            &fixture_url(&fixture),
            GitService::ReceivePack,
            Some(identity),
            Arc::new(|_, _| {}),
            Arc::new(|_| {}),
        )
        .unwrap();
    read_advertisement(&mut stream);
    stream.write_all(b"0000").unwrap();
    stream.end_write().unwrap();
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).unwrap();
    let started = Instant::now();
    stream
        .close()
        .expect("the push's result is final at the server's EOF");
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
    let counts = endpoint.ssh_counts_for_test().unwrap();
    assert_eq!((counts.leased, counts.idle), (1, 0), "{counts:?}");
    assert_eq!(block_on(request.finish()).pending_local_work, 0);
    block_on(runtime.shutdown());
    assert!(
        started.elapsed() < delay / 2,
        "the command's exit took {:?}, with a close {delay:?} from ending",
        started.elapsed()
    );
    assert!(!fixture.marker.exists());
}
