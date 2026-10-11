//! A machine proxy in the CONNECT tunnel (plan step 4.7): the transport's own CONNECT through a fixture proxy that
//! records it, chosen by the machine setting's rules from an injected source. The fixture runs on every platform.

use super::testing::Fixed;
use crate::git::endpoint::https_connection::Config;
use crate::git::endpoint::https_fixture::{Server, Tunnel, attach, input, response};
use crate::git::endpoint::https_worker::{Endpoint, Prepared};
use gwz_transport::protocol::{ErrorCode, Failure, GitService};
use hyper::header::{AUTHORIZATION, HOST, LOCATION};
use std::{future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// The proxy's machine setting: a bare entry for the fixture proxy, with `bypass` as the list.
fn machine_config(server: &Server, proxy_port: u16, bypass: Option<&str>) -> Config {
    let mut config = server.config();
    Fixed::named(&format!("127.0.0.1:{proxy_port}"), bypass)
        .capture()
        .apply(&mut config)
        .expect("a usable machine setting");
    config
}

pub(super) fn port_of(server: &Server) -> u16 {
    url::Url::parse(&server.url).unwrap().port().unwrap()
}

/// An origin that answers 200 to a request with no credential of any kind.
pub(super) async fn plain_server() -> Server {
    Server::start(Arc::new(|request| {
        Box::pin(async move {
            assert!(!request.headers().contains_key("Proxy-Authorization"));
            assert!(!request.headers().contains_key(AUTHORIZATION));
            response(200, GitService::UploadPackAdvertisement, "ok")
        })
    }))
    .await
}

async fn finish(prepared: Prepared) {
    let (stream, task) = attach(prepared);
    stream.end_write().await.unwrap();
    let mut buffer = [0; 113];
    while stream.read(&mut buffer).await.unwrap() != 0 {}
    stream.close().await.unwrap();
    task.await.unwrap();
}

/// One discovery request to `destination` through `config`, then the endpoint's shutdown.
pub(super) async fn discover(
    server: &Server,
    config: Config,
    destination: &str,
) -> Result<(), Failure> {
    let mut endpoint = Endpoint::new(config, None, gwz_transport::pool::Config::default()).unwrap();
    let mut request = input(server, GitService::UploadPackAdvertisement);
    request.destination = destination.to_owned();
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        endpoint
            .client
            .prepare_auto(request, &CancellationToken::new(), &mut None),
    )
    .await
    .expect("the open ends");
    let outcome = match result {
        Ok(prepared) => {
            finish(prepared).await;
            Ok(())
        }
        Err(failure) => Err(failure),
    };
    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    outcome
}

pub(super) fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn a_proxied_https_origin_is_reached_by_a_connect_to_its_authority_and_no_credential() {
    run(async {
        let server = plain_server().await;
        let port = port_of(&server);
        let tunnel = Tunnel::forwarding(200, port).await;
        let config = machine_config(&server, tunnel.port, None);
        discover(
            &server,
            config,
            &format!("https://proxied.test:{port}/repo"),
        )
        .await
        .expect("the tunnel carries the exchange");
        let seen = tunnel.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(
            seen[0].starts_with(&format!("CONNECT proxied.test:{port} HTTP/1.1\r\n")),
            "{}",
            seen[0]
        );
        // The tunnel carries no credential: not a proxy one, and no origin one either (the server asserts it too).
        assert!(
            !seen[0].to_ascii_lowercase().contains("authorization"),
            "{}",
            seen[0]
        );
        assert_eq!(
            server.connections.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    });
}

#[test]
fn loopback_origins_bypass_the_machine_proxy_with_an_empty_list() {
    run(async {
        let server = plain_server().await;
        let tunnel = Tunnel::forwarding(200, port_of(&server)).await;
        let config = machine_config(&server, tunnel.port, None);
        discover(&server, config, &server.url)
            .await
            .expect("localhost connects directly");
        assert!(tunnel.seen.lock().unwrap().is_empty());
        assert_eq!(
            server.connections.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    });
}

#[test]
fn a_bypass_list_entry_sends_the_proxied_name_around_the_proxy() {
    run(async {
        let server = plain_server().await;
        let port = port_of(&server);
        let tunnel = Tunnel::forwarding(200, port).await;
        let config = machine_config(&server, tunnel.port, Some("proxied.test"));
        // `proxied.test` has no address, so the direct attempt fails; what matters is that the proxy saw nothing.
        let outcome = discover(
            &server,
            config,
            &format!("https://proxied.test:{port}/repo"),
        )
        .await;
        assert!(outcome.is_err());
        assert!(tunnel.seen.lock().unwrap().is_empty());
        assert_eq!(
            server.connections.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    });
}

#[test]
fn a_proxy_that_cannot_be_reached_ends_the_operation_and_never_goes_direct() {
    run(async {
        let server = plain_server().await;
        let port = port_of(&server);
        let mut config = server.config();
        Fixed::named("gwz-no-such-proxy.invalid:3128", None)
            .capture()
            .apply(&mut config)
            .unwrap();
        let outcome = discover(
            &server,
            config,
            &format!("https://proxied.test:{port}/repo"),
        )
        .await;
        assert!(outcome.is_err());
        assert_eq!(
            server.connections.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    });
}

#[test]
fn a_407_on_the_connect_is_refused_as_proxy_sign_in_with_nothing_sent() {
    run(async {
        let server = plain_server().await;
        let port = port_of(&server);
        let tunnel = Tunnel::forwarding(407, port).await;
        let config = machine_config(&server, tunnel.port, None);
        let failure = discover(
            &server,
            config,
            &format!("https://proxied.test:{port}/repo"),
        )
        .await
        .expect_err("the proxy asks for sign-in");
        assert_eq!(failure.code, ErrorCode::Authentication);
        let facts = failure
            .facts
            .expect("the refusal says it was the proxy's 407");
        assert_eq!(facts.http_status, Some(407));
        assert!(!facts.credential_offered);
        let seen = tunnel.seen.lock().unwrap();
        assert_eq!(
            seen.len(),
            1,
            "one CONNECT and no retry with a credential: {seen:?}"
        );
        assert!(
            !seen[0].to_ascii_lowercase().contains("authorization"),
            "{}",
            seen[0]
        );
        assert_eq!(
            server.connections.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    });
}

#[test]
fn the_bypass_rules_apply_to_the_destination_a_discovery_redirect_names() {
    run(async {
        // The first host answers with a redirect to `localhost`, which the implicit bypass connects to directly.
        let server = Server::start(Arc::new(|request| {
            Box::pin(async move {
                let host = request
                    .headers()
                    .get(HOST)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                let port = host.rsplit_once(':').unwrap().1.to_owned();
                if host.starts_with("proxied.test") {
                    let mut redirect = response(302, GitService::UploadPackAdvertisement, "");
                    redirect.headers_mut().insert(
                        LOCATION,
                        format!("https://localhost:{port}/repo/info/refs?service=git-upload-pack")
                            .parse()
                            .unwrap(),
                    );
                    redirect
                } else {
                    response(200, GitService::UploadPackAdvertisement, "ok")
                }
            })
        }))
        .await;
        let port = port_of(&server);
        let tunnel = Tunnel::forwarding(200, port).await;
        let config = machine_config(&server, tunnel.port, None);
        discover(
            &server,
            config,
            &format!("https://proxied.test:{port}/repo"),
        )
        .await
        .expect("the redirect is followed");
        let seen = tunnel.seen.lock().unwrap();
        assert_eq!(
            seen.len(),
            1,
            "only the first host went through the proxy: {seen:?}"
        );
        assert!(seen[0].starts_with(&format!("CONNECT proxied.test:{port} ")));
        assert_eq!(
            server.connections.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
    });
}

#[test]
fn an_ip_literal_https_origin_that_is_not_local_goes_to_the_proxy_never_direct() {
    // No row of 1.0.17 ran an IP-literal https origin (design §6, the one rule for forms with no executed row): it
    // follows WinHTTP's rule toward the proxy. The fixture certificate does not name these addresses, so the exchange
    // ends in a TLS refusal after the tunnel; what is asserted is that the CONNECT reached the proxy first.
    run(async {
        let server = plain_server().await;
        let port = port_of(&server);
        for (literal, authority) in [
            ("192.0.2.1", format!("192.0.2.1:{port}")),
            ("[2001:db8::1]", format!("[2001:db8::1]:{port}")),
        ] {
            let tunnel = Tunnel::forwarding(200, port).await;
            let config = machine_config(&server, tunnel.port, None);
            let outcome =
                discover(&server, config, &format!("https://{literal}:{port}/repo")).await;
            assert!(outcome.is_err(), "the certificate does not name {literal}");
            let seen = tunnel.seen.lock().unwrap();
            assert_eq!(seen.len(), 1, "{literal}: {seen:?}");
            assert!(
                seen[0].starts_with(&format!("CONNECT {authority} HTTP/1.1\r\n")),
                "{}",
                seen[0]
            );
        }
    });
}
