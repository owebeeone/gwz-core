//! TR2.2: a configured gh helper is discovered in both Git helper forms.

use super::*;
use hyper::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

fn git_path() -> std::path::PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|entry| entry.is_absolute())
        .map(|entry| entry.join("git"))
        .find(|path| path.is_file())
        .expect("git must be installed for the credential-helper fixture")
}

fn configured_gh(
    directory: &Path,
    host: &str,
    helper_on_path: bool,
) -> (https_auth::Config, std::path::PathBuf) {
    let gh = directory.join("gh");
    let seen = directory.join("gh-seen");
    crate::git::endpoint::helper_script::write_helper_script(
        &gh,
        "[ \"$1 $2 $3\" = 'auth git-credential get' ] || exit 4\n\
/bin/cat >\"$GH_SEEN\"\n\
printf 'username=fixture\\npassword=token\\n\\n'\n",
    );
    let git = git_path();
    let git_dir = git.parent().unwrap();
    let config_file = directory.join("gitconfig");
    let helper = if helper_on_path {
        "!gh auth git-credential".to_owned()
    } else {
        format!("!{} auth git-credential", gh.display())
    };
    fs::write(
        &config_file,
        format!("[credential \"https://{host}\"]\n\thelper = {helper}\n"),
    )
    .unwrap();
    let path = if helper_on_path {
        std::env::join_paths([directory, git_dir]).unwrap()
    } else {
        std::env::join_paths([git_dir]).unwrap()
    };
    (
        https_auth::Config {
            executable: git,
            environment: vec![
                ("HOME".into(), directory.as_os_str().into()),
                ("PATH".into(), path),
                ("GIT_CONFIG_GLOBAL".into(), config_file.as_os_str().into()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                ("GH_SEEN".into(), seen.as_os_str().into()),
            ],
        },
        seen,
    )
}

fn exercise_configured_gh(helper_on_path: bool) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let requests = Arc::new(AtomicUsize::new(0));
            let observed = requests.clone();
            let server = Server::start(Arc::new(move |request| {
                let observed = observed.clone();
                Box::pin(async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    if request.headers().get(AUTHORIZATION).is_some() {
                        response(200, GitService::UploadPackAdvertisement, "ok")
                    } else {
                        let mut challenge =
                            response(401, GitService::UploadPackAdvertisement, "challenge");
                        challenge
                            .headers_mut()
                            .insert(WWW_AUTHENTICATE, "Basic realm=\"fixture\"".parse().unwrap());
                        challenge
                    }
                })
            }))
            .await;
            let directory = tempfile::tempdir().unwrap();
            let url = url::Url::parse(&server.url).unwrap();
            let host = format!("localhost:{}", url.port().unwrap());
            let (auth, seen) = configured_gh(directory.path(), &host, helper_on_path);
            let mut endpoint = Endpoint::new(
                server.config(),
                Some(auth),
                gwz_transport::pool::Config::default(),
            )
            .unwrap();
            let mut first = None;
            let prepared = endpoint
                .client
                .prepare_auto(
                    input(&server, GitService::UploadPackAdvertisement),
                    &CancellationToken::new(),
                    &mut first,
                )
                .await
                .expect("configured gh must answer the HTTPS discovery challenge");
            assert_eq!(first.unwrap().facts.unwrap().http_status, Some(401));
            finish(prepared).await;
            assert_eq!(requests.load(Ordering::SeqCst), 2);
            let input = fs::read_to_string(seen).unwrap();
            assert!(input.contains(&format!("host={host}")));
            assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
        });
}

#[test]
fn gh_configured_by_absolute_path_answers_https_challenge() {
    exercise_configured_gh(false);
}

#[test]
fn gh_configured_through_path_answers_https_challenge() {
    exercise_configured_gh(true);
}
