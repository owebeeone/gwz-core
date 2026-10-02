use super::*;

#[tokio::test]
async fn remediation_completed_output_and_parsed_answer_require_final_admission() {
    let owner = AuthOwner::new(HelperSlots::new());
    let config = Config {
        executable: "/usr/bin/git".into(),
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
        let home = tempfile::tempdir().unwrap();
        let marker = home.path().join("complete");
        let executable = home.path().join("git");
        crate::git::endpoint::helper_script::write_helper_script(
            &executable,
            "printf 'username=alice\\npassword=token\\n'\nexec 1>&-\nprintf x > \"$MARKER\"\nexit 0",
        );
        let config = Config {
            executable,
            environment: vec![("MARKER".into(), marker.as_os_str().into())],
        };
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
            deadline: Instant::now() + Duration::from_millis(100),
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
