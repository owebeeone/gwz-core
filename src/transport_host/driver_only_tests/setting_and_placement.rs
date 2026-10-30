use super::*;

pub(crate) const CANARY: &str = "CANARY-GH_TOKEN-1f7c3a";

pub(crate) fn snapshot(pairs: &[(&str, &str)]) -> EnvironmentSnapshot {
    EnvironmentSnapshot::from_os_pairs(
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value))),
    )
    .unwrap()
}

pub(crate) fn builder() -> DriverOnlyBuilder {
    DriverOnlyBuilder::new(3000, 30_000)
}

pub(crate) fn refusal(built: ModelResult<(DriverOnly, TransportPort)>) -> ModelError {
    match built {
        Ok(_) => panic!("the driver-only host was built"),
        Err(error) => error,
    }
}

pub(crate) fn cli_meta(request_id: &str) -> RequestMeta {
    RequestMeta {
        request_id: request_id.into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(TransportOptions {
            placement: Some(TransportPlacement::Cli),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// A driver-only host whose port is linked to a client endpoint of the test's own.
pub(crate) struct Harness {
    pub(crate) host: DriverOnly,
    pub(crate) endpoint: CliEndpoint,
    pub(crate) _link: carrier::CarrierLink,
    pub(crate) temp: TempDir,
}
impl Harness {
    pub(crate) fn new() -> Self {
        Self::in_dir(TempDir::new().unwrap())
    }
    pub(crate) fn in_dir(temp: TempDir) -> Self {
        let home = temp.path().join("endpoint-home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/known_hosts"), b"").unwrap();
        let (host, driver_port) = builder().build(&snapshot(&[])).unwrap();
        let (endpoint, endpoint_port) =
            CliEndpoint::new(SshEndpointConfig::fixture(home, None)).unwrap();
        let link = carrier::CarrierLink::in_process(driver_port, endpoint_port).unwrap();
        Self {
            host,
            endpoint,
            _link: link,
            temp,
        }
    }
    /// The client's registration of the call, then the core's request for it.
    pub(crate) fn request(&self, request_id: &str) -> (TransportRequest, ClientRequest) {
        let meta = cli_meta(request_id);
        let client = self.endpoint.register_request(request_id).unwrap();
        let request = block_on(self.host.request(meta, "fetch".into())).unwrap();
        (request, client)
    }
}

// The setting.

#[test]
pub(crate) fn the_native_flag_is_refused_naming_the_flag() {
    let error = refusal(
        builder()
            .with_transport_flag(Some(Transport::Native))
            .build(&snapshot(&[])),
    );
    assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    assert!(
        error.message.contains("--transport native"),
        "{}",
        error.message
    );
    assert!(error.message.contains("driver-only"), "{}", error.message);
}

#[test]
pub(crate) fn the_native_variable_is_refused_naming_the_variable_in_any_case_and_padding() {
    for value in ["native", " Native ", "NATIVE"] {
        let error = refusal(builder().build(&snapshot(&[("GWZ_TRANSPORT", value)])));
        assert_eq!(error.code, ErrorCode::UnsupportedOperation, "{value:?}");
        assert!(
            error.message.contains("GWZ_TRANSPORT=native"),
            "{value:?}: {}",
            error.message
        );
    }
}

#[test]
pub(crate) fn the_native_global_configuration_is_refused_naming_the_key_and_the_file() {
    let home = TempDir::new().unwrap();
    let file = home.path().join(".gitconfig");
    std::fs::write(&file, "[gwz]\n\ttransport = native\n").unwrap();
    let home_text = home.path().to_str().unwrap();
    let error = refusal(builder().build(&snapshot(&[("HOME", home_text)])));
    assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    assert!(
        error.message.contains("gwz.transport native"),
        "{}",
        error.message
    );
    assert!(error.message.contains(".gitconfig"), "{}", error.message);
}

#[test]
pub(crate) fn a_native_refusal_carries_no_credential_of_the_environment() {
    let socket = format!("/nowhere/{CANARY}/agent.sock");
    let environment = snapshot(&[
        ("GWZ_TRANSPORT", "native"),
        ("SSH_AUTH_SOCK", &socket),
        ("GH_TOKEN", CANARY),
        ("GITHUB_TOKEN", CANARY),
        ("https_proxy", &format!("http://user:{CANARY}@127.0.0.1:9")),
    ]);
    let error = refusal(builder().build(&environment));
    assert!(!error.message.contains(CANARY), "{}", error.message);
    assert!(!format!("{error:?}").contains(CANARY));
}

#[test]
pub(crate) fn the_gwz_setting_builds_by_default_by_variable_and_by_flag_over_a_native_variable() {
    for (flag, pairs) in [
        (None, vec![]),
        (None, vec![("GWZ_TRANSPORT", "gwz")]),
        (Some(Transport::Gwz), vec![("GWZ_TRANSPORT", "native")]),
        (Some(Transport::Gwz), vec![("GWZ_TRANSPORT", "bogus")]),
    ] {
        let (host, _port) = builder()
            .with_transport_flag(flag)
            .build(&snapshot(&pairs))
            .unwrap_or_else(|error| panic!("{flag:?} {pairs:?}: {}", error.message));
        block_on(host.shutdown());
    }
}

#[test]
pub(crate) fn a_malformed_setting_is_refused_in_its_own_words() {
    let error = refusal(builder().build(&snapshot(&[("GWZ_TRANSPORT", "bogus")])));
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(
        error
            .message
            .contains("GWZ_TRANSPORT must be gwz or native"),
        "{}",
        error.message
    );
}

// The placement.

#[test]
pub(crate) fn every_request_is_the_cli_placement() {
    let harness = Harness::new();
    let (request, client) = harness.request("placement-cli");
    assert!(request.context.is_cli());
    block_on(request.finish());
    block_on(client.finish());
    for (id, transport) in [
        ("placement-none", None),
        ("placement-unset", Some(TransportOptions::default())),
        (
            "placement-local",
            Some(TransportOptions {
                placement: Some(TransportPlacement::Local),
                ..Default::default()
            }),
        ),
    ] {
        let mut meta = cli_meta(id);
        meta.transport = transport;
        let Err(error) = block_on(harness.host.request(meta, "fetch".into())) else {
            panic!("{id}: a request that is not the cli placement was admitted");
        };
        assert_eq!(error.code, ErrorCode::UnsupportedOperation, "{id}");
        assert!(
            error.message.contains("cli placement"),
            "{id}: {}",
            error.message
        );
        assert!(
            error.message.contains("driver-only"),
            "{id}: {}",
            error.message
        );
    }
}

#[test]
pub(crate) fn a_retry_budget_is_refused_as_the_cli_placement_refuses_it() {
    let harness = Harness::new();
    let mut meta = cli_meta("placement-retries");
    meta.policy = Some(crate::OperationPolicy {
        max_retries: Some(2),
        ..Default::default()
    });
    let Err(error) = block_on(harness.host.request(meta, "fetch".into())) else {
        panic!("a retry budget was admitted");
    };
    assert!(error.message.contains("--max-retries"), "{}", error.message);
}

#[test]
pub(crate) fn a_shut_down_host_admits_nothing() {
    let harness = Harness::new();
    block_on(harness.host.shutdown());
    let Err(error) = block_on(harness.host.request(cli_meta("closed"), "fetch".into())) else {
        panic!("a closed host admitted a request");
    };
    assert!(error.message.contains("closed"), "{}", error.message);
    // A second shutdown has nothing left to do.
    let report = block_on(harness.host.shutdown());
    assert_eq!(report.pending_local_work, 0);
}

// The routes.
