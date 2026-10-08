//! A pool's supervisor sleeps until something happens to the pool or one of
//! its deadlines falls due; it does not look every 2 ms (TR8.1: the 2 ms
//! sleep was how a finished setup, a release and a shutdown were noticed).
//! The tests count the supervisor's turns, which a fixed sleep inflates.
use super::*;
use crate::git::endpoint::{
    https_connection::HttpConnector,
    https_fixture::{Server, response},
    https_pool::RunningPool,
    shared_reservation::Authority,
};
use tokio::{sync::Semaphore, time::timeout};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn server() -> Server {
    Server::start(Arc::new(|_| {
        Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
    }))
    .await
}

fn key_of(server: &Server) -> Key {
    let destination = Destination::parse(&server.url).unwrap();
    Key::https(destination.host(), destination.port())
}

fn authority() -> Authority {
    let config = pool::Config::default();
    Authority::new(config.total, config.per_host)
}

/// A pool whose connector has `slots` setup slots.
fn pool_with_slots(server: &Server, slots: Arc<Semaphore>) -> RunningPool {
    let authority = authority();
    let supervisor = authority.supervisor().clone();
    let config = server.config();
    RunningPool::with_connector(pool::Config::default(), authority, move |epoch| {
        HttpConnector::new(config, epoch, slots, supervisor)
    })
    .unwrap()
}

async fn checkout(pool: &RunningPool, server: &Server) -> Result<HttpLease, Failure> {
    pool.client
        .checkout(
            key_of(server),
            Owner::new("session", "operation"),
            20_000,
            20_000,
            &CancellationToken::new(),
        )
        .await
        .map_err(|(failed, _)| failed)
}

#[test]
fn an_idle_pool_takes_no_turns() {
    runtime().block_on(async {
        let pool = RunningPool::with_authority(
            pool::Config::default(),
            https_connection::Config::default(),
            authority(),
        )
        .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let steps = pool.client.steps();
        assert!(
            steps <= 2,
            "an idle pool's supervisor took {steps} turns in 100 ms"
        );
    });
}

#[test]
fn a_connection_waiting_for_a_setup_slot_takes_no_turns_and_goes_on_when_one_frees() {
    runtime().block_on(async {
        let server = server().await;
        let slots = Arc::new(Semaphore::new(1));
        let held = slots.clone().try_acquire_owned().unwrap();
        let mut pool = pool_with_slots(&server, slots);
        let waiting = {
            let (pool, server) = (pool.client.clone(), &server);
            let (key, cancel) = (key_of(server), CancellationToken::new());
            tokio::spawn(async move {
                pool.checkout(
                    key,
                    Owner::new("session", "operation"),
                    20_000,
                    20_000,
                    &cancel,
                )
                .await
                .map_err(|(failed, _)| failed)
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiting.is_finished(), "no setup slot was free");
        let steps = pool.client.steps();
        assert!(
            steps <= 8,
            "a pool waiting on a setup slot took {steps} turns in 100 ms"
        );
        drop(held);
        let lease = timeout(Duration::from_secs(10), waiting)
            .await
            .expect("the freed slot never moved the connection on")
            .unwrap()
            .expect("the connection connects");
        lease.finish(Disposition::Discarded).unwrap();
        assert_eq!(pool.shutdown(Duration::from_secs(5)).await, 0);
    });
}

#[test]
fn a_pool_with_an_idle_connection_settles_its_shutdown_at_once() {
    runtime().block_on(async {
        let server = server().await;
        let mut pool = pool_with_slots(&server, Arc::new(Semaphore::new(8)));
        let lease = checkout(&pool, &server).await.expect("connects");
        lease.finish(Disposition::Reusable).unwrap();
        let began = Instant::now();
        assert_eq!(pool.shutdown(Duration::from_secs(5)).await, 0);
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "the shutdown of an idle pool took {:?}",
            began.elapsed()
        );
    });
}

#[test]
fn a_shutdown_ends_a_connection_still_waiting_for_a_setup_slot() {
    runtime().block_on(async {
        let server = server().await;
        let slots = Arc::new(Semaphore::new(1));
        let _held = slots.clone().try_acquire_owned().unwrap();
        let mut pool = pool_with_slots(&server, slots);
        let waiting = {
            let (pool, key) = (pool.client.clone(), key_of(&server));
            tokio::spawn(async move {
                pool.checkout(
                    key,
                    Owner::new("session", "operation"),
                    20_000,
                    20_000,
                    &CancellationToken::new(),
                )
                .await
                .map(|lease| lease.finish(Disposition::Discarded))
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let began = Instant::now();
        assert_eq!(pool.shutdown(Duration::from_secs(5)).await, 0);
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "{:?}",
            began.elapsed()
        );
        assert!(
            waiting.await.unwrap().is_err(),
            "the waiting connection was served"
        );
    });
}

/// A lease held when the shutdown begins is ended by it, and the lease's own
/// release (which then finds its connection gone) is what lets the pool end.
#[test]
fn a_shutdown_with_a_lease_out_settles_when_the_lease_is_released() {
    runtime().block_on(async {
        let server = server().await;
        let mut pool = pool_with_slots(&server, Arc::new(Semaphore::new(8)));
        let lease = checkout(&pool, &server).await.expect("connects");
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            let _ = lease.finish(Disposition::Discarded);
        });
        let began = Instant::now();
        assert_eq!(pool.shutdown(Duration::from_secs(5)).await, 0);
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "the release did not settle the shutdown: {:?}",
            began.elapsed()
        );
        release.await.unwrap();
    });
}
