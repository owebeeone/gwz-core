use crate::git::endpoint::ssh_fixture as common;

use crate::git::endpoint::ssh_close_fixture::{delayed_eof_fixture, stuck_close_fixture};
use common::*;
use std::io::{self, Read, Write};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn receive_pack_reuses_one_authenticated_connection_and_quotes_repository() {
    let mut fixture = SshdFixture::new();
    let mut session = fixture.session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    for _ in 0..2 {
        let mut channel = SshChannel::new(session, GitService::ReceivePack, &repository).unwrap();
        open(&mut channel);
        let (stdout, stderr, status) = exchange(&mut channel);
        assert!(stdout.windows(4).any(|window| window == b"0000"));
        assert!(
            stderr.is_empty(),
            "unexpected receive-pack diagnostics: {stderr:?}"
        );
        assert_eq!(status, 0);
        session = match channel.into_session() {
            Ok(session) => session,
            Err(_) => panic!("receive-pack channel was not fully cleaned up"),
        };
    }
    assert_eq!(fixture.authenticated_sessions, 1);
    assert!(
        !fixture.marker.exists(),
        "repository path was shell-injected"
    );
}

#[test]
fn upload_pack_advertisement_and_abort_keep_cleanup_explicit() {
    let mut fixture = SshdFixture::new();
    let session = fixture.session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
    open(&mut channel);
    channel = match channel.into_session() {
        Ok(_) => panic!("open channel yielded a reusable session"),
        Err(channel) => channel,
    };
    let (stdout, stderr, status) = exchange(&mut channel);
    assert!(stdout.windows(4).any(|window| window == b"0000"));
    assert!(
        stderr.is_empty(),
        "unexpected upload-pack diagnostics: {stderr:?}"
    );
    assert_eq!(status, 0);
    let session = match channel.into_session() {
        Ok(session) => session,
        Err(_) => panic!("finished upload-pack did not release its session"),
    };
    let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
    open(&mut channel);
    channel.abort();
    let mut output = [0; 1];
    let error = channel.read(&mut output).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert!(
        channel.into_session().is_err(),
        "aborted channel must never yield a session"
    );
}

/// Writes the client's whole request and its EOF, reading nothing.
fn send_request_and_eof(channel: &mut SshChannel) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut written = 0;
    while written < 4 {
        match channel.write(&b"0000"[written..]) {
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("SSH request write failed: {error}"),
        }
        assert!(Instant::now() < deadline, "SSH request write timed out");
    }
    loop {
        match channel.send_eof() {
            Ok(()) => return,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("SSH EOF failed: {error}"),
        }
        assert!(Instant::now() < deadline, "SSH EOF timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Closes early until the server's CHANNEL_CLOSE has arrived. The status is
/// the server's, which a client's close may have stopped: it is not checked.
fn finish_early(channel: &mut SshChannel) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match channel.finish_early() {
            Ok(status) => return status,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("early close failed: {error}"),
        }
        assert!(Instant::now() < deadline, "early close timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn closing_early_leaves_the_connection_clean_for_the_next_command() {
    let mut fixture = SshdFixture::new();
    let mut session = fixture.session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    for _ in 0..2 {
        let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
        open(&mut channel);
        send_request_and_eof(&mut channel);
        finish_early(&mut channel);
        session = match channel.into_session() {
            Ok(session) => session,
            Err(_) => panic!("an early-closed channel did not release its session"),
        };
    }
    // A command that runs to the server's EOF still works on the same connection.
    let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
    open(&mut channel);
    let (stdout, _, status) = exchange(&mut channel);
    assert!(stdout.windows(4).any(|window| window == b"0000"));
    assert_eq!(status, 0);
    assert!(channel.into_session().is_ok());
    assert_eq!(fixture.authenticated_sessions, 1);
}

#[test]
fn an_early_close_waits_only_for_the_servers_close() {
    let delay = Duration::from_millis(1500);
    let mut fixture = delayed_eof_fixture(delay);
    let session = fixture.session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
    open(&mut channel);
    send_request_and_eof(&mut channel);
    let started = Instant::now();
    // The server is holding its output open: only its close is awaited, and
    // that comes with its exit, `delay` after the service ended.
    finish_early(&mut channel);
    assert!(
        started.elapsed() >= delay - Duration::from_millis(500),
        "the close finished before the server's exit: {:?}",
        started.elapsed()
    );
    assert!(channel.into_session().is_ok());
}

#[test]
fn disposing_a_closing_channel_does_not_wait_for_the_server() {
    let mut fixture = stuck_close_fixture();
    let session = fixture.session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    let mut channel = SshChannel::new(session, GitService::ReceivePack, &repository).unwrap();
    open(&mut channel);
    send_request_and_eof(&mut channel);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stdout = [0_u8; 4096];
    let (mut stdout_eof, mut stderr_eof) = (false, false);
    while !(stdout_eof && stderr_eof) {
        if !stdout_eof {
            match channel.read(&mut stdout) {
                Ok(0) => stdout_eof = true,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("stdout read failed: {error}"),
            }
        }
        if !stderr_eof {
            match channel.read_stderr(&mut stdout) {
                Ok(0) => stderr_eof = true,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("stderr read failed: {error}"),
            }
        }
        assert!(Instant::now() < deadline, "the outputs never ended");
        thread::sleep(Duration::from_millis(2));
    }
    // Both outputs have ended and the server's process has not exited: the
    // close waits for its CHANNEL_CLOSE.
    let waiting = Instant::now() + Duration::from_millis(100);
    while Instant::now() < waiting {
        let error = channel.finish().expect_err("the server has not closed");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        thread::sleep(Duration::from_millis(5));
    }
    // A second libssh2 close would wait for a packet the server will not send.
    let started = Instant::now();
    channel
        .poll_dispose()
        .expect("a closing channel is terminated, not closed twice");
    assert!(channel.is_disposed());
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn hosted_git_command_grammar_and_option_safety() {
    for (service, executable) in [
        (GitService::UploadPack, "git-upload-pack"),
        (GitService::ReceivePack, "git-receive-pack"),
    ] {
        assert_eq!(
            service.command("owner/repo.git").unwrap(),
            format!("{executable} 'owner/repo.git'")
        );
        for operand in ["--help", "-repo", ""] {
            assert_eq!(
                service.command(operand).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}
