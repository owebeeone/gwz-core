//! Start and Finish ownership, and retention of cleanup charges until they are confirmed.
use super::*;

cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
    struct HeldStart { entered: Arc<AtomicUsize>, gate: Arc<AtomicUsize>, cleanup: Arc<AtomicUsize>, registered: bool, steps: Arc<AtomicUsize> }
    impl Port for HeldStart {
        fn start(&self, _: gwz_sspi::AuthRequest, _: Instant, cancel: gwz_sspi::Cancellation) -> Work<'static, Result<Box<dyn Session>, BridgeError>> {
            let entered = self.entered.clone(); let gate = self.gate.clone(); let cleanup = self.cleanup.clone(); let registered = self.registered; let steps = self.steps.clone();
            Box::pin(async move {
                entered.store(1, Ordering::Release);
                while gate.load(Ordering::Acquire) == 0 { tokio::time::sleep(Duration::from_millis(1)).await; }
                assert!(cancel.is_cancelled());
                if !registered { return Err(BridgeError { code: ErrorCode::Cancelled, pending: None }); }
                Ok(Box::new(FakeSession { complete: true, steps, finishes: Arc::new(AtomicUsize::new(0)), cleanup }) as Box<dyn Session>)
            })
        }
    }
    #[test]
    fn production_abort_pending_start_retains_registration_and_pre_registration_owners() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            for registered in [false, true] {
                let server = Server::start(Arc::new(|_| Box::pin(async {
                    let mut r = response(401, GitService::UploadPackAdvertisement, Bytes::new()); r.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); r
                }))).await;
                let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
                let port = Arc::new(HeldStart { entered: Arc::new(AtomicUsize::new(0)), gate: Arc::new(AtomicUsize::new(0)), cleanup: Arc::new(AtomicUsize::new(0)), registered, steps: Arc::new(AtomicUsize::new(0)) });
                endpoint.client.set_native(NativeCaller { port: Ok(port.clone()), qualification_direct: None });
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
                let client = endpoint.client.clone(); let task = tokio::spawn(async move { client.prepare_attempt(request, &CancellationToken::new(), &mut client.budget(), &mut None).await });
                while port.entered.load(Ordering::Acquire) == 0 { tokio::task::yield_now().await; }
                task.abort(); assert!(matches!(task.await, Err(error) if error.is_cancelled()));
                assert!(endpoint.client.pending_cleanup() >= 1);
                assert_eq!(endpoint.client.native_cleanup.lock().unwrap().records.len(), 1); assert_eq!(endpoint.client.slots.available_permits(), 63);
                endpoint.client.finish_operation("operation"); assert!(endpoint.client.operation("operation").is_err());
                port.gate.store(1, Ordering::Release); tokio::time::sleep(Duration::from_millis(2)).await;
                if registered {
                    assert!(endpoint.client.pending_cleanup() >= 1);
                assert_eq!(endpoint.client.native_cleanup.lock().unwrap().records.len(), 1);
                    port.cleanup.store(1, Ordering::Release); assert!(endpoint.client.pending_cleanup() >= 1);
                assert_eq!(endpoint.client.native_cleanup.lock().unwrap().records.len(), 1);
                    port.cleanup.store(2, Ordering::Release);
                }
                let until = Instant::now() + Duration::from_secs(1);
                while endpoint.client.pending_cleanup() != 0 {
                    assert!(Instant::now() < until, "physical and native cleanup must settle");
                    tokio::time::sleep(Duration::from_millis(1)).await;
                } assert_eq!(endpoint.client.slots.available_permits(), 64);
                assert!(endpoint.client.operation("operation").is_ok()); assert_eq!(port.steps.load(Ordering::Acquire), 0);
            }
        });
    }
    struct PausedProbe { entered: Arc<std::sync::Barrier>, resume: Arc<std::sync::Barrier>, status: Arc<AtomicUsize>, first: AtomicUsize }
    impl Probe for PausedProbe {
        fn confirmed(&self) -> bool { if self.first.fetch_add(1, Ordering::Relaxed) == 0 { self.entered.wait(); self.resume.wait(); } self.status.load(Ordering::Acquire) == 2 }
    }
    #[test]
    fn production_cleanup_count_covers_claimed_records_and_concurrent_insertion() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap(); let _entered_runtime = runtime.enter();
        for status in [0, 1, 2] {
            let endpoint = Endpoint::new(https_connection::Config::default(), None, pool::Config::default()).unwrap();
            let entered = Arc::new(std::sync::Barrier::new(2)); let resume = Arc::new(std::sync::Barrier::new(2));
            let proof = Arc::new(AtomicUsize::new(status));
            endpoint.client.native_cleanup.lock().unwrap().records.push(Pending { probe: Box::new(PausedProbe { entered: entered.clone(), resume: resume.clone(), status: proof.clone(), first: AtomicUsize::new(0) }),
                _operation: endpoint.client.operation("operation").unwrap(), _slot: endpoint.client.slots.clone().try_acquire_owned().unwrap() });
            let client = endpoint.client.clone(); let reaper = std::thread::spawn(move || client.pending_cleanup()); entered.wait();
            endpoint.client.finish_operation("operation"); assert!(endpoint.client.operation("operation").is_err());
            let claimed_count = endpoint.client.pending_cleanup();
            let second = Arc::new(AtomicUsize::new(0));
            endpoint.client.native_cleanup.lock().unwrap().records.push(Pending { probe: Box::new(FakeProbe(second.clone())),
                _operation: endpoint.client.operation("second").unwrap(), _slot: endpoint.client.slots.clone().try_acquire_owned().unwrap() });
            let inserted_count = endpoint.client.pending_cleanup(); resume.wait();
            let final_count = reaper.join().unwrap();
            assert_eq!(claimed_count, 1, "claimed work remains counted status={status}");
            assert_eq!(inserted_count, 2); assert_eq!(final_count, if status == 2 { 1 } else { 2 });
            assert_eq!(endpoint.client.slots.available_permits(), if status == 2 { 63 } else { 62 });
            assert_eq!(endpoint.client.operation("operation").is_ok(), status == 2);
            endpoint.client.finish_operation("second");
            proof.store(2, Ordering::Release); second.store(2, Ordering::Release);
            assert_eq!(endpoint.client.pending_cleanup(), 0);
            assert_eq!(endpoint.client.slots.available_permits(), 64);
            assert!(endpoint.client.operation("operation").is_ok());
            assert!(endpoint.client.operation("second").is_ok());
        }
    }
    #[test]
    fn production_remote_complete_and_cleanup_ownership_are_independent() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response, attach};
            for (complete, cleanup) in [(false, 2), (true, 0), (true, 2)] {
                let server = Server::start(Arc::new(|request| Box::pin(async move {
                    let mut result = response(if request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                    if result.status() == 401 { result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); }
                    result
                }))).await;
                let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
                let (caller, port) = fake(complete, cleanup); endpoint.client.set_native(caller);
                let mut input = input(&server, GitService::UploadPackAdvertisement); input.policy = AuthPolicy::WindowsDefault;
                let mut budget = endpoint.client.budget();
                let (result, _) = endpoint.client.prepare_attempt(input, &CancellationToken::new(), &mut budget, &mut None).await;
                assert_eq!(port.starts.load(Ordering::Acquire), 1);
                assert_eq!(port.deadlines.lock().unwrap().as_slice(), &[budget.logical_deadline.unwrap()]);
                match result {
                    Ok(prepared) => {
                        assert!(complete && cleanup == 2); assert_eq!(prepared.opened.facts.authenticated, Some(true));
                        let (stream, task) = attach(prepared); stream.end_write().await.unwrap();
                        let mut bytes = [0; 1]; assert_eq!(stream.read(&mut bytes).await.unwrap(), 0);
                        assert_eq!(stream.close().await.unwrap().disposition, Disposition::Reusable); task.await.unwrap();
                    }
                    Err(error) => {
                        assert_eq!(error.code, if complete { ErrorCode::Timeout } else { ErrorCode::Protocol });
                        let facts = error.facts.unwrap(); assert!(facts.native.unwrap().authoritative);
                        assert_eq!(facts.authenticated, if complete { Some(true) } else { None });
                    }
                }
                assert_eq!(port.steps.load(Ordering::Acquire), 1);
                // Unknown (1) never releases operation/slot; only Confirmed (2).
                if cleanup == 0 {
                    assert_eq!(endpoint.client.slots.available_permits(), 63);
                    assert_eq!(reap(&endpoint.client.native_cleanup), 1);
                    port.cleanup.store(1, Ordering::Release); assert_eq!(reap(&endpoint.client.native_cleanup), 1);
                    port.cleanup.store(2, Ordering::Release); assert_eq!(reap(&endpoint.client.native_cleanup), 0);
                    assert_eq!(endpoint.client.slots.available_permits(), 64);
                }
            }
        });
    }

    #[test]
    fn production_drop_during_finish_retains_both_charges() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let server = Server::start(Arc::new(|request| Box::pin(async move {
                let mut result = response(if request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                if result.status() == 401 { result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); } result
            }))).await;
            let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
            let (caller, port) = fake(true, 3); endpoint.client.set_native(caller);
            let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
            let client = endpoint.client.clone();
            let task = tokio::spawn(async move { client.prepare_attempt(request, &CancellationToken::new(), &mut client.budget(), &mut None).await });
            while port.steps.load(Ordering::Acquire) == 0 { tokio::task::yield_now().await; }
            tokio::time::sleep(Duration::from_millis(20)).await; assert!(!task.is_finished());
            task.abort(); assert!(matches!(task.await, Err(error) if error.is_cancelled()));
            assert_eq!(reap(&endpoint.client.native_cleanup), 1); assert_eq!(endpoint.client.slots.available_permits(), 63);
            endpoint.client.finish_operation("operation");
            assert!(endpoint.client.operation("operation").is_err());
            port.cleanup.store(1, Ordering::Release); assert_eq!(reap(&endpoint.client.native_cleanup), 1);
            port.cleanup.store(2, Ordering::Release); tokio::time::sleep(Duration::from_millis(2)).await;
            assert_eq!(reap(&endpoint.client.native_cleanup), 0);
            assert_eq!(endpoint.client.slots.available_permits(), 64); assert!(endpoint.client.operation("operation").is_ok());
        });
    }
    #[test]
    fn production_finish_observes_cancel_and_expiry_while_receipt_is_pending() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            for cancel_request in [false, true] {
                let server = Server::start(Arc::new(|request| Box::pin(async move {
                    let mut result = response(if request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                    if result.status() == 401 { result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); } result
                }))).await;
                let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
                let (caller, port) = fake(true, 3); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
                let client = endpoint.client.clone(); let cancel = CancellationToken::new(); let signal = cancel.clone();
                let task = tokio::spawn(async move {
                    let mut budget = client.budget(); budget.connect = Some(Duration::from_millis(if cancel_request { 1000 } else { 250 }));
                    client.prepare_attempt(request, &cancel, &mut budget, &mut None).await
                });
                while port.steps.load(Ordering::Acquire) == 0 { tokio::task::yield_now().await; }
                tokio::time::sleep(Duration::from_millis(20)).await;
                if cancel_request { signal.cancel(); }
                let (result, _) = task.await.unwrap(); let error = result.err().unwrap();
                assert_eq!(error.code, if cancel_request { ErrorCode::Cancelled } else { ErrorCode::Timeout });
                assert_eq!(error.facts.unwrap().authenticated, Some(true)); assert_eq!(reap(&endpoint.client.native_cleanup), 1);
                assert_eq!(endpoint.client.slots.available_permits(), 63);
                port.cleanup.store(2, Ordering::Release); tokio::time::sleep(Duration::from_millis(2)).await;
                assert_eq!(reap(&endpoint.client.native_cleanup), 0);
            }
        });
    }
    #[test]
    fn production_seeded_finish_publication_and_retention_orders() {
        const SEED: u64 = 0x53435049;
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let mut state = SEED;
            for turn in 0..8 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1); let order = (state >> 32) % 3;
                let server = Server::start(Arc::new(|request| Box::pin(async move {
                    let mut result = response(if request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                    if result.status() == 401 { result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); } result
                }))).await;
                let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
                let (caller, port) = fake(true, 3); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
                let client = endpoint.client.clone(); let cancel = CancellationToken::new(); let signal = cancel.clone();
                let task = tokio::spawn(async move { client.prepare_attempt(request, &cancel, &mut client.budget(), &mut None).await });
                while port.finishes.load(Ordering::Acquire) == 0 { tokio::task::yield_now().await; }
                if order == 0 {
                    signal.cancel(); let (result, _) = task.await.unwrap();
                    assert_eq!(result.err().unwrap().code, ErrorCode::Cancelled, "seed={SEED:x} turn={turn} order={order}");
                    port.cleanup.store(1, Ordering::Release); assert_eq!(reap(&endpoint.client.native_cleanup), 1);
                    assert_eq!(endpoint.client.slots.available_permits(), 63);
                    port.cleanup.store(2, Ordering::Release);
                } else {
                    port.cleanup.store(2, Ordering::Release);
                    if order == 1 { signal.cancel(); }
                    let (result, _) = task.await.unwrap();
                    if order == 1 { assert_eq!(result.err().unwrap().code, ErrorCode::Cancelled, "seed={SEED:x} turn={turn}"); }
                    else { let prepared = result.unwrap(); assert_eq!(prepared.opened.facts.authenticated, Some(true)); signal.cancel(); drop(prepared); }
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
                assert_eq!(reap(&endpoint.client.native_cleanup), 0, "seed={SEED:x} turn={turn}");
                assert_eq!(endpoint.client.slots.available_permits(), 64);
            }
        });
    }
} }
