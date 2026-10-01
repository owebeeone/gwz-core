//! TR2.9: streams that close together finish together.
//!
//! The fixture delays each channel's close by `CLOSE_DELAY`: after the
//! server's upload-pack exits, its forced command closes stdout and stderr and
//! waits before it exits, so the server's exit status and channel close
//! arrive that much later. N exchanges, each on its own connection, then
//! close at once, as a workspace fetch's members do. The closes must overlap:
//! all of them complete within 2 × `CLOSE_DELAY` plus `MARGIN`, for 8 streams
//! as for 32, never N × `CLOSE_DELAY`.
use super::driver_tests::{block_on, commit, common, endpoint_home, fixture_url, local_meta};
use super::*;
use crate::git::endpoint::{ssh_channel::GitService, stream_io::BlockingStream};
use gwz_transport::{protocol::Disposition, stream::CloseResult};
use std::{
    io::{self, Read, Write},
    sync::Barrier,
    thread,
    time::{Duration, Instant},
};

const CLOSE_DELAY: Duration = Duration::from_secs(1);
const MARGIN: Duration = Duration::from_secs(2);

/// An SSH fixture whose every channel closes `CLOSE_DELAY` after its Git
/// service exits. The output ends first, so only the close waits.
fn delayed_close_fixture() -> common::SshdFixture {
    let fixture = common::SshdFixture::new();
    let script = fixture.temp.path().join("delayed-close.sh");
    crate::git::endpoint::helper_script::write_helper_script(
        &script,
        &format!(
            "eval \"$SSH_ORIGINAL_COMMAND\"\nstatus=$?\nexec 1>&- 2>&-\nsleep {}\nexit $status\n",
            CLOSE_DELAY.as_secs()
        ),
    );
    let public = std::fs::read_to_string(fixture.temp.path().join("client_ed25519.pub")).unwrap();
    std::fs::write(
        fixture.temp.path().join("authorized_keys"),
        format!("command=\"{}\" {}", script.display(), public),
    )
    .unwrap();
    fixture
}

/// Reads one pkt-line advertisement through its closing flush-pkt.
fn read_advertisement(stream: &mut BlockingStream) {
    loop {
        let mut length = [0_u8; 4];
        stream.read_exact(&mut length).unwrap();
        let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
        if length == 0 {
            return;
        }
        let mut line = vec![0_u8; length - 4];
        stream.read_exact(&mut line).unwrap();
    }
}

struct Closed {
    released: Instant,
    finished: Instant,
    result: io::Result<CloseResult>,
}

/// Opens `count` upload-pack exchanges, one connection each, reads each past
/// its advertisement, then closes them all at once as libgit2's smart
/// transport closes a fetch: a flush-pkt, then the stream's close.
fn assert_closes_overlap(count: usize) {
    let fixture = delayed_close_fixture();
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
    let bound = 2 * CLOSE_DELAY + MARGIN;
    eprintln!("{count} closes, each delayed {CLOSE_DELAY:?}: {elapsed:?}; ends in ms: {ends:?}");
    assert!(
        elapsed < bound,
        "{count} closes started together took {elapsed:?}, past {bound:?}, \
         each delayed {CLOSE_DELAY:?}; their ends in ms: {ends:?}"
    );
    for close in &closed {
        let result = close.result.as_ref().map_err(io::Error::kind);
        assert_eq!(
            result.map(|result| result.disposition),
            Ok(Disposition::Reusable),
            "every close is clean"
        );
    }
    // The endpoint's worker returns each connection to the pool as it sends
    // that exchange's Closed, after the clean close it reports.
    let endpoint = runtime
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .local_endpoint
        .clone();
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
