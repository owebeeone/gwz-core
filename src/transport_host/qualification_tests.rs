//! Exercises the real request constructor and endpoint with no SSH settings.
use super::*;
use crate::git::CredentialHelperPolicy;

fn meta(id: &str) -> RequestMeta {
    RequestMeta {
        schema_version: "gwz.protocol/v0".into(),
        request_id: id.into(),
        ..Default::default()
    }
}

#[test]
fn https_only_request_retains_context_and_selects_the_builds_actual_backend() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let settings = EndpointSettings {
            ssh: None,
            pool: pool::Config::default(),
            io_timeout_ms: 3000,
        };
        let runtime = TransportRuntime::build_native(
            settings,
            Some((
                HttpsEndpointConfig {
                    tls: Default::default(),
                    auth: None,
                },
                HelperSlots::new(),
            )),
            None,
        )
        .unwrap();
        let request = runtime
            .request(meta("qualification-backend"), "operation".into())
            .await
            .unwrap();
        let backend = request.backend();
        let expected = if cfg!(all(
            windows,
            gwz_transport_candidate,
            gwz_windows_https_qualification
        )) {
            CredentialHelperPolicy::Disabled
        } else {
            CredentialHelperPolicy::AllowConfigured
        };
        assert_eq!(backend.credential_helpers, expected);
        let mapped = backend
            .ssh
            .https_policy_for_test(backend.credential_helpers);
        let expected_policy = if cfg!(all(
            windows,
            gwz_transport_candidate,
            gwz_windows_https_qualification
        )) {
            Some(AuthPolicy::WindowsDefault)
        } else {
            None
        };
        assert_eq!(mapped, expected_policy);
        let context = backend.ssh.host_context().expect("actual request context");
        context
            .validate(&meta("qualification-backend"), "operation")
            .unwrap();
        assert!(
            context
                .validate(&meta("different-request"), "operation")
                .is_err()
        );
        assert_eq!(
            Git2Backend::new().credential_helpers,
            CredentialHelperPolicy::AllowConfigured
        );
        assert!(Git2Backend::new().ssh.host_context().is_none());
        let endpoint = runtime.0.lock().unwrap().local_endpoint.clone();
        assert!(!endpoint.endpoint_offer_for_test().0);
        let refused = endpoint
            .open(
                "qualification-backend",
                "operation",
                "ssh://git@example.invalid/repo",
                crate::git::endpoint::ssh_channel::GitService::UploadPack,
                Default::default(),
                true,
                Arc::new(|_, _| {}),
                Arc::new(|_| {}),
            )
            .err()
            .unwrap();
        assert_eq!(refused.kind(), std::io::ErrorKind::Unsupported);
        request.finish().await;
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

cfg_if::cfg_if! { if #[cfg(all(windows, gwz_transport_candidate, gwz_windows_https_qualification))] {
    #[test]
    fn qualification_capabilities_and_forged_opens_agree_before_effects() {
        let executor = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        executor.block_on(async {
            let settings = EndpointSettings { ssh: None, pool: pool::Config::default(), io_timeout_ms: 3000 };
            let runtime = TransportRuntime::build_native(settings, Some((HttpsEndpointConfig { tls: Default::default(), auth: None }, HelperSlots::new())), None).unwrap();
            let caps = runtime.capabilities(TransportCapabilitiesRequest { schema_version: "gwz.protocol/v0".into(), ..Default::default() }).unwrap();
            assert_eq!(caps.schemes, Some(vec![Scheme::Ssh, Scheme::Https]));
            assert_eq!(caps.auth_policies, Some(vec![AuthPolicy::SshAmbient, AuthPolicy::SshExplicit, AuthPolicy::Anonymous, AuthPolicy::WindowsDefault]));
            assert!(!caps.file_identity);
            assert!(!caps.exact_agent_identity);
            let request = runtime.request(meta("qualification-refusal"), "operation".into()).await.unwrap();
            let endpoint = runtime.0.lock().unwrap().local_endpoint.clone();
            let (_, bound) = endpoint.endpoint_offer_for_test();
            assert_eq!(Some(bound.schemes.clone()), caps.schemes);
            assert_eq!(Some(bound.policies.clone()), caps.auth_policies);
            for policy in [AuthPolicy::Gh, AuthPolicy::WindowsConfigured, AuthPolicy::SshAmbient, AuthPolicy::SshExplicit] {
                let failure = endpoint.open_https("qualification-refusal", "operation", "https://example.invalid/repo", gwz_transport::protocol::GitService::UploadPackAdvertisement, policy, None, None, Arc::new(|_, _| {}), Arc::new(|_| {})).err().unwrap();
                assert_eq!(failure.code, gwz_transport::protocol::ErrorCode::UnsupportedOperation);
                assert_eq!(failure.effect, gwz_transport::protocol::Effect::None);
            }
            assert_eq!(endpoint.https_counts_for_test().unwrap().total(), 0);
            use crate::git::GitBackend;
            let fixture = tempfile::tempdir().unwrap();
            let target = fixture.path().join("no-clone");
            // SSH is no longer refused by the qualification boundary (plan step 1.6); remotes that are neither SSH nor HTTPS still are.
            request.backend().validate_url_identity(None, "origin", "ssh://git@example.invalid/repo").unwrap();
            request.backend().validate_url_identity(None, "origin", "git@example.invalid:repo").unwrap();
            for url in ["http://example.invalid/repo", "file:///unavailable/repo"] {
                let refused = request.backend().validate_url_identity(None, "origin", url).unwrap_err();
                assert_eq!(refused.code, ErrorCode::UnsupportedOperation);
                let refused = request.backend().clone_repo(url, &target).unwrap_err();
                assert_eq!(refused.code, ErrorCode::UnsupportedOperation);
                assert!(!target.exists());
            }
            let options = crate::TransportOptions { default_identity: Some("/must-not-read-key".into()), ..Default::default() };
            // The identity may be refused as a path that is not absolute, but no longer as unavailable in qualification.
            if let Err(error) = request.backend().with_transport(&target, Some(&options)) {
                assert_ne!(error.code, ErrorCode::UnsupportedOperation, "{error:?}");
            }
            let authority = crate::git::endpoint::shared_reservation::Authority::new(8, 8);
            let mut http = crate::git::endpoint::https_worker::Endpoint::with_authority(Default::default(), None, Default::default(), 3000, authority, HelperSlots::new()).unwrap();
            for policy in [AuthPolicy::Gh, AuthPolicy::WindowsConfigured] {
                let input = crate::git::endpoint::https_worker::Input { destination: "https://example.invalid/repo".into(), service: gwz_transport::protocol::GitService::UploadPackAdvertisement, policy, session: "session".into(), operation: "operation".into() };
                let failure = http.client.prepare_budget_for_transition(input, &tokio_util::sync::CancellationToken::new(), &mut http.client.budget(), &mut None).await.err().unwrap();
                assert_eq!(failure.code, gwz_transport::protocol::ErrorCode::UnsupportedOperation);
                assert_eq!(failure.effect, gwz_transport::protocol::Effect::None);
                assert_eq!(http.client.pool().counts().total(), 0);
                assert_eq!(http.client.pending_cleanup(), 0);
            }
            assert_eq!(http.shutdown(Duration::from_secs(1)).await, 0);
            request.finish().await;
            assert_eq!(runtime.shutdown().await.pending_local_work, 0);
        });
    }
} }

fn https_only_runtime() -> TransportRuntime {
    TransportRuntime::build_native(
        EndpointSettings {
            ssh: None,
            pool: pool::Config::default(),
            io_timeout_ms: 3000,
        },
        Some((
            HttpsEndpointConfig {
                tls: Default::default(),
                auth: None,
            },
            HelperSlots::new(),
        )),
        None,
    )
    .unwrap()
}
fn capacity_meta(id: &str, limit: i64) -> RequestMeta {
    let mut value = meta(id);
    value.policy = Some(crate::OperationPolicy {
        max_connections_per_host: Some(limit),
        ..Default::default()
    });
    value
}
#[test]
fn https_only_capacity_first_later_and_incompatible_overlap_use_the_actual_owner() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let runtime = https_only_runtime();
        let endpoint = runtime.0.lock().unwrap().local_endpoint.clone();
        let first = runtime
            .request(capacity_meta("capacity-first", 1), "first".into())
            .await
            .unwrap();
        assert_eq!(endpoint.capacity_for_test().unwrap().per_host, 1);
        let authority = endpoint.authority_for_test();
        let reservation = authority.try_reserve("example.invalid").unwrap();
        assert!(authority.try_reserve("example.invalid").is_none());
        drop(reservation);
        let error = runtime
            .request(capacity_meta("capacity-next", 2), "overlap".into())
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.code,
            crate::model::ErrorCode::TransportCapacityConflict
        );
        first.finish().await;
        let next = runtime
            .request(capacity_meta("capacity-next", 2), "next".into())
            .await
            .unwrap();
        assert_eq!(endpoint.capacity_for_test().unwrap().per_host, 2);
        let one = authority.try_reserve("example.invalid").unwrap();
        let two = authority.try_reserve("example.invalid").unwrap();
        assert!(authority.try_reserve("example.invalid").is_none());
        drop((one, two));
        next.finish().await;
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
#[test]
fn https_only_cancelled_capacity_retirement_closes_the_mutated_generation() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let runtime = https_only_runtime();
        let endpoint = runtime.0.lock().unwrap().local_endpoint.clone();
        endpoint.hold_retirement_for_test();
        let mut request =
            Box::pin(runtime.request(capacity_meta("capacity-cancel", 1), "cancel".into()));
        let mut cx = Context::from_waker(Waker::noop());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !endpoint.retirement_waiting_for_test() {
            match request.as_mut().poll(&mut cx) {
                Poll::Pending => {}
                Poll::Ready(Err(error)) => {
                    panic!("capacity installation failed before retirement: {error:?}")
                }
                Poll::Ready(Ok(_)) => panic!("held retirement unexpectedly completed"),
            }
            assert!(std::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
        drop(request);
        assert!(endpoint.is_closed());
        assert!(
            runtime
                .request(meta("after-cancel"), "after".into())
                .await
                .is_err()
        );
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
#[test]
fn public_ssh_constructor_retains_its_engine_and_defaults() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let home = tempfile::tempdir().unwrap();
        let runtime =
            TransportRuntime::new(SshEndpointConfig::fixture(home.path().to_path_buf(), None))
                .unwrap();
        let endpoint = runtime.0.lock().unwrap().local_endpoint.clone();
        assert!(endpoint.endpoint_offer_for_test().0);
        assert_eq!(
            endpoint.capacity_for_test(),
            Some(pool::Capacity::from(&pool::Config::default()))
        );
        let caps = runtime
            .capabilities(TransportCapabilitiesRequest {
                schema_version: "gwz.protocol/v0".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(caps.schemes, Some(vec![Scheme::Ssh]));
        assert_eq!(
            caps.auth_policies,
            Some(vec![AuthPolicy::SshAmbient, AuthPolicy::SshExplicit])
        );
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}

#[test]
fn ssh_and_https_runtime_offers_both_schemes_and_binds_what_it_offers() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let home = tempfile::tempdir().unwrap();
        let config = SshEndpointConfig::fixture(home.path().to_path_buf(), None);
        let runtime = TransportRuntime::build_native(
            config.into(),
            Some((
                HttpsEndpointConfig {
                    tls: Default::default(),
                    auth: None,
                },
                HelperSlots::new(),
            )),
            None,
        )
        .unwrap();
        let caps = runtime
            .capabilities(TransportCapabilitiesRequest {
                schema_version: "gwz.protocol/v0".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(caps.schemes, Some(vec![Scheme::Ssh, Scheme::Https]));
        let endpoint = runtime.0.lock().unwrap().local_endpoint.clone();
        assert!(
            endpoint.endpoint_offer_for_test().0,
            "the SSH engine is constructed"
        );
        let (_, bound) = endpoint.endpoint_offer_for_test();
        assert_eq!(Some(bound.schemes.clone()), caps.schemes);
        assert_eq!(runtime.shutdown().await.pending_local_work, 0);
    });
}
