use super::*;
use gwz_transport::protocol::HelperFailureCause;

fn malformed() -> Failure {
    Failure {
        code: ErrorCode::Authentication,
        effect: Effect::None,
        detail: Some(Box::new(FailureDetail {
            helper_cause: Some(HelperFailureCause::MalformedOutput),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn pool_observer_waits_only_until_exact_detail_is_associated_even_on_publisher_unwind() {
    for (unwind, zero) in [(false, false), (true, false), (false, true)] {
        let (mut pool, request, _, context) = pool_fixture();
        let failed = if zero {
            Failure {
                code: ErrorCode::Timeout,
                effect: Effect::None,
                setup_cause: Some(SetupFailureCause::Allocation),
                detail: Some(Box::new(FailureDetail {
                    helper_budget_ms: Some(0),
                    ..Default::default()
                })),
                ..Default::default()
            }
        } else {
            malformed()
        };
        let mut publication = super::super::publication::Publication::new(&context, failed.clone());
        let record = publication.commit();
        let (seen, observed) = std::sync::mpsc::channel();
        let other = context.clone();
        let observer = std::thread::spawn(move || {
            pool.advance(0);
            let Err(gwz_transport::pool::Error::SetupEnded(actual)) = pool.take(request) else {
                panic!("pool terminal");
            };
            seen.send(actual).unwrap();
            other.failure(actual)
        });
        assert_eq!(observed.recv().unwrap(), record);
        if unwind {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    let _owned = publication;
                    panic!("owned publication unwind");
                }))
                .is_err()
            );
        } else {
            drop(publication);
        }
        assert_eq!(observer.join().unwrap(), failed);
        assert_eq!(context.failure(record), failed);
        if zero {
            let model = crate::transport_host::SshOpenFailure(failed, None)
                .model_error(false)
                .unwrap();
            assert_eq!(model.code, crate::model::ErrorCode::CredentialHelperTimeout);
            assert!(
                model
                    .message
                    .starts_with("No credential helper could start in the 0 seconds")
            );
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
        assert!(!context.state.lock().unwrap().publishing);
    }
}

#[test]
fn equal_prior_resource_and_competing_publishers_cannot_replace_first_detail() {
    let (_, context) = fixture();
    let prior = context
        .clock
        .terminate(SetupCause::ResourceFailure {
            code: ErrorCode::Authentication,
            effect: Effect::None,
            setup_cause: None,
        })
        .deliver();
    context.terminate_failure(malformed());
    assert!(context.failure(prior).detail.is_none());

    let (_, context) = fixture();
    let first = malformed();
    let mut owned = super::super::publication::Publication::new(&context, first.clone());
    let other = context.clone();
    let publisher = std::thread::spawn(move || {
        other.terminate_failure(Failure {
            detail: Some(Box::new(FailureDetail {
                helper_cause: Some(HelperFailureCause::MissingField),
                ..Default::default()
            })),
            ..malformed()
        })
    });
    let record = owned.commit();
    drop(owned);
    publisher.join().unwrap();
    assert_eq!(context.failure(record), first);
}

#[test]
fn abandoned_precommit_and_prior_terminal_do_not_install_unadmitted_detail() {
    for cause in [SetupCause::Cancelled, SetupCause::NetworkAggregate] {
        let (_, context) = fixture();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _publication =
                    super::super::publication::Publication::new(&context, malformed());
                panic!("precommit unwind");
            }))
            .is_err()
        );
        let record = context.clock.terminate(cause).deliver();
        let before = context.failure(record);
        context.terminate_failure(malformed());
        assert_eq!(context.failure(record), before);
        assert!(before.detail.is_none());
        assert!(!context.state.lock().unwrap().publishing);
    }
}
