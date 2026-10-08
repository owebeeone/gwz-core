//! One endpoint's connections share one TLS configuration, built once, and the
//! trust each connection has is the trust its endpoint's configuration names:
//! sharing the configuration neither widens nor narrows it.
use super::*;
use crate::git::endpoint::{
    https_connection::HttpConnector,
    https_fixture::{Server, response},
    ssh_pool::{Connector, Resource},
};
use std::task::{Context, Poll, Waker};
use tokio::sync::Semaphore;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn server() -> Server {
    Server::start(Arc::new(|_| {
        Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
    }))
    .await
}

fn key_of(server: &Server) -> Key {
    let destination = Destination::parse(&server.url).unwrap();
    Key::https(destination.host(), destination.port())
}

fn connector(config: https_connection::Config) -> HttpConnector {
    HttpConnector::new(
        config,
        std::time::Instant::now(),
        Arc::new(Semaphore::new(8)),
        crate::git::endpoint::agent_job::Supervisor::new(),
    )
}

/// A CA certificate, in PEM, that issued nothing the fixture serves.
fn unrelated_ca() -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("openssl")
        .current_dir(dir.path())
        .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes"])
        .args(["-keyout", "key.pem", "-out", "ca.pem", "-days", "2"])
        .args(["-subj", "/CN=GWZ Unrelated CA"])
        .args(["-addext", "basicConstraints=critical,CA:TRUE"])
        .args(["-addext", "keyUsage=critical,keyCertSign,cRLSign"])
        .output()
        .unwrap();
    assert!(output.status.success(), "openssl made no CA certificate");
    std::fs::read(dir.path().join("ca.pem")).unwrap()
}

/// Polls `resource` until it has connected or failed, and says which.
async fn settle(resource: &mut <HttpConnector as Connector>::Resource) -> Result<(), Failure> {
    let mut cx = Context::from_waker(Waker::noop());
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if let Poll::Ready(result) = resource.poll_connected(&mut cx) {
            return result.map(|_| ());
        }
        assert!(Instant::now() < until, "a connection never settled");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

async fn dispose(resource: &mut <HttpConnector as Connector>::Resource) {
    let mut cx = Context::from_waker(Waker::noop());
    while resource.poll_dispose(&mut cx, false).is_pending() {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// The measured cost: a TLS connector per connection parses the CA bundle
/// twice each, 17 times at 16 members, which is most of the command's CPU.
#[test]
fn sixteen_connections_of_one_connector_build_one_tls_configuration() {
    runtime().block_on(async {
        let server = server().await;
        let mut connector = connector(server.config());
        let key = key_of(&server);
        let mut resources: Vec<_> = (0..16)
            .map(|_| {
                connector
                    .start(&key, &pool::Identity::Https, None)
                    .expect("connection starts")
            })
            .collect();
        for resource in &mut resources {
            settle(resource).await.expect("the fixture's CA verifies");
        }
        assert_eq!(server.connections.load(Ordering::SeqCst), 16);
        assert_eq!(
            connector.tls_builds(),
            1,
            "every connection built its own TLS configuration"
        );
        for resource in &mut resources {
            dispose(resource).await;
        }
    });
}

/// A connector that builds its configuration at its first connection keeps
/// that one for connections that start after the first has finished.
#[test]
fn a_connection_started_after_the_first_has_ended_reuses_the_configuration() {
    runtime().block_on(async {
        let server = server().await;
        let mut connector = connector(server.config());
        let key = key_of(&server);
        for _ in 0..3 {
            let mut resource = connector.start(&key, &pool::Identity::Https, None).unwrap();
            settle(&mut resource).await.unwrap();
            dispose(&mut resource).await;
        }
        assert_eq!(connector.tls_builds(), 1);
    });
}

/// The CA file's certificates are the trust: the fixture's CA verifies the
/// fixture's server, through every connection of one shared configuration.
#[test]
fn the_ca_a_connector_names_verifies_every_connection_it_makes() {
    runtime().block_on(async {
        let server = server().await;
        let mut connector = connector(server.config());
        let key = key_of(&server);
        let mut first = connector.start(&key, &pool::Identity::Https, None).unwrap();
        let mut second = connector.start(&key, &pool::Identity::Https, None).unwrap();
        settle(&mut first).await.expect("first connection");
        settle(&mut second).await.expect("second connection");
        dispose(&mut first).await;
        dispose(&mut second).await;
    });
}

/// A server whose certificate no configured root issued is refused as a
/// trust failure, by a connector with no roots of its own and by one whose
/// roots are another CA's, on every connection.
#[test]
fn a_connector_without_the_servers_ca_refuses_it_on_every_connection() {
    runtime().block_on(async {
        let server = server().await;
        let key = key_of(&server);
        let unrelated = https_connection::Config {
            ca_roots: crate::git::endpoint::ca_bundle::certificates(&unrelated_ca()).unwrap(),
            ..Default::default()
        };
        for config in [https_connection::Config::default(), unrelated] {
            let mut connector = connector(config);
            for _ in 0..3 {
                let mut resource = connector.start(&key, &pool::Identity::Https, None).unwrap();
                let failed = settle(&mut resource)
                    .await
                    .expect_err("an untrusted certificate must not verify");
                assert_eq!(failed.code, ErrorCode::Trust);
                dispose(&mut resource).await;
            }
        }
    });
}

/// Two endpoints' configurations are their own: one that trusts the
/// fixture's CA does not lend that trust to another that does not, in either
/// order of building.
#[test]
fn one_connectors_trust_is_never_another_connectors() {
    runtime().block_on(async {
        let server = server().await;
        let key = key_of(&server);
        let mut trusting = connector(server.config());
        let mut refusing = connector(https_connection::Config::default());
        for _ in 0..2 {
            let mut good = trusting.start(&key, &pool::Identity::Https, None).unwrap();
            settle(&mut good)
                .await
                .expect("the trusting connector connects");
            dispose(&mut good).await;
            let mut bad = refusing.start(&key, &pool::Identity::Https, None).unwrap();
            let failed = settle(&mut bad)
                .await
                .expect_err("the other has no such root");
            assert_eq!(failed.code, ErrorCode::Trust);
            dispose(&mut bad).await;
        }
        assert_eq!((trusting.tls_builds(), refusing.tls_builds()), (1, 1));
    });
}

/// A build started ahead of the connections is the one they use.
#[test]
fn a_build_started_ahead_of_the_connections_is_the_one_they_use() {
    runtime().block_on(async {
        let server = server().await;
        let mut connector = connector(server.config());
        connector
            .tls()
            .prebuild_on(&tokio::runtime::Handle::current());
        let until = Instant::now() + Duration::from_secs(20);
        while connector.tls_builds() == 0 {
            assert!(Instant::now() < until, "the prebuild never ran");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let key = key_of(&server);
        let mut resources: Vec<_> = (0..4)
            .map(|_| connector.start(&key, &pool::Identity::Https, None).unwrap())
            .collect();
        for resource in &mut resources {
            settle(resource).await.unwrap();
            dispose(resource).await;
        }
        assert_eq!(connector.tls_builds(), 1);
    });
}
