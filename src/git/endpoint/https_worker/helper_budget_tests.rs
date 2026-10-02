use super::*;

fn input() -> Input {
    Input {
        destination: "https://example.com/repo".into(),
        service: GitService::UploadPackAdvertisement,
        policy: AuthPolicy::Gh,
        session: "helper-budget".into(),
        operation: "operation".into(),
    }
}

fn endpoint(config: https_auth::Config) -> Endpoint {
    Endpoint::new(https_connection::Config::default(), Some(config), pool::Config::default()).unwrap()
}

fn assert_timing(failed: &Failure, cause: SetupFailureCause, milliseconds: i64) {
    assert_eq!(failed.code, ErrorCode::Timeout);
    assert_eq!(failed.effect, Effect::None);
    assert_eq!(failed.setup_cause, Some(cause));
    assert_eq!(failed.detail.as_ref().unwrap().helper_budget_ms, Some(milliseconds));
    assert!(failed.detail.as_ref().unwrap().helper_cause.is_none());
    assert_eq!(gwz_transport::codec::admit(&Envelope {
        version: 2,
        session_id: "helper-budget".into(),
        stream_id: 1,
        kind: MessageKind::OpenFailed,
        open_failed: Some(failed.clone()),
        ..Default::default()
    }), Ok(()));
}

#[tokio::test]
async fn zero_and_submillisecond_allocation_with_free_slots_has_truthful_provenance() {
    let endpoint = endpoint(https_auth::Config {
        executable: "/missing-fixture-git".into(),
        environment: Vec::new(),
    });
    for remaining in [Duration::ZERO, Duration::from_nanos(999_999)] {
        let mut budget = endpoint.client.budget();
        budget.allocation = remaining;
        let (result, _) = endpoint.client.prepare_attempt(input(), &CancellationToken::new(), &mut budget, &mut None).await;
        assert_timing(&result.err().unwrap(), SetupFailureCause::Allocation, 0);
        assert_eq!(endpoint.client.helpers.available_permits(), 8);
        assert_eq!(endpoint.client.auth_owner.active_count(), 0);
        assert_eq!(endpoint.client.auth_owner.pending_cleanup_count(), 0);
    }
}

#[tokio::test]
async fn endpoint_helper_saturation_reports_original_captured_allocation() {
    let endpoint = endpoint(https_auth::Config {
        executable: "/missing-fixture-git".into(),
        environment: Vec::new(),
    });
    let held = endpoint.client.helpers.clone().acquire_many_owned(8).await.unwrap();
    let mut budget = endpoint.client.budget();
    budget.allocation = Duration::from_millis(75);
    let (result, _) = endpoint.client.prepare_attempt(input(), &CancellationToken::new(), &mut budget, &mut None).await;
    let failed = result.err().unwrap();
    let captured = failed.detail.as_ref().unwrap().helper_budget_ms.unwrap();
    assert!((1..=75).contains(&captured));
    assert_timing(&failed, SetupFailureCause::Allocation, captured);
    assert_eq!(endpoint.client.auth_owner.active_count(), 0);
    drop(held);
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[tokio::test]
        async fn started_helper_timeout_reports_full_effective_interaction() {
            let home = tempfile::tempdir().unwrap();
            let executable = home.path().join("git");
            crate::git::endpoint::helper_script::write_git_fixture(&executable, "/bin/sleep 1\n");
            let endpoint = endpoint(https_auth::Config { executable, environment: Vec::new() });
            let mut budget = endpoint.client.budget();
            budget.helper = Duration::from_millis(25);
            let (result, _) = endpoint.client.prepare_attempt(input(), &CancellationToken::new(), &mut budget, &mut None).await;
            assert_timing(&result.err().unwrap(), SetupFailureCause::Interaction, 25);
            assert_eq!(endpoint.client.auth_owner.pending_cleanup_count(), 0);
        }
    }
}
