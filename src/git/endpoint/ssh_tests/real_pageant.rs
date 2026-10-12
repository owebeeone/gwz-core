//! Plan step 3.6's rows against PuTTY 0.83's own `pageant.exe` (B05, B06, X9 and X4's Pageant column, the confirmation
//! timeout, a declined confirmation and an answered one, and a Pageant that goes away). **They are ignored, and owed
//! to the operator's console session**: Pageant's window is found on the desktop of the process that looks, so these
//! must run in the session where the operator started Pageant (an `ssh` session is on another desktop and finds none),
//! and three of them need a person at Pageant's dialogs. No parity claim for Pageant rests on the synthetic rows
//! (`pageant_endpoint.rs`, `agent_pageant/windows_tests.rs`) without these.
//!
//! Each row names the Pageant it needs and checks that it has it, and says so if it does not. The operator starts
//! Pageant (a pinned PuTTY 0.83 `pageant.exe` and disposable keys; nothing here starts or stops it):
//!
//! - **A**: `pageant.exe rsa.ppk ecdsa.ppk ed25519.ppk`, unencrypted keys of the three types: `b05_x4_*`, `b06_*`.
//! - **B**: `pageant.exe -encrypted encrypted.ppk`, a passphrase-protected key that Pageant decrypts on first use
//!   (the 3.5 fixture's; its passphrase was not kept): `t6_*` leaves Pageant's passphrase dialog open, and `declined_*`
//!   needs the operator to press Cancel in it. `answered_*` needs a key whose passphrase the operator knows (made in
//!   PuTTYgen, loaded the same way): the passphrase typed into the dialog within 20 seconds of the row's start.
//! - **C**: `pageant.exe` with no key: `x9_*`.
//! - **D**: any of the above, to be quit by the operator when the row asks: `gone_*`.
//!
//! `GWZ_TEST_REAL_PAGEANT=1 cargo test --lib real_pageant -- --ignored --test-threads=1 <row>`, from a shell in that
//! session. A row prints what it needs from the operator, and when, to its own output.
use super::{
    agent_fixture_pipe::Fixture as Keys,
    attachment,
    local_endpoint::{connect, failure, finish},
};
use crate::git::endpoint::{
    agent_job::{Control, wall_clock},
    agent_source::AgentSource,
    ssh_channel::GitService,
    ssh_fixture::SshdFixture,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gwz_transport::{
    pool::{Config, Key},
    protocol::{AuthMethod, ErrorCode, SetupFailureCause},
};
use std::{
    ffi::OsStr,
    fs,
    io::{self, Read, Write},
    time::{Duration, Instant},
};

const MISSING: &str = "no Pageant window is visible to this process: run the row from the session where the operator started Pageant";

fn config() -> Config {
    Config {
        total: 2,
        per_host: 2,
        per_user_host: 1,
        cleanup_timeout_ms: 100,
        ..Config::default()
    }
}

/// The source a runtime would choose now, with `sock` as its `SSH_AUTH_SOCK`; it must be Pageant.
fn pageant(sock: Option<&str>) -> AgentSource {
    assert!(
        std::env::var_os("GWZ_TEST_REAL_PAGEANT").is_some(),
        "set GWZ_TEST_REAL_PAGEANT=1 to run the rows that need PuTTY's Pageant"
    );
    let source = AgentSource::capture(sock.map(OsStr::new));
    assert!(
        matches!(source, AgentSource::Pageant { .. }),
        "{MISSING}: {source:?}"
    );
    source
}

/// The public keys Pageant lists, each as the `authorized_keys` line that would trust it.
fn held(source: &AgentSource) -> Vec<(String, String)> {
    let control = Control::scripted(None, Duration::ZERO, Duration::from_secs(5), wall_clock());
    let mut agent = source.open(control).expect("the Pageant opens");
    let blobs = agent.identities().expect("the Pageant lists its keys");
    blobs
        .iter()
        .map(|blob| {
            let length = u32::from_be_bytes(blob[..4].try_into().unwrap()) as usize;
            let kind = String::from_utf8(blob[4..4 + length].to_vec()).unwrap();
            (kind, STANDARD.encode(blob))
        })
        .collect()
}

/// One upload-pack exchange against `server` as the endpoint production builds it, with Pageant as the agent and
/// `stall_ms` as `--ssh-timeout`: the receipt, or why it failed.
fn exchange(
    server: &SshdFixture,
    source: AgentSource,
    stall_ms: u64,
) -> io::Result<gwz_transport::protocol::Opened> {
    let config = config();
    let e = connect(config.clone(), server.known_hosts.clone(), source, stall_ms);
    let result = attachment::open(
        &e,
        Key::ssh(&server.user, "127.0.0.1", server.port),
        None,
        GitService::UploadPack,
        server.repository.to_str().unwrap(),
        attachment::deadlines(&config, stall_ms),
    )
    .and_then(|(mut stream, opened)| {
        stream.write_all(b"0000")?;
        stream.end_write()?;
        stream.read_to_end(&mut Vec::new())?;
        stream.close()?;
        Ok(opened)
    });
    finish(&e);
    result
}

fn trust(server: &SshdFixture, keys: &[(String, String)]) {
    let lines: String = keys
        .iter()
        .map(|(kind, blob)| format!("{kind} {blob}\n"))
        .collect();
    fs::write(server.temp.path().join("authorized_keys"), lines).unwrap();
}

#[test]
#[ignore = "needs PuTTY 0.83 Pageant A in the console session (GWZ_TEST_REAL_PAGEANT=1)"]
fn b05_x4_each_key_type_pageant_holds_authenticates_through_the_transport() {
    let source = pageant(None);
    let keys = held(&source);
    let kinds: Vec<&str> = keys.iter().map(|(kind, _)| kind.as_str()).collect();
    println!("Pageant holds: {kinds:?}");
    for wanted in ["ssh-rsa", "ssh-ed25519"] {
        assert!(
            kinds.contains(&wanted),
            "Pageant A holds an {wanted} key: {kinds:?}"
        );
    }
    assert!(
        kinds.iter().any(|kind| kind.starts_with("ecdsa-sha2-")),
        "Pageant A holds an ECDSA key: {kinds:?}"
    );
    for key in &keys {
        let server = SshdFixture::new();
        trust(&server, std::slice::from_ref(key));
        let opened = exchange(&server, pageant(None), 9_000)
            .unwrap_or_else(|error| panic!("{}: {error}", key.0));
        assert_eq!(opened.facts.method, AuthMethod::SshAgent, "{}", key.0);
        assert_eq!(opened.facts.authenticated, Some(true), "{}", key.0);
        println!("{}: authenticated through Pageant", key.0);
    }
}

#[test]
#[ignore = "needs PuTTY 0.83 Pageant A in the console session (GWZ_TEST_REAL_PAGEANT=1)"]
fn b06_pageant_beats_a_pipe_that_would_authenticate_and_the_pipe_sees_no_connection() {
    // The pipe is an owned agent holding a key the server trusts, and `SSH_AUTH_SOCK` names it.
    let pipe = Keys::new();
    let source = pageant(Some(pipe.agent.name()));
    assert!(matches!(
        source,
        AgentSource::Pageant {
            sock_ignored: true,
            ..
        }
    ));
    let keys = held(&source);
    let server = SshdFixture::new();
    trust(&server, &keys[..1]);
    let opened = exchange(&server, source, 9_000).unwrap();
    assert_eq!(opened.facts.method, AuthMethod::SshAgent);
    assert!(pipe.requests().is_empty(), "the pipe was never tried");
    assert!(
        !pipe.wait_client_closed(Duration::from_millis(200)),
        "nothing connected to the pipe"
    );
}

#[test]
#[ignore = "needs PuTTY 0.83 Pageant C (no key) in the console session (GWZ_TEST_REAL_PAGEANT=1)"]
fn x9_a_pageant_with_no_key_refuses_and_a_working_pipe_is_not_tried() {
    let pipe = Keys::new();
    let source = pageant(Some(pipe.agent.name()));
    assert!(held(&source).is_empty(), "Pageant C holds no key");
    let server = SshdFixture::new();
    let error = exchange(&server, source, 9_000).unwrap_err();
    let refused = failure(&error);
    assert_eq!(refused.code, ErrorCode::Authentication, "{refused:?}");
    assert!(!refused.facts.unwrap().credential_offered);
    assert!(pipe.requests().is_empty(), "the pipe was never tried");
}

#[test]
#[ignore = "needs PuTTY 0.83 Pageant B in the console session; leaves its passphrase dialog open (GWZ_TEST_REAL_PAGEANT=1)"]
fn t6_a_passphrase_dialog_left_open_fails_the_attempt_at_the_stall_limit() {
    let source = pageant(None);
    let keys = held(&source);
    assert_eq!(keys.len(), 1, "Pageant B holds the one encrypted key");
    let server = SshdFixture::new();
    trust(&server, &keys);
    println!(
        "Pageant's passphrase dialog opens now; leave it open until the row ends, then press Cancel."
    );
    let begun = Instant::now();
    let error = exchange(&server, source, 3_000).unwrap_err();
    let took = begun.elapsed();
    let refused = failure(&error);
    assert_eq!(refused.code, ErrorCode::Timeout, "{refused:?}");
    assert_eq!(refused.setup_cause, Some(SetupFailureCause::Stall));
    assert_eq!(refused.facts.unwrap().method, AuthMethod::SshAgent);
    assert!(
        took >= Duration::from_secs(3) && took < Duration::from_secs(8),
        "{took:?}"
    );
    println!("failed at the stall limit after {took:?}; press Cancel in Pageant's dialog now.");
}

#[test]
#[ignore = "needs PuTTY 0.83 Pageant B and the operator to press Cancel in its dialog (GWZ_TEST_REAL_PAGEANT=1)"]
fn declined_a_cancelled_passphrase_dialog_is_a_declined_signature_not_a_timeout() {
    let source = pageant(None);
    let keys = held(&source);
    let server = SshdFixture::new();
    trust(&server, &keys);
    println!("Pageant's passphrase dialog opens now: press Cancel within 20 seconds.");
    let error = exchange(&server, source, 20_000).unwrap_err();
    let refused = failure(&error);
    // The agent declined, the key was skipped, and no key was left: the server's side of it, not a clock's.
    assert_eq!(refused.code, ErrorCode::Authentication, "{refused:?}");
    assert!(refused.facts.unwrap().credential_offered);
}

#[test]
#[ignore = "needs PuTTY 0.83 Pageant B and the operator to type the passphrase into its dialog (GWZ_TEST_REAL_PAGEANT=1)"]
fn answered_a_passphrase_typed_in_time_authenticates_and_the_next_use_asks_nothing() {
    let source = pageant(None);
    let keys = held(&source);
    let server = SshdFixture::new();
    trust(&server, &keys);
    println!(
        "Pageant's passphrase dialog opens now: type the passphrase and press OK within 20 seconds."
    );
    let opened = exchange(&server, source, 20_000).unwrap();
    assert_eq!(opened.facts.method, AuthMethod::SshAgent);
    // The key is loaded now: the next attempt signs without a dialog.
    let begun = Instant::now();
    exchange(&server, pageant(None), 3_000).unwrap();
    assert!(
        begun.elapsed() < Duration::from_secs(3),
        "{:?}",
        begun.elapsed()
    );
}

#[test]
#[ignore = "needs a Pageant in the console session that the operator quits when asked (GWZ_TEST_REAL_PAGEANT=1)"]
fn gone_a_pageant_quit_after_the_runtime_started_is_refused_and_never_replaced_by_the_pipe() {
    let pipe = Keys::new();
    let source = pageant(Some(pipe.agent.name()));
    println!("Quit Pageant now (tray icon, Exit). The row waits up to 60 seconds.");
    let until = Instant::now() + Duration::from_secs(60);
    while matches!(AgentSource::capture(None), AgentSource::Pageant { .. }) {
        assert!(Instant::now() < until, "Pageant was not quit");
        std::thread::sleep(Duration::from_millis(250));
    }
    let server = SshdFixture::new();
    let refused = failure(&exchange(&server, source, 9_000).unwrap_err());
    assert_eq!(refused.code, ErrorCode::Unavailable, "{refused:?}");
    assert_eq!(refused.setup_cause, Some(SetupFailureCause::NotFound));
    assert!(pipe.requests().is_empty(), "the pipe was never tried");
}
