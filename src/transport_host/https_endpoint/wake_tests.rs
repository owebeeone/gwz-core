//! The endpoint's own thread ends after the shutdown, and the placement
//! thread that steps the endpoint learns that by being woken, not at its next
//! timer tick (TR8.1: every endpoint session carries this endpoint, so each
//! command's shutdown waited for the tick).
use super::*;
use crate::git::endpoint::https_connection;
use std::{
    sync::atomic::AtomicUsize,
    task::{Wake, Waker},
    thread,
    time::Instant,
};

/// Counts the times it is woken.
#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn the_endpoint_wakes_its_stepper_when_its_shutdown_has_settled() {
    let config = pool::Config::default();
    let authority = Authority::new(config.total, config.per_host);
    let mut endpoint = HttpsEndpoint::new(
        HttpsEndpointConfig {
            tls: https_connection::Config::default(),
            auth: None,
        },
        config,
        3_000,
        authority,
        "endpoint".into(),
        HelperSlots::new(),
    )
    .unwrap();
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes.clone());
    endpoint.step(0, &mut Context::from_waker(&waker)).unwrap();
    let before = wakes.0.load(Ordering::SeqCst);
    endpoint.shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    while endpoint.pending() != 0 || wakes.0.load(Ordering::SeqCst) == before {
        assert!(
            Instant::now() < deadline,
            "the endpoint's shutdown settled without waking its stepper"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
