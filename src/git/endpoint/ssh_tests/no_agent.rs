//! A setup that needs an agent where the endpoint has none is refused with the no-agent refusal (S4.3): before any
//! credential is offered, on every platform (plan step 1.4). A source that is refused or cannot be used ends the same
//! way, and says that it was the agent step (plan step 3.2b), so that the host can word the failure.
use super::attachment;
use crate::git::endpoint::{
    agent_source::AgentSource, shared_reservation::Authority, ssh_channel::GitService,
    ssh_fixture::SshdFixture, ssh_local, ssh_worker::EndpointOpenFailure,
};
use gwz_transport::{
    pool::{Config, Key},
    protocol::{AuthMethod, ErrorCode, Facts, Failure},
};
use std::{
    thread,
    time::{Duration, Instant},
};

/// An ambient open against the fixture server that needs an agent and has `agent`: the facts of the setup failure
/// it ends with, and the failure's code.
fn ambient_open_refused(agent: AgentSource) -> (Facts, Failure) {
    let fixture = SshdFixture::new();
    let config = Config {
        total: 1,
        per_host: 1,
        per_user_host: 1,
        ..Config::default()
    };
    let authority = Authority::new(config.total, config.per_host);
    let endpoint = ssh_local::connect_with_helpers(
        config.clone(),
        fixture.known_hosts.clone(),
        agent,
        3_000,
        authority,
        Default::default(),
        None,
    )
    .unwrap();
    let path = fixture.repository.to_str().unwrap().to_owned();
    let error = attachment::open(
        &endpoint,
        Key::ssh(&fixture.user, "127.0.0.1", fixture.port),
        None,
        GitService::UploadPack,
        &path,
        attachment::deadlines(&config, 3_000),
    )
    .err()
    .expect("an open that needs an agent has none to use");
    let failure = error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<EndpointOpenFailure>())
        .expect("the bridge keeps the typed setup failure");
    let facts = failure
        .failure
        .facts
        .clone()
        .expect("a setup failure carries its facts");
    let ended = failure.failure.clone();
    assert!(!facts.credential_offered, "{facts:?}");
    assert_eq!(fixture.authenticated_sessions, 0);
    endpoint.shutdown();
    let until = Instant::now() + Duration::from_secs(5);
    while !endpoint.shutdown_status().cleanup_complete {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    (facts, ended)
}

#[test]
fn an_ambient_open_with_no_agent_source_is_refused_before_any_credential_is_offered() {
    let (facts, ended) = ambient_open_refused(AgentSource::None);
    assert_eq!(ended.code, ErrorCode::Unavailable, "{ended:?}");
    // Nothing says an agent was meant: the refusal keeps its generic words.
    assert_ne!(facts.method, AuthMethod::SshAgent, "{facts:?}");
}

/// A source that the platform refused or has no channel for still ends the setup before any credential, and the
/// failure says it was the agent step, so the host can word it from the source it chose (plan step 3.2b).
#[test]
fn a_refused_or_unusable_agent_source_fails_the_agent_step_before_any_credential() {
    for source in [
        AgentSource::Refused(refusal("/tmp/ssh-abc/agent.1")),
        AgentSource::Pageant { sock_ignored: true },
    ] {
        let (facts, ended) = ambient_open_refused(source.clone());
        assert_eq!(ended.code, ErrorCode::Unavailable, "{source:?}: {ended:?}");
        assert_eq!(facts.method, AuthMethod::SshAgent, "{source:?}: {facts:?}");
    }
}

fn refusal(value: &str) -> crate::git::endpoint::agent_source::AgentRefusal {
    match crate::git::endpoint::agent_source::select(Some(std::ffi::OsStr::new(value)), || false) {
        AgentSource::Refused(refusal) => refusal,
        other => panic!("{value}: {other:?}"),
    }
}
