use super::*;
use crate::git::endpoint::https_auth::helper_fixture::{Behavior, Fixture};

#[tokio::test]
async fn remediation_completed_output_and_parsed_answer_require_final_admission() {
    let owner = AuthOwner::new(HelperSlots::new());
    let config = Config {
        executable: "git".into(),
        environment: Vec::new(),
    };
    let cancelled = CancellationToken::new();
    let permits = Arc::new(owner::AdmissionPermits {
        _helper_slot: owner
            .inner
            .helper_slots
            .0
            .clone()
            .acquire_owned()
            .await
            .unwrap(),
        _endpoint_slot: None,
    });
    let mut runner = Runner {
        owner: &owner,
        config: &config,
        executable: &config.executable,
        permits,
        cancelled: &cancelled,
        deadline: Instant::now(),
        setup: None,
    };
    assert!(matches!(
        runner.admit_child_output(Ok(SecretBuffer(
            b"username=alice\npassword=token\n".to_vec()
        ))),
        Err(AuthError::Timeout)
    ));
    runner.deadline = Instant::now() + Duration::from_secs(1);
    cancelled.cancel();
    assert!(matches!(
        runner.admit_child_output(Ok(SecretBuffer(Vec::new()))),
        Err(AuthError::Cancelled)
    ));
    let cancelled = CancellationToken::new();
    runner.cancelled = &cancelled;
    assert!(matches!(
        runner.parse_answer(b"username=alice\npassword=token\n", |bytes| {
            let secret = parse_secret(bytes)?;
            cancelled.cancel();
            Ok(secret)
        }),
        Err(AuthError::Cancelled)
    ));
    let cancelled = CancellationToken::new();
    runner.cancelled = &cancelled;
    runner.deadline = Instant::now() + Duration::from_millis(5);
    assert!(matches!(
        runner.parse_answer(b"username=alice\npassword=token\n", |bytes| {
            let secret = parse_secret(bytes)?;
            std::thread::sleep(Duration::from_millis(10));
            Ok(secret)
        }),
        Err(AuthError::Timeout)
    ));
    drop(runner);
    assert_eq!(owner.inner.helper_slots.available(), 8);
    assert_eq!(owner.pending_cleanup_count(), 0);
}

#[tokio::test]
async fn completed_native_child_and_ready_timeout_or_cancel_never_admit_answer() {
    use std::{
        future::Future,
        task::{Context, Poll, Wake, Waker},
    };
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    for cancel in [false, true] {
        let fixture = Fixture::new(Behavior::AnswersAndMarks);
        let marker = fixture.marker.clone();
        let config = fixture.config.clone();
        let owner = AuthOwner::new(HelperSlots::new());
        let cancelled = CancellationToken::new();
        let runner = Runner {
            owner: &owner,
            config: &config,
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
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_millis(300),
            setup: None,
        };
        let mut work = std::pin::pin!(runner.run(&[], &[], Some(&[]), OUTPUT_LIMIT, false));
        let waker = Waker::from(Arc::new(Noop));
        assert!(matches!(
            work.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        ));
        let until = Instant::now() + Duration::from_secs(1);
        while !marker.exists() {
            assert!(Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        // Keep the lookup future unpolled until both completed work and the
        // relevant refusal are ready. Success must lose whichever select arm wins.
        if cancel {
            cancelled.cancel();
        } else {
            tokio::time::sleep_until(runner.deadline).await;
        }
        let result = work.await;
        assert!(matches!(
            result,
            Err(AuthError::Timeout | AuthError::Cancelled)
        ));
        assert_eq!(
            owner
                .reap_pending(Instant::now() + Duration::from_secs(1))
                .await,
            0
        );
        assert_eq!(owner.pending_cleanup_count(), 0);
    }
}

#[derive(Clone, Copy)]
enum Boundary {
    Equality,
    Cancel,
    Success,
}

async fn completed_leader_boundary(boundary: Boundary) {
    let fixture = Fixture::new(Behavior::AnswersLeavingDetachedDescendant);
    let heartbeat = fixture.heartbeat.clone();
    let config = fixture.config.clone();
    let owner = AuthOwner::new(HelperSlots::new());
    let endpoint = Arc::new(Semaphore::new(1));
    let permits = Arc::new(owner::AdmissionPermits {
        _helper_slot: owner
            .inner
            .helper_slots
            .0
            .clone()
            .acquire_owned()
            .await
            .unwrap(),
        _endpoint_slot: Some(endpoint.clone().acquire_owned().await.unwrap()),
    });
    let cancelled = CancellationToken::new();
    let runner = Runner {
        owner: &owner,
        config: &config,
        executable: &config.executable,
        permits: permits.clone(),
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(10),
        setup: None,
    };
    let mut command = Command::new(&config.executable);
    command
        .env_clear()
        .envs(config.environment.iter().map(|(key, value)| (key, value)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let (mut child, tree) = process_tree::spawn(&mut command).unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    // The job owns the tree from here on, and its drop ends the independent descendant even when an assertion
    // below fails.
    let mut job = HelperJob::new((child, tree), permits.clone(), owner.clone());
    let mut output = SecretBuffer(Vec::new());
    let mut diagnostic = SecretBuffer(Vec::new());
    let (status, out, err) = tokio::join!(
        job.child_mut().wait(),
        stdout.read_to_end(&mut output.0),
        stderr.read_to_end(&mut diagnostic.0)
    );
    assert!(status.unwrap().success());
    out.unwrap();
    err.unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while std::fs::metadata(&heartbeat).map_or(0, |m| m.len()) < 2 {
        assert!(Instant::now() < until, "independent descendant must write");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let answer = runner.parse_answer(&output.0, parse_secret).unwrap();
    let mut owned_at_boundary = false;
    let result = runner
        .finish_job(&mut job, Ok(answer), |result| {
            owned_at_boundary = Arc::strong_count(&permits) == 3;
            assert_eq!(owner.inner.helper_slots.available(), 7);
            assert_eq!(endpoint.available_permits(), 0);
            let answer = result?;
            match boundary {
                Boundary::Equality => runner.check_with_now(|| runner.deadline)?,
                Boundary::Cancel => {
                    cancelled.cancel();
                    runner.check()?;
                }
                Boundary::Success => {
                    runner.check()?;
                    cancelled.cancel();
                }
            }
            Ok(answer)
        })
        .await;
    let admitted = result.is_ok();
    let expected = match boundary {
        Boundary::Equality => matches!(result, Err(AuthError::Timeout)),
        Boundary::Cancel => matches!(result, Err(AuthError::Cancelled)),
        Boundary::Success => admitted,
    };
    drop(result);
    drop(job);
    drop(runner);
    drop(permits);
    assert!(expected, "boundary outcome");
    assert_eq!(
        owner
            .reap_pending(Instant::now() + Duration::from_secs(1))
            .await,
        0
    );
    assert_eq!(owner.inner.helper_slots.available(), 8);
    assert_eq!(endpoint.available_permits(), 1);
    if !matches!(boundary, Boundary::Success) {
        assert!(!admitted, "no answer or Authorization may be derived");
        fixture
            .assert_stopped("refused helper's independent descendant must stop writing")
            .await;
    }
    assert!(
        owned_at_boundary,
        "final admission must still own the helper group and permits"
    );
}

#[tokio::test]
async fn final_refusal_at_deadline_equality_kills_closed_pipe_descendant() {
    completed_leader_boundary(Boundary::Equality).await;
}
#[tokio::test]
async fn final_refusal_on_cancel_kills_closed_pipe_descendant() {
    completed_leader_boundary(Boundary::Cancel).await;
}
#[tokio::test]
async fn normal_final_admission_survives_later_cancellation_without_refusal() {
    completed_leader_boundary(Boundary::Success).await;
}
