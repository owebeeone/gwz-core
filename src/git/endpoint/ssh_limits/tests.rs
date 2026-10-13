//! The two Windows limits TD5 keeps, their failures and their words; pure, so they run on every platform.
use super::*;
use crate::git::endpoint::setup_retry::{Phase, Verdict, classify};
use crate::git::endpoint::ssh_home::HomeSource;
use crate::git::endpoint::ssh_setup::failure_from_io;
use gwz_transport::protocol::{AuthMethod, ErrorCode, Facts};
use std::path::PathBuf;

const PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----\n";

#[test]
fn only_an_rsa_pem_key_is_read_by_cng() {
    assert!(cng_reads(PEM));
    // Blank lines before the header are skipped, as libssh2 skips them; anything else on the header's line is not.
    assert!(cng_reads(&format!("\n \n\t\n{PEM}")));
    assert!(!cng_reads(&format!("\n  {PEM}")));
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

/// `body` as lines of `width` characters, the way `ssh-keygen -m PEM` and OpenSSL write one.
fn wrapped(body: &str, width: usize) -> String {
    body.as_bytes()
        .chunks(width)
        .map(|line| String::from_utf8(line.to_vec()).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}
const RSA_BEGIN: &str = "-----BEGIN RSA PRIVATE KEY-----";
const RSA_END: &str = "-----END RSA PRIVATE KEY-----";
const OPENSSH: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAA\n-----END OPENSSH PRIVATE KEY-----\n";

/// A base64 body of `length` characters (the shape matters here, not the key).
fn body(length: usize) -> String {
    "MIIEowIBAAKCAQEA".chars().cycle().take(length).collect()
}

/// State review P2-6: libssh2's in-memory PEM reader never returns for text its header and footer searches cannot
/// match exactly, so the guard before it admits nothing but a block that they match.
#[test]
fn cng_refuses_what_libssh2_would_never_finish_reading() {
    // The header with a trailing space does not match, however well formed the OpenSSH key below it is.
    let spaced = format!("{RSA_BEGIN} \n{OPENSSH}");
    assert!(!cng_reads(&spaced));
    // A traditional PEM whose base64 is on one line: libssh2 drops the 128th character of any line that long.
    let single = format!("{RSA_BEGIN}\n{}\n{RSA_END}\n", body(2400));
    assert!(!cng_reads(&single));
    // A line of 127 characters is already past what the reader keeps; one of 126 is not.
    assert!(!cng_reads(&format!(
        "{RSA_BEGIN}\n{}\n{RSA_END}\n",
        body(127)
    )));
    assert!(cng_reads(&format!(
        "{RSA_BEGIN}\n{}\n{RSA_END}\n",
        body(126)
    )));
    // The header must come first; text before it is not a PEM file.
    assert!(!cng_reads(&format!(
        "# my key\n{RSA_BEGIN}\nMIIB\n{RSA_END}\n"
    )));
    // The block must end: no footer, another label's footer, and a footer that is not exactly the line.
    for footer in [
        "",
        "-----END PRIVATE KEY-----",
        "-----END RSA PRIVATE KEY----- ",
        "-----END RSA PRIVATE KEY",
    ] {
        assert!(
            !cng_reads(&format!("{RSA_BEGIN}\nMIIB\n{footer}\n")),
            "{footer:?}"
        );
    }
    // The RSA block is the key: another armor line inside it means it is not one block.
    assert!(!cng_reads(&format!(
        "{RSA_BEGIN}\nMIIB\n{OPENSSH}{RSA_END}\n"
    )));
    assert!(!cng_reads(&format!(
        "{RSA_BEGIN}\n{RSA_BEGIN}\nMIIB\n{RSA_END}\n"
    )));
}

#[test]
fn cng_reads_the_pem_ssh_keygen_and_openssl_write() {
    // 64 columns (OpenSSL's width, and `ssh-keygen -m PEM`'s), a real key's length, and the ends they write.
    for ending in ["\n", "\r\n"] {
        let text =
            format!("{RSA_BEGIN}\n{}\n{RSA_END}\n", wrapped(&body(1596), 64)).replace('\n', ending);
        assert!(cng_reads(&text), "{ending:?}");
        assert!(
            cng_reads(text.trim_end()),
            "{ending:?} with no final line ending"
        );
    }
    // Lines that old tools wrote at 76 columns.
    assert!(cng_reads(&format!(
        "{RSA_BEGIN}\n{}\n{RSA_END}\n",
        wrapped(&body(1596), 76)
    )));
    // Text after the block is ignored by the reader, and may hold other blocks.
    assert!(cng_reads(&format!(
        "{RSA_BEGIN}\nMIIB\n{RSA_END}\n-----BEGIN EC PARAMETERS-----\nBggq\n-----END EC PARAMETERS-----\n"
    )));
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

/// A `known_hosts` file as the home contract names it, for the words that quote it.
fn trust_file(source: HomeSource) -> KnownHostsFile {
    KnownHostsFile {
        path: PathBuf::from("C:\\Users\\Ana Lee\\.ssh\\known_hosts"),
        source,
    }
}

#[test]
fn the_words_name_the_cause_and_the_fix_and_nothing_secret() {
    let keys = SshLimit::HostKeys.reason("git.example.com", 22, None);
    assert!(keys.contains("git.example.com"), "{keys}");
    assert!(keys.contains("RSA"), "{keys}");
    assert!(
        keys.contains("ssh-keyscan -t rsa git.example.com"),
        "{keys}"
    );
    assert!(!keys.contains("-p 22"), "{keys}");
    let keys = SshLimit::HostKeys.reason("git.example.com", 2222, None);
    assert!(
        keys.contains("ssh-keyscan -t rsa -p 2222 git.example.com"),
        "{keys}"
    );
    assert!(!keys.contains("failed to set hostkey preference"), "{keys}");
    let file = SshLimit::KeyFile.reason("git.example.com", 22, None);
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
fn the_known_hosts_words_print_the_file_the_home_source_and_the_commands_to_run() {
    // The user guide's promise (design section 14, item 13, U8): the file gwz read, which home it came from, and what
    // to run, in both shells, with the fingerprint check.
    let words = SshLimit::HostKeys.reason(
        "git.example.com",
        2222,
        Some(&trust_file(HomeSource::UserProfile)),
    );
    let path = "C:\\Users\\Ana Lee\\.ssh\\known_hosts";
    for needle in [
        path,
        "USERPROFILE",
        "ssh-keyscan -t rsa -p 2222 git.example.com | Add-Content -Encoding ascii \"C:\\Users\\Ana Lee\\.ssh\\known_hosts\"",
        "ssh-keyscan -t rsa -p 2222 git.example.com >> \"C:\\Users\\Ana Lee\\.ssh\\known_hosts\"",
        "ssh-keygen -lf \"C:\\Users\\Ana Lee\\.ssh\\known_hosts\"",
        "PowerShell",
        "Git Bash",
    ] {
        assert!(words.contains(needle), "{needle}: {words}");
    }
    // Each home source is named as the variable or variables that supplied it.
    for (source, named) in [
        (HomeSource::Home, "HOME"),
        (HomeSource::HomeDrivePath, "HOMEDRIVE and HOMEPATH"),
        (HomeSource::UserProfile, "USERPROFILE"),
    ] {
        let words = SshLimit::HostKeys.reason("h.example", 22, Some(&trust_file(source)));
        assert!(
            words.contains(&format!("home came from {named}")),
            "{named}: {words}"
        );
    }
    // The key-file limit has no known_hosts file to name.
    assert!(
        !SshLimit::KeyFile
            .reason("h.example", 22, Some(&trust_file(HomeSource::Home)))
            .contains("known_hosts")
    );
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
