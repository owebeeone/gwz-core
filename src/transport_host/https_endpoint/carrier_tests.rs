//! The test of a site's limit is given back on the HTTPS carrier's exit that
//! ends its open before an attempt starts: the open's allocation ran out
//! (adaptive concurrency design §5.2). A wave of four opens is refused, which
//! opens a confirmation, and a fifth open held behind the wave is the first
//! offered the confirming test. Its allocation is gone when the refusals are
//! processed, so it fails without an attempt, and the test must go back to the
//! wave's members: a gate left shut would hang the site.
use super::{
    retry_tests::{closing, code, endpoint, open, runtime, settle, shut},
    *,
};
use crate::git::endpoint::https_connection;
use std::sync::atomic::Ordering;

#[test]
fn a_carrier_whose_allocation_ran_out_gives_the_test_back() {
    runtime().block_on(async {
        let (port, accepted) = closing();
        let mut endpoint = endpoint(https_connection::Config::default(), 4, 3);
        let mut cx = Context::from_waker(std::task::Waker::noop());
        // The wave: four setups against a host that closes every connection.
        for stream in 2..=5 {
            endpoint
                .accept("request".into(), open(stream, port, 30_000))
                .unwrap();
        }
        endpoint.step(0, &mut cx).unwrap();
        // Stream 1 is held behind the full per-host limit, with an allocation
        // of 5 ms that runs while it is held.
        endpoint.accept("request".into(), open(1, port, 5)).unwrap();
        endpoint.step(0, &mut cx).unwrap();
        // The wave's connects are refused while no pass looks; the next pass,
        // 100 ms on, finds the refusals, a confirmation, and stream 1 first in
        // line for the confirming test with nothing left of its allocation.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let published = settle(&mut endpoint, 100).await;
        let late = published
            .iter()
            .find(|message| message.stream_id == 1)
            .expect("stream 1 ended");
        assert_eq!(code(late), ErrorCode::Timeout);
        assert_eq!(
            accepted.load(Ordering::Acquire),
            5,
            "the confirming test went to a member of the wave"
        );
        shut(&mut endpoint).await;
    });
}
