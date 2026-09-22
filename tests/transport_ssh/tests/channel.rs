mod common;

use common::*;
use std::io::{self, Read};

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
