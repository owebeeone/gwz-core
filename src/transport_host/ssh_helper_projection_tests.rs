//! Real operation projection from trusted password-only SSH helper failures.
use super::https_tests::{endpoint_home, meta, repository};
use super::*;
mod workspace;
mod enablement;
use crate::git::{
    GitBackend,
    endpoint::{helper_script, https_auth, ssh_password_fixture::PasswordSshd},
};

#[derive(Clone, Copy, Debug)]
enum Fault {
    MissingGit,
    UnexecutableGit,
    Interaction,
    Allocation,
}

#[test]
fn ssh_clone_fetch_and_push_keep_typed_helper_outcomes() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        for fault in [Fault::MissingGit, Fault::UnexecutableGit, Fault::Interaction, Fault::Allocation] {
            let root = tempfile::tempdir().unwrap();
            let (bare, _) = repository(root.path());
            let server = PasswordSshd::start(&root.path().join("server"), "private-answer-sentinel", &["password"], &[]);
            let home = endpoint_home(root.path());
            std::fs::write(home.join(".ssh/known_hosts"), server.known_host("127.0.0.1")).unwrap();
            let executable = root.path().join("git");
            if matches!(fault, Fault::UnexecutableGit) {
                std::fs::write(&executable, b"non executable fixture").unwrap();
            } else if !matches!(fault, Fault::MissingGit) {
                helper_script::write_git_fixture(&executable, "printf started > \"$HOME/started\"\nsleep 30\nprintf 'username=private-user-sentinel\\npassword=private-answer-sentinel\\n'\n");
            }
            let slots = HelperSlots::new();
            let held = if matches!(fault, Fault::Allocation) { Some(slots.hold_all().await) } else { None };
            let mut local = SshEndpointConfig::fixture(home, None);
            if !matches!(fault, Fault::MissingGit | Fault::UnexecutableGit) {
                local.pool.interaction_timeout_ms = 500;
                local.pool.allocation_timeout_ms = 1_250;
            }
            let runtime = TransportRuntime::with_https(local, HttpsEndpointConfig {
                tls: Default::default(), auth: Some(https_auth::Config { executable, environment: vec![("HOME".into(), root.path().as_os_str().into())] }),
            }, slots.clone()).unwrap();
            let url = format!("ssh://git@127.0.0.1:{}{}", server.port, bare.display());
            let work = root.path().join("work");
            let repo = git2::Repository::clone(bare.to_str().unwrap(), &work).unwrap();
            repo.remote_set_url("origin", &url).unwrap();
            drop(repo);
            for operation in ["clone", "fetch", "push"] {
                let mut request_meta = meta(operation);
                request_meta.policy = Some(crate::OperationPolicy { max_retries: Some(0), ..Default::default() });
                let request = runtime.request(request_meta, operation.into()).await.unwrap();
                let backend = request.backend().clone();
                let work = work.clone();
                let url = url.clone();
                let target = root.path().join("private-clone");
                let error = tokio::task::spawn_blocking(move || match operation {
                    "clone" => backend.clone_repo(&url, &target).map(|_| ()),
                    "fetch" => backend.fetch(&work, "origin").map(|_| ()),
                    _ => backend.push(&work, "origin", "refs/heads/main:refs/heads/main").map(|_| ()),
                }).await.unwrap().unwrap_err();
                let (code, prefix) = match fault {
                    Fault::MissingGit | Fault::UnexecutableGit => (ErrorCode::ExternalToolMissing, "SSH authentication needs `git` on PATH:"),
                    Fault::Interaction => (ErrorCode::CredentialHelperTimeout, "No credential helper answered within 0.5 seconds, a fixed bound"),
                    Fault::Allocation => (ErrorCode::CredentialHelperTimeout, "No credential helper could start in the "),
                };
                assert_eq!(error.code, code, "{fault:?}/{operation}: {}", error.message);
                assert!(error.message.starts_with(prefix), "{fault:?}/{operation}: {}", error.message);
                // setup_retry::Final only reports counts for retriable/multiple attempts.
                assert!(!error.message.contains("(attempt "));
                assert!(!error.message.contains("sentinel"));
                assert_eq!(request.finish().await.pending_local_work, 0);
            }
            let mut request_meta = meta("typed-budget");
            request_meta.policy = Some(crate::OperationPolicy { max_retries: Some(0), ..Default::default() });
            let request = runtime.request(request_meta, "clone".into()).await.unwrap();
            let context = request.context.clone();
            let typed_url = url.clone();
            let (failure, message) = tokio::task::spawn_blocking(move || {
                let error = match context.open(&typed_url, crate::git::endpoint::ssh_channel::GitService::UploadPack, None, Arc::new(|_, _| {}), Arc::new(|_| {})) { Err(error) => error, Ok(_) => panic!("faulted helper opened a stream"), };
                let typed = error.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap();
                (typed.0.clone(), typed.model_error(false).unwrap().message)
            }).await.unwrap();
            assert_eq!(failure.facts.as_ref().unwrap().method, gwz_transport::protocol::AuthMethod::Gh);
            if matches!(fault, Fault::Interaction | Fault::Allocation) {
                let captured = failure.detail.as_ref().unwrap().helper_budget_ms.unwrap();
                if matches!(fault, Fault::Interaction) { assert_eq!(captured, 500); }
                else { assert!((1..=1_250).contains(&captured)); }
                assert_eq!(message, super::helper_failure::timeout_reason(&failure).unwrap());
            } else if matches!(fault, Fault::UnexecutableGit) {
                assert!(message.contains("could not start the `git` it found"));
            } else { assert!(message.contains("found no `git`")); }
            assert_eq!(request.finish().await.pending_local_work, 0);
            workspace::check(&runtime, root.path(), &bare, &url, fault).await;
            assert!(!server.attempts().iter().any(|a| a.starts_with("password:")));
            assert_eq!(root.path().join("started").exists(), matches!(fault, Fault::Interaction));
            assert_eq!(runtime.shutdown().await.pending_local_work, 0);
            drop(held);
            assert_eq!(slots.available(), 8);
        }
    });
}

#[test]
fn ssh_helper_projection_requires_typed_provenance_and_preserves_non_helper_failures() {
    use gwz_transport::protocol::{
        AuthMethod, Effect, ErrorCode, Facts, Failure, FailureDetail, SetupFailureCause,
    };
    let timeout = |cause, budget| Failure {
        code: ErrorCode::Timeout,
        effect: Effect::None,
        setup_cause: Some(cause),
        detail: Some(Box::new(FailureDetail {
            helper_budget_ms: Some(budget),
            ..Default::default()
        })),
        facts: Some(Facts {
            method: AuthMethod::Gh,
            ..Default::default()
        }),
    };
    for (cause, budget, seconds) in [
        (SetupFailureCause::Interaction, 1_250, "1.25"),
        (SetupFailureCause::Allocation, 0, "0"),
        (SetupFailureCause::Allocation, 86_400_000, "86400"),
    ] {
        let failed = timeout(cause, budget);
        let common = super::helper_failure::timeout_reason(&failed).unwrap();
        let error = SshOpenFailure(failed, Some((2, 4)));
        let model = error.model_error(false).unwrap();
        assert_eq!(model.code, crate::model::ErrorCode::CredentialHelperTimeout);
        assert_eq!(model.message, format!("{common} (attempt 2 of 4)"));
        assert_eq!(error.to_string(), model.message);
        assert!(model.message.contains(&format!("{seconds} seconds")));
    }
    for cause in [
        SetupFailureCause::Allocation,
        SetupFailureCause::Interaction,
        SetupFailureCause::Stall,
        SetupFailureCause::Aggregate,
    ] {
        let failed = Failure {
            code: ErrorCode::Timeout,
            setup_cause: Some(cause),
            facts: Some(Facts {
                method: AuthMethod::Gh,
                ..Default::default()
            }),
            ..Default::default()
        };
        let error = SshOpenFailure(failed, Some((1, 2)));
        assert!(error.model_error(false).is_none());
        assert!(error.to_string().starts_with("ssh setup timeout:"));
        assert!(error.to_string().ends_with("(attempt 1 of 2)"));
    }
    for (cause, budget) in [
        (SetupFailureCause::Interaction, 0),
        (SetupFailureCause::Interaction, 120_001),
        (SetupFailureCause::Allocation, -1),
        (SetupFailureCause::Allocation, 86_400_001),
        (SetupFailureCause::Aggregate, 1),
    ] {
        assert!(
            SshOpenFailure(timeout(cause, budget), None)
                .model_error(false)
                .is_none()
        );
    }
    let ordinary = SshOpenFailure(
        Failure {
            code: ErrorCode::Unavailable,
            setup_cause: Some(SetupFailureCause::NotFound),
            facts: Some(Facts {
                method: AuthMethod::SshKey,
                ..Default::default()
            }),
            ..Default::default()
        },
        None,
    );
    assert!(ordinary.model_error(false).is_none());
    assert_eq!(ordinary.to_string(), "ssh setup failed: Unavailable");
    let auth = SshOpenFailure(
        Failure {
            code: ErrorCode::Authentication,
            ..Default::default()
        },
        None,
    );
    assert!(auth.model_error(false).is_none());
    assert!(
        std::error::Error::source(&auth)
            .unwrap()
            .is::<crate::git::endpoint::ssh_remote::AuthenticationRejected>()
    );
}
