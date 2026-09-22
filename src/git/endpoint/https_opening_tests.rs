use super::*;
use crate::git::endpoint::{
    https_connection, https_policy,
    https_worker::{Endpoint, Prepared},
};
use gwz_transport::stream::{MessageEndpoint, Side, Stream};
use http_body_util::BodyExt;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;
#[path = "https_fixture.rs"]
mod fixture;
use fixture::*;
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
async fn opening(client: &Client, input: Input, automatic: bool) -> (Outcome, OpeningSession) {
    let open = open_for(client, &input).unwrap();
    let mut session = OpeningSession::new(
        "test-session".into(),
        "rpc".into(),
        open,
        "https-endpoint".into(),
        "endpoint-account".into(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let result = if automatic {
        let mut gh = input.clone();
        gh.policy = AuthPolicy::Gh;
        let open = open_for(client, &gh).unwrap();
        session
            .prepare_automatic(client, input, open, gh, &cancel)
            .await
    } else {
        session.prepare(client, input, &cancel).await
    };
    (result, session)
}
fn failed(outcome: Outcome, code: ErrorCode) -> Failure {
    match outcome {
        Outcome::Failed {
            failure, receipt, ..
        } => {
            assert_eq!(failure.code, code);
            assert_eq!(receipt.kind, MessageKind::OpenFailed);
            assert_eq!(receipt.open_failed.as_ref(), Some(&failure));
            assert!(receipt.opened.is_none() && receipt.failed.is_none());
            failure
        }
        Outcome::Rejected(failure) => panic!("mux rejected composition: {failure:?}"),
        Outcome::Ready { .. } => panic!("unexpected stream preparation"),
    }
}
#[test]
fn actual_status_failures_cross_mux_once_before_any_stream() {
    runtime().block_on(async {
        for (status, code) in [
            (401, ErrorCode::Authentication),
            (403, ErrorCode::RepositoryRefused),
            (404, ErrorCode::RepositoryRefused),
            (500, ErrorCode::Io),
        ] {
            let server = Server::start(Arc::new(move |_| {
                Box::pin(async move { response(status, GitService::UploadPackAdvertisement, "") })
            }))
            .await;
            let mut endpoint = Endpoint::new(
                server.config(),
                None,
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            let (outcome, session) = opening(
                &endpoint.client,
                input(&server, GitService::UploadPackAdvertisement),
                false,
            )
            .await;
            assert_eq!(
                failed(outcome, code).facts.unwrap().http_status,
                Some(status as i64)
            );
            assert_eq!(session.receipts().len(), 1);
            assert_eq!(session.initiator.active_streams(), 0);
            assert_eq!(session.endpoint.active_streams(), 0);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        }
    });
}
#[test]
fn anonymous_failure_crosses_mux_before_distinct_gh_open() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async { response(401, GitService::UploadPackAdvertisement, "") })
        }))
        .await;
        let mut endpoint = Endpoint::new(
            server.config(),
            None,
            gwz_transport::pool::Config::default(),
        )
        .unwrap();
        let (outcome, session) = opening(
            &endpoint.client,
            input(&server, GitService::UploadPackAdvertisement),
            true,
        )
        .await;
        match &outcome {
            Outcome::Failed {
                first_failure: Some(first),
                ..
            } => assert_eq!(first.facts.as_ref().unwrap().http_status, Some(401)),
            _ => panic!("missing first failure"),
        }
        failed(outcome, ErrorCode::Authentication);
        assert_eq!(session.receipts().len(), 2);
        assert_ne!(
            session.receipts()[0].stream_id,
            session.receipts()[1].stream_id
        );
        assert!(
            session
                .receipts()
                .iter()
                .all(|r| r.kind == MessageKind::OpenFailed)
        );
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}
#[test]
fn successful_headers_cross_mux_before_stream_construction() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
        }))
        .await;
        let mut endpoint = Endpoint::new(
            server.config(),
            None,
            gwz_transport::pool::Config::default(),
        )
        .unwrap();
        let (outcome, session) = opening(
            &endpoint.client,
            input(&server, GitService::UploadPackAdvertisement),
            false,
        )
        .await;
        match outcome {
            Outcome::Ready {
                prepared, receipt, ..
            } => {
                assert_eq!(receipt.kind, MessageKind::Opened);
                assert_eq!(receipt.stream_id, session.stream_id());
                drop(prepared);
            }
            _ => panic!("missing Opened"),
        }
        assert_eq!(session.receipts().len(), 1);
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}
#[test]
fn actual_malformed_headers_and_network_loss_have_distinct_mux_failures() {
    runtime().block_on(async {
        for (raw, code) in [
            (b"HTTP/1.1 WHAT\r\n\r\n".to_vec(), ErrorCode::Protocol),
            (
                b"HTTP/1.1 200 OK\r\nbad header\r\n\r\n".to_vec(),
                ErrorCode::Protocol,
            ),
            (Vec::new(), ErrorCode::Io),
        ] {
            let server = Server::raw(raw).await;
            let mut endpoint = Endpoint::new(
                server.config(),
                None,
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            let (outcome, session) = opening(
                &endpoint.client,
                input(&server, GitService::UploadPackAdvertisement),
                false,
            )
            .await;
            failed(outcome, code);
            assert_eq!(session.receipts().len(), 1);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        }
    });
}
#[test]
fn trust_and_exhausted_network_budget_are_open_failed() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|_| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(40)).await;
                response(200, GitService::UploadPackAdvertisement, "ok")
            })
        }))
        .await;
        for trust in [false, true] {
            let tls = if trust {
                server.config()
            } else {
                https_connection::Config::default()
            };
            let mut endpoint =
                Endpoint::new_with_io_timeout(tls, None, gwz_transport::pool::Config::default(), 5)
                    .unwrap();
            let (outcome, session) = opening(
                &endpoint.client,
                input(&server, GitService::UploadPackAdvertisement),
                false,
            )
            .await;
            failed(
                outcome,
                if trust {
                    ErrorCode::Timeout
                } else {
                    ErrorCode::Trust
                },
            );
            assert_eq!(session.receipts().len(), 1);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        }
    });
}
cfg_if::cfg_if! { if #[cfg(unix)] {
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use crate::git::endpoint::https_auth;
    fn helper(body: &str, environment: Vec<(std::ffi::OsString,std::ffi::OsString)>) -> (tempfile::TempDir, https_auth::Config) {
        let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("gh");
        std::fs::write(&path,format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir,https_auth::Config{executable:path,environment})
    }
    #[test]
    fn redirected_helper_failure_preserves_earlier_credential_offer_in_open_failed() {
        runtime().block_on(async {
            let target=Server::start(Arc::new(|_|Box::pin(async {panic!("target helper failed; request must not run")}))).await;
            let target_url=target.url.clone();
            let source=Server::start(Arc::new(move |request| { let url=target_url.clone(); Box::pin(async move {
                assert_eq!(request.headers()["Authorization"],"Basic Zml4dHVyZTpzZW50aW5lbA==");
                let mut r=response(302,GitService::UploadPackAdvertisement,"");
                r.headers_mut().insert("Location",format!("{url}/info/refs").parse().unwrap());r
            })})).await;
            let source_host=HttpsDestination::parse(&source.url).unwrap().authority();
            let (_dir,auth)=helper("input=$(/bin/cat)\ncase \"$input\" in *\"host=$SOURCE\"*) printf 'username=fixture\\npassword=sentinel\\n\\n';; *) exit 1;; esac",vec![("SOURCE".into(),source_host.into())]);
            let mut endpoint=Endpoint::new(source.config(),Some(auth),gwz_transport::pool::Config::default()).unwrap();
            let mut request=input(&source,GitService::UploadPackAdvertisement);request.policy=AuthPolicy::Gh;
            let (outcome,session)=opening(&endpoint.client,request,false).await;
            let failure=failed(outcome,ErrorCode::Authentication);
            let facts=failure.facts.as_ref().unwrap();assert!(facts.credential_offered);assert_eq!(facts.http_status,None);assert_eq!(facts.authenticated,None);
            let rendered=format!("{failure:?}");assert!(!rendered.contains("sentinel")&&!rendered.contains(&source.url)&&!rendered.contains(&target.url));
            assert_eq!(session.receipts().len(),1);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
        });
    }
    #[test]
    fn anonymous_auth_transition_does_not_refill_the_network_allowance() {
        runtime().block_on(async {
            let server=Server::start(Arc::new(|request|Box::pin(async move {tokio::time::sleep(Duration::from_millis(40)).await;response(if request.headers().contains_key("Authorization"){200}else{401},GitService::UploadPackAdvertisement,"ok")}))).await;
            let (_dir,auth)=helper("/bin/cat >/dev/null\nprintf 'username=fixture\\npassword=token\\n\\n'",vec![]);
            let mut endpoint=Endpoint::new_with_io_timeout(server.config(),Some(auth),gwz_transport::pool::Config::default(),65).unwrap();
            let (outcome,session)=opening(&endpoint.client,input(&server,GitService::UploadPackAdvertisement),true).await;
            failed(outcome,ErrorCode::Timeout);
            assert_eq!(session.receipts().len(),2);assert_eq!(session.receipts()[0].open_failed.as_ref().unwrap().facts.as_ref().unwrap().http_status,Some(401));
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
        });
    }
}
} }
