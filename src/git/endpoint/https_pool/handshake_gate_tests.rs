//! The real pool starts several connections' setups at once (decision D33, run `2026-10-17-https611`). The handshake
//! of a connection is synchronous CPU inside one poll of its connect task; it must be polled by that task, on a
//! worker of its own, and never by the pool host's step, which holds the host's lock: driven there, the setups of N
//! members would run one after another on whichever thread steps the host, however many workers the runtime has.
//! The test puts a gate in the handshake's place (`Config::handshake_hook`): N setups that each block their thread
//! until all N are inside get through only if no lock holds the others out.
use super::*;
use crate::git::endpoint::{
    https_connection::HandshakeHook, https_fixture::Server, rendezvous::Gate,
};

/// The setups started at once. The test runtime has one worker more, for the pool's supervisor.
const SETUPS: usize = 3;

#[test]
fn n_setups_through_the_real_pool_run_their_handshake_work_at_once() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(SETUPS + 1)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = Server::start(Arc::new(|_| {
                Box::pin(async {
                    crate::git::endpoint::https_fixture::response(
                        200,
                        gwz_transport::protocol::GitService::UploadPackAdvertisement,
                        "ok",
                    )
                })
            }))
            .await;
            let port: u16 = server
                .url
                .rsplit(':')
                .next()
                .unwrap()
                .trim_end_matches("/repo")
                .parse()
                .unwrap();
            let gate = Gate::new(SETUPS);
            let outcomes = Arc::new(Mutex::new(Vec::new()));
            let mut config = server.config();
            config.handshake_hook = HandshakeHook::new({
                let (gate, outcomes) = (gate.clone(), outcomes.clone());
                Arc::new(move || {
                    // `block_in_place` hands the worker's queue (its LIFO slot too) to another thread before this
                    // one blocks, so that what the test sees is the pool's locking and not which worker a spawned
                    // task happened to be queued on; the thread is blocked all the same.
                    let through =
                        tokio::task::block_in_place(|| gate.pass(Duration::from_secs(5)));
                    outcomes.lock().unwrap().push(through);
                })
            });
            let running =
                RunningPool::with_authority(pool::Config::default(), config, Authority::new(8, 8))
                    .unwrap();
            let pool = running.client.clone();
            // The operation's limit machine starts saturated at the ceiling, as an endpoint's admission arms it; a pool
            // without one starts its connections to a host one at a time while the first settles.
            pool.governor()
                .begin_operation("operation", 32, false, pool.now());
            let cancel = CancellationToken::new();
            let setups: Vec<_> = (0..SETUPS)
                .map(|n| {
                    let (pool, cancel) = (pool.clone(), cancel.clone());
                    tokio::spawn(async move {
                        pool.checkout_scoped(
                            Key::https("localhost", port),
                            Owner::new("session", format!("operation-{n}")),
                            30_000,
                            30_000,
                            &cancel,
                            None,
                            true,
                            false,
                            Some(format!("operation\u{1f}member-{n}")),
                        )
                        .await
                    })
                })
                .collect();
            for setup in setups {
                let lease = setup.await.unwrap().expect("the setup completes");
                lease.finish(Disposition::Discarded).unwrap();
            }
            let outcomes = outcomes.lock().unwrap().clone();
            assert_eq!(
                outcomes,
                vec![true; SETUPS],
                "{SETUPS} setups that each block their thread until all are inside must all get through: \
                 their handshake work was serialized"
            );
        });
}
