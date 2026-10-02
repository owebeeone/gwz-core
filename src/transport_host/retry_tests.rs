//! An operation's `--max-retries` through the transport host: the request's
//! policy reaches the SSH and the HTTPS endpoint that retry its setups, and
//! the driver waits out every attempt (the retry plan's §5; TR2.1).
use super::session::SshOpenFailure;
use super::*;
use crate::{RequestMeta, TransportOptions, TransportPlacement};
use gwz_transport::protocol::{ErrorCode, SetupFailureCause};
use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Accepts connections and never answers, so an SSH setup stalls; counts the
/// connections it accepted.
struct Silent {
    port: u16,
    accepted: Arc<AtomicUsize>,
}
impl Silent {
    fn new() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = accepted.clone();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept() {
                counter.fetch_add(1, Ordering::AcqRel);
                held.push(stream);
            }
        });
        Self { port, accepted }
    }
}

#[test]
fn the_operations_max_retries_reaches_its_endpoint_and_its_open_waits_out_every_attempt() {
    for (max_retries, attempts) in [(0, 1), (1, 2)] {
        let server = Silent::new();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
        std::fs::write(home.path().join(".ssh/known_hosts"), "").unwrap();
        let config =
            SshEndpointConfig::fixture(home.path().to_path_buf(), None).with_io_timeout_ms(300);
        let runtime = TransportRuntime::new(config).unwrap();
        let meta = RequestMeta {
            request_id: format!("retries-{max_retries}"),
            schema_version: "gwz.protocol/v0".into(),
            transport: Some(TransportOptions {
                placement: Some(TransportPlacement::Local),
                ..Default::default()
            }),
            policy: Some(crate::OperationPolicy {
                max_retries: Some(max_retries),
                ..Default::default()
            }),
            ..Default::default()
        };
        let request = super::driver_tests::block_on(runtime.request(meta, "fetch".into())).unwrap();
        let result = request.context.open(
            &format!("ssh://git@127.0.0.1:{}/repo.git", server.port),
            crate::git::endpoint::ssh_channel::GitService::UploadPack,
            None,
            Arc::new(|_, _| panic!("a stalled setup opens no stream")),
            Arc::new(|_| {}),
        );
        let Err(error) = result else {
            panic!("a stalled setup cannot open");
        };
        let failure = &error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<SshOpenFailure>())
            .expect("an open failure carries the endpoint's failure")
            .0;
        assert_eq!(
            (failure.code, failure.setup_cause),
            (ErrorCode::Timeout, Some(SetupFailureCause::Stall)),
            "--max-retries {max_retries}"
        );
        assert_eq!(
            server.accepted.load(Ordering::Acquire),
            attempts,
            "--max-retries {max_retries}"
        );
        // The final failure's display names the attempt it ended (§5).
        assert_eq!(
            error.to_string(),
            format!("ssh setup timeout: stall (attempt {attempts} of {attempts})")
        );
        super::driver_tests::block_on(request.finish());
        super::driver_tests::block_on(runtime.shutdown());
    }
}

#[test]
fn an_https_setup_that_stalls_is_retried_and_its_display_names_the_attempt() {
    use gwz_transport::protocol::{AuthPolicy, GitService};
    for (max_retries, attempts) in [(0, 1), (1, 2)] {
        // An HTTPS connect has no stall clock: a handshake that never answers
        // runs out of the 300 ms aggregate, which is retried (§4).
        let server = Silent::new();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
        std::fs::write(home.path().join(".ssh/known_hosts"), "").unwrap();
        let config = SshEndpointConfig::fixture(home.path().to_path_buf(), None)
            .with_connect_timeout_ms(300);
        let https = HttpsEndpointConfig {
            tls: crate::git::endpoint::https_connection::Config::default(),
            auth: None,
        };
        let runtime = TransportRuntime::with_https(config, https, HelperSlots::new()).unwrap();
        let meta = RequestMeta {
            request_id: format!("https-retries-{max_retries}"),
            schema_version: "gwz.protocol/v0".into(),
            transport: Some(TransportOptions {
                placement: Some(TransportPlacement::Local),
                ..Default::default()
            }),
            policy: Some(crate::OperationPolicy {
                max_retries: Some(max_retries),
                ..Default::default()
            }),
            ..Default::default()
        };
        let request = super::driver_tests::block_on(runtime.request(meta, "fetch".into())).unwrap();
        let result = request.context.open_https_recording(
            &format!("https://127.0.0.1:{}/repo", server.port),
            GitService::UploadPackAdvertisement,
            Some(AuthPolicy::Anonymous),
            Arc::new(|_, _| panic!("a stalled setup opens no stream")),
            Arc::new(|_| {}),
            Arc::new(Mutex::new(None)),
        );
        let Err(error) = result else {
            panic!("a stalled setup cannot open");
        };
        let failure = &error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<HttpsOpenFailure>())
            .expect("an open failure carries the endpoint's failure")
            .failure;
        assert_eq!(
            (failure.code, failure.setup_cause),
            (ErrorCode::Timeout, Some(SetupFailureCause::Aggregate)),
            "--max-retries {max_retries}"
        );
        assert_eq!(
            server.accepted.load(Ordering::Acquire),
            attempts,
            "--max-retries {max_retries}"
        );
        // The final failure's display names its origin and the attempt it
        // ended (§5).
        assert_eq!(
            error.to_string(),
            format!(
                "HTTPS endpoint request failed: Timeout: aggregate (attempt {attempts} of {attempts})"
            )
        );
        super::driver_tests::block_on(request.finish());
        super::driver_tests::block_on(runtime.shutdown());
    }
}

#[test]
fn the_cli_placement_refuses_a_retry_budget_it_cannot_carry_to_its_endpoint() {
    use super::driver_tests::{CliHarness, block_on};
    // The Cli placement's endpoint is beyond the port, and the session
    // protocol carries no budget to it: it would retry three times while
    // this driver counted the request's budget (the State review's P3-1).
    let harness = CliHarness::new();
    let mut meta = harness.meta("cli-budget");
    meta.policy = Some(crate::OperationPolicy {
        max_retries: Some(0),
        ..Default::default()
    });
    let _budgeted = harness.endpoint.register_request(&meta.request_id).unwrap();
    let Err(error) = block_on(harness.runtime.request(meta, "fetch".into())) else {
        panic!("a Cli-placed request cannot carry --max-retries");
    };
    assert_eq!(error.code, crate::model::ErrorCode::UnsupportedOperation);
    assert!(error.message.contains("Cli placement"), "{}", error.message);
    // Without one it opens, and the driver's record keeps the default.
    let meta = harness.meta("cli-default");
    let _endpoint = harness.endpoint.register_request(&meta.request_id).unwrap();
    let request = block_on(harness.runtime.request(meta, "fetch".into())).unwrap();
    assert_eq!(
        request.context.session.max_retries("cli-default"),
        crate::git::endpoint::setup_retry::DEFAULT_MAX_RETRIES
    );
    block_on(request.finish());
    block_on(harness.runtime.shutdown());
}

#[test]
fn a_negative_max_retries_is_refused_before_the_request_registers() {
    let mut meta = RequestMeta {
        request_id: "negative".into(),
        schema_version: "gwz.protocol/v0".into(),
        ..Default::default()
    };
    meta.policy = Some(crate::OperationPolicy {
        max_retries: Some(-1),
        ..Default::default()
    });
    let error = validate_request_context(&meta, "fetch").unwrap_err();
    assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
}

#[test]
fn a_max_retries_beyond_u32_is_refused_before_the_request_registers_and_u32_max_is_kept() {
    let meta = |max_retries: i64| RequestMeta {
        request_id: "range".into(),
        schema_version: "gwz.protocol/v0".into(),
        policy: Some(crate::OperationPolicy {
            max_retries: Some(max_retries),
            ..Default::default()
        }),
        ..Default::default()
    };
    // A value the endpoint's count cannot hold is refused, not saturated to
    // one the request never named (the State review's P3-2).
    let error = validate_request_context(&meta(i64::from(u32::MAX) + 1), "fetch").unwrap_err();
    assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
    validate_request_context(&meta(i64::from(u32::MAX)), "fetch").unwrap();
    assert_eq!(request::max_retries(&meta(i64::from(u32::MAX))), u32::MAX);
}
