//! What a server's `MaxStartups` drop looks like to the transport's setup, per
//! platform. A stock OpenSSH server drops unauthenticated connections beyond
//! its `MaxStartups` start, 10 by default; amendment 2's §3.20 (OD18) makes
//! such a drop a retriable setup failure.
//!
//! The server writes one line and closes with the client's banner unread:
//! OpenSSH 10.3 (macOS 26) writes "Not allowed at this time", OpenSSH 9.6p1
//! (Ubuntu 24.04, the Linux CI leg) "Exceeded MaxStartups". The client then
//! reads a reset, or the end of the stream when the close wins the race with
//! its banner. libssh2's handshake reports either as its own error, which
//! `ssh2` turns into `ErrorKind::Other`, so `ssh_setup` maps the drop to `Io`,
//! which is retried. Neither platform reports `ConnectionAborted`, which
//! `ssh_setup` maps to `Cancelled` for the transport's own cancellation.
//!
//! OpenSSH 10.3 counts a connection that has not authenticated against
//! `MaxStartups`, so on macOS a silent connection holds the one start of a
//! `MaxStartups 1` server and the real server's drop is deterministic.
//! OpenSSH 9.6p1 frees a start once the connection's child process runs, so
//! the real server drops only connections that arrive inside that window; on
//! Linux a scripted server that answers as 9.6p1 does stands in for it.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use crate::git::endpoint::{
            agent_job::Job,
            setup_retry::{Phase, Verdict, classify},
            ssh_network, ssh_setup,
        };
        use gwz_transport::{pool::Key, protocol::ErrorCode};
        use std::{
            io::{self, Read, Write},
            net::{Shutdown, TcpListener},
            path::PathBuf,
            task::{Context, Poll, Waker},
            thread,
            time::{Duration, Instant},
        };

        /// Runs the production network setup for `key` in a supervised job,
        /// as the SSH connector does, and returns how it ended.
        fn establish(key: Key, known_hosts: PathBuf) -> io::Result<()> {
            let mut job = Job::start_isolated(
                Some(Instant::now() + Duration::from_secs(10)),
                Duration::from_secs(1),
                move |control| ssh_network::establish(&key, &known_hosts, &control).map(|_| ()),
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                if let Poll::Ready(result) =
                    job.poll_result(&mut Context::from_waker(Waker::noop()))
                {
                    return result;
                }
                assert!(Instant::now() < deadline, "setup did not end");
                thread::sleep(Duration::from_millis(1));
            }
        }

        /// The drop's kind, failure and verdict, asserted as one pin.
        fn assert_dropped(error: &io::Error, context: &str) {
            assert_eq!(error.kind(), io::ErrorKind::Other, "{context}: {error}");
            let failure = ssh_setup::failure_from_io(error);
            assert_eq!(failure.code, ErrorCode::Io, "{context}");
            assert_eq!(classify(&failure, Phase::Setup), Verdict::Retry, "{context}");
        }

        /// One connection answered as OpenSSH answers one past its
        /// `MaxStartups`: `line`, then a close. With `reset` the close leaves
        /// the client's banner unread, so the client reads a reset; otherwise
        /// the server half-closes first and drains, so it reads the end of the
        /// stream.
        fn dropping_server(line: &'static str, reset: bool) -> (u16, thread::JoinHandle<()>) {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                stream.write_all(format!("{line}\r\n").as_bytes()).unwrap();
                if reset {
                    let mut banner = [0; 1];
                    let _ = stream.peek(&mut banner);
                } else {
                    stream.shutdown(Shutdown::Write).unwrap();
                    let _ = stream.read_to_end(&mut Vec::new());
                }
            });
            (port, server)
        }

        #[test]
        fn a_max_startups_drop_fails_setup_as_a_retriable_io_whichever_way_it_closes() {
            let temp = tempfile::tempdir().unwrap();
            let known_hosts = temp.path().join("known_hosts");
            std::fs::write(&known_hosts, "").unwrap();
            for line in ["Not allowed at this time", "Exceeded MaxStartups"] {
                for reset in [true, false] {
                    let (port, server) = dropping_server(line, reset);
                    let error = establish(Key::ssh("git", "127.0.0.1", port), known_hosts.clone())
                        .expect_err("a dropped connection cannot set up");
                    server.join().unwrap();
                    assert_dropped(&error, &format!("{line:?}, reset {reset}"));
                }
            }
        }

        cfg_if::cfg_if! {
            if #[cfg(target_os = "macos")] {
                use crate::git::endpoint::ssh_fixture::SshdFixture;
                use std::{io::BufRead, net::TcpStream};

                #[test]
                fn the_fixture_server_drops_setups_beyond_its_max_startups_as_other() {
                    let fixture =
                        SshdFixture::with_startups("MaxStartups 1\nPerSourcePenalties no\n");
                    let key = || Key::ssh(&fixture.user, "127.0.0.1", fixture.port);
                    // A connection that never sends its banner holds the one
                    // start; the server's banner shows its child is running.
                    // Until the fixture's readiness probe has left, the start
                    // is the probe's and the holder is dropped too.
                    let deadline = Instant::now() + Duration::from_secs(10);
                    let holder = loop {
                        let holder = TcpStream::connect(("127.0.0.1", fixture.port)).unwrap();
                        holder.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                        let mut banner = String::new();
                        io::BufReader::new(&holder).read_line(&mut banner).unwrap();
                        if banner.starts_with("SSH-2.0-") {
                            break holder;
                        }
                        assert!(Instant::now() < deadline, "the start stayed taken: {banner:?}");
                        thread::sleep(Duration::from_millis(50));
                    };
                    for attempt in 1..=3 {
                        let error = establish(key(), fixture.known_hosts.clone())
                            .expect_err("a setup past MaxStartups is dropped");
                        assert_dropped(&error, &format!("attempt {attempt}"));
                    }
                    // With the start free again the same setup succeeds, so the
                    // drops were the server's MaxStartups.
                    drop(holder);
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while let Err(error) = establish(key(), fixture.known_hosts.clone()) {
                        assert!(Instant::now() < deadline, "setup still dropped: {error}");
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        }
    }
}
