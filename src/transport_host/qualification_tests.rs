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
            assert_eq!(caps.schemes, Some(vec![Scheme::Https]));
            assert_eq!(caps.auth_policies, Some(vec![AuthPolicy::Anonymous, AuthPolicy::WindowsDefault]));
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
            for url in ["ssh://git@example.invalid/repo", "http://example.invalid/repo", "file:///unavailable/repo", "git@example.invalid:repo"] {
                let refused = request.backend().validate_url_identity(None, "origin", url).unwrap_err();
                assert_eq!(refused.code, ErrorCode::UnsupportedOperation);
                let refused = request.backend().clone_repo(url, &target).unwrap_err();
                assert_eq!(refused.code, ErrorCode::UnsupportedOperation);
                assert!(!target.exists());
            }
            let options = crate::TransportOptions { default_identity: Some("/must-not-read-key".into()), ..Default::default() };
            let refused = request.backend().with_transport(&target, Some(&options)).unwrap_err();
            assert_eq!(refused.code, ErrorCode::UnsupportedOperation);
            assert!(SshEndpointConfig::from_environment().is_err());
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
