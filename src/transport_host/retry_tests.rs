//! An operation's `--max-retries` through the transport host: the request's
//! policy reaches the endpoint that retries its setups, and the driver waits
//! out every attempt (the retry plan's §5; TR2.1).
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
        super::driver_tests::block_on(request.finish());
        super::driver_tests::block_on(runtime.shutdown());
    }
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
