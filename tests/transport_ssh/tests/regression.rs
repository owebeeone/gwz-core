mod common;

use common::*;
use std::io::{self, Read, Write};
use std::thread;
use std::time::{Duration, Instant};

fn drain_until_would_block(channel: &mut SshChannel) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stdout_blocked = false;
    let mut stderr_blocked = false;
    let mut drained = 0usize;
    while Instant::now() < deadline {
        if !stdout_blocked {
            let mut output = [0; 4096];
            match channel.read(&mut output) {
                Ok(0) => panic!("paused SSH peer ended stdout before disposal"),
                Ok(count) => drained += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => stdout_blocked = true,
                Err(error) => panic!("paused SSH stdout drain failed: {error}"),
            }
        }
        if !stderr_blocked {
            let mut output = [0; 4096];
            match channel.read_stderr(&mut output) {
                Ok(0) => panic!("paused SSH peer ended stderr before disposal"),
                Ok(count) => drained += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => stderr_blocked = true,
                Err(error) => panic!("paused SSH stderr drain failed: {error}"),
            }
        }
        assert!(drained <= 64 * 1024, "paused SSH output exceeded bound");
        if stdout_blocked && stderr_blocked {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("paused SSH output did not reach bounded WouldBlock state");
}

#[test]
fn flush_preserves_buffered_upload_pack_advertisement() {
    let mut fixture = SshdFixture::new();
    let expected = {
        let session = fixture.session();
        let repository = fixture.repository.to_str().unwrap().to_owned();
        let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
        open(&mut channel);
        let (stdout, stderr, status) = exchange(&mut channel);
        assert!(stderr.is_empty());
        assert_eq!(status, 0);
        stdout
    };
    let session = fixture.session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    let mut channel = SshChannel::new(session, GitService::UploadPack, &repository).unwrap();
    open(&mut channel);
    let deadline = Instant::now() + Duration::from_secs(5);
    let first = loop {
        let mut byte = [0; 1];
        match channel.read(&mut byte) {
            Ok(1) => break byte[0],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("initial upload-pack read failed: {error}"),
            Ok(count) => panic!("unexpected initial read count: {count}"),
        }
        assert!(
            Instant::now() < deadline,
            "initial upload-pack read timed out"
        );
        thread::sleep(Duration::from_millis(2));
    };
    loop {
        match channel.flush() {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("upload-pack flush failed: {error}"),
        }
        assert!(Instant::now() < deadline, "upload-pack flush timed out");
        thread::sleep(Duration::from_millis(2));
    }
    let mut advertisement = vec![first];
    while !advertisement.windows(4).any(|window| window == b"0000") {
        let mut output = [0; 4096];
        match channel.read(&mut output) {
            Ok(0) => panic!("upload-pack ended before its advertisement flush"),
            Ok(count) => {
                advertisement.extend_from_slice(&output[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("upload-pack advertisement read failed: {error}"),
        }
        assert!(
            advertisement.len() <= 64 * 1024,
            "advertisement exceeded bound"
        );
        assert!(
            Instant::now() < deadline,
            "upload-pack advertisement timed out"
        );
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(advertisement, expected);
    channel.abort();
}

#[test]
fn stalled_disposal_retains_then_forces_owned_connection() {
    let mut fixture = SshdFixture::new_debug();
    let session = fixture.debug_session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    let mut channel = SshChannel::new(session, GitService::ReceivePack, &repository).unwrap();
    open(&mut channel);
    let mut paused = pause_process_tree(fixture.child.id());
    drain_until_would_block(&mut channel);
    let first = channel.poll_dispose();
    assert_eq!(first.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert!(!channel.is_disposed());
    let second = channel.poll_dispose();
    assert_eq!(second.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert!(!channel.is_disposed());
    channel.force_dispose().unwrap();
    assert!(channel.is_disposed());
    channel.force_dispose().unwrap();
    assert!(channel.is_disposed());
    paused.resume();
}

#[test]
fn poll_dispose_completes_or_waits_without_releasing_session() {
    let mut fixture = SshdFixture::new();
    let session = fixture.session();
    let repository = fixture.repository.to_str().unwrap().to_owned();
    let mut channel = SshChannel::new(session, GitService::ReceivePack, &repository).unwrap();
    open(&mut channel);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match channel.poll_dispose() {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("poll_dispose failed: {error}"),
        }
        assert!(Instant::now() < deadline, "poll_dispose did not finish");
        thread::sleep(Duration::from_millis(2));
    }
    assert!(channel.is_disposed());
    channel = match channel.into_session() {
        Ok(_) => panic!("disposed channel yielded a reusable session"),
        Err(channel) => channel,
    };
    channel.force_dispose().unwrap();
    assert!(channel.is_disposed());
}
