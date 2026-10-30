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
    ssh_close_fixture::{delayed_close_fixture, gated_close_server, read_advertisement},
};
use gwz_transport::{protocol::Disposition, stream::CloseResult};
use std::{
    io::{self, Read, Write},
    sync::{Barrier, mpsc},
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
    let deadline = Instant::now() + Duration::from_secs(30);
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

/// How long a command's exit may take, from when it begins, with a close held in flight. The product does not wait for
/// ever for a close: an Open grants its endpoint `OPEN_CLEANUP_MS` (5 s, `session/driver.rs`) for it, the endpoint
/// discards a connection whose close is still open that long after the handoff, and the pool's `cleanup_timeout_ms`
/// can only tighten that bound (`validate_deadlines`), not extend it. So a command that waited for the close would
/// finish at about 5 s, not hang. The bound is half of that, and nothing ties it to `OPEN_CLEANUP_MS`: a command that
/// waits for the close is caught only if its exit begins less than `OPEN_CLEANUP_MS` - `EXIT_BOUND` after the handoff
/// (2.5 s today), so a lower `OPEN_CLEANUP_MS` narrows that window and one at `EXIT_BOUND` closes it. It is one-sided: a healthy exit takes
/// milliseconds (the whole process, fixture setup included, took at most 4.7 s in 102 runs with 18 CPU burners and
/// 2.0 s in 600 with 24 on dabeest; the exit alone was not timed).
const EXIT_BOUND: Duration = Duration::from_millis(2_500);

/// A command that has all its results does not wait for a connection's close
/// to end: its request finishes, and its runtime shuts down, with a close in
/// flight, which is discarded.
///
/// The server holds the close in flight until the test releases it, after the request and the runtime are done, so
/// no timer of the server's ends it (one did, on a hosted Windows runner: `(leased, idle)` was `(0, 1)`). The product
/// itself ends it 5 s after the handoff (see [`EXIT_BOUND`]), so the count below must be read within that window:
/// 30 ms was the window of the timer, and about 5 s is this one. A read stalled past it fails on the count, `(0, 0)`.
/// A command that waited for the close fails on [`EXIT_BOUND`], provided its exit began less than `OPEN_CLEANUP_MS` -
/// `EXIT_BOUND` (2.5 s) after the handoff: past that the product's own bound leaves it less than `EXIT_BOUND` to wait.
#[test]
fn command_exit_does_not_wait_for_a_close_in_flight() {
    // The test-controlled server: this test reads the output to its end, so the server must send its EOF at once
    // and close later, which `sshd` on Windows cannot do (the EOF comes with the exit).
    let fixture = gated_close_server();
    let server = git2::Repository::open_bare(&fixture.repository).unwrap();
    commit(&server, "advertised");
    let home = fixture.temp.path().join("endpoint-home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::copy(&fixture.known_hosts, home.join(".ssh/known_hosts")).unwrap();
    std::fs::copy(
        fixture.temp.path().join("client_ed25519"),
        home.join("client_ed25519"),
    )
    .unwrap();
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let meta = local_meta("command-exit", &home);
    let request = block_on(runtime.request(meta, "push".into())).unwrap();
    let identity = home.join("client_ed25519").to_string_lossy().into_owned();
    let url = format!(
        "ssh://{}@127.0.0.1:{}{}",
        fixture.user,
        fixture.port,
        crate::git::endpoint::ssh_fixture::server_path(&fixture.repository)
    );
    let mut stream = request
        .context
        .open(
            &url,
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
    // The close is held, so a finish or a shutdown that waited for it would wait until the product ends the close, at
    // its 5 s bound: this ends the wait at `EXIT_BOUND`, and no healthy run is near it. The gate is opened before the
    // failure is reported, so the failure does not wait for the close either.
    let (done, finished) = mpsc::channel();
    thread::scope(|threads| {
        threads.spawn(move || {
            assert_eq!(block_on(request.finish()).pending_local_work, 0);
            block_on(runtime.shutdown());
            let _ = done.send(());
        });
        let waited = finished.recv_timeout(EXIT_BOUND);
        fixture.release_closes();
        match waited {
            Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                panic!(
                    "the command's exit had not finished {EXIT_BOUND:?} after it began, with a close held in \
                     flight: it waited for the close"
                );
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!(
                    "the command's exit ended without finishing: its thread panicked (its own message is above), \
                     which is not a wait for the close"
                );
            }
        }
    });
    assert!(!fixture.marker.exists());
}
