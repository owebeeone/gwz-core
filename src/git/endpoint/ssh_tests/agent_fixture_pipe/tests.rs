//! The pipe agent's own rows (GwzTransportWindowsParityPlan.md, step 1.7): it lists its keys, a sign verifies
//! against the key's public part, and each scripted fault occurs as scripted.
use super::*;
use crate::git::endpoint::{
    agent_client::{Agent, Channel},
    agent_job::{Control, Job},
};
use std::{
    io::{self, Read, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

const DATA: &[u8] = b"data the client wants signed";

/// Runs `body` as the supervised job the agent client runs in, and returns its result.
fn within<T: Send + 'static>(
    stall: Duration,
    body: impl FnOnce(Arc<Control>) -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    let mut job =
        Job::start_isolated(Some(Instant::now() + Duration::from_secs(10)), stall, body).unwrap();
    let until = Instant::now() + Duration::from_secs(12);
    loop {
        if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
            return result;
        }
        assert!(Instant::now() < until, "the job did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn the_agent_lists_its_keys_in_order() {
    let fixture = Fixture::new();
    let blobs = fixture.blobs();
    assert_eq!(blobs.len(), 2);
    assert_ne!(blobs[0], blobs[1]);
    let listed = within(Duration::from_secs(1), {
        let channel = fixture.connect();
        move |control| Agent::new(channel, control).identities()
    })
    .unwrap();
    assert_eq!(listed, blobs);
    assert_eq!(fixture.requests(), vec![Request::List]);
}

#[test]
fn a_signature_verifies_against_the_keys_public_part_and_not_against_other_data() {
    let fixture = Fixture::new();
    let blobs = fixture.blobs();
    for flags in [2_u32, 4] {
        let signature = within(Duration::from_secs(1), {
            let (channel, key) = (fixture.connect(), blobs[1].clone());
            move |control| Agent::new(channel, control).sign(&key, DATA, flags)
        })
        .unwrap()
        .expect("the agent signs for rsa-sha2");
        assert!(
            fixture.identity(1).verify(flags, DATA, &signature),
            "flags {flags}"
        );
        assert!(!fixture.identity(1).verify(flags, b"other data", &signature));
        assert!(
            !fixture.identity(0).verify(flags, DATA, &signature),
            "the other key verified it"
        );
    }
    assert_eq!(
        fixture.requests(),
        vec![
            Request::Sign { key: 1, flags: 2 },
            Request::Sign { key: 1, flags: 4 }
        ]
    );
}

#[test]
fn a_sha1_request_is_refused_and_leaves_the_connection_usable() {
    let fixture = Fixture::new();
    let key = fixture.blobs()[1].clone();
    let channel = fixture.connect();
    within(Duration::from_secs(1), move |control| {
        let mut agent = Agent::new(channel, control);
        assert_eq!(agent.sign(&key, DATA, 0)?, None);
        assert!(agent.sign(&key, DATA, 2)?.is_some());
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_chunked_reply_arrives_one_byte_then_two_then_the_rest() {
    let fixture = Fixture::new();
    fixture.script(Fault::Chunked);
    let mut channel = fixture.connect();
    channel.write_all(&[0, 0, 0, 1, 11]).unwrap();
    let mut first = [0_u8; 64];
    let first = read_when_ready(&mut channel, &mut first);
    assert_eq!(first, 1, "the first piece is one byte");
    let mut second = [0_u8; 64];
    let second = read_when_ready(&mut channel, &mut second);
    assert_eq!(second, 2, "the second piece is two bytes");
}

#[test]
fn the_client_reassembles_a_chunked_reply() {
    let fixture = Fixture::new();
    fixture.script(Fault::Chunked);
    let listed = within(Duration::from_secs(2), {
        let channel = fixture.connect();
        move |control| Agent::new(channel, control).identities()
    })
    .unwrap();
    assert_eq!(listed, fixture.blobs());
}

#[test]
fn a_stalled_sign_is_never_answered_and_the_agent_sees_the_client_leave() {
    let fixture = Fixture::new();
    fixture.script(Fault::Stall);
    let key = fixture.blobs()[1].clone();
    let channel = fixture.connect();
    let started = Instant::now();
    let error = within(Duration::from_millis(400), move |control| {
        Agent::new(channel, control).sign(&key, DATA, 4)
    })
    .expect_err("a stalled agent never answers");
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "{error:?} after {:?}",
        started.elapsed()
    );
    assert_eq!(fixture.requests(), vec![Request::Sign { key: 1, flags: 4 }]);
    assert!(
        fixture.wait_client_closed(Duration::from_secs(3)),
        "the agent did not see the client leave"
    );
}

#[test]
fn a_vanishing_pipe_ends_the_exchange_and_the_agent_serves_the_next_client() {
    let fixture = Fixture::new();
    fixture.script(Fault::Vanish);
    let key = fixture.blobs()[1].clone();
    let channel = fixture.connect();
    let error = within(Duration::from_secs(1), move |control| {
        Agent::new(channel, control).sign(&key, DATA, 4)
    })
    .expect_err("the pipe vanished");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    let listed = within(Duration::from_secs(1), {
        let channel = fixture.connect();
        move |control| Agent::new(channel, control).identities()
    })
    .unwrap();
    assert_eq!(listed, fixture.blobs());
}

/// The signature blob's two strings: the algorithm name and the signature.
fn parts(blob: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let take = |input: &[u8]| {
        let length = u32::from_be_bytes(input[..4].try_into().unwrap()) as usize;
        (input[4..4 + length].to_vec(), 4 + length)
    };
    let (name, used) = take(blob);
    let (signature, rest) = take(&blob[used..]);
    assert_eq!(used + rest, blob.len(), "bytes follow the signature");
    (name, signature)
}

#[test]
fn each_damage_comes_as_scripted_and_only_once() {
    let fixture = Fixture::new();
    let key = fixture.blobs()[1].clone();
    let sign = |fixture: &Fixture, flags: u32| {
        within(Duration::from_secs(1), {
            let (channel, key) = (fixture.connect(), key.clone());
            move |control| Agent::new(channel, control).sign(&key, DATA, flags)
        })
        .unwrap()
        .unwrap()
    };
    let (name, signature) = parts(&{
        fixture.script(Fault::Damage(Damage::ShortSignature));
        sign(&fixture, 4)
    });
    assert_eq!(
        (name.as_slice(), signature.len()),
        (b"rsa-sha2-512".as_slice(), 1)
    );
    fixture.script(Fault::Damage(Damage::WrongAlgorithm));
    let (name, _) = parts(&sign(&fixture, 4));
    assert_eq!(name, b"Xsa-sha2-512");
    fixture.script(Fault::Damage(Damage::OneByteRsa));
    let (name, signature) = parts(&sign(&fixture, 4));
    assert_eq!(
        (name.as_slice(), signature.len()),
        (b"ssh-rsa".as_slice(), 1)
    );
    fixture.script(Fault::Damage(Damage::OversizeRsa));
    let (name, signature) = parts(&sign(&fixture, 4));
    assert_eq!(
        (name.as_slice(), signature.len()),
        (b"ssh-rsa".as_slice(), 257)
    );
    let intact = sign(&fixture, 4);
    assert!(
        fixture.identity(1).verify(4, DATA, &intact),
        "a damage outlived its request"
    );
}

#[test]
fn a_second_agent_on_the_same_pipe_name_is_refused_and_the_first_keeps_serving() {
    let fixture = Fixture::new();
    let error = PipeAgent::start(fixture.agent.name(), Vec::new())
        .err()
        .expect("the name is taken");
    // ERROR_ACCESS_DENIED or, with the one instance in use, ERROR_PIPE_BUSY.
    assert!(matches!(error.raw_os_error(), Some(5 | 231)), "{error:?}");
    let listed = within(Duration::from_secs(1), {
        let channel = fixture.connect();
        move |control| Agent::new(channel, control).identities()
    })
    .unwrap();
    assert_eq!(listed.len(), 2);
}

#[test]
fn dropping_the_agent_frees_its_pipe_and_ends_its_thread() {
    let fixture = Fixture::new();
    let name = fixture.agent.name().to_owned();
    drop(fixture);
    assert_eq!(
        PipeChannel::open(&name)
            .err()
            .expect("the pipe is gone")
            .kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn a_start_that_fails_after_the_pipe_exists_leaves_no_pipe_behind() {
    let dir = tempfile::tempdir().unwrap();
    let name = pipe_name(dir.path());
    let failed = catch_unwind(AssertUnwindSafe(|| {
        PipeAgent::start_observing(&name, Vec::new(), |stage| {
            assert!(stage != Stage::PipeCreated, "injected startup failure");
        })
    }));
    assert!(failed.is_err());
    assert_eq!(
        PipeChannel::open(&name)
            .err()
            .expect("the pipe is gone")
            .kind(),
        io::ErrorKind::NotFound
    );
}

/// Reads once the channel has bytes, and returns how many came.
fn read_when_ready(channel: &mut PipeChannel, buffer: &mut [u8]) -> usize {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match channel.read(buffer) {
            Ok(count) => return count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < until, "no bytes came");
                std::thread::sleep(Duration::from_micros(500));
            }
            Err(error) => panic!("read failed: {error}"),
        }
    }
}

#[test]
fn the_channel_is_a_channel() {
    fn is_channel<C: Channel>() {}
    is_channel::<PipeChannel>();
}
