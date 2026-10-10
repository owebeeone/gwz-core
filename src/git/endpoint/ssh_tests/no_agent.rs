//! A setup that needs an agent where the endpoint has none is refused with the no-agent refusal (S4.3): before any
//! credential is offered, on every platform. Windows has no agent source until Phase 3 of the Windows parity plan,
//! so every ambient open there is such a setup (plan step 1.4).
use super::attachment;
use crate::git::endpoint::{
    shared_reservation::Authority, ssh_channel::GitService, ssh_fixture::SshdFixture, ssh_local,
    ssh_worker::EndpointOpenFailure,
};
use gwz_transport::pool::{Config, Key};
use std::{
    thread,
    time::{Duration, Instant},
};

#[test]
fn an_ambient_open_with_no_agent_source_is_refused_before_any_credential_is_offered() {
    let fixture = SshdFixture::new();
    let config = Config {
        total: 1,
        per_host: 1,
        per_user_host: 1,
        ..Config::default()
    };
    let authority = Authority::new(config.total, config.per_host);
    let endpoint = ssh_local::connect_with_authority(
        config.clone(),
        fixture.known_hosts.clone(),
        None,
        3_000,
        authority,
        Default::default(),
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
        .as_ref()
        .expect("a setup failure carries its facts");
    assert!(!facts.credential_offered, "{facts:?}");
    assert_eq!(fixture.authenticated_sessions, 0);
    endpoint.shutdown();
    let until = Instant::now() + Duration::from_secs(5);
    while !endpoint.shutdown_status().cleanup_complete {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
}
