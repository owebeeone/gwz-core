use super::super::agent_job::{Control, wall_clock};
use super::*;
use gwz_transport::pool::{Action, Config, Identity, Key, Owner, PoolMachine, Request};
use std::sync::atomic::{AtomicU64, Ordering};
mod publication;

fn pool_fixture() -> (
    PoolMachine,
    gwz_transport::pool::RequestId,
    Arc<AtomicU64>,
    Arc<SetupContext>,
) {
    let mut pool = PoolMachine::new(Config {
        connect_timeout_ms: 100,
        ..Default::default()
    })
    .unwrap();
    let request = pool
        .request(Request::new(
            Key::ssh("user", "host", 22),
            Identity::Ambient,
            Owner::new("s", "o"),
        ))
        .unwrap();
    let Action::Connect { connection, .. } = pool.next_action().unwrap() else {
        panic!("connect")
    };
    let now = Arc::new(AtomicU64::new(0));
    let source = now.clone();
    let clock = pool
        .install_setup_clock(
            connection,
            Arc::new(move || source.load(Ordering::SeqCst)),
            10,
        )
        .unwrap();
    (pool, request, now, SetupContext::new(clock, Instant::now()))
}
fn fixture() -> (Arc<AtomicU64>, Arc<SetupContext>) {
    let (_, _, now, context) = pool_fixture();
    (now, context)
}

#[tokio::test]
async fn remediation_immediate_observer_gets_zero_allocation_detail() {
    struct Observe {
        context: Arc<SetupContext>,
        answer: Mutex<Option<Failure>>,
    }
    impl std::task::Wake for Observe {
        fn wake(self: Arc<Self>) { self.wake_by_ref(); }
        fn wake_by_ref(self: &Arc<Self>) {
            if let Observation::Terminal(record) = self.context.clock.observe().value {
                *self.answer.lock().unwrap() = Some(self.context.failure(record));
            }
        }
    }
    let (_, context) = fixture();
    let observer = Arc::new(Observe { context: context.clone(), answer: Mutex::new(None) });
    context.clock.register_driver(Arc::new(std::task::Waker::from(observer.clone()))).deliver();
    context.enter(LocalPhase::Admission, 0).await.unwrap_err();
    let first = observer.answer.lock().unwrap().clone().expect("publication woke observer");
    assert_eq!(first.detail.as_ref().and_then(|d| d.helper_budget_ms), Some(0));
    let Observation::Terminal(record) = context.clock.observe().deliver() else { panic!("terminal"); };
    assert_eq!(context.failure(record), first);
}

#[test]
fn actual_control_cannot_latch_displaced_network_deadline_before_ack() {
    let (now, context) = fixture();
    let control = Control::new_shared(context.clone(), Duration::from_millis(100), wall_clock());
    now.store(99, Ordering::SeqCst);
    let token = context
        .clock
        .prepare_local(LocalPhase::Admission, 200)
        .deliver()
        .unwrap();
    let receipt = context.clock.publish_local(token).deliver().unwrap();
    now.store(101, Ordering::SeqCst);
    assert!(control.check().is_ok());
    assert!(control.begin_wait().is_err());
    context.clock.acknowledge(receipt).deliver().unwrap();
    assert!(control.check().is_ok());
}

#[test]
fn all_local_expiry_consumers_keep_same_exact_witness() {
    for winner in 0..3 {
        let (mut pool, request, now, context) = pool_fixture();
        let token = context
            .clock
            .prepare_local(LocalPhase::Interaction, 1250)
            .deliver()
            .unwrap();
        let phase = token.phase();
        context.state.lock().unwrap().witnesses[1] = Some(Witness {
            phase,
            cause: SetupFailureCause::Interaction,
            milliseconds: 1250,
        });
        let receipt = context.clock.publish_local(token).deliver().unwrap();
        context.clock.acknowledge(receipt).deliver().unwrap();
        now.store(1250, Ordering::SeqCst);
        let control =
            Control::new_shared(context.clone(), Duration::from_millis(100), wall_clock());
        match winner {
            0 => {
                pool.advance(1250);
                let Err(gwz_transport::pool::Error::SetupEnded(record)) = pool.take(request) else {
                    panic!("typed pool expiry")
                };
                let captured = super::super::ssh_worker::EndpointOpenFailure::capture(
                    io::Error::other(gwz_transport::pool::Error::SetupEnded(record)),
                    Default::default(),
                    Some(context.clone()),
                );
                let capture = captured
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<super::super::ssh_worker::EndpointOpenFailure>()
                    .unwrap();
                assert_eq!(
                    capture.failure.detail.as_ref().unwrap().helper_budget_ms,
                    Some(1250)
                );
                assert_eq!(pool.counts().opening, 1);
            }
            1 => {
                assert!(control.check().is_err());
            }
            _ => {
                context.terminate_failure(Failure {
                    code: ErrorCode::Timeout,
                    effect: Effect::None,
                    ..Default::default()
                });
            }
        }
        let Observation::Terminal(record) = context.clock.observe().deliver() else {
            panic!("terminal")
        };
        let failed = context.failure(record);
        assert_eq!(failed.setup_cause, Some(SetupFailureCause::Interaction));
        assert_eq!(failed.detail.unwrap().helper_budget_ms, Some(1250));
    }
}

#[test]
fn generic_network_timeout_has_no_helper_detail() {
    let (now, context) = fixture();
    now.store(100, Ordering::SeqCst);
    let Observation::Terminal(record) = context.clock.observe().deliver() else {
        panic!("terminal")
    };
    assert!(context.failure(record).detail.is_none());
}

#[tokio::test]
async fn zero_retained_allocation_is_typed_without_a_phase_or_launch() {
    let (_, context) = fixture();
    let error = context.enter(LocalPhase::Admission, 0).await.unwrap_err();
    let record = error
        .get_ref()
        .unwrap()
        .downcast_ref::<SetupEnded>()
        .unwrap()
        .0;
    let failed = context.failure(record);
    assert_eq!(failed.code, ErrorCode::Timeout);
    assert_eq!(failed.setup_cause, Some(SetupFailureCause::Allocation));
    assert_eq!(failed.detail.unwrap().helper_budget_ms, Some(0));
    assert!(
        context
            .state
            .lock()
            .unwrap()
            .witnesses
            .iter()
            .all(Option::is_none)
    );
}

#[tokio::test]
async fn actual_preparation_refusal_keeps_original_witness_without_terminal_wait() {
    use std::sync::atomic::AtomicBool;
    for (network, kind) in [
        (0, LocalPhase::Admission),
        (100, LocalPhase::Admission),
        (100, LocalPhase::Interaction),
    ] {
        let mut pool = PoolMachine::new(Config {
            connect_timeout_ms: network,
            ..Default::default()
        })
        .unwrap();
        pool.request(Request::new(
            Key::ssh("u", "h", 22),
            Identity::Ambient,
            Owner::new("s", "o"),
        ))
        .unwrap();
        let Action::Connect { connection, .. } = pool.next_action().unwrap() else {
            panic!("connect")
        };
        let enabled = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicU64::new(0));
        let flag = enabled.clone();
        let samples = calls.clone();
        let clock = pool
            .install_setup_clock(
                connection,
                Arc::new(move || {
                    if flag.load(Ordering::SeqCst) && samples.fetch_add(1, Ordering::SeqCst) >= 2 {
                        2
                    } else {
                        0
                    }
                }),
                10,
            )
            .unwrap();
        let context = SetupContext::new(clock, Instant::now());
        if kind == LocalPhase::Interaction {
            let token = context
                .clock
                .prepare_local(LocalPhase::Admission, 50)
                .deliver()
                .unwrap();
            let receipt = context.clock.publish_local(token).deliver().unwrap();
            context.clock.acknowledge(receipt).deliver().unwrap();
        }
        enabled.store(true, Ordering::SeqCst);
        let error = context.enter(kind, 1).await.unwrap_err();
        let record = error
            .get_ref()
            .unwrap()
            .downcast_ref::<SetupEnded>()
            .unwrap()
            .0;
        assert_eq!(record.cause, SetupCause::PreparationDeadline);
        let failed = context.failure(record);
        assert_eq!(failed.detail.unwrap().helper_budget_ms, Some(1));
        assert_eq!(
            failed.setup_cause,
            Some(if kind == LocalPhase::Admission {
                SetupFailureCause::Allocation
            } else {
                SetupFailureCause::Interaction
            })
        );
    }
}

#[test]
fn foreign_terminal_has_no_witness_and_cannot_replace_first_failure() {
    let (_, first) = fixture();
    let (_, second) = fixture();
    let own = first.clock.terminate(SetupCause::Cancelled).deliver();
    let failed = first.failure(own);
    let foreign = second
        .clock
        .terminate(SetupCause::NetworkAggregate)
        .deliver();
    let refused = first.failure(foreign);
    assert_eq!(refused.code, ErrorCode::InvalidRequest);
    assert!(refused.detail.is_none());
    assert_eq!(first.failure(own), failed);
}
