//! The endpoint's tasks run on several threads (decision D33, run `2026-10-17-https611`). A connection's setup
//! does synchronous CPU work inside a poll (the TLS stack's handshake and its certificate-chain check, about 4 ms on
//! Windows), so on one thread the setups of N members queue behind one another. The tests stand a task that
//! blocks its thread in for that work and ask whether the endpoint's real runtime runs several at once.
use super::*;
use crate::git::endpoint::https_connection;
use std::sync::{Condvar, Mutex};

/// Blocks each caller until `members` callers are inside, or `patience` has passed: with enough
/// threads every caller returns as the last one enters; with too few the first ones can never be released.
struct Gate {
    state: Mutex<(usize, bool)>,
    changed: Condvar,
    members: usize,
}
impl Gate {
    fn new(members: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new((0, false)),
            changed: Condvar::new(),
            members,
        })
    }
    /// True if all the members were inside at once.
    fn pass(&self, patience: Duration) -> bool {
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        self.changed.notify_all();
        let deadline = std::time::Instant::now() + patience;
        while state.0 < self.members && !state.1 {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                // Give up for everyone: the ones after it do not wait their turn.
                state.1 = true;
                self.changed.notify_all();
                return false;
            }
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        state.0 >= self.members
    }
}

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
    let gate = Gate::new(RUNTIME_WORKERS);
    let tasks: Vec<_> = (0..RUNTIME_WORKERS)
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
        vec![true; RUNTIME_WORKERS],
        "{RUNTIME_WORKERS} tasks that each block their thread until all are inside must all get through: \
         the endpoint's setups were serialized on one thread"
    );
}
