//! Characterizes the setup API gap; does not implement an agent or use user keys.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use std::{
                io::Read,
                os::unix::net::UnixListener,
                process::{Child, Command, Stdio},
                thread,
                time::{Duration, Instant},
            };
            struct Reap(Child);
            impl Drop for Reap {
                fn drop(&mut self) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
            #[test]
            #[ignore = "child only; parent supplies an isolated fake agent"]
            fn agent_child() {
                let socket = std::env::var_os("GWZ_TEST_AGENT_WAIT_SOCKET").expect("child socket");
                let session = ssh2::Session::new().unwrap();
                session.set_blocking(false);
                session.set_timeout(25);
                let mut agent = session.agent().unwrap();
                agent.set_identity_path(std::path::Path::new(&socket)).unwrap();
                agent.connect().unwrap();
                let _ = agent.list_identities();
            }
            #[test]
            fn session_timeout_does_not_bound_native_agent_reply_wait() {
                let temp = tempfile::TempDir::new().unwrap();
                let path = temp.path().join("agent.sock");
                let listener = UnixListener::bind(&path).unwrap();
                listener.set_nonblocking(true).unwrap();
                let mut child = Reap(Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "unix::agent_child", "--ignored", "--nocapture"])
                    .env("GWZ_TEST_AGENT_WAIT_SOCKET", &path)
                    .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut peer = loop {
                    match listener.accept() {
                        Ok((peer, _)) => break peer,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "child did not connect");
                            assert!(child.0.try_wait().unwrap().is_none(), "child exited before connect");
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("accept: {error}"),
                    }
                };
                peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request = [0; 5];
                peer.read_exact(&mut request).unwrap();
                assert_eq!(request, [0, 0, 0, 1, 11], "agent identity request");
                // Deliberately withhold a reply. No real agent, keys or host involved.
                thread::sleep(Duration::from_millis(200));
                assert!(child.0.try_wait().unwrap().is_none(), "API behavior changed; revisit setup design");
                println!("Native agent list blocked beyond 25 ms session timeout and nonblocking mode; parent terminates/reaps child after 200 ms observation.");
                child.0.kill().unwrap();
                child.0.wait().unwrap();
            }
        }
    }
}
