#![allow(dead_code, unused_imports)]
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod common;
        use common::{SshConnection, ssh_channel, ssh_connection};
        #[path = "../../../src/git/endpoint/agent_auth.rs"]
        mod agent_auth;
        #[path = "../../../src/git/endpoint/agent_client.rs"]
        mod agent_client;
        #[path = "../../../src/git/endpoint/agent_job.rs"]
        mod agent_job;
        #[path = "../../../src/git/endpoint/agent_socket.rs"]
        mod agent_socket;
        #[path = "../../../src/git/endpoint/ssh_admission.rs"]
        mod ssh_admission;
        #[path = "../../../src/git/endpoint/ssh_destination.rs"]
        mod ssh_destination;
        #[path = "../../../src/git/endpoint/ssh_endpoint.rs"]
        mod ssh_endpoint;
        #[path = "../../../src/git/endpoint/ssh_key_auth.rs"]
        mod ssh_key_auth;
        #[path = "../../../src/git/endpoint/ssh_key_container.rs"]
        mod ssh_key_container;
        #[path = "../../../src/git/endpoint/ssh_key_snapshot.rs"]
        mod ssh_key_snapshot;
        #[path = "../../../src/git/endpoint/ssh_local.rs"]
        mod ssh_local;
        #[path = "../../../src/git/endpoint/ssh_network.rs"]
        mod ssh_network;
        #[path = "../../../src/git/endpoint/ssh_pool.rs"]
        mod ssh_pool;
        #[path = "../../../src/git/endpoint/ssh_pump.rs"]
        mod ssh_pump;
        #[path = "../../../src/git/endpoint/ssh_remote.rs"]
        mod ssh_remote;
        #[path = "../../../src/git/endpoint/ssh_setup.rs"]
        mod ssh_setup;
        #[path = "../../../src/git/endpoint/ssh_shutdown.rs"]
        mod ssh_shutdown;
        #[path = "../../../src/git/endpoint/ssh_worker.rs"]
        mod ssh_worker;
        #[path = "../../../src/git/endpoint/stream_io.rs"]
        mod stream_io;
        #[path = "../support/agent_auth.rs"]
        mod support;
        use gwz_transport::{
            pool::Config,
            protocol::{AuthMethod, Opened},
        };
        use ssh_remote::OpenStream;
        use ssh_worker::Endpoint;
        use std::{
            fs,
            io::{self, Read, Write},
            path::{Path, PathBuf},
            sync::{Arc, Mutex},
            time::{Duration, Instant},
        };
        type Observations = Arc<Mutex<Vec<Opened>>>;
        fn endpoint(f: &common::SshdFixture, agent: Option<PathBuf>) -> Endpoint {
            ssh_local::connect(
                Config {
                    total: 2,
                    per_host: 2,
                    per_user_host: 1,
                    cleanup_timeout_ms: 100,
                    ..Config::default()
                },
                f.known_hosts.clone(),
                agent,
                3000,
            )
            .unwrap()
        }
        fn route(
            e: &Endpoint,
            selected: Option<PathBuf>,
            observed: &Observations,
        ) -> Arc<ssh_endpoint::Route> {
            let observed = observed.clone();
            Arc::new(ssh_endpoint::Route::local(
                e.clone(),
                selected,
                Arc::new(move |o| observed.lock().unwrap().push(o.clone())),
            ))
        }
        fn url(f: &common::SshdFixture, path: &Path) -> String {
            let path = path
                .to_str()
                .unwrap()
                .replace('%', "%25")
                .replace(' ', "%20");
            format!("ssh://{}@127.0.0.1:{}{}", f.user, f.port, path)
        }
        fn exchange(route: &dyn OpenStream, url: &str) -> io::Result<()> {
            let mut stream = route.open(url, ssh_channel::GitService::UploadPack)?;
            stream.write_all(b"0000")?;
            stream.end_write()?;
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes)?;
            assert!(!bytes.is_empty());
            stream.close()?;
            Ok(())
        }
        fn finish(e: &Endpoint) {
            e.shutdown();
            let until = Instant::now() + Duration::from_secs(5);
            while !e.shutdown_status().cleanup_complete {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(e.shutdown_status().failure, None);
        }
        fn commit(repo: &git2::Repository, message: &str) -> git2::Oid {
            let blob = repo.blob(message.as_bytes()).unwrap();
            let mut tree = repo.treebuilder(None).unwrap();
            tree.insert("payload", blob, 0o100644).unwrap();
            let tree = repo.find_tree(tree.write().unwrap()).unwrap();
            let signature = git2::Signature::now("fixture", "fixture@example.invalid").unwrap();
            let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            repo.set_head("refs/heads/main").unwrap();
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent.iter().collect::<Vec<_>>(),
            )
            .unwrap()
        }
        #[test]
        fn selected_routes_share_connection_across_git_operations_with_separate_observers() {
            let f = common::SshdFixture::new();
            let e = endpoint(&f, None);
            let key = f.temp.path().join("client_ed25519");
            let server = git2::Repository::open_bare(&f.repository).unwrap();
            let first = commit(&server, "first");
            let second_path = f.temp.path().join("second.git");
            let second_server = git2::Repository::init_bare(&second_path).unwrap();
            second_server.set_head("refs/heads/main").unwrap();
            let a = Observations::default();
            let b = Observations::default();
            let r1 = route(&e, Some(key.clone()), &a);
            let r2 = route(&e.clone(), Some(key), &b);
            let mut fetch = git2::FetchOptions::new();
            fetch.remote_callbacks(ssh_remote::callbacks(r1.clone()));
            let repo = git2::build::RepoBuilder::new()
                .fetch_options(fetch)
                .clone(&url(&f, &f.repository), &f.temp.path().join("clone"))
                .unwrap();
            assert_eq!(repo.head().unwrap().target(), Some(first));
            assert_eq!(a.lock().unwrap().len(), 1, "clone route");
            let mut remote = repo.remote_anonymous(&url(&f, &second_path)).unwrap();
            let mut push = git2::PushOptions::new();
            push.remote_callbacks(ssh_remote::callbacks(r2.clone()));
            remote
                .push(&["refs/heads/main:refs/heads/main"], Some(&mut push))
                .unwrap();
            remote.disconnect().unwrap();
            assert_eq!(second_server.head().unwrap().target(), Some(first));
            assert_eq!(b.lock().unwrap().len(), 1, "push route");
            let second = commit(&server, "second");
            let mut remote = repo.find_remote("origin").unwrap();
            let mut fetch = git2::FetchOptions::new();
            fetch.remote_callbacks(ssh_remote::callbacks(r1.clone()));
            remote
                .fetch(
                    &["refs/heads/main:refs/remotes/origin/main"],
                    Some(&mut fetch),
                    None,
                )
                .unwrap();
            remote.disconnect().unwrap();
            assert_eq!(a.lock().unwrap().len(), 2, "fetch route");
            assert_eq!(
                repo.find_reference("refs/remotes/origin/main")
                    .unwrap()
                    .target(),
                Some(second)
            );
            // Native disconnect retains its transport. A new operation with a
            // different observer needs a fresh Remote, while sharing the Endpoint.
            drop(remote);
            let mut remote = repo.find_remote("origin").unwrap();
            remote
                .connect_auth(
                    git2::Direction::Fetch,
                    Some(ssh_remote::callbacks(r2)),
                    None,
                )
                .unwrap();
            assert!(remote.list().unwrap().iter().any(|r| r.oid() == second));
            remote.disconnect().unwrap();
            let a = a.lock().unwrap();
            let b = b.lock().unwrap();
            assert_eq!(a.len(), 2);
            assert_eq!(b.len(), 2);
            assert!(!a[0].reused);
            assert!(a[0].facts.credential_offered);
            for receipt in a[1..].iter().chain(b.iter()) {
                assert_eq!(receipt.connection_id, a[0].connection_id);
                assert!(receipt.reused);
                assert!(!receipt.facts.credential_offered);
            }
            assert!(
                a.iter()
                    .chain(b.iter())
                    .all(|r| r.facts.method == AuthMethod::SshKey && r.facts.authenticated == Some(true))
            );
            finish(&e);
            assert!(!f.marker.exists());
        }
        #[test]
        fn ambient_reuse_and_selected_refusal_do_not_cross_authority() {
            let f = support::Fixture::new("ssh-ed25519", false);
            let e = endpoint(&f.ssh, Some(f.path.clone()));
            let observed = Observations::default();
            let ambient = route(&e, None, &observed);
            let url = url(&f.ssh, &f.ssh.repository);
            exchange(ambient.as_ref(), &url).unwrap();
            exchange(ambient.as_ref(), &url).unwrap();
            // This key is valid but unauthorized by the fixture; the ambient session
            // already in the capacity-one pool must never satisfy it or retry agent auth.
            let selected = route(
                &e,
                Some(f.ssh.temp.path().join("client_ed25519")),
                &Observations::default(),
            );
            assert!(exchange(selected.as_ref(), &url).is_err());
            assert_eq!(observed.lock().unwrap().len(), 2);
            let receipts = observed.lock().unwrap();
            assert_eq!(receipts[0].facts.method, AuthMethod::SshAgent);
            assert!(!receipts[0].reused);
            assert!(receipts[1].reused);
            assert_eq!(receipts[0].connection_id, receipts[1].connection_id);
            assert!(!receipts[1].facts.credential_offered);
            f.assert_requests("ssh-ed25519", 1);
            finish(&e);
        }
        #[test]
        fn selected_reuse_reloads_path_and_never_contacts_available_agent() {
            let f = support::Fixture::new("ssh-ed25519", false);
            let e = endpoint(&f.ssh, Some(f.path.clone()));
            let selected_path = f.ssh.temp.path().join("auth_key");
            let observed = Observations::default();
            let selected = route(&e, Some(selected_path.clone()), &observed);
            let url = url(&f.ssh, &f.ssh.repository);
            exchange(selected.as_ref(), &url).unwrap();
            exchange(selected.as_ref(), &url).unwrap();
            fs::remove_file(selected_path).unwrap();
            assert!(exchange(selected.as_ref(), &url).is_err());
            assert_eq!(observed.lock().unwrap().len(), 2);
            assert!(observed.lock().unwrap()[1].reused);
            f.no_requests();
            finish(&e);
        }
        #[test]
        fn ambient_cannot_reuse_selected_connection_without_agent() {
            let f = common::SshdFixture::new();
            let e = endpoint(&f, None);
            let url = url(&f, &f.repository);
            let selected = route(
                &e,
                Some(f.temp.path().join("client_ed25519")),
                &Observations::default(),
            );
            exchange(selected.as_ref(), &url).unwrap();
            let observed = Observations::default();
            let ambient = route(&e, None, &observed);
            assert!(exchange(ambient.as_ref(), &url).is_err());
            assert!(observed.lock().unwrap().is_empty());
            finish(&e);
        }
        #[test]
        fn untrusted_host_refuses_before_agent_io() {
            let f = support::Fixture::new("ssh-ed25519", false);
            let e = endpoint(&f.ssh, Some(f.path.clone()));
            fs::write(&f.ssh.known_hosts, "").unwrap();
            let observed = Observations::default();
            let route = route(&e, None, &observed);
            assert!(exchange(route.as_ref(), &url(&f.ssh, &f.ssh.repository)).is_err());
            assert!(observed.lock().unwrap().is_empty());
            f.no_requests();
            finish(&e);
        }
    }
}
