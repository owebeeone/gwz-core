//! Actual trusted native password-only setup, through the endpoint attachment.
use super::attachment;
use crate::git::endpoint::{
    helper_script, https_auth, shared_reservation::Authority, ssh_channel::GitService, ssh_local,
    ssh_password_fixture::PasswordSshd, ssh_password_helpers::Helpers,
    ssh_worker::EndpointOpenFailure,
};
use gwz_transport::{
    pool::{Config, Key},
    protocol::{AuthMethod, ErrorCode, SetupFailureCause},
};
use std::{
    fs,
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum Fault {
    None,
    EndpointSlots,
    HostSlots,
    Cancel,
    Refused,
}

fn run(
    methods: &[&str],
    enabled: bool,
    delay: &str,
    interaction: u64,
) -> (
    Result<gwz_transport::protocol::Opened, gwz_transport::protocol::Failure>,
    Vec<String>,
    bool,
) {
    run_fault(methods, enabled, delay, interaction, Fault::None)
}

fn run_fault(
    methods: &[&str],
    enabled: bool,
    delay: &str,
    interaction: u64,
    fault: Fault,
) -> (
    Result<gwz_transport::protocol::Opened, gwz_transport::protocol::Failure>,
    Vec<String>,
    bool,
) {
    let temp = tempfile::tempdir().unwrap();
    let server = PasswordSshd::start(
        &temp.path().join("server"),
        "fixture-password",
        methods,
        &[],
    );
    let known = temp.path().join("known_hosts");
    fs::write(&known, server.known_host("127.0.0.1")).unwrap();
    let repo = temp.path().join("repository with space.git");
    git2::Repository::init_bare(&repo).unwrap();
    let marker = temp.path().join("helper-ran");
    let executable = temp.path().join("git");
    let password = if matches!(fault, Fault::Refused) {
        "wrong-fixture-password"
    } else {
        "fixture-password"
    };
    helper_script::write_git_fixture(
        &executable,
        &format!(
            "printf '%s' $$ > '{}'\n{delay}\nprintf 'username=git\\npassword={password}\\n'\n",
            marker.display()
        ),
    );
    let slots = https_auth::HelperSlots::new();
    let helpers = enabled.then(|| {
        Arc::new(Helpers::new(
            https_auth::Config {
                executable,
                environment: Vec::new(),
            },
            slots.clone(),
        ))
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let held = match fault {
        Fault::HostSlots => Some(runtime.block_on(slots.hold_all())),
        Fault::EndpointSlots => Some(runtime.block_on(helpers.as_ref().unwrap().hold_endpoint())),
        _ => None,
    };
    let config = Config {
        allocation_timeout_ms: 100,
        connect_timeout_ms: 250,
        interaction_timeout_ms: interaction,
        ..Config::default()
    };
    let endpoint = ssh_local::connect_with_helpers(
        config.clone(),
        known,
        None,
        2000,
        Authority::new(config.total, config.per_host),
        Default::default(),
        helpers.clone(),
    )
    .unwrap();
    let key = Key::ssh("git", "127.0.0.1", server.port);
    let result = if matches!(fault, Fault::Cancel) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let open = endpoint
            .start_endpoint_open(
                key,
                None,
                GitService::UploadPack,
                repo.to_str().unwrap(),
                attachment::context(attachment::deadlines(&config, 2000)),
                cancelled.clone(),
            )
            .unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        // The helper's shell creates the marker before it writes its PID into it, in
        // one write; a cancel that kills it between the two leaves an empty marker.
        while !fs::read_to_string(&marker).is_ok_and(|pid| !pid.is_empty()) {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        cancelled.store(true, Ordering::Release);
        attachment::finish(&open).map(|_| panic!("cancelled setup offered a password"))
    } else {
        attachment::open(
            &endpoint,
            key,
            None,
            GitService::UploadPack,
            repo.to_str().unwrap(),
            attachment::deadlines(&config, 2000),
        )
    };
    let result = match result {
        Ok((mut stream, opened)) => {
            stream.write_all(b"0000").unwrap();
            stream.end_write().unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
            stream.close().unwrap();
            Ok(opened)
        }
        Err(error) => Err(error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<EndpointOpenFailure>())
            .expect("typed failure")
            .failure
            .clone()),
    };
    endpoint.shutdown();
    let until = Instant::now() + Duration::from_secs(3);
    while !endpoint.shutdown_status().cleanup_complete {
        assert!(Instant::now() < until, "setup cleanup remains owned");
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(held);
    assert_eq!(slots.available(), 8);
    if let Some(helpers) = &helpers {
        assert_eq!(helpers.available(), 8);
    }
    if marker.exists() {
        let pid: i32 = fs::read_to_string(&marker).unwrap().parse().unwrap();
        // SAFETY: positive PID captured from this fixture's owned helper child.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "helper was not reaped");
    }
    (result, server.attempts(), marker.exists())
}

#[test]
fn password_only_helper_outlasts_original_connect_clock() {
    let (result, attempts, ran) = run(&["password"], true, "/bin/sleep 0.35", 1500);
    let opened = result.unwrap();
    assert!(ran);
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt == "password:accepted")
    );
    assert_eq!(opened.facts.method, AuthMethod::Gh);
}

#[test]
fn disabled_publickey_and_combined_offers_never_run_helpers() {
    for (methods, enabled) in [
        (&["password"][..], false),
        (&["publickey"][..], true),
        (&["password", "publickey"][..], true),
    ] {
        let (_, attempts, ran) = run(methods, enabled, "", 1500);
        assert!(!ran);
        assert!(
            !attempts
                .iter()
                .any(|attempt| attempt.starts_with("password:"))
        );
    }
}

#[test]
fn helper_expiry_has_exact_interaction_and_offers_no_password() {
    let (result, attempts, _ran) = run(&["password"], true, "/bin/sleep 30", 75);
    let failed = result.unwrap_err();
    assert_eq!(failed.code, ErrorCode::Timeout);
    assert_eq!(failed.setup_cause, Some(SetupFailureCause::Interaction));
    assert_eq!(failed.detail.unwrap().helper_budget_ms, Some(75));
    assert!(
        !attempts
            .iter()
            .any(|attempt| attempt.starts_with("password:"))
    );
}

#[test]
fn both_admission_limits_timeout_without_a_helper_or_password() {
    for fault in [Fault::EndpointSlots, Fault::HostSlots] {
        let (result, attempts, ran) = run_fault(&["password"], true, "", 1500, fault);
        let failed = result.unwrap_err();
        assert_eq!(failed.code, ErrorCode::Timeout);
        assert_eq!(failed.setup_cause, Some(SetupFailureCause::Allocation));
        let captured = failed.detail.unwrap().helper_budget_ms.unwrap();
        assert!((1..=100).contains(&captured));
        assert!(!ran);
        assert!(
            !attempts
                .iter()
                .any(|attempt| attempt.starts_with("password:"))
        );
    }
}

#[test]
fn cancellation_kills_reaps_and_never_offers_the_helper_answer() {
    let (result, attempts, ran) =
        run_fault(&["password"], true, "/bin/sleep 30", 1500, Fault::Cancel);
    assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    assert!(ran);
    assert!(
        !attempts
            .iter()
            .any(|attempt| attempt.starts_with("password:"))
    );
}

#[test]
fn helper_password_rejection_is_authentication_and_has_no_timeout_detail() {
    let (result, attempts, ran) = run_fault(&["password"], true, "", 1500, Fault::Refused);
    let failed = result.unwrap_err();
    assert_eq!(failed.code, ErrorCode::Authentication);
    assert!(failed.detail.is_none());
    assert!(ran);
    assert!(attempts.iter().any(|attempt| attempt == "password:refused"));
}
