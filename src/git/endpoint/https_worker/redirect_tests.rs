//! What the endpoint remembers of a discovery redirect, for the words around it (parity design section 14 item 13,
//! U9 and text T5, step 4.9): a followed redirect, so `--verbose` can print it, and a redirect from https to http,
//! which is refused and which the host then explains with the address the server sent.
use super::*;
use crate::git::endpoint::https_fixture::{Server, input, response};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// A server that answers every discovery with a 302 to `location` and counts what it is asked.
async fn redirecting(location: &'static str) -> (Server, Arc<AtomicUsize>) {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let server = Server::start(Arc::new(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            let mut moved = response(302, GitService::UploadPackAdvertisement, "");
            moved
                .headers_mut()
                .insert("Location", location.parse().unwrap());
            moved
        })
    }))
    .await;
    (server, requests)
}

async fn discover(endpoint: &Endpoint, server: &Server) -> Result<Prepared, Failure> {
    endpoint
        .client
        .prepare_attempt(
            input(server, GitService::UploadPackAdvertisement),
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await
        .0
}

#[test]
fn a_followed_discovery_redirect_is_remembered_with_where_it_went() {
    runtime().block_on(async {
        let server = Server::start(Arc::new(|request| {
            Box::pin(async move {
                if request.uri().path() == "/repo/info/refs" {
                    let mut moved = response(302, GitService::UploadPackAdvertisement, "");
                    moved.headers_mut().insert(
                        "Location",
                        "/moved/info/refs?service=git-upload-pack".parse().unwrap(),
                    );
                    moved
                } else {
                    response(200, GitService::UploadPackAdvertisement, "ok")
                }
            })
        }))
        .await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let prepared = discover(&endpoint, &server)
            .await
            .expect("the redirect is followed");
        drop(prepared);
        let followed = endpoint.client.redirects().drain_followed();
        assert_eq!(followed.len(), 1);
        assert_eq!(followed[0].from, server.url);
        assert_eq!(followed[0].to, server.url.replace("/repo", "/moved"));
        // Said once: the next drain has nothing new.
        assert!(endpoint.client.redirects().drain_followed().is_empty());
        // Nothing was refused.
        assert!(
            endpoint
                .client
                .redirects()
                .take_refused("operation")
                .is_none()
        );
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn a_redirect_from_https_to_http_is_refused_and_remembered_with_the_address_the_server_sent() {
    runtime().block_on(async {
        let (server, requests) =
            redirecting("http://elsewhere.example:8080/moved/info/refs?service=git-upload-pack")
                .await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        let failure = discover(&endpoint, &server)
            .await
            .err()
            .expect("a downgrade is refused");
        assert_eq!(failure.code, ErrorCode::InvalidRequest);
        assert_eq!(
            requests.load(Ordering::Acquire),
            1,
            "the redirect was not followed"
        );
        assert!(endpoint.client.redirects().drain_followed().is_empty());
        let refused = endpoint
            .client
            .redirects()
            .take_refused("operation")
            .expect("the refusal is remembered for the operation");
        assert_eq!(refused.from, server.url);
        assert_eq!(
            refused.to,
            "http://elsewhere.example:8080/moved/info/refs?service=git-upload-pack"
        );
        // The refusal is told once.
        assert!(
            endpoint
                .client
                .redirects()
                .take_refused("operation")
                .is_none()
        );
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn a_downgrade_names_no_credential_the_server_put_in_the_address() {
    runtime().block_on(async {
        let (server, _) = redirecting("http://user:secret@elsewhere.example/moved/info/refs").await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        discover(&endpoint, &server).await.err().expect("refused");
        let refused = endpoint
            .client
            .redirects()
            .take_refused("operation")
            .unwrap();
        assert!(
            !refused.to.contains("secret") && !refused.to.contains("user"),
            "{}",
            refused.to
        );
        assert!(
            refused
                .to
                .starts_with("http://elsewhere.example/moved/info/refs")
        );
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}

#[test]
fn a_location_that_is_not_a_downgrade_is_refused_as_before_and_remembers_nothing() {
    runtime().block_on(async {
        // Other schemes, a query that is not the service's, and a relative Location that stays on https.
        for location in [
            "ftp://elsewhere.example/moved/info/refs",
            "/moved/info/refs?service=git-receive-pack",
            "https://elsewhere.example/moved/info/refs#fragment",
        ] {
            let (server, _) = redirecting(location).await;
            let mut endpoint =
                Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
            let failure = discover(&endpoint, &server).await.err().expect(location);
            assert_eq!(failure.code, ErrorCode::InvalidRequest, "{location}");
            assert!(
                endpoint
                    .client
                    .redirects()
                    .take_refused("operation")
                    .is_none(),
                "{location}"
            );
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        }
    });
}

#[test]
fn a_long_address_is_cut_before_it_is_remembered() {
    runtime().block_on(async {
        let path = "a".repeat(1_000);
        let location: &'static str =
            Box::leak(format!("http://elsewhere.example/{path}/info/refs").into_boxed_str());
        let (server, _) = redirecting(location).await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        discover(&endpoint, &server).await.err().expect("refused");
        let refused = endpoint
            .client
            .redirects()
            .take_refused("operation")
            .unwrap();
        assert!(refused.to.len() <= 259, "{}", refused.to.len());
        assert!(refused.to.ends_with("..."));
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    });
}
