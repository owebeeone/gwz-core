//! TR2.18: host-name case in `known_hosts`. libssh2 compares a plain name
//! byte for byte and hashes a hashed (`|1|`) name's host as given, and 1.0.17
//! gives it the host as the URL wrote it. The transport looks a host up
//! lowercased, as it pools it, so it matches plain names ignoring case, and a
//! hashed name on the host as the URL wrote it or lowercased. Each case runs
//! against the loopback `sshd`, reached under a name that resolves to it here.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use crate::git::endpoint::{agent_job::Job, ssh_fixture as common, ssh_network};
        use gwz_transport::pool::Key;
        use std::{
            io,
            net::SocketAddr,
            task::{Context, Poll, Waker},
            time::{Duration, Instant},
        };

        /// The pool key's host, which the transport has lowercased.
        const HOST: &str = "githost.example";

        /// Establishes trust in the fixture's server for an open whose URL wrote
        /// the host as `written`, with `known` as its `known_hosts`.
        fn establish(f: &common::SshdFixture, written: &str, known: String) -> io::Result<()> {
            let key = Key::ssh(&f.user, HOST, f.port);
            let address = SocketAddr::from(([127, 0, 0, 1], f.port));
            let written = written.to_owned();
            let path = f.temp.path().join("unused_known_hosts");
            let mut job = Job::start(
                Some(Instant::now() + Duration::from_secs(5)),
                Duration::from_secs(1),
                move |control| {
                    ssh_network::establish_inner(
                        &key,
                        &written,
                        &path,
                        &control,
                        move |_, _| Ok(known),
                        move |_, _, _| Ok(vec![address]),
                    )
                    .map(|_| ())
                },
            )
            .unwrap();
            let until = Instant::now() + Duration::from_secs(10);
            loop {
                if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
                    return result;
                }
                assert!(Instant::now() < until, "trust setup never finished");
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        /// The fixture's host key line, under `name` instead of its address.
        fn plain(f: &common::SshdFixture, name: &str) -> String {
            f.known_host(name, f.port, false)
        }

        /// The same line with `name` hashed.
        fn hashed(f: &common::SshdFixture, name: &str) -> String {
            f.known_host(name, f.port, true)
        }

        fn refused(result: io::Result<()>) -> bool {
            result.is_err_and(|error| error.kind() == io::ErrorKind::PermissionDenied)
        }

        #[test]
        fn a_plain_name_matches_in_any_case() {
            let f = common::SshdFixture::new();
            for name in ["GitHost.Example", "githost.example", "GITHOST.EXAMPLE"] {
                for written in ["GitHost.Example", "githost.example", "gITHOST.eXAMPLE"] {
                    let result = establish(&f, written, plain(&f, name));
                    assert!(result.is_ok(), "{name} for {written}: {result:?}");
                }
            }
        }

        #[test]
        fn a_hashed_name_matches_the_host_as_written_or_lowercased() {
            let f = common::SshdFixture::new();
            for (name, written) in [
                ("githost.example", "githost.example"),
                ("githost.example", "GitHost.Example"),
                ("GitHost.Example", "GitHost.Example"),
            ] {
                let result = establish(&f, written, hashed(&f, name));
                assert!(result.is_ok(), "{name} for {written}: {result:?}");
            }
            // As on 1.0.17: a hash of a mixed-case name names no other spelling.
            for written in ["githost.example", "GITHOST.EXAMPLE"] {
                let result = establish(&f, written, hashed(&f, "GitHost.Example"));
                assert!(refused(result), "GitHost.Example for {written}");
            }
        }

        #[test]
        fn case_folding_trusts_no_other_host_and_no_other_key() {
            let f = common::SshdFixture::new();
            for known in [plain(&f, "OtherHost.Example"), hashed(&f, "OtherHost.Example")] {
                assert!(refused(establish(&f, "GitHost.Example", known.clone())), "{known}");
            }
            // The right name in another case, with a key the server does not hold.
            let other = common::SshdFixture::new();
            let wrong = other.known_host("GitHost.Example", f.port, false);
            assert!(refused(establish(&f, "githost.example", wrong)));
            // A spelling that is not the key's host is refused before any connection.
            let result = establish(&f, "otherhost.example", plain(&f, "otherhost.example"));
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
    }
}
