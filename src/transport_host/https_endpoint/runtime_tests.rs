//! The endpoint's tasks run on several threads (decision D33, run `2026-10-17-https611`). A connection's setup
//! does synchronous CPU work inside a poll (the TLS stack's handshake and its certificate-chain check, about 4 ms on
//! Windows), so on one thread the setups of N members queue behind one another. The tests stand a task that
//! blocks its thread in for that work and ask whether the endpoint's real runtime runs several at once.
use super::*;
use crate::git::endpoint::https_connection;
use crate::git::endpoint::rendezvous::Gate;

/// The tasks the test blocks at once: more than one, so that there are others to hold, and no more than the
/// runtime has workers, whatever `RUNTIME_WORKERS` is later set to. The probe count is not the constant under
/// test: a constant of 1 would make the probe pass on any runtime.
const PROBES: usize = 2;
const _: () = assert!(RUNTIME_WORKERS >= PROBES);

fn endpoint() -> HttpsEndpoint {
    let config = pool::Config::default();
    let authority = Authority::new(config.total, config.per_host);
    HttpsEndpoint::new(
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
    .unwrap()
}

#[test]
fn one_task_that_blocks_its_thread_does_not_hold_the_others_of_the_endpoint() {
    let endpoint = endpoint();
    let gate = Gate::new(PROBES);
    let tasks: Vec<_> = (0..PROBES)
        .map(|_| {
            let gate = gate.clone();
            endpoint
                .runtime
                .spawn(async move { gate.pass(Duration::from_secs(5)) })
        })
        .collect();
    let (done, results) = std::sync::mpsc::channel();
    for task in tasks {
        let done = done.clone();
        endpoint.runtime.spawn(async move {
            let _ = done.send(task.await.unwrap_or(false));
        });
    }
    drop(done);
    let outcomes: Vec<bool> = results.iter().collect();
    assert_eq!(
        outcomes,
        vec![true; PROBES],
        "{PROBES} tasks that each block their thread until all are inside must all get through: \
         the endpoint's setups were serialized on one thread"
    );
}
