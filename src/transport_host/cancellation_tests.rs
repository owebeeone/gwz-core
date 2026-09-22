use super::*;
use crate::git::endpoint::{
    https_destination::Destination as HttpsDestination, shared_reservation::Authority,
};
use gwz_transport::{pool, protocol::*};
use std::{
    sync::Arc,
    task::{Context, Waker},
    time::Duration,
};

use crate::transport_host::https_tests::fixture;

#[test]
fn cancel_after_opened_is_queued_replaces_opened_with_cancelled_terminal() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = fixture::Server::start(Arc::new(|_| {
                Box::pin(async {
                    fixture::response(200, GitService::UploadPackAdvertisement, "ok")
                })
            }))
            .await;
            let destination = HttpsDestination::parse(&server.url).unwrap();
            let config = pool::Config::default();
            let authority = Authority::new(config.total, config.per_host);
            let mut endpoint = HttpsEndpoint::new(
                HttpsEndpointConfig {
                    tls: server.config(),
                    auth: None,
                },
                config,
                3_000,
                authority,
                "endpoint".into(),
            )
            .unwrap();
            let open = Open {
                endpoint_id: "endpoint".into(),
                operation_id: "operation".into(),
                destination: Destination {
                    scheme: Scheme::Https,
                    host: destination.host().into(),
                    port: destination.port() as i64,
                    path: destination.url.path().into(),
                    ssh_username: None,
                },
                service: GitService::UploadPackAdvertisement,
                identity: Identity {
                    mode: IdentityMode::CredentialsDisabled,
                    ..Default::default()
                },
                policy: AuthPolicy::Anonymous,
                deadlines: Deadlines {
                    allocation_ms: 30_000,
                    connect_ms: 10_000,
                    io_ms: 3_000,
                    interaction_ms: 120_000,
                    cleanup_ms: 5_000,
                },
                receive_limits: gwz_transport::binding::default_limits(),
            };
            endpoint
                .accept(
                    "request".into(),
                    Envelope {
                        version: 2,
                        session_id: "session".into(),
                        stream_id: 1,
                        kind: MessageKind::Open,
                        open: Some(open),
                        ..Default::default()
                    },
                )
                .unwrap();
            let mut cx = Context::from_waker(Waker::noop());
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                endpoint.step(0, &mut cx).unwrap();
                if endpoint
                    .entries
                    .get(&("request".into(), 1))
                    .and_then(|entry| entry.output.as_ref())
                    .is_some()
                {
                    break;
                }
                assert!(tokio::time::Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            endpoint
                .accept(
                    "request".into(),
                    Envelope {
                        version: 2,
                        session_id: "session".into(),
                        stream_id: 1,
                        kind: MessageKind::Cancel,
                        ..Default::default()
                    },
                )
                .unwrap();
            endpoint.step(0, &mut cx).unwrap();
            let terminal = endpoint.take_outbound(&mut cx).expect("cancel terminal");
            assert_eq!(terminal.envelope.kind, MessageKind::OpenFailed);
            let failure = terminal.envelope.open_failed.unwrap();
            assert_eq!(failure.code, ErrorCode::Cancelled);
            assert_eq!(failure.effect, Effect::None);
            endpoint.shutdown();
            let until = tokio::time::Instant::now() + Duration::from_secs(2);
            while endpoint.pending() != 0 {
                endpoint.step(0, &mut cx).unwrap();
                assert!(
                    endpoint.take_outbound(&mut cx).is_none(),
                    "exactly one opening terminal"
                );
                assert!(
                    tokio::time::Instant::now() < until,
                    "cleanup retains live owner"
                );
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        });
}
