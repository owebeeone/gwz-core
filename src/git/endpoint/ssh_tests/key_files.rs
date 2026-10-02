//! TR2.8: the transport's key files (`--identity`, `--remote-identity`) cover
//! the containers 1.0.17's libssh2 reads (dev-docs/GwzTransportSshKeyTypes.md
//! §5). Each file authenticates on the production path, the snapshot registry
//! reading and checking it and `ssh_key_auth` signing, against a disposable
//! sshd that authorizes only its key.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::key_fixture as keys;
        use crate::git::endpoint::{
            agent_job::Job, ssh_fixture::{self as common, SshdFixture}, ssh_key_auth,
            ssh_key_snapshot::Registry, ssh_network,
        };
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use gwz_transport::pool::Key;
        use std::{
            fs, io,
            path::{Path, PathBuf},
            process::Command,
            task::{Context, Poll, Waker},
            time::{Duration, Instant},
        };

        /// What `openssl ecparam -name prime256v1 -genkey` writes before the key.
        const P256_PARAMETERS: &str = "-----BEGIN EC PARAMETERS-----\nBggqhkjOPQMBBw==\n-----END EC PARAMETERS-----\n";

        fn finish<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
            let until = Instant::now() + Duration::from_secs(15);
            loop {
                if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
                    return result;
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        /// Authenticates with the key file `path` against a server that
        /// authorizes only `public`, a key's type and blob.
        fn authenticate(path: &Path, public: &(String, String)) -> io::Result<()> {
            let server = SshdFixture::new();
            let line = format!("{} {}\n", public.0, public.1);
            fs::write(server.temp.path().join("authorized_keys"), line).unwrap();
            let registry = Registry::new();
            let key = Key::ssh(&server.user, "127.0.0.1", server.port);
            let loaded = finish(&mut registry.start(key.clone(), path.into(), None, Duration::from_secs(1))?)?;
            let entry = registry.intern(loaded, || Ok(()))?;
            let known = server.known_hosts.clone();
            let mut job = Job::start(
                Some(Instant::now() + Duration::from_secs(10)),
                Duration::from_secs(1),
                move |control| {
                    let (connection, trusted) = ssh_network::establish(&key, &known, &control)?;
                    ssh_key_auth::authenticate_reporting(connection, &trusted, entry, control, || {}, || {})
                        .map(drop)
                },
            )?;
            finish(&mut job)
        }

        /// A copy of the key file `from` at `dir/name`, converted by
        /// `ssh-keygen` to `format` (`PEM` or `PKCS8`) unless it is `openssh`.
        fn container(dir: &Path, from: &Path, name: &str, format: &str) -> PathBuf {
            let path = dir.join(name);
            fs::copy(from, &path).unwrap();
            if format != "openssh" {
                common::run(
                    Command::new("ssh-keygen")
                        .args(["-q", "-p", "-m", format, "-P", "", "-N", "", "-f"])
                        .arg(&path),
                );
            }
            path
        }

        fn armor(label: &str, der: &[u8]) -> String {
            let body = STANDARD.encode(der);
            let lines: Vec<_> = body.as_bytes().chunks(64).map(|line| std::str::from_utf8(line).unwrap()).collect();
            format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", lines.join("\n"))
        }

        /// The PKCS#8 file `text` with an empty attributes set (`[0]`) after
        /// its key, as PKCS#8 allows.
        fn with_attributes(text: &str) -> String {
            let body: String = text.lines().filter(|line| !line.starts_with("-----")).collect();
            let der = STANDARD.decode(body).unwrap();
            let start = if der[1] & 0x80 == 0 { 2 } else { 2 + (der[1] & 0x7f) as usize };
            let mut content = der[start..].to_vec();
            content.extend_from_slice(&[0xa0, 0x00]);
            let length = content.len();
            let mut outer = vec![0x30, 0x82, (length >> 8) as u8, length as u8];
            outer.extend_from_slice(&content);
            armor("PRIVATE KEY", &outer)
        }

        #[test]
        fn text_and_blocks_around_the_key_and_pkcs8_attributes_are_read_as_libssh2_reads_them() {
            let dir = tempfile::tempdir().unwrap();
            let rsa = keys::keygen(dir.path(), "rsa", "rsa", Some(2048));
            let rsa_public = keys::public(dir.path(), "rsa");
            let pkcs8 = fs::read_to_string(container(dir.path(), &rsa, "rsa-pkcs8", "PKCS8")).unwrap();
            let pem = fs::read_to_string(container(dir.path(), &rsa, "rsa-pem", "PEM")).unwrap();
            let ecdsa = keys::keygen(dir.path(), "ecdsa", "ecdsa", Some(256));
            let ecdsa_public = keys::public(dir.path(), "ecdsa");
            let sec1 = fs::read_to_string(container(dir.path(), &ecdsa, "ecdsa-pem", "PEM")).unwrap();
            for (name, text, public) in [
                // openssl pkcs12 -nodes writes its bag's attributes before the key.
                ("bag", format!("Bag Attributes\n    localKeyID: 01 02 03 \nKey Attributes: <No Attributes>\n{pkcs8}"), &rsa_public),
                ("parameters", format!("{P256_PARAMETERS}{sec1}"), &ecdsa_public),
                ("certificate", format!("{pem}{}", armor("CERTIFICATE", &[0x30, 0x03, 0x02, 0x01, 0x00])), &rsa_public),
                ("attributes", with_attributes(&pkcs8), &rsa_public),
            ] {
                let path = dir.path().join(name);
                fs::write(&path, text).unwrap();
                authenticate(&path, public).unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        }

        #[test]
        fn snapshot_rejects_keys_hidden_behind_carriage_returns_before_native_authentication() {
            let dir = tempfile::tempdir().unwrap();
            let first = keys::keygen(dir.path(), "first", "ed25519", None);
            let second = keys::keygen(dir.path(), "second", "ed25519", None);
            let plain = fs::read_to_string(&first).unwrap();
            let second_text = fs::read_to_string(&second).unwrap();
            common::run(Command::new("ssh-keygen").args(["-q", "-p", "-P", "", "-N", "fixture-passphrase", "-f"]).arg(&first));
            let encrypted = fs::read_to_string(&first).unwrap();
            for (name, hidden) in [("plain", &plain), ("encrypted", &encrypted)] {
                let path = dir.path().join(format!("ambiguous-{name}"));
                fs::write(&path, format!("ignored\r{}ignored\n{second_text}", hidden.replace('\n', "\r"))).unwrap();
                let registry = Registry::new();
                // Snapshot admission finishes before any caller can dispatch native auth.
                let mut read = registry.start(Key::ssh("git", "127.0.0.1", 22), path, None, Duration::from_secs(1)).unwrap();
                assert_eq!(finish(&mut read).err().unwrap().kind(), io::ErrorKind::InvalidInput, "{name}");
            }
            let public = keys::public(dir.path(), "second");
            for (name, ending) in [("cr", "\r"), ("lf", "\n"), ("crlf", "\r\n")] {
                let path = dir.path().join(name);
                fs::write(&path, format!("ignored{ending}{}", second_text.replace('\n', ending))).unwrap();
                authenticate(&path, &public).unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        }

        #[test]
        fn ecdsa_and_ed25519_key_files_in_each_container_authenticate() {
            let dir = tempfile::tempdir().unwrap();
            for bits in [256, 384, 521] {
                let ecdsa = keys::keygen(dir.path(), &format!("ecdsa{bits}"), "ecdsa", Some(bits));
                let public = keys::public(dir.path(), &format!("ecdsa{bits}"));
                for format in ["openssh", "PEM", "PKCS8"] {
                    let path = container(dir.path(), &ecdsa, &format!("ecdsa{bits}-{format}"), format);
                    authenticate(&path, &public).unwrap_or_else(|e| panic!("P-{bits} {format}: {e}"));
                }
            }
            let ed25519 = keys::keygen(dir.path(), "ed25519", "ed25519", None);
            authenticate(&ed25519, &keys::public(dir.path(), "ed25519")).unwrap();
            // PKCS#8 Ed25519, which 1.0.17 offers and then cannot sign with.
            let pkcs8 = dir.path().join("ed25519-pkcs8");
            common::run(Command::new("openssl").args(["genpkey", "-algorithm", "ed25519", "-out"]).arg(&pkcs8));
            let output = Command::new("openssl")
                .args(["pkey", "-pubout", "-outform", "DER", "-in"])
                .arg(&pkcs8)
                .output()
                .unwrap();
            assert!(output.status.success());
            let point = &output.stdout[output.stdout.len() - 32..];
            let mut blob = Vec::new();
            for field in [b"ssh-ed25519".as_slice(), point] {
                blob.extend_from_slice(&(field.len() as u32).to_be_bytes());
                blob.extend_from_slice(field);
            }
            authenticate(&pkcs8, &("ssh-ed25519".to_owned(), STANDARD.encode(blob))).unwrap();
        }
    }
}
