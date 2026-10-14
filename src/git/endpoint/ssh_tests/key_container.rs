use super::key_material as keys;
use crate::git::endpoint::agent_job;
use crate::git::endpoint::ssh_key_container;
use crate::git::endpoint::ssh_limits::SshLimit;
use crate::git::endpoint::{fixture_host, ssh_fixture as common};

use agent_job::Job;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    fs, io,
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

fn finish<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
            return result;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn check(text: String) -> io::Result<()> {
    let mut job = Job::start_isolated(
        Some(Instant::now() + Duration::from_secs(2)),
        Duration::from_secs(1),
        move |control| ssh_key_container::check(&text, &control),
    )?;
    finish(&mut job)
}

/// Whether `result` is the refusal of a key file protected by a passphrase.
fn encrypted_limit(result: io::Result<()>) -> bool {
    result
        .err()
        .is_some_and(|error| SshLimit::of_error(&error) == Some(SshLimit::EncryptedKeyFile))
}

fn armor(label: &str, bytes: &[u8]) -> String {
    format!(
        "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
        STANDARD.encode(bytes)
    )
}

fn ssh_string(value: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}

fn openssh(cipher: &[u8], kdf: &[u8], options: &[u8]) -> Vec<u8> {
    let mut value = b"openssh-key-v1\0".to_vec();
    ssh_string(cipher, &mut value);
    ssh_string(kdf, &mut value);
    ssh_string(options, &mut value);
    value.push(0);
    value
}

#[test]
fn carriage_return_boundaries_cannot_hide_a_second_or_encrypted_key() {
    let plain = armor("OPENSSH PRIVATE KEY", &openssh(b"none", b"none", b""));
    let encrypted = armor("ENCRYPTED PRIVATE KEY", &[1, 2, 3]);
    let text = format!("ignored\r{}ignored\n{plain}", plain.replace('\n', "\r"));
    assert_eq!(check(text).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    // A passphrase hidden behind carriage returns is still seen, and named.
    let text = format!("ignored\r{}ignored\n{plain}", encrypted.replace('\n', "\r"));
    assert!(encrypted_limit(check(text)));
    for ending in ["\r", "\n", "\r\n"] {
        check(format!("ignored{ending}{}", plain.replace('\n', ending))).unwrap();
    }
}

#[test]
fn accepts_unencrypted_traditional_pkcs8_and_openssh_framing() {
    for label in ["RSA PRIVATE KEY", "DSA PRIVATE KEY", "EC PRIVATE KEY"] {
        check(armor(label, &[1, 2, 3, 4])).unwrap();
    }
    check(armor(
        "PRIVATE KEY",
        &[0x30, 0x07, 0x02, 1, 0, 0x30, 0, 0x04, 0],
    ))
    .unwrap();
    check(armor(
        "OPENSSH PRIVATE KEY",
        &openssh(b"none", b"none", b""),
    ))
    .unwrap();
}

#[test]
fn rejects_encryption_headers_and_extra_or_unknown_armor() {
    assert!(check(armor("ENCRYPTED PRIVATE KEY", &[1, 2])).is_err());
    assert!(check("-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nAQ==\n-----END RSA PRIVATE KEY-----\n".into()).is_err());
    assert!(
        check(format!(
            "{}-----BEGIN RSA PRIVATE KEY-----\nAQ==\n-----END RSA PRIVATE KEY-----\n",
            "x"
        ))
        .is_err()
    );
    assert!(
        check(format!(
            "{}-----BEGIN RSA PRIVATE KEY-----\nAQ==\n-----END RSA PRIVATE KEY-----\n",
            armor("RSA PRIVATE KEY", &[1])
        ))
        .is_err()
    );
    assert!(check(armor("UNKNOWN", &[1])).is_err());
    assert!(
        check("-----BEGIN RSA PRIVATE KEY-----\nAQ==\n-----END EC PRIVATE KEY-----\n".into())
            .is_err()
    );
    assert!(
        check("-----BEGIN RSA PRIVATE KEY-----\n\n-----END RSA PRIVATE KEY-----\n".into()).is_err()
    );
}

#[test]
fn skips_text_and_other_blocks_around_one_unencrypted_key_as_libssh2_does() {
    let key = armor("RSA PRIVATE KEY", &[1, 2, 3, 4]);
    for text in [
        format!("Bag Attributes\n    localKeyID: 01 02 03\nKey Attributes: <No Attributes>\n{key}"),
        format!("{}{key}", armor("EC PARAMETERS", &[6, 8])),
        format!("{key}{}", armor("CERTIFICATE", &[0x30, 0])),
        format!("{key}trailing text\n"),
    ] {
        check(text.clone()).unwrap_or_else(|e| panic!("{text}: {e}"));
    }
    // PKCS#8 with its optional attributes, [0], after the key.
    check(armor(
        "PRIVATE KEY",
        &[0x30, 0x09, 0x02, 1, 0, 0x30, 0, 0x04, 0, 0xa0, 0],
    ))
    .unwrap();
}

#[test]
fn still_refuses_encryption_a_header_or_a_second_key_anywhere_in_the_file() {
    let key = armor("RSA PRIVATE KEY", &[1, 2, 3, 4]);
    for text in [
        format!("{}{key}", armor("ENCRYPTED PRIVATE KEY", &[1])),
        format!("{key}{}", armor("ENCRYPTED PRIVATE KEY", &[1])),
        // OpenSSL decrypts a block with a PEM header whatever its label.
        format!(
            "-----BEGIN CERTIFICATE-----\nProc-Type: 4,ENCRYPTED\nAQ==\n-----END CERTIFICATE-----\n{key}"
        ),
        format!(
            "{key}{}",
            armor("OPENSSH PRIVATE KEY", &openssh(b"none", b"none", b""))
        ),
        format!("{key}{}", armor("ED448 PRIVATE KEY", &[1])),
        format!("{key}-----BEGIN CERTIFICATE-----\nAQ==\n"),
        format!("-----BEGIN CERTIFICATE-----\n{key}-----END CERTIFICATE-----\n"),
        "Bag Attributes\n".to_owned(),
    ] {
        assert!(check(text.clone()).is_err(), "{text}");
    }
}

#[test]
fn rejects_truncated_and_ambiguous_declared_lengths() {
    assert!(check(armor("PRIVATE KEY", &[0x30, 0x07, 0x02, 1, 0])).is_err());
    assert!(check(armor("PRIVATE KEY", &[0x30, 0x01, 0x02, 1, 2])).is_err());
    assert!(check(armor("PRIVATE KEY", &[0x30, 0x82, 0xff, 0xff, 0x02, 1, 0])).is_err());
    assert!(
        check(armor(
            "PRIVATE KEY",
            &[0x30, 0x07, 0x02, 1, 2, 0x30, 0, 0x04, 0]
        ))
        .is_err()
    );
    assert!(
        check(armor(
            "OPENSSH PRIVATE KEY",
            &openssh(b"aes256-ctr", b"bcrypt", &[0xff; 16])
        ))
        .is_err()
    );
    assert!(check(armor("OPENSSH PRIVATE KEY", &[b'o'; 15])).is_err());
}

#[test]
fn rejects_invalid_base64_even_after_a_valid_prefix() {
    assert!(
        check("-----BEGIN RSA PRIVATE KEY-----\nAQ==A\n-----END RSA PRIVATE KEY-----\n".into())
            .is_err()
    );
    let mut body = STANDARD.encode([1u8; 60_000]);
    body.push('!');
    assert!(
        check(format!(
            "-----BEGIN RSA PRIVATE KEY-----\n{body}\n-----END RSA PRIVATE KEY-----\n"
        ))
        .is_err()
    );
}

#[test]
fn cancellation_is_observed_while_scanning_bounded_chunks() {
    let (started, seen) = mpsc::channel();
    let text = armor("RSA PRIVATE KEY", &[1u8; 700_000]);
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        started.send(()).unwrap();
        ssh_key_container::check(&text, &control)
    })
    .unwrap();
    seen.recv_timeout(Duration::from_secs(1)).unwrap();
    job.cancel();
    assert_eq!(
        finish(&mut job).unwrap_err().kind(),
        io::ErrorKind::ConnectionAborted
    );
}

#[test]
fn checked_scan_observes_cancellation_after_first_chunk() {
    let (first, reached) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let (scanned, scan_result) = mpsc::channel();
    let observed = Arc::new(AtomicUsize::new(0));
    let worker_observed = Arc::clone(&observed);
    let input = vec![b'a'; 1_048_000];
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        let result = ssh_key_container::scan_for_test(&input, &control, |_| {
            let count = worker_observed.fetch_add(1, Ordering::AcqRel) + 1;
            if count == 128 {
                first.send(()).unwrap();
                released.recv().unwrap();
                return false;
            }
            false
        });
        scanned
            .send(result.as_ref().map(|_| ()).map_err(io::Error::kind))
            .unwrap();
        result.map(|_| ())
    })
    .unwrap();
    reached.recv_timeout(Duration::from_secs(1)).unwrap();
    job.cancel();
    release.send(()).unwrap();
    assert_eq!(
        finish(&mut job).unwrap_err().kind(),
        io::ErrorKind::ConnectionAborted
    );
    assert_eq!(
        scan_result.recv().unwrap(),
        Err(io::ErrorKind::ConnectionAborted)
    );
    assert_eq!(observed.load(Ordering::Acquire), 128);
}

/// A PEM block of `label` with `headers`, the lines OpenSSL writes before the body of a key that has a passphrase.
fn pem_with_headers(label: &str, headers: &str) -> String {
    format!(
        "-----BEGIN {label}-----\n{headers}\n{}\n-----END {label}-----\n",
        STANDARD.encode([1, 2, 3, 4])
    )
}

#[test]
fn a_key_protected_by_a_passphrase_is_refused_as_such_in_each_form() {
    // PEM with `Proc-Type` and `DEK-Info` headers, whatever the key's type; either header alone says so too.
    for label in ["RSA PRIVATE KEY", "DSA PRIVATE KEY", "EC PRIVATE KEY"] {
        for headers in [
            "Proc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,CD07FB65966AB315C1289E067C169A81\n",
            "Proc-Type: 4,ENCRYPTED\n",
            "DEK-Info: AES-128-CBC,00\n",
        ] {
            assert!(
                encrypted_limit(check(pem_with_headers(label, headers))),
                "{label}: {headers}"
            );
        }
    }
    // PKCS#8's encrypted form.
    assert!(encrypted_limit(check(armor(
        "ENCRYPTED PRIVATE KEY",
        &[1, 2, 3, 4]
    ))));
    // OpenSSH's, by its cipher, with or without a key derivation function named.
    for (cipher, kdf, options) in [
        (
            b"aes256-ctr".as_slice(),
            b"bcrypt".as_slice(),
            [0xff; 16].as_slice(),
        ),
        (b"aes256-gcm@openssh.com", b"bcrypt", &[1, 2, 3]),
        (b"aes128-ctr", b"none", b""),
    ] {
        let text = armor("OPENSSH PRIVATE KEY", &openssh(cipher, kdf, options));
        assert!(
            encrypted_limit(check(text)),
            "{}",
            String::from_utf8_lossy(cipher)
        );
    }
    // Line endings and text around the block do not hide it.
    let block = armor("ENCRYPTED PRIVATE KEY", &[1]);
    for ending in ["\r", "\n", "\r\n"] {
        let text = format!("note{ending}{}", block.replace('\n', ending));
        assert!(encrypted_limit(check(text)), "{ending:?}");
    }
}

#[test]
fn what_is_not_a_passphrase_is_refused_as_before_and_names_none() {
    let key = armor("RSA PRIVATE KEY", &[1, 2, 3, 4]);
    for text in [
        // A header that is not an encryption header.
        pem_with_headers("RSA PRIVATE KEY", "Comment: not a passphrase\n"),
        pem_with_headers("RSA PRIVATE KEY", "Proc-Type: 4,MIC-ONLY\n"),
        // A PEM header on a block that is not a key is refused whatever it says (OpenSSL decrypts it).
        format!(
            "{}{key}",
            pem_with_headers("CERTIFICATE", "Proc-Type: 4,ENCRYPTED\n")
        ),
        // Two keys, a label that is not a key type, an armor that does not close, no key at all.
        format!("{key}{key}"),
        armor("ED448 PRIVATE KEY", &[1]),
        format!("{key}-----BEGIN CERTIFICATE-----\nAQ==\n"),
        "Bag Attributes\n".to_owned(),
        // The OpenSSH framing with no cipher but a key derivation function, or with options, or too short.
        armor("OPENSSH PRIVATE KEY", &openssh(b"none", b"bcrypt", b"")),
        armor("OPENSSH PRIVATE KEY", &openssh(b"none", b"none", &[1])),
        armor("OPENSSH PRIVATE KEY", &[b'o'; 15]),
    ] {
        let error = check(text.clone()).expect_err(&text);
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{text}");
        assert_eq!(SshLimit::of_error(&error), None, "{text}");
    }
}

/// `ssh-keygen -p` rewrites the key file `path` in `format` (`PEM` or `PKCS8`; the OpenSSH form without one), with
/// the passphrase `new` (none when empty) where there was none.
fn rewrite(path: &Path, format: Option<&str>, new: &str) {
    let mut command = Command::new(fixture_host::programs().keygen);
    command.args(["-q", "-p", "-P", "", "-N", new]);
    if let Some(format) = format {
        command.args(["-m", format]);
    }
    common::run(command.arg("-f").arg(path));
}

/// `ssh-keygen`'s own keys, of each form it writes, with and without a passphrase: the unencrypted ones are admitted,
/// the others are named.
#[test]
fn keys_that_ssh_keygen_writes_are_admitted_without_a_passphrase_and_named_with_one() {
    let dir = tempfile::tempdir().unwrap();
    let rsa = keys::keygen(dir.path(), "rsa", "rsa", Some(2048));
    let ecdsa = keys::keygen(dir.path(), "ecdsa", "ecdsa", Some(256));
    let ed25519 = keys::keygen(dir.path(), "ed25519", "ed25519", None);
    let openssh = "-----BEGIN OPENSSH PRIVATE KEY-----";
    for (name, from, format, first) in [
        ("rsa-openssh", &rsa, None, openssh),
        (
            "rsa-pem",
            &rsa,
            Some("PEM"),
            "-----BEGIN RSA PRIVATE KEY-----",
        ),
        (
            "rsa-pkcs8",
            &rsa,
            Some("PKCS8"),
            "-----BEGIN PRIVATE KEY-----",
        ),
        ("ecdsa-openssh", &ecdsa, None, openssh),
        (
            "ecdsa-pem",
            &ecdsa,
            Some("PEM"),
            "-----BEGIN EC PRIVATE KEY-----",
        ),
        (
            "ecdsa-pkcs8",
            &ecdsa,
            Some("PKCS8"),
            "-----BEGIN PRIVATE KEY-----",
        ),
        ("ed25519-openssh", &ed25519, None, openssh),
    ] {
        let plain = dir.path().join(name);
        fs::copy(from, &plain).unwrap();
        rewrite(&plain, format, "");
        let text = fs::read_to_string(&plain).unwrap();
        assert!(text.starts_with(first), "{name}: {text}");
        check(text).unwrap_or_else(|error| panic!("{name}: {error}"));
        let locked = dir.path().join(format!("{name}-locked"));
        fs::copy(&plain, &locked).unwrap();
        rewrite(&locked, format, "fixture-passphrase");
        let text = fs::read_to_string(&locked).unwrap();
        assert!(encrypted_limit(check(text.clone())), "{name}: {text}");
    }
}
