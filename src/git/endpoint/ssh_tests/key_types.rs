//! TR2.8: the transport signs with every agent key type 1.0.17 uses
//! (dev-docs/GwzTransportSshKeyTypes.md §4), and with the security keys and
//! certificates of those types, each against a disposable server; a key of a
//! type outside the list is skipped and a later key tried; and an agent's
//! refusal fails only that key. Ed25519 and RSA with SHA-2 are agent_auth.rs's
//! rows, and RSA's SHA-1 cases are rsa_sha1.rs's.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::key_fixture::{self as keys, KeyAgent, Sign};
        use crate::git::endpoint::{ssh_fixture::SshdFixture, ssh_password_fixture::PasswordSshd};
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use std::{fs, io};

        /// A disposable sshd that authorizes `authorized`, each a key's type
        /// and blob, with `extra` at the head of its configuration.
        fn sshd(authorized: &[&(String, String)], extra: &str) -> SshdFixture {
            let server = SshdFixture::with_config(extra);
            let lines: String = authorized
                .iter()
                .map(|(kind, blob)| format!("{kind} {blob}\n"))
                .collect();
            fs::write(server.temp.path().join("authorized_keys"), lines).unwrap();
            server
        }

        fn authenticate(server: &SshdFixture, agent: &KeyAgent) -> io::Result<()> {
            keys::authenticate(server.port, &server.known_hosts, &server.user, &agent.path)
        }

        fn accepted(server: &SshdFixture) -> Vec<String> {
            server
                .log()
                .lines()
                .filter_map(|line| line.split_once("Accepted publickey"))
                .map(|(_, rest)| rest.split_once(" ssh2: ").unwrap().1.split(' ').next().unwrap().to_owned())
                .collect()
        }

        /// A key blob of `kind` that no list holds, with filler fields.
        fn unlisted(kind: &str) -> String {
            let mut blob = Vec::new();
            for field in [kind.as_bytes(), b"filler", &[1; 32]] {
                blob.extend_from_slice(&(field.len() as u32).to_be_bytes());
                blob.extend_from_slice(field);
            }
            STANDARD.encode(blob)
        }

        #[test]
        fn ecdsa_keys_of_each_curve_authenticate() {
            for bits in [256, 384, 521] {
                let dir = tempfile::tempdir().unwrap();
                let path = keys::keygen(dir.path(), "ecdsa", "ecdsa", Some(bits));
                let key = keys::public(dir.path(), "ecdsa");
                let server = sshd(&[&key], "");
                let agent = KeyAgent::start(dir.path(), &[&path], &[Sign::Upstream(&key.1)], false);
                authenticate(&server, &agent).unwrap_or_else(|e| panic!("P-{bits}: {e}"));
                assert_eq!(agent.requests(), ["list", "sign:0:0"], "P-{bits}");
                assert_eq!(accepted(&server), ["ECDSA"], "P-{bits}");
            }
        }

        #[test]
        fn security_keys_authenticate_through_the_software_authenticator() {
            for (kind, bits, logged) in [("ed25519", None, "ED25519-SK"), ("ecdsa", Some(256), "ECDSA-SK")] {
                let dir = tempfile::tempdir().unwrap();
                let path = keys::keygen(dir.path(), "backing", kind, bits);
                let backing = keys::public(dir.path(), "backing");
                let security = keys::security_key(&backing.1);
                let server = sshd(&[&security], "");
                let sign = Sign::Sk { key: &security.1, backing: &backing.1 };
                let agent = KeyAgent::start(dir.path(), &[&path], &[sign], false);
                authenticate(&server, &agent).unwrap_or_else(|e| panic!("{logged}: {e}"));
                // One sign request: the authenticator's one touch.
                assert_eq!(agent.requests(), ["list", "sign:0:0"], "{logged}");
                assert_eq!(accepted(&server), [logged]);
            }
        }

        #[test]
        fn certificates_of_each_type_authenticate_against_a_trusted_user_ca() {
            // ssh-keygen's type and size, whether a security key carries the
            // certificate, how sshd logs it, and the flags the agent sees:
            // an RSA certificate signs with rsa-sha2-512, which sshd lists.
            for (kind, bits, sk, logged, flags) in [
                ("ed25519", None, false, "ED25519-CERT", 0),
                ("ecdsa", Some(256), false, "ECDSA-CERT", 0),
                ("ecdsa", Some(384), false, "ECDSA-CERT", 0),
                ("ecdsa", Some(521), false, "ECDSA-CERT", 0),
                ("rsa", Some(2048), false, "RSA-CERT", 4),
                ("ed25519", None, true, "ED25519-SK-CERT", 0),
                ("ecdsa", Some(256), true, "ECDSA-SK-CERT", 0),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let ca = keys::keygen(dir.path(), "ca", "ed25519", None);
                let path = keys::keygen(dir.path(), "key", kind, bits);
                let plain = keys::public(dir.path(), "key");
                let server = sshd(&[], &format!("TrustedUserCAKeys {}.pub\n", ca.display()));
                let user = server.user.clone();
                let certificate = if sk {
                    keys::certify(dir.path(), &ca, "sk", &keys::security_key(&plain.1), &user)
                } else {
                    keys::certify(dir.path(), &ca, "key", &plain, &user)
                };
                let sign = if sk {
                    Sign::Sk { key: &certificate.1, backing: &plain.1 }
                } else {
                    Sign::Upstream(&certificate.1)
                };
                let agent = KeyAgent::start(dir.path(), &[&path], &[sign], false);
                authenticate(&server, &agent).unwrap_or_else(|e| panic!("{logged}: {e}"));
                assert_eq!(agent.requests(), ["list".to_owned(), format!("sign:0:{flags}")], "{logged}");
                assert_eq!(accepted(&server), [logged]);
            }
        }

        #[test]
        fn dsa_keys_authenticate_through_the_agent() {
            let dir = tempfile::tempdir().unwrap();
            let agent = KeyAgent::start(dir.path(), &[], &[Sign::Dsa], false);
            let authorized = [format!("ssh-dss {}", agent.blobs[0])];
            let server = PasswordSshd::start_with(&dir.path().join("server"), "unused", &["publickey"], &authorized, None);
            let known = dir.path().join("known_hosts");
            fs::write(&known, server.known_host("127.0.0.1")).unwrap();
            keys::authenticate(server.port, &known, "git", &agent.path).unwrap();
            assert_eq!(agent.requests(), ["list", "sign:0:0"]);
            assert_eq!(server.publickey_requests(), ["ssh-dss:query", "ssh-dss:accepted"]);
        }

        #[test]
        fn a_key_of_a_type_neither_supports_is_skipped_and_a_later_key_is_tried() {
            let dir = tempfile::tempdir().unwrap();
            let path = keys::keygen(dir.path(), "rsa", "rsa", Some(2048));
            let rsa = keys::public(dir.path(), "rsa");
            // A DSA certificate, which neither libssh2 nor the transport signs
            // with, and a type in no list, which 1.0.17's libssh2 offers.
            let (certificate, xmss) = (unlisted("ssh-dss-cert-v01@openssh.com"), unlisted("ssh-xmss@openssh.com"));
            let listed = [Sign::Absent(&certificate), Sign::Absent(&xmss), Sign::Upstream(&rsa.1)];
            let agent = KeyAgent::start(dir.path(), &[&path], &listed, false);
            let authorized = [
                format!("ssh-dss-cert-v01@openssh.com {certificate}"),
                format!("ssh-xmss@openssh.com {xmss}"),
                format!("{} {}", rsa.0, rsa.1),
            ];
            let sig_algs = Some("rsa-sha2-512,rsa-sha2-256,ssh-rsa");
            let server = PasswordSshd::start_with(&dir.path().join("server"), "unused", &["publickey"], &authorized, sig_algs);
            let known = dir.path().join("known_hosts");
            fs::write(&known, server.known_host("127.0.0.1")).unwrap();
            keys::authenticate(server.port, &known, "git", &agent.path).unwrap();
            assert_eq!(server.publickey_requests(), ["rsa-sha2-512:query", "rsa-sha2-512:accepted"]);
            assert_eq!(agent.requests(), ["list", "sign:2:4"]);
        }

        #[test]
        fn an_agent_refusal_fails_that_key_only_and_a_later_key_is_tried() {
            // A security key whose device is absent: the server accepts its
            // query, the agent refuses to sign, and the next key signs, as in
            // 1.0.17.
            let dir = tempfile::tempdir().unwrap();
            keys::keygen(dir.path(), "absent", "ed25519", None);
            let absent = keys::security_key(&keys::public(dir.path(), "absent").1);
            let path = keys::keygen(dir.path(), "present", "ed25519", None);
            let present = keys::public(dir.path(), "present");
            let server = sshd(&[&absent, &present], "");
            let agent = KeyAgent::start(dir.path(), &[&path], &[Sign::Absent(&absent.1), Sign::Upstream(&present.1)], false);
            authenticate(&server, &agent).unwrap();
            assert_eq!(agent.requests(), ["list", "sign:0:0", "sign:1:0"]);
            assert_eq!(accepted(&server), ["ED25519"]);
        }
    }
}
