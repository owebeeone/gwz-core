use super::*;

#[path = "https_worker_tests/discovery.rs"]
mod discovery;
#[path = "https_worker_tests/exchange.rs"]
mod exchange;
#[path = "https_worker_tests/failure.rs"]
mod failure;
#[path = "https_worker_tests/pool.rs"]
mod pool;
#[path = "https_worker_tests/proxy.rs"]
mod proxy;

use crate::git::endpoint::https_fixture as fixture;
use crate::git::endpoint::{https_connection, https_local, https_remote};
use fixture::*;
use tokio::time::timeout;
async fn finish(prepared: Prepared) -> Vec<u8> {
    let (stream, task) = attach(prepared);
    stream.end_write().await.unwrap();
    let mut output = Vec::new();
    let mut buffer = [0; 113];
    loop {
        let n = stream.read(&mut buffer).await.unwrap();
        if n == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..n]);
    }
    stream.close().await.unwrap();
    task.await.unwrap();
    output
}
cfg_if::cfg_if! { if #[cfg(unix)] {
    fn fake_gh(token:&std::path::Path)->(tempfile::TempDir,https_auth::Config) {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("gh");
        crate::git::endpoint::helper_script::write_git_fixture(&path,"[ \"${1} ${2} ${3} ${4}\" = '-c core.askPass= credential fill' ] || exit 4\n/bin/cat >/dev/null\nprintf 'username=fixture\\npassword='\n/bin/cat \"$TOKEN_FILE\"\nprintf '\\n\\n'\n");
        (dir,https_auth::Config{executable:path,environment:vec![("TOKEN_FILE".into(),token.as_os_str().into())]})
    }
    #[test]
    fn anonymous_challenge_retries_once_and_route_reuses_its_held_credential() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let temp=tempfile::tempdir().unwrap();let token=temp.path().join("token");std::fs::write(&token,"first-fixture-token").unwrap();let (_helper,auth)=fake_gh(&token);
            let seen=Arc::new(Mutex::new(Vec::new()));let observed=seen.clone();
            let server=Server::start(Arc::new(move |request|{let observed=observed.clone();Box::pin(async move {
                let auth=request.headers().get(AUTHORIZATION).map(|value|value.as_bytes().to_vec());let status=if auth.is_some(){200}else{401};observed.lock().unwrap().push(auth);response(status,GitService::UploadPackAdvertisement,"ok")
            })})).await;
            let mut endpoint=Endpoint::new(server.config(),Some(auth),gwz_transport::pool::Config::default()).unwrap();
            let mut receipt=None;let first=endpoint.client.prepare_auto(input(&server,GitService::UploadPackAdvertisement),&CancellationToken::new(),&mut receipt).await.unwrap();
            assert_eq!(receipt.unwrap().facts.unwrap().http_status,Some(401));assert!(first.opened.facts.credential_offered);assert_eq!(first.opened.facts.authenticated,None);let id=first.opened.connection_id.clone();finish(first).await;
            std::fs::write(&token,"second-fixture-token").unwrap();let mut request=input(&server,GitService::UploadPackAdvertisement);request.policy=AuthPolicy::Gh;
            let second=endpoint.client.prepare_budget_for_transition(request,&CancellationToken::new(),&mut endpoint.client.budget(),&mut None).await.unwrap();assert!(second.opened.reused);assert_eq!(second.opened.connection_id,id);assert!(second.opened.facts.credential_offered);finish(second).await;
            let seen=seen.lock().unwrap();assert_eq!(seen.len(),3);assert!(seen[0].is_none());assert!(seen[1].is_some() && seen[2].is_some() && seen[1]==seen[2]);drop(seen);
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await,0);
        });
    }
} }
#[path = "https_auth_integration_tests.rs"]
mod auth_integration;

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[path = "https_worker_tests/configured_helpers.rs"]
        mod configured_helpers;
    }
}

#[path = "https_lifecycle_tests.rs"]
mod lifecycle;
