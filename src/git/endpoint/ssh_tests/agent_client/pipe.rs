//! Plan step 3.3's rows, which run on Windows: the OpenSSH agent pipe (`agent_pipe`) as an agent channel, against
//! step 1.7's in-process pipe agent and, where a row needs a reply the fixture cannot script, a raw pipe server of its
//! own. They are the Windows twins of `mod native` (the real-socket rows) beside them, and they add the rows the pipe
//! has and a socket has not: a pipe that is busy, a pipe that is absent, the grammar's refusal never reaching a pipe, and
//! the connection's impersonation level.
use super::finish;
use crate::git::endpoint::{
    agent_job::Job,
    agent_pipe::{self, Retired},
    agent_source::{AgentFault, AgentSource, PipeName},
    ssh_tests::{
        agent_fixture_pipe::{Fault, Fixture, Request, pipe_name},
        agent_pipe_server::{RawServer, Seen},
    },
};
use std::{
    ffi::OsStr,
    io,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

fn pipe(path: &str) -> PipeName {
    PipeName::parse(OsStr::new(path)).unwrap()
}

fn source(fixture: &Fixture) -> AgentSource {
    AgentSource::NamedPipe(pipe(fixture.agent.name()))
}

#[test]
fn a_pipe_source_lists_identities_and_signs_with_a_reply_in_pieces() {
    let fixture = Fixture::new();
    let (agent, blobs) = (source(&fixture), fixture.blobs());
    // The reply to the next request, the listing, arrives as 1 byte, then 2, then the rest (P03).
    fixture.script(Fault::Chunked);
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        let mut agent = agent.open(control)?;
        let listed = agent.identities()?;
        let signed = agent.sign(&listed[1], b"data", 4)?;
        Ok((listed, signed))
    })
    .unwrap();
    let (listed, signed) = finish(&mut job).unwrap();
    assert_eq!(listed, blobs);
    assert!(signed.is_some_and(|signature| !signature.is_empty()));
    assert_eq!(
        fixture.requests(),
        [Request::List, Request::Sign { key: 1, flags: 4 }]
    );
}

#[test]
fn a_signature_reply_in_pieces_is_assembled() {
    let fixture = Fixture::new();
    let (agent, key) = (source(&fixture), fixture.blobs().remove(1));
    let mut job = Job::start_isolated(None, Duration::from_secs(1), {
        let agent = agent.clone();
        move |control| agent.open(control)?.identities()
    })
    .unwrap();
    assert_eq!(finish(&mut job).unwrap().len(), 2);
    assert!(fixture.wait_client_closed(Duration::from_secs(3)));
    fixture.script(Fault::Chunked);
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        agent.open(control)?.sign(&key, b"data", 4)
    })
    .unwrap();
    assert!(finish(&mut job).unwrap().is_some());
}

#[test]
fn stalled_agent_reply_spends_absolute_deadline() {
    let fixture = Fixture::new();
    let (agent, key) = (source(&fixture), fixture.blobs().remove(0));
    fixture.script(Fault::Stall);
    let deadline = Instant::now() + Duration::from_millis(200);
    let mut job = Job::start_isolated(Some(deadline), Duration::from_secs(1), move |control| {
        agent.open(control)?.sign(&key, b"data", 4)
    })
    .unwrap();
    assert_eq!(
        finish(&mut job).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(
        fixture.wait_client_closed(Duration::from_secs(3)),
        "the pipe must be closed once the job ends"
    );
}

#[test]
fn cancellation_interrupts_a_request_the_agent_never_answers_and_closes_the_pipe() {
    let fixture = Fixture::new();
    let (agent, key) = (source(&fixture), fixture.blobs().remove(0));
    fixture.script(Fault::Stall);
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        agent.open(control)?.sign(&key, b"data", 4)
    })
    .unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    while fixture.requests().is_empty() {
        assert!(
            Instant::now() < until,
            "the request never reached the agent"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    job.cancel();
    assert_eq!(
        finish(&mut job).unwrap_err().kind(),
        io::ErrorKind::ConnectionAborted
    );
    assert!(fixture.wait_client_closed(Duration::from_secs(3)));
    assert!(matches!(
        job.poll_disposed(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(()))
    ));
}

/// The twin of `cancellation_interrupts_every_partial_reply_and_closes_agent_socket`: whatever prefix of the reply
/// has arrived, cancelling ends the request, and the agent sees the client leave.
#[test]
fn cancellation_interrupts_every_partial_reply_and_closes_the_pipe() {
    for prefix in [vec![], vec![0, 0], vec![0, 0, 0, 5, 12]] {
        let temp = tempfile::tempdir().unwrap();
        let name = pipe_name(temp.path());
        let server = RawServer::start(&name, prefix);
        let agent = AgentSource::NamedPipe(pipe(&name));
        let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
            agent.open(control)?.identities()
        })
        .unwrap();
        server.wait_replied();
        job.cancel();
        assert_eq!(
            finish(&mut job).unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
        let seen = server.join();
        assert_eq!(seen.request, [0, 0, 0, 1, 11]);
        assert!(seen.client_left, "the agent must see the pipe closed");
        assert!(matches!(
            job.poll_disposed(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(()))
        ));
    }
}

/// P03: a cancelled request keeps its buffer, its `OVERLAPPED` and its event until the kernel says it is done with
/// them. The retire reports what it observed, and a second one finds nothing left.
#[test]
fn a_cancel_mid_read_completes_before_the_storage_is_released() {
    let fixture = Fixture::new();
    let (name, key) = (pipe(fixture.agent.name()), fixture.blobs().remove(0));
    fixture.script(Fault::Stall);
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        use crate::git::endpoint::agent_client::Channel;
        use std::io::{Read, Write};
        let mut pipe = agent_pipe::connect(&name, &control)?;
        // A sign request for a listed key, framed by hand, so that the agent stalls and a read is pending.
        let mut body = vec![13];
        body.extend_from_slice(&(key.len() as u32).to_be_bytes());
        body.extend_from_slice(&key);
        body.extend_from_slice(&4_u32.to_be_bytes());
        body.extend_from_slice(b"data");
        body.extend_from_slice(&4_u32.to_be_bytes());
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&body);
        let mut sent = 0;
        while sent < frame.len() {
            match pipe.write(&frame[sent..]) {
                Ok(n) => sent += n,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    pipe.wait(true, &control)?
                }
                Err(error) => return Err(error),
            }
        }
        let mut byte = [0; 1];
        let pending = pipe.read(&mut byte).unwrap_err().kind() == io::ErrorKind::WouldBlock;
        let first = pipe.cancel_pending();
        let second = pipe.cancel_pending();
        Ok((pending, first, second))
    })
    .unwrap();
    let (pending, first, second) = finish(&mut job).unwrap();
    assert!(pending, "the read must be pending while the agent stalls");
    assert_eq!(first, Retired::Observed);
    assert_eq!(second, Retired::Idle);
    assert!(fixture.wait_client_closed(Duration::from_secs(3)));
}

#[test]
fn a_pipe_that_vanishes_mid_request_poisons_the_channel() {
    let fixture = Fixture::new();
    let (agent, key) = (source(&fixture), fixture.blobs().remove(0));
    fixture.script(Fault::Vanish);
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        let mut agent = agent.open(control)?;
        let first = agent.sign(&key, b"data", 4).unwrap_err().kind();
        let second = agent.sign(&key, b"data", 4).unwrap_err().kind();
        Ok((first, second))
    })
    .unwrap();
    let (first, second) = finish(&mut job).unwrap();
    assert!(
        matches!(
            first,
            io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe
        ),
        "{first:?}"
    );
    assert_eq!(second, io::ErrorKind::BrokenPipe);
}

#[test]
fn a_reply_longer_than_one_read_is_assembled() {
    // 200 identities of 200-byte blobs: a frame of about 40 000 bytes, many times the size of one overlapped read.
    let mut reply = vec![12];
    reply.extend_from_slice(&200_u32.to_be_bytes());
    for at in 0..200_u32 {
        let blob = [at as u8; 200];
        reply.extend_from_slice(&(blob.len() as u32).to_be_bytes());
        reply.extend_from_slice(&blob);
        reply.extend_from_slice(&0_u32.to_be_bytes());
    }
    let mut frame = (reply.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&reply);
    let temp = tempfile::tempdir().unwrap();
    let name = pipe_name(temp.path());
    let server = RawServer::start(&name, frame);
    let agent = AgentSource::NamedPipe(pipe(&name));
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        agent.open(control)?.identities()
    })
    .unwrap();
    let listed = finish(&mut job).unwrap();
    assert_eq!(listed.len(), 200);
    assert!(
        listed
            .iter()
            .enumerate()
            .all(|(at, blob)| blob == &[at as u8; 200])
    );
    server.join();
}

/// The pipe is connected with `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION` (TR1.8 §4), which limits what a
/// server can do with the caller's token: the server sees an identification-level token and cannot act as the caller.
#[test]
fn the_pipe_is_opened_for_identification_only() {
    let temp = tempfile::tempdir().unwrap();
    let name = pipe_name(temp.path());
    let server = RawServer::start(&name, vec![0, 0, 0, 5, 12, 0, 0, 0, 0]);
    let agent = AgentSource::NamedPipe(pipe(&name));
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        agent.open(control)?.identities()
    })
    .unwrap();
    assert!(finish(&mut job).unwrap().is_empty());
    let Seen { level, .. } = server.join();
    // SecurityIdentification
    assert_eq!(level, Some(1));
}

#[test]
fn a_busy_pipe_is_waited_for_inside_control_and_taken_when_it_is_free() {
    let fixture = Fixture::new();
    let agent = source(&fixture);
    // The agent serves one client at a time: this one keeps the only instance.
    let holder = fixture.connect();
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        agent.open(control)?.identities()
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        matches!(
            job.poll_result(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ),
        "the connect must wait for the busy pipe, not fail"
    );
    drop(holder);
    assert_eq!(finish(&mut job).unwrap().len(), 2);
}

#[test]
fn a_busy_pipe_wait_ends_at_the_deadline_and_at_the_callers_cancel() {
    let fixture = Fixture::new();
    let holder = fixture.connect();
    let agent = source(&fixture);
    let deadline = Instant::now() + Duration::from_millis(250);
    let started = Instant::now();
    let mut job = Job::start_isolated(Some(deadline), Duration::from_secs(1), {
        let agent = agent.clone();
        move |control| agent.open(control).map(|_| ())
    })
    .unwrap();
    assert_eq!(
        finish(&mut job).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "libssh2's fixed second must not decide the wait"
    );
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        agent.open(control).map(|_| ())
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    job.cancel();
    assert_eq!(
        finish(&mut job).unwrap_err().kind(),
        io::ErrorKind::ConnectionAborted
    );
    drop(holder);
}

#[test]
fn an_absent_pipe_is_the_agent_not_running_and_the_error_names_no_pipe() {
    let temp = tempfile::tempdir().unwrap();
    let absent = pipe_name(temp.path());
    let agent = AgentSource::NamedPipe(pipe(&absent));
    let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
        agent.open(control).map(|_| ())
    })
    .unwrap();
    let error = finish(&mut job).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(AgentFault::of_error(&error), Some(AgentFault::NotRunning));
    assert!(!error.to_string().contains("gwz-agent-fixture"), "{error}");
}

/// A refused form never reaches a pipe, even when a working agent is serving: nothing connects, so nothing falls back.
#[test]
fn a_refused_source_and_a_pageant_source_never_reach_a_working_pipe() {
    let fixture = Fixture::new();
    let refused = match crate::git::endpoint::agent_source::select(
        Some(OsStr::new(r"\\localhost\pipe\x")),
        || false,
    ) {
        AgentSource::Refused(refusal) => AgentSource::Refused(refusal),
        other => panic!("{other:?}"),
    };
    for source in [refused, AgentSource::Pageant { sock_ignored: true }] {
        let mut job = Job::start_isolated(None, Duration::from_secs(1), move |control| {
            source.open(control).map(|_| ())
        })
        .unwrap();
        let error = finish(&mut job).unwrap_err();
        assert!(AgentFault::of_error(&error).is_some(), "{error}");
    }
    assert!(fixture.requests().is_empty());
    assert!(
        !fixture.wait_client_closed(Duration::from_millis(100)),
        "no client may have connected to the working agent"
    );
}
