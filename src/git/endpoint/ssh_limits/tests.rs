//! The two Windows limits TD5 keeps, their failures and their words; pure, so they run on every platform.
use super::*;
use crate::git::endpoint::setup_retry::{Phase, Verdict, classify};
use crate::git::endpoint::ssh_setup::failure_from_io;
use gwz_transport::protocol::{AuthMethod, ErrorCode, Facts};

const PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----\n";

#[test]
fn only_an_rsa_pem_key_is_read_by_cng() {
    assert!(cng_reads(PEM));
    assert!(cng_reads(&format!("\n  {PEM}")));
    // A new-format RSA key (what `ssh-keygen -t rsa` writes), the other key types and other containers.
    for text in [
        "-----BEGIN OPENSSH PRIVATE KEY-----\nb3Bl\n-----END OPENSSH PRIVATE KEY-----\n",
        "-----BEGIN EC PRIVATE KEY-----\nMHc\n-----END EC PRIVATE KEY-----\n",
        "-----BEGIN PRIVATE KEY-----\nMIIE\n-----END PRIVATE KEY-----\n",
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIE\n-----END ENCRYPTED PRIVATE KEY-----\n",
        "",
        "not a key",
    ] {
        assert!(!cng_reads(text), "{text:?}");
    }
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// WinCNG reads one form only.
        fn unreadable(text: &str) -> bool {
            !cng_reads(text)
        }
    } else {
        /// OpenSSL reads every form.
        fn unreadable(_text: &str) -> bool {
            false
        }
    }
}

#[test]
fn a_key_form_is_a_limit_only_where_the_library_cannot_read_it() {
    for text in [
        PEM,
        "-----BEGIN OPENSSH PRIVATE KEY-----\nb3Bl\n-----END OPENSSH PRIVATE KEY-----\n",
        "",
    ] {
        assert_eq!(key_form_limit(text).is_some(), unreadable(text), "{text:?}");
    }
}

#[test]
fn a_limit_is_an_unsupported_operation_that_a_setup_never_retries() {
    for limit in [SshLimit::HostKeys, SshLimit::KeyFile] {
        let error = limit.into_error();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{limit:?}");
        assert_eq!(SshLimit::of_error(&error), Some(limit));
        let failure = failure_from_io(&error);
        assert_eq!(failure.code, ErrorCode::UnsupportedOperation, "{limit:?}");
        assert_eq!(
            classify(&failure, Phase::Setup),
            Verdict::Close,
            "{limit:?}"
        );
        // A plain Unsupported error is not a limit.
        assert_eq!(
            SshLimit::of_error(&io::Error::from(io::ErrorKind::Unsupported)),
            None
        );
    }
}

#[test]
fn the_host_tells_the_two_limits_apart_by_what_the_open_had_offered() {
    let facts = |method| {
        Some(Facts {
            method,
            credential_offered: method != AuthMethod::None,
            ..Facts::default()
        })
    };
    let with = |limit: SshLimit, method| {
        let mut failure = limit.failure();
        failure.facts = facts(method);
        failure
    };
    assert_eq!(
        SshLimit::of_failure(&with(SshLimit::HostKeys, AuthMethod::None)),
        Some(SshLimit::HostKeys)
    );
    assert_eq!(
        SshLimit::of_failure(&with(SshLimit::KeyFile, AuthMethod::SshKey)),
        Some(SshLimit::KeyFile)
    );
    // The driver's own refusals carry no facts and are not limits; nor are other codes.
    assert_eq!(SshLimit::of_failure(&SshLimit::HostKeys.failure()), None);
    let mut other = with(SshLimit::KeyFile, AuthMethod::SshKey);
    other.code = ErrorCode::Authentication;
    assert_eq!(SshLimit::of_failure(&other), None);
    let mut agent = with(SshLimit::KeyFile, AuthMethod::SshAgent);
    agent.code = ErrorCode::UnsupportedOperation;
    assert_eq!(SshLimit::of_failure(&agent), None);
}

#[test]
fn the_words_name_the_cause_and_the_fix_and_nothing_secret() {
    let keys = SshLimit::HostKeys.reason("git.example.com", 22);
    assert!(keys.contains("git.example.com"), "{keys}");
    assert!(keys.contains("RSA"), "{keys}");
    assert!(
        keys.contains("ssh-keyscan -t rsa git.example.com"),
        "{keys}"
    );
    assert!(!keys.contains("-p 22"), "{keys}");
    let keys = SshLimit::HostKeys.reason("git.example.com", 2222);
    assert!(
        keys.contains("ssh-keyscan -t rsa -p 2222 git.example.com"),
        "{keys}"
    );
    assert!(!keys.contains("failed to set hostkey preference"), "{keys}");
    let file = SshLimit::KeyFile.reason("git.example.com", 22);
    for needle in [
        "RSA",
        "PEM",
        "BEGIN RSA PRIVATE KEY",
        "ssh-keygen -p -m PEM",
        "ECDSA",
        "Ed25519",
        "agent",
    ] {
        assert!(file.contains(needle), "{needle}: {file}");
    }
    assert!(!file.ends_with(':'), "{file}");
}

#[test]
fn the_crt_reads_text_mode_files_as_libssh2_does_on_windows() {
    let read = |bytes: &[u8]| crt_text(bytes).into_owned();
    // A carriage return before a line feed is dropped; a lone one stays.
    assert_eq!(read(b"a\r\nb\r\n"), b"a\nb\n");
    assert_eq!(read(b"a\rb\n"), b"a\rb\n");
    assert_eq!(read(b"a\r"), b"a\r");
    assert_eq!(read(b"a\r\r\nb"), b"a\r\nb");
    // Ctrl-Z ends the file.
    assert_eq!(read(b"a\nb\x1a\nc\n"), b"a\nb");
    assert_eq!(read(b"\x1aa\n"), b"");
    assert_eq!(read(b""), b"");
    assert_eq!(read(b"plain\n"), b"plain\n");
}
