//! The WH3 matrix's Git rows at the host (`GwzWindowsHttpsIntegrationDesign-DRAFT.md` section 6, step 4.9): real
//! advertisement and pack traffic over a connection the logon session authenticated, through the host's runtime, with
//! the result checked in the bare remote and not in the client's report ("native Complete alone is not Git success").
//! The SSPI side is the fake port, so the rows run on every platform; the real SSPI rows are the installed CLI and
//! wheel probes of the Windows run (step 4.10) and `native/tests/real_sspi.rs`.
use super::https_tests::{
    commit_worktree, endpoint_home, fixture, git_http_backend, meta, repository,
};
use super::*;
use crate::git::GitBackend;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// A smart-HTTP server that authenticates a connection the way NTLM and Negotiate do: a request with a credential
/// authenticates its connection, and the connection's later requests carry none. Every other request is challenged.
struct Gate {
    server: fixture::Server,
    /// Connections that sent a credential.
    authenticated: Arc<Mutex<BTreeSet<usize>>>,
    /// Requests that were challenged.
    challenges: Arc<AtomicUsize>,
    /// POSTs to `git-receive-pack`.
    pushes: Arc<AtomicUsize>,
}

async fn gate(repository: std::path::PathBuf, fail_pushes: bool) -> Gate {
    let repository = Arc::new(repository);
    let authenticated = Arc::new(Mutex::new(BTreeSet::new()));
    let challenges = Arc::new(AtomicUsize::new(0));
    let pushes = Arc::new(AtomicUsize::new(0));
    let (known, challenged, posted) = (authenticated.clone(), challenges.clone(), pushes.clone());
    let server = fixture::Server::start(Arc::new(move |request| {
        let repository = repository.clone();
        let connection = request
            .extensions()
            .get::<fixture::ConnectionId>()
            .map_or(0, |id| id.0);
        let credentialled = request.headers().contains_key("authorization");
        let admitted = {
            let mut known = known.lock().unwrap();
            if credentialled {
                known.insert(connection);
            }
            known.contains(&connection)
        };
        if !admitted {
            challenged.fetch_add(1, Ordering::SeqCst);
        }
        let push = request.method() == "POST" && request.uri().path().ends_with("git-receive-pack");
        if push {
            posted.fetch_add(1, Ordering::SeqCst);
        }
        Box::pin(async move {
            if !admitted {
                let mut reply = fixture::response(
                    401,
                    gwz_transport::protocol::GitService::UploadPackAdvertisement,
                    "",
                );
                reply
                    .headers_mut()
                    .insert("WWW-Authenticate", "NTLM".parse().unwrap());
                return reply;
            }
            if push && fail_pushes {
                return fixture::response(
                    500,
                    gwz_transport::protocol::GitService::ReceivePackExchange,
                    "",
                );
            }
            git_http_backend(repository, request).await
        })
    }))
    .await;
    Gate {
        server,
        authenticated,
        challenges,
        pushes,
    }
}

fn transport(root: &std::path::Path, gate: &Gate) -> TransportRuntime {
    TransportRuntime::with_https_native(
        SshEndpointConfig::fixture(endpoint_home(root), None),
        HttpsEndpointConfig {
            tls: gate.server.config(),
            auth: None,
        },
        HelperSlots::new(),
        crate::git::endpoint::https_worker::native::publication_fixture(),
    )
    .unwrap()
}

/// A request's backend under the Windows logon-session policy, which is what the product's CLI and Python entries
/// give a Windows build (`Disabled` helpers map to `WindowsDefault`).
fn logon_session(request: &TransportRequest) -> crate::git::Git2Backend {
    let mut backend = request.backend().clone();
    backend.credential_helpers = crate::git::CredentialHelperPolicy::Disabled;
    backend.ssh = backend.ssh.with_windows_policy_for_test();
    backend
}

#[test]
fn clone_fetch_and_push_over_authenticated_connections_are_verified_in_the_bare_remote() {
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let (bare, first) = repository(root.path());
        let gate = gate(bare.clone(), false).await;
        let transport = transport(root.path(), &gate);
        let url = gate.server.url.clone();
        let target = root.path().join("clone");

        // Clone: the tree is the expected one.
        let request = transport
            .request(meta("native-clone"), "clone".into())
            .await
            .unwrap();
        let backend = logon_session(&request);
        let (clone_url, clone_target) = (url.clone(), target.clone());
        let cloned =
            tokio::task::spawn_blocking(move || backend.clone_repo(&clone_url, &clone_target))
                .await
                .unwrap()
                .expect("the clone succeeds");
        assert_eq!(cloned.head.commit, Some(first.to_string()));
        assert_eq!(
            std::fs::read_to_string(target.join("payload")).unwrap(),
            "https host fixture\n"
        );
        assert_eq!(request.finish().await.pending_local_work, 0);

        // Fetch: the server moves on, the client sees the new commit under its tracking ref.
        let server_side = git2::Repository::open(&bare).unwrap();
        let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
        let parent = server_side.find_commit(first).unwrap();
        let tree = parent.tree().unwrap();
        let second = server_side
            .commit(
                Some("refs/heads/main"),
                &signature,
                &signature,
                "server update",
                &tree,
                &[&parent],
            )
            .unwrap();
        let request = transport
            .request(meta("native-fetch"), "fetch".into())
            .await
            .unwrap();
        let backend = logon_session(&request);
        let fetch_target = target.clone();
        tokio::task::spawn_blocking(move || backend.fetch(&fetch_target, "origin"))
            .await
            .unwrap()
            .expect("the fetch succeeds");
        let local = git2::Repository::open(&target).unwrap();
        assert_eq!(
            local
                .find_reference("refs/remotes/origin/main")
                .unwrap()
                .target(),
            Some(second)
        );
        assert_eq!(request.finish().await.pending_local_work, 0);

        // Push: the client's commit is the bare remote's ref afterwards.
        local
            .reference("refs/heads/main", second, true, "to origin/main")
            .unwrap();
        local.set_head("refs/heads/main").unwrap();
        local
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        drop(local);
        let pushed = commit_worktree(&target, "pushed through the logon session\n");
        let request = transport
            .request(meta("native-push"), "push".into())
            .await
            .unwrap();
        let backend = logon_session(&request);
        let push_target = target.clone();
        tokio::task::spawn_blocking(move || {
            backend.push(&push_target, "origin", "refs/heads/main:refs/heads/main")
        })
        .await
        .unwrap()
        .expect("the push succeeds");
        assert_eq!(
            git2::Repository::open(&bare)
                .unwrap()
                .find_reference("refs/heads/main")
                .unwrap()
                .target(),
            Some(pushed),
            "the bare remote did not take the push"
        );
        assert_eq!(request.finish().await.pending_local_work, 0);

        // Every connection that carried a request was authenticated once, by a challenge, and none was shared.
        let connections = gate.authenticated.lock().unwrap().len();
        assert!(connections >= 1);
        assert_eq!(
            gate.challenges.load(Ordering::SeqCst),
            connections,
            "a connection was challenged more or less than once"
        );
        assert_eq!(transport.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn a_push_the_server_fails_is_sent_once_and_leaves_the_remote_unchanged() {
    runtime().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let (bare, first) = repository(root.path());
        let gate = gate(bare.clone(), true).await;
        let transport = transport(root.path(), &gate);
        let target = root.path().join("clone");
        let request = transport
            .request(meta("native-clone"), "clone".into())
            .await
            .unwrap();
        let backend = logon_session(&request);
        let (url, clone_target) = (gate.server.url.clone(), target.clone());
        tokio::task::spawn_blocking(move || backend.clone_repo(&url, &clone_target))
            .await
            .unwrap()
            .expect("the clone succeeds");
        assert_eq!(request.finish().await.pending_local_work, 0);

        commit_worktree(&target, "never arrives\n");
        let request = transport
            .request(meta("native-push"), "push".into())
            .await
            .unwrap();
        let backend = logon_session(&request);
        let push_target = target.clone();
        let failed = tokio::task::spawn_blocking(move || {
            backend.push(&push_target, "origin", "refs/heads/main:refs/heads/main")
        })
        .await
        .unwrap();
        assert!(failed.is_err(), "the server failed the push");
        assert_eq!(
            gate.pushes.load(Ordering::SeqCst),
            1,
            "a failed receive-pack POST was replayed"
        );
        assert_eq!(
            git2::Repository::open(&bare)
                .unwrap()
                .find_reference("refs/heads/main")
                .unwrap()
                .target(),
            Some(first),
            "the remote changed"
        );
        assert_eq!(request.finish().await.pending_local_work, 0);
        assert_eq!(transport.shutdown().await.pending_local_work, 0);
    });
}
