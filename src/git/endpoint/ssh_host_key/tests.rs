//! The untrusted host key's failure and its words; pure, so they run on every platform.
use super::*;
use crate::git::endpoint::setup_retry::{Phase, Verdict, classify};
use crate::git::endpoint::ssh_home::HomeSource;
use crate::git::endpoint::ssh_setup::failure_from_io;
use std::path::PathBuf;

fn file(source: HomeSource) -> KnownHostsFile {
    KnownHostsFile {
        path: PathBuf::from("C:\\Users\\Ana Lee\\.ssh\\known_hosts"),
        source,
    }
}

const PATH: &str = "C:\\Users\\Ana Lee\\.ssh\\known_hosts";

#[test]
fn a_refused_host_key_is_a_permission_error_that_a_setup_never_retries() {
    let error = HostKeyRefused.into_error();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(HostKeyRefused::of_error(&error));
    // A plain permission error is not it.
    assert!(!HostKeyRefused::of_error(&io::Error::from(
        io::ErrorKind::PermissionDenied
    )));
    let failure = failure_from_io(&error);
    assert_eq!(failure.code, ErrorCode::Trust);
    assert!(HostKeyRefused::of_failure(&failure));
    assert_eq!(classify(&failure, Phase::Setup), Verdict::Close);
    // A failure that is some other code, or none of ours, is not a refused host key.
    assert!(!HostKeyRefused::of_failure(&Failure {
        code: ErrorCode::Authentication,
        ..Failure::default()
    }));
}

#[test]
fn the_words_print_the_file_the_home_source_and_both_windows_shells_commands() {
    // The user guide, "Home": the file gwz read, which home it came from, and what to run.
    let words = HostKeyRefused::reason(
        "git.example.com",
        2222,
        Some(&file(HomeSource::UserProfile)),
        true,
    );
    for needle in [
        "git.example.com",
        PATH,
        "home came from USERPROFILE",
        "ssh-keyscan -t rsa -p 2222 git.example.com | Add-Content -Encoding ascii \"C:\\Users\\Ana Lee\\.ssh\\known_hosts\"",
        "ssh-keyscan -t rsa -p 2222 git.example.com >> \"C:\\Users\\Ana Lee\\.ssh\\known_hosts\"",
        "ssh-keygen -lf \"C:\\Users\\Ana Lee\\.ssh\\known_hosts\"",
        "ssh-keygen -R \"[git.example.com]:2222\" -f \"C:\\Users\\Ana Lee\\.ssh\\known_hosts\"",
        "PowerShell",
        "Git Bash",
    ] {
        assert!(words.contains(needle), "{needle}: {words}");
    }
    // The default port adds no flag and the host alone names the entry to remove.
    let plain = HostKeyRefused::reason("git.example.com", 22, Some(&file(HomeSource::Home)), true);
    assert!(
        plain.contains("ssh-keyscan -t rsa git.example.com |"),
        "{plain}"
    );
    assert!(
        plain.contains("ssh-keygen -R git.example.com -f"),
        "{plain}"
    );
    assert!(plain.contains("home came from HOME"), "{plain}");
}

#[test]
fn elsewhere_one_command_form_is_shown_and_any_key_type_is_scanned() {
    let words = HostKeyRefused::reason("h.example", 22, Some(&file(HomeSource::Home)), false);
    assert!(
        words.contains("ssh-keyscan h.example >> \"C:\\Users\\Ana Lee\\.ssh\\known_hosts\""),
        "{words}"
    );
    assert!(
        !words.contains("PowerShell") && !words.contains("-t rsa"),
        "{words}"
    );
}

#[test]
fn every_variant_says_what_may_be_wrong_and_what_to_check_before_trusting() {
    for windows in [false, true] {
        let words = HostKeyRefused::reason("h.example", 22, Some(&file(HomeSource::Home)), windows);
        // Not listed, no file, or a different key: gwz cannot say which, so it names all three.
        assert!(words.contains("does not list"), "{words}");
        assert!(words.contains("does not exist"), "{words}");
        assert!(words.contains("different key"), "{words}");
        assert!(words.contains("fingerprint"), "{words}");
    }
}

#[test]
fn without_a_file_the_words_still_name_the_cause_and_the_check() {
    let words = HostKeyRefused::reason("h.example", 22, None, false);
    assert!(
        words.contains("h.example") && words.contains("known_hosts"),
        "{words}"
    );
    assert!(words.contains("fingerprint"), "{words}");
    assert!(!words.contains("home came from"), "{words}");
}
