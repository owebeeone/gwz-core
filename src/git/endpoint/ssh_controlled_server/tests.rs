//! The test-controlled server, against the libssh2 client the transport uses: it serves what the stuck-close rows
//! need (a host key, public-key authentication and an exec channel that runs Git), and it decides when the channel's
//! EOF, exit status and close are sent.
use super::{ControlledSshd, Ending, command, host_key};
use crate::git::endpoint::ssh_channel::GitService;
use crate::git::endpoint::ssh_fixture::{self as common, Reach, SshChannel, run_fixture_key};
use std::{
    fs,
    io::{self, Read},
    net::TcpStream,
    thread,
    time::{Duration, Instant},
};

fn channel_for(server: &mut ControlledSshd, service: GitService) -> SshChannel {
    let session = server.session();
    let repository = server.repository.to_str().unwrap().to_owned();
    let mut channel = SshChannel::new(session, service, &repository).unwrap();
    common::open(&mut channel);
    channel
}

#[test]
fn the_command_the_client_sends_is_read_back_whatever_the_path_holds() {
    for path in [
        "/plain/repo.git",
        "with space/repo.git",
        "quote'in/the name",
        "bang!/and'!both",
        "repo'$(touch /tmp/marker)'",
        "back\\slash/C:\\repo",
        "semi;colon && pipe | $HOME `x` \"double\"",
    ] {
        for (service, name) in [
            (GitService::UploadPack, "upload-pack"),
            (GitService::ReceivePack, "receive-pack"),
        ] {
            let sent = service.command(path).unwrap();
            let read =
                command::parse(sent.as_bytes()).unwrap_or_else(|| panic!("{sent:?} was not read"));
            assert_eq!((read.service, read.path.as_str()), (name, path), "{sent:?}");
        }
    }
}

#[test]
fn only_the_two_git_services_are_served() {
    for sent in [
        "ls",
        "git-upload-pack",
        "git-upload-pack ''",
        "git-upload-pack /unquoted",
        "git-upload-pack 'one' 'two'",
        "git-upload-pack 'unclosed",
        "git-upload-archive 'repo'",
        "git-receive-packs 'repo'",
        "sh -c 'git-upload-pack repo'",
        "",
    ] {
        assert!(command::parse(sent.as_bytes()).is_none(), "{sent:?}");
    }
    assert!(command::parse(&[0xff, 0xfe]).is_none());
}

#[test]
fn host_key_files_of_both_forms_load() {
    // The fixture's host key is OpenSSH's own form where libssh2 reads it and a traditional PEM RSA key where it
    // does not (Windows), and the server presents either.
    let dir = tempfile::tempdir().unwrap();
    for (name, arguments) in [
        ("openssh_ed25519", &["-t", "ed25519"][..]),
        ("openssh_ecdsa", &["-t", "ecdsa", "-b", "256"]),
        ("openssh_rsa", &["-t", "rsa", "-b", "2048"]),
        ("pem_rsa", &["-t", "rsa", "-b", "2048", "-m", "PEM"]),
    ] {
        let path = dir.path().join(name);
        let keygen = crate::git::endpoint::fixture_host::programs().keygen;
        common::run(
            std::process::Command::new(keygen)
                .args(["-q", "-N", ""])
                .args(arguments)
                .arg("-f")
                .arg(&path),
        );
        let key = host_key::load(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let public = fs::read_to_string(path.with_extension("pub")).unwrap();
        let blob = public.split_whitespace().nth(1).unwrap();
        assert_eq!(
            key.public_key()
                .to_openssh()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap(),
            blob,
            "{name}"
        );
    }
}

#[test]
fn a_prompt_server_runs_the_git_service_and_closes_the_channel_with_its_status() {
    let mut server = ControlledSshd::start(Ending::Prompt);
    let mut channel = channel_for(&mut server, GitService::UploadPack);
    let (stdout, stderr, status) = common::exchange(&mut channel);
    assert!(
        stdout.windows(4).any(|window| window == b"0000"),
        "{stdout:?}"
    );
    assert!(stderr.is_empty(), "{stderr:?}");
    assert_eq!(status, 0);
    assert!(
        channel.into_session().is_ok(),
        "the channel was not fully closed"
    );
    assert!(
        !server.marker.exists(),
        "the repository path was evaluated by a shell"
    );
    assert_eq!(server.authenticated_sessions, 1);
}

#[test]
fn a_prompt_server_serves_receive_pack_and_reuses_the_connection() {
    let mut server = ControlledSshd::start(Ending::Prompt);
    let mut session = Some(server.session());
    let repository = server.repository.to_str().unwrap().to_owned();
    for _ in 0..2 {
        let mut channel = SshChannel::new(
            session.take().unwrap(),
            GitService::ReceivePack,
            &repository,
        )
        .unwrap();
        common::open(&mut channel);
        let (stdout, _, status) = common::exchange(&mut channel);
        assert!(stdout.windows(4).any(|window| window == b"0000"));
        assert_eq!(status, 0);
        session = Some(match channel.into_session() {
            Ok(session) => session,
            Err(_) => panic!("the channel was not fully closed"),
        });
    }
    assert_eq!(server.authenticated_sessions, 1);
}

#[test]
fn a_stuck_server_ends_both_outputs_and_then_withholds_its_exit_status_and_close() {
    let mut server = ControlledSshd::start(Ending::Stuck);
    let mut channel = channel_for(&mut server, GitService::ReceivePack);
    // The request and the client's EOF.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut written = 0;
    while written < 4 {
        match io::Write::write(&mut channel, &b"0000"[written..]) {
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("request write failed: {error}"),
        }
        assert!(Instant::now() < deadline, "request write timed out");
    }
    while let Err(error) = channel.send_eof() {
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock, "{error}");
        assert!(Instant::now() < deadline, "EOF timed out");
        thread::sleep(Duration::from_millis(2));
    }
    let (mut stdout, mut stderr) = (false, false);
    let mut buffer = [0_u8; 4096];
    while !(stdout && stderr) {
        if !stdout {
            match channel.read(&mut buffer) {
                Ok(0) => stdout = true,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("stdout read failed: {error}"),
            }
        }
        if !stderr {
            match channel.read_stderr(&mut buffer) {
                Ok(0) => stderr = true,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("stderr read failed: {error}"),
            }
        }
        assert!(Instant::now() < deadline, "the outputs never ended");
        thread::sleep(Duration::from_millis(2));
    }
    // Both outputs are over; the server's process is "still running", so there is no exit status and no close.
    let held = Instant::now() + Duration::from_millis(1_000);
    while Instant::now() < held {
        let error = channel
            .finish()
            .expect_err("the stuck server closed the channel");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn only_the_fixtures_client_key_and_user_authenticate() {
    let mut server = ControlledSshd::start(Ending::Prompt);
    let stranger = server.temp.path().join("stranger");
    run_fixture_key(&stranger);
    let dial = |user: &str, key: std::path::PathBuf| {
        common::dialer(
            server.port(),
            server.port(),
            user.to_owned(),
            server.known_hosts().to_owned(),
            key,
        )()
    };
    assert!(
        dial(server.user(), stranger).is_err(),
        "an unknown key authenticated"
    );
    assert!(
        dial("someone-else", server.client_key()).is_err(),
        "an unknown user authenticated"
    );
    assert!(dial(server.user(), server.client_key()).is_ok());
    assert!(server.try_session().is_ok());
}

#[test]
fn many_clients_are_served_at_once() {
    let mut server = ControlledSshd::start(Ending::Prompt);
    let repository = server.repository.to_str().unwrap().to_owned();
    let mut channels = Vec::new();
    for _ in 0..8 {
        let mut channel =
            SshChannel::new(server.session(), GitService::UploadPack, &repository).unwrap();
        common::open(&mut channel);
        channels.push(channel);
    }
    for channel in &mut channels {
        let (stdout, _, status) = common::exchange(channel);
        assert!(stdout.windows(4).any(|window| window == b"0000"));
        assert_eq!(status, 0);
    }
    assert_eq!(server.authenticated_sessions, 8);
}

#[test]
fn dropping_the_server_stops_it_and_releases_its_port() {
    let server = ControlledSshd::start(Ending::Stuck);
    let port = server.port();
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
    drop(server);
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "the port still accepts"
    );
}
