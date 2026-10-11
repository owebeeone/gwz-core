//! TR2.8: an RSA agent key signs with `ssh-rsa` (SHA-1) exactly where
//! 1.0.17's libssh2 does, in dev-docs/GwzTransportSshKeyTypes.md §3's three
//! cases, and with `rsa-sha2-*` whenever `server-sig-algs` lists it; when
//! `server-sig-algs` lists none of the three the key fails, as libssh2's does.
//! `password_sshd.py` sends exactly the `server-sig-algs` a row names, or none.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::key_fixture::{self as keys, KeyAgent, Sign};
        use crate::git::endpoint::ssh_password_fixture::PasswordSshd;
        use std::{fs, io};
        use tempfile::TempDir;

        struct Row {
            agent: KeyAgent,
            server: PasswordSshd,
            known: std::path::PathBuf,
            _dir: TempDir,
        }

        /// The agent lists `listed` (`rsa` and `dsa`, in order); the server
        /// authorizes them and sends `sig_algs`.
        fn row(listed: &[&str], sig_algs: Option<&str>, rsa_sha1: bool) -> Row {
            row_inner(listed, sig_algs, rsa_sha1, None)
        }

        fn row_inner(listed: &[&str], sig_algs: Option<&str>, rsa_sha1: bool, malformed: Option<usize>) -> Row {
            let dir = tempfile::tempdir().unwrap();
            let path = keys::keygen(dir.path(), "rsa", "rsa", Some(2048));
            let rsa = keys::public(dir.path(), "rsa");
            let signs: Vec<_> = listed
                .iter()
                .map(|name| if *name == "rsa" {
                    malformed.map_or(Sign::Upstream(&rsa.1), |length| Sign::MalformedRsa { key: &rsa.1, length })
                } else { Sign::Dsa })
                .collect();
            let agent = KeyAgent::start(dir.path(), &[&path], &signs, rsa_sha1);
            let authorized: Vec<_> = listed
                .iter()
                .zip(&agent.blobs)
                .map(|(name, blob)| format!("{} {blob}", if *name == "rsa" { "ssh-rsa" } else { "ssh-dss" }))
                .collect();
            let server = PasswordSshd::start_with(&dir.path().join("server"), "unused", &["publickey"], &authorized, sig_algs);
            let known = dir.path().join("known_hosts");
            fs::write(&known, server.known_host("127.0.0.1")).unwrap();
            Row { agent, server, known, _dir: dir }
        }

        impl Row {
            fn authenticate(&self) -> io::Result<()> {
                keys::authenticate(self.server.port, &self.known, "git", &self.agent.path)
            }
        }

        #[test]
        fn malformed_sha1_answer_never_requests_a_fallback_query_or_signature() {
            for length in [1, 257] {
                let row = row_inner(&["rsa"], Some("rsa-sha2-512,ssh-rsa"), false, Some(length));
                assert_eq!(row.authenticate().unwrap_err().kind(), io::ErrorKind::InvalidData);
                assert_eq!(row.agent.requests(), ["list", "sign:0:4"]);
                assert_eq!(row.server.publickey_requests(), ["rsa-sha2-512:query"]);
            }
        }

        #[test]
        fn ssh_rsa_without_server_sig_algs() {
            let row = row(&["rsa"], None, false);
            row.authenticate().unwrap();
            assert_eq!(row.agent.requests(), ["list", "sign:0:0"]);
            assert_eq!(row.server.publickey_requests(), ["ssh-rsa:query", "ssh-rsa:accepted"]);
        }

        #[test]
        fn ssh_rsa_when_server_sig_algs_lists_it_and_no_rsa_sha2() {
            let row = row(&["rsa"], Some("ssh-ed25519,ssh-rsa"), false);
            row.authenticate().unwrap();
            assert_eq!(row.agent.requests(), ["list", "sign:0:0"]);
            assert_eq!(row.server.publickey_requests(), ["ssh-rsa:query", "ssh-rsa:accepted"]);
        }

        #[test]
        fn ssh_rsa_once_when_the_agent_answers_rsa_sha2_with_ssh_rsa() {
            let row = row(&["rsa"], Some("rsa-sha2-512,rsa-sha2-256,ssh-rsa"), true);
            row.authenticate().unwrap();
            // The agent was asked for SHA-512, answered with SHA-1, and the
            // key was offered once more, as ssh-rsa.
            assert_eq!(row.agent.requests(), ["list", "sign:0:4", "sign:0:0"]);
            assert_eq!(
                row.server.publickey_requests(),
                ["rsa-sha2-512:query", "ssh-rsa:query", "ssh-rsa:accepted"]
            );
        }

        #[test]
        fn rsa_sha2_whenever_server_sig_algs_lists_it() {
            for (sig_algs, flags, algorithm) in [
                ("rsa-sha2-512,rsa-sha2-256,ssh-rsa", 4, "rsa-sha2-512"),
                ("rsa-sha2-256,ssh-rsa", 2, "rsa-sha2-256"),
                ("ssh-rsa,rsa-sha2-256", 2, "rsa-sha2-256"),
            ] {
                let row = row(&["rsa"], Some(sig_algs), false);
                row.authenticate().unwrap_or_else(|e| panic!("{sig_algs}: {e}"));
                assert_eq!(row.agent.requests(), ["list".to_owned(), format!("sign:0:{flags}")], "{sig_algs}");
                assert_eq!(
                    row.server.publickey_requests(),
                    [format!("{algorithm}:query"), format!("{algorithm}:accepted")],
                    "{sig_algs}"
                );
            }
        }

        #[test]
        fn rsa_fails_as_an_authentication_failure_when_server_sig_algs_lists_none_of_its_algorithms() {
            let row = row(&["rsa"], Some("ssh-ed25519"), false);
            assert_eq!(row.authenticate().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            // libssh2 finds no algorithm before it sends anything.
            assert_eq!(row.agent.requests(), ["list"]);
            assert!(row.server.publickey_requests().is_empty());
        }

        #[test]
        fn later_keys_fail_after_an_rsa_key_without_an_algorithm_as_in_1_0_17() {
            // libssh2 keeps the failed RSA key's method, so a DSA key after it
            // fails too, though the server lists ssh-dss; ahead of it, the
            // DSA key signs. 1.0.17 behaves the same (the list's §3).
            let after = row(&["rsa", "dsa"], Some("ssh-dss"), false);
            assert_eq!(after.authenticate().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            assert!(after.server.publickey_requests().is_empty());
            let ahead = row(&["dsa", "rsa"], Some("ssh-dss"), false);
            ahead.authenticate().unwrap();
            assert_eq!(ahead.agent.requests(), ["list", "sign:0:0"]);
            assert_eq!(ahead.server.publickey_requests(), ["ssh-dss:query", "ssh-dss:accepted"]);
        }
    } else {
        // Windows. These rows run libssh2's own agent client against the pipe agent (`key_fixture_pipe`), as gwz 1.0.17
        // authenticates, because the transport's own agent signing is Unix-only until step 3.4. libssh2 chooses the
        // RSA algorithm from the server's `server-sig-algs`, as on Unix (the three cases above), but the agent
        // fixture signs only SHA-2: `ring` has no SHA-1 RSA, so the cases that end at `ssh-rsa` end at the agent. The
        // TD5 baseline (`2026-10-10-tr18-windows-ssh-rows`, x4-pipe-rsa-*) recorded 1.0.17 asking for `rsa-sha2-512`
        // only, and refusing a server that lists `rsa-sha2-256` without it; the libssh2 this build links (1.11.1_DEV,
        // libssh2-sys 0.3.3) signs `rsa-sha2-256` where that is all the server lists.
        use super::key_fixture_pipe::{self as keys, KeyAgent, Sign};
        use crate::git::endpoint::ssh_password_fixture::PasswordSshd;
        use std::{fs, io};
        use tempfile::TempDir;

        struct Row {
            agent: KeyAgent,
            server: PasswordSshd,
            known: std::path::PathBuf,
            _dir: TempDir,
        }

        /// The agent lists one RSA key; the server authorizes it and sends `sig_algs`.
        fn row(sig_algs: Option<&str>) -> Row {
            let dir = tempfile::tempdir().unwrap();
            let path = keys::keygen_pem_rsa(dir.path(), "rsa", 2048);
            let rsa = keys::public(dir.path(), "rsa");
            let agent = KeyAgent::start(dir.path(), &[&path], &[Sign::Upstream(&rsa.1)]);
            let authorized = [format!("ssh-rsa {}", agent.blobs[0])];
            let server = PasswordSshd::start_with(&dir.path().join("server"), "unused", &["publickey"], &authorized, sig_algs);
            let known = dir.path().join("known_hosts");
            fs::write(&known, server.known_host("127.0.0.1")).unwrap();
            Row { agent, server, known, _dir: dir }
        }

        impl Row {
            fn authenticate(&self) -> io::Result<()> {
                self.agent.authenticate(self.server.port, &self.known, "git")
            }
        }

        #[test]
        fn rsa_signs_at_rsa_sha2_512_whenever_server_sig_algs_lists_it() {
            for sig_algs in ["rsa-sha2-512,rsa-sha2-256,ssh-rsa", "ssh-rsa,rsa-sha2-256,rsa-sha2-512", "rsa-sha2-512,ssh-rsa", "rsa-sha2-512"] {
                let row = row(Some(sig_algs));
                row.authenticate().unwrap_or_else(|e| panic!("{sig_algs}: {e}"));
                assert_eq!(row.agent.requests(), ["list", "sign:0:4"], "{sig_algs}");
                assert_eq!(
                    row.server.publickey_requests(),
                    ["rsa-sha2-512:query", "rsa-sha2-512:accepted"],
                    "{sig_algs}"
                );
            }
        }

        #[test]
        fn rsa_signs_at_rsa_sha2_256_where_that_is_all_server_sig_algs_lists_of_the_sha2_pair() {
            for sig_algs in ["rsa-sha2-256", "rsa-sha2-256,ssh-rsa"] {
                let row = row(Some(sig_algs));
                row.authenticate().unwrap_or_else(|e| panic!("{sig_algs}: {e}"));
                assert_eq!(row.agent.requests(), ["list", "sign:0:2"], "{sig_algs}");
                assert_eq!(
                    row.server.publickey_requests(),
                    ["rsa-sha2-256:query", "rsa-sha2-256:accepted"],
                    "{sig_algs}"
                );
            }
        }

        #[test]
        fn rsa_asks_for_ssh_rsa_where_that_is_all_there_is_and_the_fixture_agent_has_no_sha1() {
            // The server lists `ssh-rsa` alone, or says nothing: libssh2 offers the SHA-1 form, and the agent
            // fixture, which signs SHA-2 only, refuses. The key fails with an authentication failure.
            for sig_algs in [Some("ssh-rsa"), None] {
                let row = row(sig_algs);
                assert_eq!(row.authenticate().unwrap_err().kind(), io::ErrorKind::PermissionDenied, "{sig_algs:?}");
                assert_eq!(row.agent.requests(), ["list", "sign:0:0"], "{sig_algs:?}");
                assert_eq!(row.server.publickey_requests(), ["ssh-rsa:query"], "{sig_algs:?}");
            }
        }

        #[test]
        fn rsa_fails_before_any_request_when_server_sig_algs_lists_none_of_its_algorithms() {
            let row = row(Some("ssh-ed25519"));
            assert_eq!(row.authenticate().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            // libssh2 finds no algorithm before it sends anything.
            assert_eq!(row.agent.requests(), ["list"]);
            assert!(row.server.publickey_requests().is_empty());
        }
    }
}
