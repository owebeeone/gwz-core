use super::*;
use std::{sync::mpsc, thread};

fn fixture() -> PlacementEndpoint {
    struct NoConnect;
    impl super::super::ssh_pool::Connector for NoConnect {
        type Resource = super::super::ssh_setup::NativeResource;
        fn start(
            &mut self,
            _: &Key,
            _: &gwz_transport::pool::Identity,
            _: Option<u64>,
        ) -> Result<Self::Resource, Failure> {
            panic!("identity check must not connect");
        }
    }
    PlacementEndpoint::new(
        Endpoint::with_connector(gwz_transport::pool::Config::default(), |_| NoConnect, 100)
            .unwrap(),
        PathBuf::from("/tmp"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap()
}
fn check(path: &Path, timeout_ms: i64) -> Envelope {
    Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id: 1,
        kind: MessageKind::CheckIdentity,
        check_identity: Some(gwz_transport::protocol::CheckIdentity {
            endpoint_id: "endpoint".into(),
            operation_id: "check".into(),
            identity: Identity {
                mode: IdentityMode::ExplicitKey,
                key_path: Some(path.to_str().unwrap().into()),
                path_base: None,
            },
            timeout_ms,
        }),
        ..Default::default()
    }
}
#[test]
fn blocked_check_times_out_before_physical_disposal() {
    let mut endpoint = fixture();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let job = Job::start(None, Duration::from_millis(1), move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        Ok(())
    })
    .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let envelope = check(Path::new("/tmp/unused"), 10);
    let key = ("request".into(), 1);
    endpoint
        .requests
        .insert(key.clone(), request_state(&envelope, "check".into()));
    endpoint.checks.push(CheckJob {
        key,
        job,
        deadline: 10,
        cancelled: false,
    });
    let mut cx = Context::from_waker(std::task::Waker::noop());
    endpoint.step(10, &mut cx).unwrap();
    let result = endpoint.take_outbound();
    let pending = endpoint.pending_request_count("request");
    endpoint.step(11, &mut cx).unwrap();
    let duplicate = endpoint.take_outbound();
    release_tx.send(()).unwrap();
    for _ in 0..500 {
        endpoint.step(12, &mut cx).unwrap();
        if endpoint.pending_request_count("request") == 0 {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        result
            .expect("deadline terminal must not await disposal")
            .envelope
            .identity_check_failed
            .unwrap()
            .code,
        ErrorCode::Timeout
    );
    assert!(pending > 0, "blocked physical work must stay charged");
    assert!(duplicate.is_none());
    assert_eq!(endpoint.pending_request_count("request"), 0);
}
#[test]
fn fifo_identity_without_writer_rejects_without_blocking() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, os::unix::fs::OpenOptionsExt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity-fifo");
    let cpath = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let mut endpoint = fixture();
    endpoint
        .accept("request".into(), check(&path, 1000))
        .unwrap();
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let until = Instant::now() + Duration::from_millis(200);
    let result = loop {
        endpoint.step(0, &mut cx).unwrap();
        if let Some(result) = endpoint.take_outbound() {
            break Some(result);
        }
        if Instant::now() >= until {
            break None;
        }
        thread::sleep(Duration::from_millis(1));
    };
    // Release the original buggy blocking open before asserting, so red leaves no hung job.
    let _writer = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path);
    assert_eq!(
        result
            .expect("special file admission blocked")
            .envelope
            .identity_check_failed
            .unwrap()
            .code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn unsupported_deadline_policy_has_no_physical_or_queued_work() {
    let mut endpoint = fixture();
    for (index, deadlines) in [
        gwz_transport::protocol::Deadlines {
            allocation_ms: i64::MAX,
            connect_ms: 1,
            io_ms: 1,
            interaction_ms: i64::MAX,
            cleanup_ms: 1,
        },
        gwz_transport::protocol::Deadlines {
            allocation_ms: 1,
            connect_ms: 1,
            io_ms: 1,
            interaction_ms: 1,
            cleanup_ms: i64::MAX,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let envelope = Envelope {
            version: 2,
            session_id: "session".into(),
            stream_id: index as i64 + 1,
            kind: MessageKind::Open,
            open: Some(gwz_transport::protocol::Open {
                endpoint_id: "endpoint".into(),
                operation_id: "operation".into(),
                destination: Destination {
                    scheme: gwz_transport::protocol::Scheme::Ssh,
                    host: "host".into(),
                    port: 22,
                    path: "/repo".into(),
                    ssh_username: Some("git".into()),
                },
                service: GitService::UploadPackExchange,
                identity: Identity::default(),
                policy: gwz_transport::protocol::AuthPolicy::SshAmbient,
                deadlines,
                receive_limits: gwz_transport::binding::default_limits(),
            }),
            ..Default::default()
        };
        gwz_transport::codec::admit(&envelope).unwrap();
        endpoint.accept("request".into(), envelope).unwrap();
        assert_eq!(
            endpoint
                .take_outbound()
                .unwrap()
                .envelope
                .open_failed
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
        assert!(endpoint.opens.is_empty());
        assert!(endpoint.queued_opens.is_empty());
        assert_eq!(endpoint.endpoint.pending_requests(), 0);
    }
}
