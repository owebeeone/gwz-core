use super::*;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct State {
    ended: usize,
    flushed: usize,
    closed: usize,
    cancelled: usize,
    data: Vec<u8>,
    reply: Vec<u8>,
    failure: Option<gwz_transport::protocol::Failure>,
    failure_facts: Option<gwz_transport::protocol::Facts>,
}
struct Fake(Arc<Mutex<State>>);
impl Read for Fake {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        assert!(self.0.lock().unwrap().ended > 0);
        if out.is_empty() {
            return Ok(0);
        }
        let mut state = self.0.lock().unwrap();
        let count = out.len().min(state.reply.len());
        out[..count].copy_from_slice(&state.reply[..count]);
        state.reply.drain(..count);
        Ok(count)
    }
}
impl Write for Fake {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let n = data.len().min(3);
        self.0.lock().unwrap().data.extend_from_slice(&data[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flushed += 1;
        Ok(())
    }
}
impl HalfClose for Fake {
    fn end_write(&self) -> io::Result<()> {
        self.0.lock().unwrap().ended += 1;
        Ok(())
    }
    fn finish(&self) -> io::Result<()> {
        let mut state = self.0.lock().unwrap();
        state.closed += 1;
        if state.failure.is_some() { return Err(io::Error::other("stream terminal")); }
        Ok(())
    }
    fn cancel(&self) {
        self.0.lock().unwrap().cancelled += 1;
    }
    fn retained_failure(&self) -> Option<gwz_transport::protocol::Failure> {
        self.0.lock().unwrap().failure.clone()
    }
    fn retained_failure_facts(&self) -> Option<gwz_transport::protocol::Facts> {
        self.0.lock().unwrap().failure_facts.clone()
    }
}

#[test]
fn rpc_reports_retained_close_failure_with_timing_and_retry_provenance() {
    use gwz_transport::protocol::*;
    let failure = Failure { code: ErrorCode::Timeout, effect: Effect::None,
        setup_cause: Some(SetupFailureCause::Interaction),
        detail: Some(Box::new(FailureDetail { helper_budget_ms: Some(1250),
            retry_attempt: Some(RetryAttempt { attempt: 2, attempts: 3 }), ..Default::default() })), ..Default::default()
    };
    let facts = Facts { method: AuthMethod::Gh, ..Default::default() };
    let state = Arc::new(Mutex::new(State { failure: Some(failure.clone()),
        failure_facts: Some(facts.clone()), ..State::default() }));
    let report = Arc::new(Mutex::new(None));
    let received = report.clone();
    let mut rpc = RpcIo::new(Fake(state.clone()), false);
    rpc.report = Some(Arc::new(move |failed, service| { *received.lock().unwrap() = Some((failed, service)); }));
    let error = rpc.read(&mut [0]).unwrap_err();
    let typed = error.get_ref().unwrap().downcast_ref::<crate::transport_host::HttpsOpenFailure>().unwrap();
    let mut derived = failure.clone();
    derived.facts = Some(facts);
    assert_eq!(typed.failure, derived);
    assert_eq!(typed.model_error().unwrap().code, crate::model::ErrorCode::CredentialHelperTimeout);
    assert_eq!(*report.lock().unwrap(), Some((derived, GitService::UploadPackExchange)));
    assert_eq!(state.lock().unwrap().failure, Some(failure));
}
#[test]
fn rpc_keeps_failed_facts_in_preference_to_later_close_facts() {
    use gwz_transport::protocol::*;
    let failure = Failure { code: ErrorCode::Authentication,
        facts: Some(Facts { method: AuthMethod::Gh, authenticated: Some(false), ..Default::default() }),
        ..Default::default() };
    let state = Arc::new(Mutex::new(State { failure: Some(failure.clone()),
        failure_facts: Some(Facts { method: AuthMethod::None, ..Default::default() }),
        ..State::default() }));
    let mut rpc = RpcIo::new(Fake(state), false);
    let error = rpc.read(&mut [0]).unwrap_err();
    let typed = error.get_ref().unwrap().downcast_ref::<crate::transport_host::HttpsOpenFailure>().unwrap();
    assert_eq!(typed.failure, failure);
}
#[test]
fn rpc_first_nonempty_read_ends_body_once_not_flush_or_empty_read() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut rpc = RpcIo::new(Fake(state.clone()), false);
    rpc.write_all(b"a request larger than a partial write")
        .unwrap();
    rpc.flush().unwrap();
    assert_eq!(rpc.read(&mut []).unwrap(), 0);
    assert_eq!(state.lock().unwrap().ended, 0);
    assert_eq!(rpc.read(&mut [0; 16]).unwrap(), 0);
    assert_eq!(rpc.read(&mut [0; 16]).unwrap(), 0);
    assert_eq!(state.lock().unwrap().ended, 1);
    assert_eq!(state.lock().unwrap().closed, 1);
    assert!(rpc.write(b"late").is_err());
    drop(rpc);
    assert_eq!(state.lock().unwrap().cancelled, 0);
}
#[test]
fn rpc_advertisement_rejects_body_and_unfinished_drop_cancels() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut rpc = RpcIo::new(Fake(state.clone()), true);
    assert!(rpc.write(b"body").is_err());
    drop(rpc);
    assert_eq!(state.lock().unwrap().cancelled, 1);
}

#[test]
fn advertisement_prefix_drop_gracefully_closes_unread_response() {
    let state = Arc::new(Mutex::new(State {
        reply: b"0000".to_vec(),
        ..State::default()
    }));
    let mut rpc = RpcIo::new(Fake(state.clone()), true);
    assert_eq!(rpc.read(&mut [0; 1]).unwrap(), 1);
    drop(rpc);
    let state = state.lock().unwrap();
    assert_eq!(state.closed, 1);
    assert_eq!(state.cancelled, 0);
}

fn open_failure(
    service: GitService,
    status: i64,
    method: gwz_transport::protocol::AuthMethod,
    authenticated: Option<bool>,
) -> git2::Error {
    let failure = crate::transport_host::HttpsOpenFailure {
            service: None, helpers_disabled: false, cli_hint: true,
        failure: gwz_transport::protocol::Failure {
            detail: None,
            setup_cause: None,
            code: gwz_transport::protocol::ErrorCode::RepositoryRefused,
            effect: gwz_transport::protocol::Effect::None,
            facts: Some(gwz_transport::protocol::Facts {
                method,
                authenticated,
                http_status: Some(status),
                ..Default::default()
            }),
        },
        anonymous: None,
        attempts: None,
    };
    map_open_error(io::Error::new(io::ErrorKind::Other, failure), service)
}

#[test]
fn a_failed_open_names_a_timeouts_origin_and_the_attempt_it_ended() {
    use gwz_transport::protocol::SetupFailureCause;
    let message = |setup_cause, attempts| {
        let failure = crate::transport_host::HttpsOpenFailure {
            service: None, helpers_disabled: false, cli_hint: true,
            failure: gwz_transport::protocol::Failure {
                detail: None,
                setup_cause,
                code: gwz_transport::protocol::ErrorCode::Timeout,
                effect: gwz_transport::protocol::Effect::None,
                facts: None,
            },
            anonymous: None,
            attempts,
        };
        map_open_error(
            io::Error::other(failure),
            GitService::UploadPackAdvertisement,
        )
        .message()
        .to_owned()
    };
    // The retry plan's §5 suffix, after the origin §4 decides by.
    assert_eq!(
        message(Some(SetupFailureCause::Aggregate), Some((4, 4))),
        "HTTPS endpoint request failed: Timeout: aggregate (attempt 4 of 4)"
    );
    assert_eq!(
        message(Some(SetupFailureCause::Allocation), None),
        "HTTPS endpoint request failed: Timeout: allocation"
    );
    assert_eq!(
        message(Some(SetupFailureCause::Interaction), None),
        "HTTPS endpoint request failed: Timeout: interaction"
    );
    assert_eq!(
        message(None, None),
        "HTTPS endpoint request failed: Timeout"
    );
}

#[test]
fn only_anonymous_or_gh_discovery_refusal_maps_to_private_repository_marker() {
    let error = open_failure(
        GitService::UploadPackAdvertisement,
        404,
        gwz_transport::protocol::AuthMethod::Gh,
        None,
    );
    assert_eq!(error.code(), git2::ErrorCode::NotFound);
    assert_eq!(error.class(), git2::ErrorClass::Http);
    assert_eq!(error.message(), REPOSITORY_REFUSED);

    let authenticated = open_failure(
        GitService::UploadPackAdvertisement,
        404,
        gwz_transport::protocol::AuthMethod::Gh,
        Some(true),
    );
    assert_eq!(authenticated.code(), git2::ErrorCode::GenericError);
    assert_ne!(authenticated.message(), REPOSITORY_REFUSED);
}

#[test]
fn exchange_refusal_never_maps_to_private_repository_marker() {
    let error = open_failure(
        GitService::ReceivePackExchange,
        403,
        gwz_transport::protocol::AuthMethod::None,
        None,
    );
    assert_eq!(error.code(), git2::ErrorCode::GenericError);
    assert_ne!(error.message(), REPOSITORY_REFUSED);
}

#[test]
fn helper_timeout_keeps_public_code_75_without_reclassifying_general_timeouts() {
    use gwz_transport::protocol::*;
    for budget in [None, Some(1_250)] {
        let failure = crate::transport_host::HttpsOpenFailure {
            service: None, helpers_disabled: false, cli_hint: true,
            failure: Failure {
                code: ErrorCode::Timeout,
                effect: Effect::None,
                setup_cause: Some(SetupFailureCause::Interaction),
                detail: budget.map(|ms| {
                    Box::new(FailureDetail {
                        helper_budget_ms: Some(ms),
                        ..Default::default()
                    })
                }),
                ..Default::default()
            },
            anonymous: None,
            attempts: None,
        };
        let git = map_open_error(
            io::Error::other(failure),
            GitService::UploadPackAdvertisement,
        );
        let model = crate::git::git_error(git);
        assert_eq!(
            model.code,
            if budget.is_some() {
                crate::model::ErrorCode::CredentialHelperTimeout
            } else {
                crate::model::ErrorCode::GitCommandFailed
            }
        );
    }
    let forged = git2::Error::new(
        git2::ErrorCode::GenericError,
        git2::ErrorClass::Http,
        "No credential helper answered within 1.25 seconds",
    );
    assert_eq!(
        crate::git::git_error(forged).code,
        crate::model::ErrorCode::GitCommandFailed
    );
}
