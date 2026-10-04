//! Native authentication tests: the fake Port/Session seam and the real SSPI request validator.
mod exchange;
mod protocol;
mod retention;

cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
    use super::*;
    use hyper::header::{HeaderMap, WWW_AUTHENTICATE};
    struct FakePort {
        complete: bool,
        starts: Arc<AtomicUsize>,
        steps: Arc<AtomicUsize>,
        finishes: Arc<AtomicUsize>,
        cleanup: Arc<AtomicUsize>,
        deadlines: Arc<Mutex<Vec<Instant>>>,
    }
    struct FakeSession { complete: bool, steps: Arc<AtomicUsize>, finishes: Arc<AtomicUsize>, cleanup: Arc<AtomicUsize> }
    struct FakeProbe(Arc<AtomicUsize>);
    impl Probe for FakeProbe { fn confirmed(&self) -> bool { self.0.load(Ordering::Acquire) == 2 } }
    impl Port for FakePort {
        fn start(&self, request: gwz_sspi::AuthRequest, deadline: Instant, _: gwz_sspi::Cancellation) -> Work<'static, Result<Box<dyn Session>, BridgeError>> {
            assert!(!request.channel_binding.as_bytes().is_empty());
            assert_eq!(request.target.as_str(), "HTTP/localhost");
            self.starts.fetch_add(1, Ordering::Relaxed);
            self.deadlines.lock().unwrap().push(deadline);
            let session = FakeSession { complete: self.complete, steps: self.steps.clone(), finishes: self.finishes.clone(), cleanup: self.cleanup.clone() };
            Box::pin(async move { Ok(Box::new(session) as Box<dyn Session>) })
        }
    }
    impl Session for FakeSession {
        fn step(&mut self, challenge: Option<gwz_sspi::SecretBytes>) -> Work<'_, Result<gwz_sspi::TokenStep, BridgeError>> {
            let n = self.steps.fetch_add(1, Ordering::Relaxed);
            assert_eq!(challenge.is_some(), n > 0);
            let final_token = self.cleanup.load(Ordering::Acquire) == 4 && n > 0;
            if final_token { self.cleanup.store(2, Ordering::Release); }
            Box::pin(async move { Ok(gwz_sspi::TokenStep {
                status: if self.complete || final_token { gwz_sspi::TokenStatus::Complete } else { gwz_sspi::TokenStatus::Continue }, attributes: 0,
                observation: gwz_sspi::MechanismObservation::Selected { mechanism: gwz_sspi::Mechanism::Ntlm, authoritative: true },
                payload: gwz_sspi::SecretBytes::new(if final_token { b"" } else { b"synthetic" }),
            }) })
        }
        fn finish(self: Box<Self>) -> Work<'static, Result<(), BridgeError>> {
            self.finishes.fetch_add(1, Ordering::Release);
            Box::pin(async move {
                if self.cleanup.load(Ordering::Acquire) == 3 {
                    while self.cleanup.load(Ordering::Acquire) != 2 { tokio::time::sleep(Duration::from_millis(1)).await; }
                }
                if self.cleanup.load(Ordering::Acquire) != 2 {
                    Err(BridgeError { code: ErrorCode::Timeout, pending: Some(Box::new(FakeProbe(self.cleanup))) })
                } else { Ok(()) }
            })
        }
        fn cancel(self: Box<Self>) -> Option<Box<dyn Probe>> { Some(Box::new(FakeProbe(self.cleanup))) }
    }
    fn fake(complete: bool, cleanup: usize) -> (NativeCaller, Arc<FakePort>) {
        let port = Arc::new(FakePort { complete, starts: Arc::new(AtomicUsize::new(0)), steps: Arc::new(AtomicUsize::new(0)), finishes: Arc::new(AtomicUsize::new(0)),
            cleanup: Arc::new(AtomicUsize::new(cleanup)), deadlines: Arc::new(Mutex::new(Vec::new())) });
        (NativeCaller { port: Ok(port.clone()), qualification_direct: None }, port)
    }
    pub(super) fn publication_caller() -> NativeCaller { fake(true, 2).0 }
    fn real_request_validation(request: &gwz_sspi::AuthRequest) -> bool {
        use std::io::Read;
        use std::os::unix::process::CommandExt;
        assert!(matches!(request.package, gwz_sspi::Package::Ntlm));
        assert!(matches!(request.identity, gwz_sspi::Identity::CurrentLogon));
        assert_eq!(request.target.as_str(), "HTTP/localhost");
        assert!(request.digest.is_none()); assert!(request.token_limit.raw_bytes() > 0);
        let source = std::fs::canonicalize(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")).unwrap();
        let manifest = source.parent().unwrap().parent().unwrap().join("gwz-sspi/Cargo.toml");
        let target = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from).expect("external target required");
        assert!(!target.starts_with(source.parent().unwrap().parent().unwrap()));
        let mut command = std::process::Command::new("cargo");
        command.args(["+1.95.0", "test", "--manifest-path"]).arg(manifest)
            .args(["--lib", "--locked", "--offline", "composition_request_validator_fixture", "--", "--nocapture", "--test-threads=1"])
            .env("CARGO_TARGET_DIR", target.parent().unwrap().join("sspi-validator"))
            .env("GWZ_SSPI_TEST_COMPOSITION_CBT", request.channel_binding.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>())
            .stdout(std::process::Stdio::piped()).process_group(0);
        let mut child = command.spawn().unwrap(); let until = std::time::Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() { break status; }
            if std::time::Instant::now() >= until {
                let _ = std::process::Command::new("/bin/kill").args(["-KILL", &format!("-{}", child.id())]).status();
                let _ = child.wait(); panic!("real SSPI validator fixture exceeded bound");
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(status.success(), "validator fixture build/execution failed");
        let mut output = String::new(); child.stdout.take().unwrap().take(8192).read_to_string(&mut output).unwrap();
        let admitted = output.matches("gwz-sspi-private-validator:admit").count();
        let refused = output.matches("gwz-sspi-private-validator:refuse").count();
        assert_eq!(admitted + refused, 1, "missing or ambiguous real-validator receipt"); admitted == 1
    }
    struct ValidatorPort(Arc<FakePort>);
    impl Port for ValidatorPort {
        fn start(&self, request: gwz_sspi::AuthRequest, deadline: Instant, cancel: gwz_sspi::Cancellation) -> Work<'static, Result<Box<dyn Session>, BridgeError>> {
            if !real_request_validation(&request) { return Box::pin(async { Err(BridgeError { code: ErrorCode::InvalidRequest, pending: None }) }); }
            self.0.start(request, deadline, cancel)
        }
    }
    #[test]
    fn production_tls_binding_crosses_real_sspi_request_validator() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let server = Server::start(Arc::new(|request| Box::pin(async move {
                let mut r = response(if request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                if r.status() == 401 { r.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); } r
            }))).await;
            let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
            let (_, port) = fake(true, 2); endpoint.client.set_native(NativeCaller { port: Ok(Arc::new(ValidatorPort(port))), qualification_direct: None });
            let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
            let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
            assert_eq!(prepared.unwrap().opened.facts.authenticated, Some(true));
        });
    }
    #[test]
    fn production_binding_shapes_are_admitted_or_refused_by_real_validator() {
        let request = |binding| gwz_sspi::AuthRequest {
            package: gwz_sspi::Package::Ntlm, target: gwz_sspi::SecretText::new("HTTP/localhost").unwrap(),
            identity: gwz_sspi::Identity::CurrentLogon, channel_binding: binding,
            token_limit: gwz_sspi::TokenLimit::new(1024).unwrap(), digest: None,
        };
        for length in [32, 48, 64] {
            let mut digest = vec![0x41; length];
            let binding = https_auth::SecretHeader::channel_binding_digest(&mut digest).unwrap();
            assert!(digest.iter().all(|byte| *byte == 0));
            assert!(real_request_validation(&request(binding)));
            assert!(!real_request_validation(&request(gwz_sspi::SecretBytes::new(&vec![0x41; length]))));
        }
        let mut malformed = b"xls-server-end-point:".to_vec(); malformed.extend([0x41; 32]);
        assert!(!real_request_validation(&request(gwz_sspi::SecretBytes::new(&malformed))));
        for length in [0, 31, 33, 47, 49, 63, 65] {
            let mut digest = vec![0x41; length];
            assert!(https_auth::SecretHeader::channel_binding_digest(&mut digest).is_none());
            assert!(digest.iter().all(|byte| *byte == 0));
        }
    }
} }
