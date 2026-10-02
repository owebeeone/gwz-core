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
            let dir = tempfile::tempdir().unwrap();
            let path = keys::keygen(dir.path(), "rsa", "rsa", Some(2048));
            let rsa = keys::public(dir.path(), "rsa");
            let signs: Vec<_> = listed
                .iter()
                .map(|name| if *name == "rsa" { Sign::Upstream(&rsa.1) } else { Sign::Dsa })
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
    }
}
