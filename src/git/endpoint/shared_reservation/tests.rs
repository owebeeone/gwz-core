use super::super::ssh_pool::{Connector, Resource};
use super::super::ssh_worker::ChannelResource;
use super::super::{ssh_channel::GitService, ssh_pump::SshPump};
use super::{Authority, ReservedConnector};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{Effect, ErrorCode, Failure},
};
use std::{
    io,
    task::{Context, Poll, Waker},
};

struct FakeConnector {
    dispose_pending: bool,
    fail: bool,
}
struct FakeResource {
    dispose_pending: bool,
}
impl Connector for FakeConnector {
    type Resource = FakeResource;
    fn start(
        &mut self,
        _key: &Key,
        _identity: &Identity,
        _deadline: Option<u64>,
    ) -> Result<Self::Resource, Failure> {
        if self.fail {
            return Err(Failure {
                detail: None,
                setup_cause: None,
                code: ErrorCode::Io,
                effect: Effect::None,
                facts: None,
            });
        }
        Ok(FakeResource {
            dispose_pending: self.dispose_pending,
        })
    }
}
impl Resource for FakeResource {
    fn poll_connected(&mut self, _cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        Poll::Ready(Ok(Some(Identity::Ambient)))
    }
    fn poll_dispose(&mut self, _cx: &mut Context<'_>, _force: bool) -> Poll<io::Result<()>> {
        if self.dispose_pending {
            self.dispose_pending = false;
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }
    fn reusable(&self) -> bool {
        false
    }
    fn tcp_connect_ms(&self) -> Option<u64> {
        Some(42)
    }
    fn poll_idle_lost(&mut self, _cx: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
    fn poll_hung_up(&mut self, _cx: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}
impl ChannelResource for FakeResource {
    fn observation(&self) -> (bool, gwz_transport::protocol::Facts) {
        (false, gwz_transport::protocol::Facts::default())
    }
    fn start_exchange(
        &mut self,
        _stream: gwz_transport::stream::Stream,
        _endpoint: gwz_transport::stream::MessageEndpoint,
        _service: GitService,
        _path: &str,
    ) -> io::Result<()> {
        Ok(())
    }
    fn pump(&mut self) -> Option<&mut SshPump<super::super::ssh_channel::SshChannel>> {
        None
    }
    fn reclaim(&mut self) -> bool {
        true
    }
}

#[test]
fn ssh_and_https_share_host_and_total_limits() {
    let authority = Authority::new(2, 1);
    let ssh = authority.try_reserve("github.example").unwrap();
    assert!(authority.try_reserve("github.example").is_none());
    let https = authority.try_reserve("gitlab.example:443").unwrap();
    assert!(authority.try_reserve("other.example:443").is_none());
    assert_eq!(authority.counts("github.example"), (2, 1));
    drop(ssh);
    assert!(authority.try_reserve("github.example").is_some());
    drop(https);
}

#[test]
fn a_reserved_resource_reports_the_socket_connect_time_of_the_one_it_wraps() {
    let mut connector = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        Authority::new(1, 1),
    );
    let key = Key::ssh("git", "github.example", 22);
    let started = connector.start(&key, &Identity::Ambient, None).unwrap();
    assert_eq!(started.tcp_connect_ms(), Some(42));
    // A connection still waiting for its reservation has connected nothing.
    let waiting = connector.start(&key, &Identity::Ambient, None).unwrap();
    assert_eq!(waiting.tcp_connect_ms(), None);
}

#[test]
fn disposal_releases_the_shared_slot_for_another_scheme() {
    let authority = Authority::new(1, 1);
    let ssh = authority.try_reserve("github.example").unwrap();
    assert!(authority.try_reserve("github.example").is_none());
    drop(ssh);
    let https = authority.try_reserve("github.example").unwrap();
    assert_eq!(authority.counts("github.example"), (1, 1));
    drop(https);
    assert_eq!(authority.counts("github.example"), (0, 0));
}

#[test]
fn reservation_survives_pending_disposal_until_completion() {
    let authority = Authority::new(1, 1);
    let mut ssh = ReservedConnector::new(
        FakeConnector {
            dispose_pending: true,
            fail: false,
        },
        authority.clone(),
    );
    let key = Key::ssh("git", "github.example", 22);
    let mut resource = ssh.start(&key, &Identity::Ambient, None).unwrap();
    assert!(!resource.observation().0);
    assert!(resource.reclaim());
    assert!(resource.pump().is_none());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(matches!(
        resource.poll_dispose(&mut cx, false),
        Poll::Pending
    ));
    let mut https = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        authority.clone(),
    );
    // The endpoint-wide reservation is full: the second scheme's
    // connection waits for it, it is not refused (§7.5).
    let mut waiting = https
        .start(&Key::https("github.example", 443), &Identity::Https, None)
        .expect("a full reservation queues the connection; it is not refused");
    assert!(matches!(waiting.poll_connected(&mut cx), Poll::Pending));
    assert_eq!(authority.counts("github.example"), (1, 1));
    assert!(matches!(
        resource.poll_dispose(&mut cx, false),
        Poll::Ready(Ok(()))
    ));
    assert!(matches!(
        waiting.poll_connected(&mut cx),
        Poll::Ready(Ok(_))
    ));
    assert_eq!(authority.counts("github.example"), (1, 1));
    assert!(matches!(
        waiting.poll_dispose(&mut cx, false),
        Poll::Ready(Ok(()))
    ));
    assert_eq!(authority.counts("github.example"), (0, 0));
}

#[test]
fn a_waiting_connection_is_woken_by_a_released_reservation_and_disposes_without_one() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Count(AtomicUsize);
    impl std::task::Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let authority = Authority::new(1, 1);
    let held = authority.try_reserve("host").unwrap();
    let mut connector = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        authority.clone(),
    );
    let mut waiting = connector
        .start(&Key::https("host", 443), &Identity::Https, None)
        .unwrap();
    let woken = Arc::new(Count(AtomicUsize::new(0)));
    let waker = Waker::from(woken.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(matches!(waiting.poll_connected(&mut cx), Poll::Pending));
    assert_eq!(woken.0.load(Ordering::SeqCst), 0);
    drop(held);
    assert_eq!(
        woken.0.load(Ordering::SeqCst),
        1,
        "the release wakes the waiter"
    );
    // A waiter disposed before it was served never held a reservation.
    assert!(matches!(
        waiting.poll_dispose(&mut cx, false),
        Poll::Ready(Ok(()))
    ));
    assert_eq!(authority.counts("host"), (0, 0));
}

#[test]
fn an_admitted_limit_above_the_ceilings_is_neither_clamped_nor_refused() {
    // Reuse design section 5: a new connection needs fewer than max(ceiling, its limit) reservations, so a lone
    // operation at 16 per host gets 16 where the ceilings are 2 (the no-clamp rule).
    let authority = Authority::new(2, 2);
    authority.admit_operation("lone", 16, 16);
    let held: Vec<_> = (0..16)
        .map(|_| authority.try_reserve("host").expect("within its own limit"))
        .collect();
    assert!(authority.try_reserve("host").is_none());
    assert_eq!(authority.counts("host"), (16, 16));
    drop(held);
}

#[test]
fn admitting_a_lower_limit_neither_shrinks_the_ceilings_nor_touches_a_reservation() {
    // Reserve-before-connect under two differing limits: the second operation (4) is admitted while the first (16)
    // holds reservations; nothing is refused, resized or released, and the physical ceiling stays the larger.
    let authority = Authority::new(2, 2);
    authority.admit_operation("large", 16, 16);
    let first: Vec<_> = (0..10)
        .map(|_| authority.try_reserve("host").unwrap())
        .collect();
    authority.admit_operation("small", 4, 4);
    assert_eq!(authority.counts("host"), (10, 10));
    let more: Vec<_> = (0..6)
        .map(|_| authority.try_reserve("host").unwrap())
        .collect();
    assert!(authority.try_reserve("host").is_none());
    drop(first);
    drop(more);
    assert!(authority.try_reserve("host").is_some());
}

#[test]
fn a_connection_waits_for_a_reservation_under_a_higher_limit_too() {
    let authority = Authority::new(1, 1);
    authority.admit_operation("higher", 2, 2);
    let a = authority.try_reserve("host").unwrap();
    let b = authority.try_reserve("host").unwrap();
    let mut connector = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        authority.clone(),
    );
    let mut waiting = connector
        .start(&Key::ssh("git", "host", 22), &Identity::Ambient, None)
        .unwrap();
    assert!(
        waiting.waiting_locally(),
        "a full authority defers the connect, never fails it"
    );
    drop(a);
    let mut cx = Context::from_waker(Waker::noop());
    assert!(matches!(
        waiting.poll_connected(&mut cx),
        Poll::Ready(Ok(_))
    ));
    drop(b);
}

#[test]
fn a_limit_admitted_by_a_request_that_has_ended_no_longer_raises_the_ceilings() {
    // State review P2-2: the ceilings are the live requests' limits, not the largest ever admitted.
    let authority = Authority::new(2, 2);
    authority.admit_operation("big", 16, 16);
    assert_eq!(authority.ceilings(), (16, 16));
    authority.end_operation("big");
    assert_eq!(
        authority.ceilings(),
        (2, 2),
        "no request is live: the base applies"
    );
    authority.admit_operation("small", 4, 4);
    assert_eq!(authority.ceilings(), (4, 4));
    let held: Vec<_> = (0..4)
        .map(|_| {
            authority
                .try_reserve("host")
                .expect("within the live request's limit")
        })
        .collect();
    assert!(
        authority.try_reserve("host").is_none(),
        "the ended request's 16 no longer applies"
    );
    drop(held);
}

#[test]
fn live_requests_bound_the_ceilings_by_the_largest_and_the_largest_ending_lowers_them() {
    let authority = Authority::new(2, 2);
    authority.admit_operation("sixteen", 16, 16);
    authority.admit_operation("four", 4, 4);
    assert_eq!(authority.ceilings(), (16, 16));
    assert_eq!(authority.live_operations(), 2);
    authority.end_operation("sixteen");
    assert_eq!(authority.ceilings(), (4, 4));
    authority.end_operation("sixteen");
    authority.end_operation("never admitted");
    assert_eq!(
        authority.live_operations(),
        1,
        "ending twice, or a stranger, changes nothing"
    );
    authority.end_operation("four");
    assert_eq!(authority.ceilings(), (2, 2));
    assert_eq!(authority.live_operations(), 0);
}

#[test]
fn a_lone_request_below_the_base_bounds_one_host_across_both_schemes() {
    // State review P2-2, consequence 2: a lone operation at 4 per host keeps a cross-scheme bound of 4 on a host
    // reached over SSH and HTTPS (as `install_capacity` gave), where the base per-host ceiling is 32.
    let authority = Authority::new(256, 32);
    authority.admit_operation("lone", 256, 4);
    let mut ssh = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        authority.clone(),
    );
    let mut https = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        authority.clone(),
    );
    let mut started = Vec::new();
    for _ in 0..2 {
        started.push(
            ssh.start(&Key::ssh("git", "host", 22), &Identity::Ambient, None)
                .unwrap(),
        );
        started.push(
            https
                .start(&Key::https("host", 443), &Identity::Https, None)
                .unwrap(),
        );
    }
    assert!(
        started.iter().all(|resource| !resource.waiting_locally()),
        "four reservations fit"
    );
    assert_eq!(authority.counts("host"), (4, 4));
    let fifth = https
        .start(&Key::https("host", 443), &Identity::Https, None)
        .unwrap();
    assert!(
        fifth.waiting_locally(),
        "a fifth reservation to the host waits, on either scheme"
    );
    assert_eq!(authority.counts("host"), (4, 4));
}

#[test]
fn admitting_a_larger_limit_wakes_the_connections_waiting_for_a_reservation() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Count(AtomicUsize);
    impl std::task::Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let authority = Authority::new(1, 1);
    authority.admit_operation("one", 1, 1);
    let held = authority.try_reserve("host").unwrap();
    let woken = Arc::new(Count(AtomicUsize::new(0)));
    let waker = Waker::from(woken.clone());
    assert!(authority.reserve_or_wait("host", &waker).is_none());
    authority.admit_operation("two", 2, 2);
    assert_eq!(
        woken.0.load(Ordering::SeqCst),
        1,
        "the raised ceiling wakes the waiter"
    );
    drop(held);
}

#[test]
fn a_request_s_owner_session_is_unique_per_request_and_within_the_pool_s_bound() {
    use super::request_owner_session as owner;
    assert_eq!(owner("ssh-worker-1", "r1"), "ssh-worker-1/r1");
    assert_ne!(owner("ssh-worker-1", "r1"), owner("ssh-worker-1", "r2"));
    let (a, b) = ("a".repeat(128), "b".repeat(128));
    for (base, request) in [
        ("ssh-worker-1", &a),
        ("ssh-worker-1", &b),
        (&a, &b),
        (&b, &a),
    ] {
        let session = owner(base, request);
        assert!(session.len() <= 128, "{} bytes", session.len());
        assert!(!session.is_empty() && !session.chars().any(char::is_control));
        assert_eq!(
            session,
            owner(base, request),
            "the same pair, the same owner"
        );
    }
    assert_ne!(owner("ssh-worker-1", &a), owner("ssh-worker-1", &b));
    assert_ne!(owner(&a, &b), owner(&b, &a));
}

#[test]
fn idle_loss_reaches_the_host_through_the_reservation() {
    let mut connector = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        Authority::new(1, 1),
    );
    let mut resource = connector
        .start(
            &Key::ssh("git", "github.example", 22),
            &Identity::Ambient,
            None,
        )
        .unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    assert!(resource.poll_idle_lost(&mut cx).is_ready());
}

#[test]
fn a_connection_waiting_for_its_reservation_is_not_idle() {
    // The wrapped resource would report itself lost (`FakeResource` always
    // does), but a connection that has not started has no session to lose.
    let authority = Authority::new(1, 1);
    let held = authority.try_reserve("host").unwrap();
    let mut connector = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        authority.clone(),
    );
    let mut waiting = connector
        .start(&Key::ssh("git", "host", 22), &Identity::Ambient, None)
        .unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    assert!(waiting.waiting_locally());
    assert!(waiting.poll_idle_lost(&mut cx).is_pending());
    drop(held);
    assert!(matches!(
        waiting.poll_connected(&mut cx),
        Poll::Ready(Ok(_))
    ));
    assert!(!waiting.waiting_locally());
    assert!(waiting.poll_idle_lost(&mut cx).is_ready());
}

#[test]
fn a_hang_up_reaches_the_host_through_the_reservation() {
    // Production SSH resources are `ReservedResource<NativeResource>`; this
    // forward is all that connects the worker's hang-up watch to them.
    let mut connector = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        Authority::new(1, 1),
    );
    let mut resource = connector
        .start(
            &Key::ssh("git", "github.example", 22),
            &Identity::Ambient,
            None,
        )
        .unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    assert!(resource.poll_hung_up(&mut cx).is_ready());
}

#[test]
fn a_connection_waiting_for_its_reservation_is_not_hung_up() {
    // The wrapped resource would report a hang-up (`FakeResource` always
    // does), but a connection that has not started has no session to lose.
    let authority = Authority::new(1, 1);
    let held = authority.try_reserve("host").unwrap();
    let mut connector = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: false,
        },
        authority.clone(),
    );
    let mut waiting = connector
        .start(&Key::ssh("git", "host", 22), &Identity::Ambient, None)
        .unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    assert!(waiting.waiting_locally());
    assert!(waiting.poll_hung_up(&mut cx).is_pending());
    drop(held);
    assert!(matches!(
        waiting.poll_connected(&mut cx),
        Poll::Ready(Ok(_))
    ));
    assert!(!waiting.waiting_locally());
    assert!(waiting.poll_hung_up(&mut cx).is_ready());
}

#[test]
fn failed_connect_releases_unowned_reservation() {
    let authority = Authority::new(1, 1);
    let mut connector = ReservedConnector::new(
        FakeConnector {
            dispose_pending: false,
            fail: true,
        },
        authority.clone(),
    );
    assert!(
        connector
            .start(&Key::https("github.example", 443), &Identity::Https, None)
            .is_err()
    );
    assert!(authority.try_reserve("github.example").is_some());
}
