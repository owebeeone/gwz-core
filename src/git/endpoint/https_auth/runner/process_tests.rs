//! The helper process owner at the runner's level, the same tests on both platforms (step 4.2). The Unix process
//! group rows of `runner_tests.rs`, generalized: a helper's descendants die on timeout and on cancel, the host's
//! capacity is held until the whole tree is reaped, and a pipe that nobody serves cannot stall a lookup.
use super::*;
use crate::git::endpoint::https_auth::helper_fixture::{Behavior, Fixture};
use std::{
    future::Future,
    task::{Context, Poll, Wake, Waker},
};

struct Noop;
impl Wake for Noop {
    fn wake(self: Arc<Self>) {}
}

async fn permits(owner: &AuthOwner) -> Arc<owner::AdmissionPermits> {
    Arc::new(owner::AdmissionPermits {
        _helper_slot: owner
            .inner
            .helper_slots
            .0
            .clone()
            .acquire_owned()
            .await
            .unwrap(),
        _endpoint_slot: None,
    })
}

fn runner<'a>(
    owner: &'a AuthOwner,
    config: &'a Config,
    cancelled: &'a CancellationToken,
    permits: Arc<owner::AdmissionPermits>,
    deadline: Instant,
) -> Runner<'a> {
    Runner {
        owner,
        config,
        executable: &config.executable,
        permits,
        cancelled,
        deadline,
        setup: None,
    }
}

/// The owner holds nothing: nothing is retained, and every host slot is back. `reap_ready` alone, which every
/// lookup runs before it asks for a slot, must be enough once the tree is gone; `reap_pending` then has nothing
/// left to join.
async fn assert_reaped(owner: &AuthOwner) {
    let until = Instant::now() + Duration::from_secs(5);
    while owner.pending_cleanup_count() > 0 {
        assert!(
            Instant::now() < until,
            "reap_ready did not release a retained helper"
        );
        owner.reap_ready();
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(owner.inner.helper_slots.available(), 8);
    assert_eq!(
        owner
            .reap_pending(Instant::now() + Duration::from_secs(2))
            .await,
        0
    );
    assert_eq!(owner.pending_cleanup_count(), 0);
    assert_eq!(owner.inner.helper_slots.available(), 8);
}

#[tokio::test]
async fn timeout_kills_descendants_even_after_the_helper_exited_with_stdout_open() {
    let fixture = Fixture::new(Behavior::ExitsLeavingDescendantOnOutput);
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        Instant::now() + Duration::from_millis(1500),
    );
    let result = runner
        .run(&fixture.argv(), &[], None, OUTPUT_LIMIT, false)
        .await;
    assert!(matches!(result, Err(AuthError::Timeout)));
    assert!(fixture.heartbeats() > 0, "the descendant must have started");
    fixture
        .assert_stopped("the timed-out helper's descendant must stop")
        .await;
    drop(runner);
    assert_reaped(&owner).await;
}

#[tokio::test]
async fn cancel_kills_the_helper_and_its_descendants() {
    let fixture = Fixture::new(Behavior::RunsWithDescendantOnOutput);
    let argv = fixture.argv();
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        Instant::now() + Duration::from_secs(60),
    );
    let cancel = async {
        fixture.started().await;
        cancelled.cancel();
    };
    let (result, ()) = tokio::join!(runner.run(&argv, &[], None, OUTPUT_LIMIT, false), cancel);
    assert!(matches!(result, Err(AuthError::Cancelled)));
    fixture
        .assert_stopped("the cancelled helper's descendant must stop")
        .await;
    drop(runner);
    assert_reaped(&owner).await;
}

#[tokio::test]
async fn the_owners_cancel_kills_the_helper_and_its_descendants() {
    let fixture = Fixture::new(Behavior::RunsWithDescendantOnOutput);
    let argv = fixture.argv();
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        Instant::now() + Duration::from_secs(60),
    );
    let cancel = async {
        fixture.started().await;
        owner.cancel();
    };
    let (result, ()) = tokio::join!(runner.run(&argv, &[], None, OUTPUT_LIMIT, false), cancel);
    assert!(matches!(result, Err(AuthError::Cancelled)));
    fixture
        .assert_stopped("the owner's cancel must end the descendant")
        .await;
    drop(runner);
    assert_reaped(&owner).await;
}

#[tokio::test]
async fn dropping_a_lookup_mid_io_ends_the_tree_and_the_owner_reaps_it() {
    let fixture = Fixture::new(Behavior::RunsWithDescendantOnOutput);
    let argv = fixture.argv();
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        Instant::now() + Duration::from_secs(60),
    );
    let mut work = Box::pin(runner.run(&argv, &[], None, OUTPUT_LIMIT, false));
    let waker = Waker::from(Arc::new(Noop));
    assert!(matches!(
        work.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    fixture.started().await;
    // The lookup is dropped with its pipes open and a read pending: the job's drop ends the tree and keeps the
    // helper's admission until the owner has reaped it.
    drop(work);
    fixture
        .assert_stopped("a dropped lookup must not leave its descendant running")
        .await;
    drop(runner);
    assert_reaped(&owner).await;
}

#[tokio::test]
async fn a_helper_that_never_reads_a_large_request_times_out_instead_of_blocking() {
    let fixture = Fixture::new(Behavior::Hangs);
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let started = Instant::now();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        started + Duration::from_secs(2),
    );
    let request = vec![b'x'; 4 * 1024 * 1024];
    let result = runner
        .run(&fixture.argv(), &request, None, OUTPUT_LIMIT, false)
        .await;
    assert!(matches!(result, Err(AuthError::Timeout)));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "a write that nobody reads must end with the deadline"
    );
    drop(runner);
    assert_reaped(&owner).await;
}

#[tokio::test]
async fn diagnostics_over_the_limit_do_not_reject_the_answer() {
    let fixture = Fixture::new(Behavior::FloodsDiagnosticsThenAnswers);
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        Instant::now() + Duration::from_secs(10),
    );
    let output = runner
        .run(
            &fixture.argv(),
            b"url=https://example.test/\n\n",
            None,
            OUTPUT_LIMIT,
            false,
        )
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.0).contains("password=token"),
        "only stdout has the credential-answer bound"
    );
    drop(runner);
    assert_reaped(&owner).await;
}

#[tokio::test]
async fn the_answer_of_a_helper_that_exits_normally_is_returned_and_its_slot_is_freed() {
    let fixture = Fixture::new(Behavior::Answer);
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        Instant::now() + Duration::from_secs(10),
    );
    let output = runner
        .run(
            &fixture.argv(),
            b"url=https://example.test/\n\n",
            None,
            OUTPUT_LIMIT,
            false,
        )
        .await
        .unwrap();
    let secret = runner.parse_answer(&output.0, parse_secret).unwrap();
    assert_eq!(secret.header(), "Basic YWxpY2U6dG9rZW4=");
    drop(runner);
    assert_reaped(&owner).await;
}

#[tokio::test]
async fn a_helper_that_succeeded_releases_its_slot_and_leaves_its_survivors_running() {
    // OQ-A: a success lets go of the tree. What the helper started and left running (a browser that a credential
    // manager launched) lives on, as on Unix, under Git and in 1.0.17; the helper's slot is free at once.
    let fixture = Fixture::new(Behavior::AnswersLeavingDetachedDescendant);
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        Instant::now() + Duration::from_secs(20),
    );
    let output = runner
        .run(&fixture.argv(), b"", None, OUTPUT_LIMIT, false)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&output.0).contains("password=token"));
    drop(runner);
    assert_eq!(owner.inner.helper_slots.available(), 8);
    assert_eq!(owner.pending_cleanup_count(), 0);
    fixture
        .assert_alive("the survivor of a successful helper must keep running")
        .await;
}

#[tokio::test]
async fn a_missing_executable_starts_nothing() {
    let mut fixture = Fixture::new(Behavior::Answer);
    fixture.config.executable = fixture.directory().join("missing").join("helper");
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        Instant::now() + Duration::from_secs(10),
    );
    let result = runner
        .run(&fixture.argv(), &[], None, OUTPUT_LIMIT, false)
        .await;
    assert!(matches!(result, Err(AuthError::MissingExecutable)));
    drop(runner);
    assert_reaped(&owner).await;
}

#[tokio::test]
async fn askpass_is_never_in_the_helpers_environment() {
    let mut fixture = Fixture::new(Behavior::RefusesAskPass);
    fixture
        .config
        .environment
        .push(("GIT_ASKPASS".into(), "askpass".into()));
    if cfg!(windows) {
        fixture
            .config
            .environment
            .push(("Git_AskPass".into(), "askpass".into()));
    }
    let owner = AuthOwner::new(HelperSlots::new());
    let cancelled = CancellationToken::new();
    let runner = runner(
        &owner,
        &fixture.config,
        &cancelled,
        permits(&owner).await,
        Instant::now() + Duration::from_secs(10),
    );
    let result = runner
        .run(&fixture.argv(), &[], None, OUTPUT_LIMIT, false)
        .await;
    assert!(
        result.is_ok(),
        "the helper must not see GIT_ASKPASS in any capitalization"
    );
    drop(runner);
    assert_reaped(&owner).await;
}
