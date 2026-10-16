//! D17 through the whole open, on every platform: a key file protected by a passphrase is refused when it is admitted
//! (`ssh_key_container::check`), before the server is asked anything, and the open ends with a limit of its own that
//! the host words (`ssh_limits`), not with a generic `InvalidRequest` that says nothing. The words themselves are
//! `ssh_limits`' and the driver's; this suite is the endpoint's half: the failure and what the retry machine does with it.
use super::{attachment, key_material as keys};
use crate::git::endpoint::{
    fixture_host,
    setup_retry::{Phase, Verdict, classify},
    shared_reservation::Authority,
    ssh_channel::GitService,
    ssh_fixture::{self as common, SshdFixture},
    ssh_limits::SshLimit,
    ssh_local,
    ssh_worker::{Endpoint, EndpointOpenFailure},
};
use gwz_transport::{
    pool::{Config, Key},
    protocol::{AuthMethod, ErrorCode},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn config() -> Config {
    Config {
        total: 1,
        per_host: 1,
        per_user_host: 1,
        ..Config::default()
    }
}

/// The endpoint for `fixture`'s server, with the fixture's trust file and no agent.
fn endpoint(fixture: &SshdFixture, config: &Config) -> Endpoint {
    let authority = Authority::new(config.total, config.per_host);
    ssh_local::connect_with_authority(
        config.clone(),
        fixture.known_hosts.clone(),
        None,
        3_000,
        authority,
        Default::default(),
    )
    .unwrap()
}

/// Opens a stream with the identity file `selected` and returns the failure the open ended with.
fn failed_open(
    endpoint: &Endpoint,
    fixture: &SshdFixture,
    config: &Config,
    selected: PathBuf,
) -> EndpointOpenFailure {
    let path = fixture.repository.to_str().unwrap().to_owned();
    let error = attachment::open(
        endpoint,
        Key::ssh(&fixture.user, "127.0.0.1", fixture.port),
        Some(selected),
        GitService::UploadPack,
        &path,
        attachment::deadlines(config, 3_000),
    )
    .err()
    .expect("the open meets the passphrase");
    error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<EndpointOpenFailure>())
        .map(|failure| EndpointOpenFailure {
            failure: failure.failure.clone(),
            phase: failure.phase,
        })
        .unwrap_or_else(|| panic!("the bridge keeps the typed failure: {error}"))
}

fn finish(endpoint: &Endpoint) {
    endpoint.shutdown();
    let until = Instant::now() + Duration::from_secs(30);
    while !endpoint.shutdown_status().cleanup_complete {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
}

/// A copy of the key file `from` at `dir/name`, in `format` (`PEM` or `PKCS8`; OpenSSH's own form without one), with
/// the passphrase `fixture-passphrase`.
fn protected(dir: &Path, from: &Path, name: &str, format: Option<&str>) -> PathBuf {
    let path = dir.join(name);
    std::fs::copy(from, &path).unwrap();
    let mut command = Command::new(fixture_host::programs().keygen);
    command.args(["-q", "-p", "-P", "", "-N", "fixture-passphrase"]);
    if let Some(format) = format {
        command.args(["-m", format]);
    }
    common::run(command.arg("-f").arg(&path));
    path
}

#[test]
fn a_key_file_with_a_passphrase_ends_the_open_with_its_own_limit_before_any_credential() {
    let fixture = SshdFixture::new();
    let dir = tempfile::tempdir().unwrap();
    let rsa = keys::keygen(dir.path(), "rsa", "rsa", Some(2048));
    let ed25519 = keys::keygen(dir.path(), "ed25519", "ed25519", None);
    let config = config();
    let endpoint = endpoint(&fixture, &config);
    for (name, from, format) in [
        ("openssh", &ed25519, None),
        ("rsa-pem", &rsa, Some("PEM")),
        ("rsa-pkcs8", &rsa, Some("PKCS8")),
    ] {
        let key = protected(dir.path(), from, name, format);
        let failed = failed_open(&endpoint, &fixture, &config, key.clone());
        // The limit's carrier is the key-file limit's (D16): an unsupported operation, which tells the host which
        // limit by the facts the open ended with. The file was refused as it was admitted, so no credential was
        // offered, which is what tells it from the form of a key file that was offered (`SshLimit::KeyFile`).
        assert_eq!(
            failed.failure.code,
            ErrorCode::UnsupportedOperation,
            "{name}: {:?}",
            failed.failure
        );
        let facts = failed
            .failure
            .facts
            .as_ref()
            .expect("an open carries its facts");
        assert_eq!(facts.method, AuthMethod::SshKey, "{name}: {facts:?}");
        assert!(!facts.credential_offered, "{name}: {facts:?}");
        assert_eq!(
            SshLimit::of_failure(&failed.failure),
            Some(SshLimit::EncryptedKeyFile),
            "{name}"
        );
        // Never retried: an admission failure belongs to its own member (`Phase::Other`), whose open the retry
        // machine returns once and does not requeue.
        assert_eq!(failed.phase, Phase::Other, "{name}");
        assert_eq!(
            classify(&failed.failure, failed.phase),
            Verdict::Return,
            "{name}"
        );
        // The words name the file the open selected.
        let words = SshLimit::of_failure(&failed.failure).unwrap().reason(
            "127.0.0.1",
            fixture.port,
            None,
            key.to_str(),
        );
        assert!(words.contains(key.to_str().unwrap()), "{name}: {words}");
        assert!(
            words.contains("protected by a passphrase"),
            "{name}: {words}"
        );
    }
    assert_eq!(fixture.authenticated_sessions, 0);
    finish(&endpoint);
}

#[test]
fn a_key_file_that_cannot_be_used_for_another_reason_is_not_named_a_passphrase() {
    let fixture = SshdFixture::new();
    let dir = tempfile::tempdir().unwrap();
    let garbage = dir.path().join("garbage");
    std::fs::write(&garbage, "not a key\n").unwrap();
    let config = config();
    let endpoint = endpoint(&fixture, &config);
    // The refusals as they were: a file that is no key is an invalid request, a path with no file is unavailable.
    for (selected, code) in [
        (garbage, ErrorCode::InvalidRequest),
        (dir.path().join("missing"), ErrorCode::Unavailable),
    ] {
        let failed = failed_open(&endpoint, &fixture, &config, selected);
        assert_eq!(failed.failure.code, code, "{:?}", failed.failure);
        assert_eq!(SshLimit::of_failure(&failed.failure), None);
    }
    finish(&endpoint);
}
