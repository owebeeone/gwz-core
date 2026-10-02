use super::*;
use crate::git::endpoint::https_fixture::{self as fixture, Server, input, response};
use hyper::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use std::fs;

fn fake_git(path: &std::path::Path, counter: &std::path::Path) -> https_auth::Config {
    crate::git::endpoint::helper_script::write_git_fixture(path,
        "printf 'lookup\\n' >> \"$COUNTER\"\nprintf 'username=fixture\\npassword=private-fixture\\n\\n'");
    https_auth::Config { executable: path.into(), environment: vec![("COUNTER".into(), counter.as_os_str().into())] }
}
async fn finish(prepared: Prepared) {
    let (stream, task) = fixture::attach(prepared);
    stream.end_write().await.unwrap();
    let mut bytes = [0; 64];
    while stream.read(&mut bytes).await.unwrap() != 0 {}
    stream.close().await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn route_owns_one_lookup_and_other_routes_operations_never_get_its_connection() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let records = seen.clone();
    let server = Server::start(Arc::new(move |request| {
        let records = records.clone();
        Box::pin(async move {
            let offered = request.headers().contains_key(AUTHORIZATION);
            let connection = request.extensions().get::<fixture::ConnectionId>().unwrap().0;
            records.lock().unwrap().push((offered, connection));
            response(if offered {200} else {401}, GitService::UploadPackAdvertisement, "fixture")
        })
    })).await;
    let dir = tempfile::tempdir().unwrap();
    let counter = dir.path().join("count");
    let config = fake_git(&dir.path().join("git"), &counter);
    let mut endpoint = Endpoint::new(server.config(), Some(config), pool::Config::default()).unwrap();
    let first = input(&server, GitService::UploadPackAdvertisement);
    finish(endpoint.client.prepare_auto(first.clone(), &CancellationToken::new(), &mut None).await.unwrap()).await;
    let mut held = first.clone(); held.policy = AuthPolicy::Gh;
    finish(endpoint.client.prepare_budget_for_transition(held, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await.unwrap()).await;
    let mut route = first.clone(); route.destination = route.destination.replace("/repo", "/other");
    finish(endpoint.client.prepare_auto(route, &CancellationToken::new(), &mut None).await.unwrap()).await;
    let mut operation = first; operation.operation = "another-operation".into();
    finish(endpoint.client.prepare_auto(operation, &CancellationToken::new(), &mut None).await.unwrap()).await;
    assert_eq!(fs::read_to_string(counter).unwrap().lines().count(), 3);
    let records = seen.lock().unwrap().clone();
    assert_eq!(records.iter().map(|r| r.0).collect::<Vec<_>>(), [false,true,true,false,true,false,true]);
    assert_eq!(records[0].1, records[1].1);
    assert_eq!(records[1].1, records[2].1);
    assert_ne!(records[2].1, records[3].1);
    assert_ne!(records[4].1, records[5].1);
    endpoint.client.finish_operation("operation");
    endpoint.client.finish_operation("another-operation");
    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
}

#[tokio::test]
async fn missing_git_latches_only_this_operation_without_starting_a_repaired_helper() {
    let server = Server::start(Arc::new(|_| Box::pin(async { response(200, GitService::UploadPackAdvertisement, "fixture") }))).await;
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("git"); let counter = dir.path().join("count");
    let config = https_auth::Config { executable: executable.clone(), environment: vec![("COUNTER".into(), counter.as_os_str().into())] };
    let mut endpoint = Endpoint::new(server.config(), Some(config), pool::Config::default()).unwrap();
    let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::Gh;
    let failed = endpoint.client.prepare_budget_for_transition(request.clone(), &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await.err().unwrap();
    assert_eq!(failed.code, ErrorCode::Unavailable);
    fake_git(&executable, &counter);
    request.destination = request.destination.replace("/repo", "/other");
    assert_eq!(endpoint.client.prepare_budget_for_transition(request.clone(), &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await.err().unwrap().code, ErrorCode::Unavailable);
    assert!(!counter.exists());
    request.operation = "fresh-operation".into();
    finish(endpoint.client.prepare_budget_for_transition(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await.unwrap()).await;
    assert_eq!(fs::read_to_string(counter).unwrap().lines().count(), 1);
    assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
}

#[tokio::test]
async fn unsupported_or_absent_challenge_scheme_never_runs_helper_or_exposes_realm() {
    for named in [false, true] {
        let server = Server::start(Arc::new(move |_| Box::pin(async move {
            let mut response = response(401, GitService::UploadPackAdvertisement, "private-body");
            response.headers_mut().remove(WWW_AUTHENTICATE);
            if named { response.headers_mut().insert(WWW_AUTHENTICATE, "Bearer realm=\"private-realm\"".parse().unwrap()); }
            response
        }))).await;
        let dir = tempfile::tempdir().unwrap(); let counter = dir.path().join("count");
        let config = fake_git(&dir.path().join("git"), &counter);
        let mut endpoint = Endpoint::new(server.config(), Some(config), pool::Config::default()).unwrap();
        let failed = endpoint.client.prepare_auto(input(&server, GitService::UploadPackAdvertisement), &CancellationToken::new(), &mut None).await.err().unwrap();
        assert_eq!(failed.code, ErrorCode::Authentication);
        let schemes = failed.detail.unwrap().schemes.unwrap();
        assert_eq!(schemes, if named { vec!["Bearer".to_owned()] } else { vec![] });
        assert!(!counter.exists());
        assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
    }
}
