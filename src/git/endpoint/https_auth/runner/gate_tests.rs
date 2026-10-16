//! A helper's launch holds its host's spawn gate (`git/spawn_gate.rs`; WH2 contract C9; operator decision D25): it is
//! not started while another hold exists, the wait is spent from the lookup's own clock and cancellation, and the
//! gate is held for the launch only. The gate here excludes on every platform, so the rows run everywhere; on
//! Windows the gate excludes by itself.
use super::*;
use crate::git::endpoint::https_auth::helper_fixture::{Behavior, Fixture};
use crate::git::endpoint::https_worker::helper_timeout;
use crate::git::spawn_gate::SpawnGate;
use gwz_transport::protocol::{ErrorCode, Facts, SetupFailureCause};

/// A runner whose owner uses `gate`, with its two admission permits.
async fn runner<'a>(
    owner: &'a AuthOwner,
    config: &'a Config,
    cancelled: &'a CancellationToken,
    deadline: Instant,
) -> Runner<'a> {
    Runner {
        owner,
        config,
        executable: &config.executable,
        permits: Arc::new(owner::AdmissionPermits {
            _helper_slot: owner
                .inner
                .helper_slots
                .0
                .clone()
                .acquire_owned()
                .await
                .unwrap(),
            _endpoint_slot: None,
        }),
        cancelled,
        deadline,
        setup: None,
    }
}

fn host() -> (SpawnGate, AuthOwner) {
    let gate = SpawnGate::with_exclusion(true);
    let owner = AuthOwner::new(HelperSlots::with_gate(gate.clone()));
    (gate, owner)
}

#[tokio::test]
async fn a_helper_is_not_started_while_the_hosts_gate_is_held() {
    let fixture = Fixture::new(Behavior::AnswersAndMarks);
    let (gate, owner) = host();
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        Instant::now() + Duration::from_secs(60),
    )
    .await;
    let argv = fixture.argv();
    let hold = gate.hold();
    let mut work = Box::pin(runner.run(&argv, &[], Some(&[]), OUTPUT_LIMIT, false));
    assert!(
        timeout(Duration::from_millis(500), &mut work)
            .await
            .is_err(),
        "the lookup finished while another hold on the gate existed"
    );
    assert!(
        !fixture.marker.exists(),
        "the helper was started while another hold on the gate existed"
    );
    drop(hold);
    let answer = timeout(Duration::from_secs(60), work)
        .await
        .expect("the lookup goes on once the gate is released");
    assert!(answer.is_ok(), "the helper answered once the gate was free");
    assert!(fixture.marker.exists());
    assert!(gate.try_hold().is_some(), "the launch released the gate");
}

#[tokio::test]
async fn a_lookup_that_waits_for_the_gate_gives_up_at_its_deadline_and_starts_nothing() {
    let fixture = Fixture::new(Behavior::AnswersAndMarks);
    let (gate, owner) = host();
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        Instant::now() + Duration::from_millis(300),
    )
    .await;
    let argv = fixture.argv();
    let hold = gate.hold();
    let result = runner.run(&argv, &[], Some(&[]), OUTPUT_LIMIT, false).await;
    // Its own outcome: no helper of this lookup was started, so it is not "the helper did not answer".
    assert_eq!(result.as_ref().err(), Some(&AuthError::GateTimeout));
    assert!(
        !fixture.marker.exists(),
        "a helper started past its deadline"
    );
    // It is classified as a wait for a resource, with the lookup's clock as the budget it waited.
    let failure = helper_timeout(AuthError::GateTimeout, &Facts::default(), 0, 300);
    assert_eq!(failure.code, ErrorCode::Timeout);
    assert_eq!(failure.setup_cause, Some(SetupFailureCause::Allocation));
    assert_eq!(failure.detail.unwrap().helper_budget_ms, Some(300));
    drop(runner);
    assert_eq!(owner.inner.helper_slots.available(), 8);
    assert_eq!(owner.pending_cleanup_count(), 0);
    drop(hold);
}

#[tokio::test]
async fn a_lookup_whose_deadline_passed_before_it_waited_is_a_plain_timeout() {
    // Nothing held the gate: the lookup ran out of time on its own, which is the interaction clock's outcome.
    let fixture = Fixture::new(Behavior::AnswersAndMarks);
    let (_gate, owner) = host();
    let cancelled = CancellationToken::new();
    let runner = runner(&owner, &fixture.config, &cancelled, Instant::now()).await;
    let argv = fixture.argv();
    let result = runner.run(&argv, &[], Some(&[]), OUTPUT_LIMIT, false).await;
    assert_eq!(result.as_ref().err(), Some(&AuthError::Timeout));
    assert!(!fixture.marker.exists());
}

#[tokio::test]
async fn a_lookup_that_waits_for_the_gate_is_cancelled_by_its_token() {
    let fixture = Fixture::new(Behavior::AnswersAndMarks);
    let (gate, owner) = host();
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        Instant::now() + Duration::from_secs(60),
    )
    .await;
    let argv = fixture.argv();
    let hold = gate.hold();
    let canceller = {
        let cancelled = cancelled.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(100)).await;
            cancelled.cancel();
        })
    };
    let result = timeout(
        Duration::from_secs(30),
        runner.run(&argv, &[], Some(&[]), OUTPUT_LIMIT, false),
    )
    .await
    .expect("a cancelled lookup does not wait for the gate");
    canceller.await.unwrap();
    assert!(
        matches!(result, Err(AuthError::Cancelled)),
        "{:?}",
        result.err()
    );
    assert!(
        !fixture.marker.exists(),
        "a cancelled lookup started a helper"
    );
    drop(hold);
}

#[tokio::test]
async fn the_gate_is_held_for_the_launch_and_not_for_the_helpers_life() {
    let fixture = Fixture::new(Behavior::RunsWithDescendantOnOutput);
    let (gate, owner) = host();
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        Instant::now() + Duration::from_secs(60),
    )
    .await;
    let argv = fixture.argv();
    let mut work = Box::pin(runner.run(&argv, &[], Some(&[]), OUTPUT_LIMIT, false));
    tokio::select! {
        _ = fixture.started() => {}
        result = &mut work => panic!("the helper ended: {:?}", result.err()),
    }
    assert!(
        gate.try_hold().is_some(),
        "the gate stayed held while the helper ran"
    );
    cancelled.cancel();
    let result = timeout(Duration::from_secs(30), work).await.unwrap();
    assert!(
        matches!(result, Err(AuthError::Cancelled)),
        "{:?}",
        result.err()
    );
    drop(runner);
    assert_eq!(
        owner
            .reap_pending(Instant::now() + Duration::from_secs(2))
            .await,
        0
    );
}
