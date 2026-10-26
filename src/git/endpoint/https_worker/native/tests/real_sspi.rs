//! Real Windows SSPI through the product's native exchange (plan step 4.8; TR1.8 sections 7, 8 and 11, rows B11 to
//! B14 and B16 on the transport side). The client is the product: the production SSPI worker process, the logon
//! session's identity and the channel binding of the actual TLS connection. The server is a loopback HTTPS fixture
//! whose handler is an SSPI acceptor, which checks the binding as an Extended Protection server does: the binding is
//! the `tls-server-end-point` digest of the fixture's own certificate, or other bytes for the control that must be
//! refused. Loopback authentication to the local machine takes the NTLM local-call form (limitation L7), as the 1.0.17
//! rows did. No account is created and no password is used; the identity is the logon session of the test.
use super::*;
use crate::git::endpoint::https_fixture::{ConnectionId, Handler, Server, input, response};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{SEC_E_BAD_BINDINGS, SEC_I_CONTINUE_NEEDED},
    Security::{
        Authentication::Identity::{
            ASC_REQ_CONNECTION, AcceptSecurityContext, AcquireCredentialsHandleW,
            DeleteSecurityContext, FreeCredentialsHandle, SECBUFFER_CHANNEL_BINDINGS,
            SECBUFFER_TOKEN, SECBUFFER_VERSION, SECPKG_CRED_INBOUND, SECURITY_NATIVE_DREP,
            SecBuffer, SecBufferDesc,
        },
        Credentials::SecHandle,
    },
};

const FINGERPRINT: [u8; 32] = [0x42; 32];
const BINDING_PREFIX: &[u8] = b"tls-server-end-point:";
/// The size of `SEC_CHANNEL_BINDINGS` before its application data.
const BINDING_HEADER: usize = 32;

fn invalid() -> SecHandle {
    SecHandle {
        dwLower: usize::MAX,
        dwUpper: usize::MAX,
    }
}
fn valid(handle: &SecHandle) -> bool {
    handle.dwLower != usize::MAX || handle.dwUpper != usize::MAX
}

/// The server side of one authentication: an inbound credential, a context, and the binding it requires.
struct Acceptor {
    credentials: SecHandle,
    context: SecHandle,
    binding: Box<[u64]>,
    binding_length: usize,
}
impl Acceptor {
    fn new(package: &str, application_data: &[u8]) -> Self {
        let total = BINDING_HEADER + application_data.len();
        let mut binding = vec![0u64; total.div_ceil(8)].into_boxed_slice();
        // SAFETY: the storage is aligned for the header's u32 fields, zeroed and large enough for it and the data.
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(binding.as_mut_ptr().cast::<u8>(), total) };
        // SEC_CHANNEL_BINDINGS: dwInitiatorAddrType, cbInitiatorLength, dwInitiatorOffset, dwAcceptorAddrType,
        // cbAcceptorLength, dwAcceptorOffset, cbApplicationDataLength, dwApplicationDataOffset; only the last two.
        bytes[24..28].copy_from_slice(&(application_data.len() as u32).to_ne_bytes());
        bytes[28..32].copy_from_slice(&(BINDING_HEADER as u32).to_ne_bytes());
        bytes[BINDING_HEADER..].copy_from_slice(application_data);
        let mut name: Vec<u16> = package.encode_utf16().chain([0]).collect();
        let mut credentials = invalid();
        let mut expiry = 0;
        // SAFETY: a writable NUL-terminated package name and initialized outputs; null identity selects the inbound
        // credentials of the local acceptor, with no password.
        let status = unsafe {
            AcquireCredentialsHandleW(
                ptr::null(),
                name.as_mut_ptr(),
                SECPKG_CRED_INBOUND,
                ptr::null(),
                ptr::null(),
                None,
                ptr::null(),
                &mut credentials,
                &mut expiry,
            )
        };
        assert_eq!(status, 0, "inbound credential acquisition failed");
        Self {
            credentials,
            context: invalid(),
            binding,
            binding_length: total,
        }
    }
    /// One `AcceptSecurityContext` call: the status, and the token to send back.
    fn accept(&mut self, token: &[u8]) -> (i32, Vec<u8>) {
        let mut incoming = token.to_vec();
        let mut outgoing = vec![0u8; 65536];
        let mut inputs = [
            SecBuffer {
                cbBuffer: incoming.len() as u32,
                BufferType: SECBUFFER_TOKEN,
                pvBuffer: incoming.as_mut_ptr().cast(),
            },
            SecBuffer {
                cbBuffer: self.binding_length as u32,
                BufferType: SECBUFFER_CHANNEL_BINDINGS,
                pvBuffer: self.binding.as_mut_ptr().cast(),
            },
        ];
        let input_description = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 2,
            pBuffers: inputs.as_mut_ptr(),
        };
        let mut output = SecBuffer {
            cbBuffer: outgoing.len() as u32,
            BufferType: SECBUFFER_TOKEN,
            pvBuffer: outgoing.as_mut_ptr().cast(),
        };
        let mut output_description = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 1,
            pBuffers: &mut output,
        };
        let previous = if valid(&self.context) {
            &self.context as *const SecHandle
        } else {
            ptr::null()
        };
        let mut next = invalid();
        let mut attributes = 0;
        let mut expiry = 0;
        // SAFETY: a live credential and (after the first call) context; the descriptors refer to buffers that this
        // frame owns and keeps alive through the synchronous call. Neither ALLOCATE_MEMORY nor
        // ALLOW_MISSING_BINDINGS is requested: a missing or wrong binding fails.
        let status = unsafe {
            AcceptSecurityContext(
                &self.credentials,
                previous,
                &input_description,
                ASC_REQ_CONNECTION,
                SECURITY_NATIVE_DREP,
                &mut next,
                &mut output_description,
                &mut attributes,
                &mut expiry,
            )
        };
        if valid(&next) {
            self.context = next;
        }
        outgoing.truncate(output.cbBuffer as usize);
        (status, outgoing)
    }
}
impl Drop for Acceptor {
    fn drop(&mut self) {
        if valid(&self.context) {
            // SAFETY: the sole owner of a live context.
            let _ = unsafe { DeleteSecurityContext(&self.context) };
        }
        if valid(&self.credentials) {
            // SAFETY: the sole owner of a live credential, after its context.
            let _ = unsafe { FreeCredentialsHandle(&self.credentials) };
        }
    }
}

/// How the server's binding relates to the connection's own.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Expects {
    /// The `tls-server-end-point` digest of the fixture's certificate: what an enforcing server requires.
    Right,
    /// Other bytes: the control that must be refused.
    Wrong,
}

/// An HTTPS handler that authenticates `scheme` (`NTLM` or `Negotiate`) with SSPI, one acceptor per connection, and
/// answers every other request 401 with the `offers` it names. `statuses` records each `AcceptSecurityContext` status.
pub(crate) fn acceptor_handler(
    offers: &'static str,
    scheme: &'static str,
    expects: Expects,
    statuses: Arc<Mutex<Vec<i32>>>,
) -> Handler {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let mut data = BINDING_PREFIX.to_vec();
    data.extend(match expects {
        Expects::Right => crate::git::endpoint::https_fixture::leaf_digest(),
        Expects::Wrong => vec![0x5a; 32],
    });
    let acceptors: Arc<Mutex<HashMap<usize, Acceptor>>> = Arc::new(Mutex::new(HashMap::new()));
    Arc::new(move |request| {
        let statuses = statuses.clone();
        let connection = request
            .extensions()
            .get::<ConnectionId>()
            .map_or(0, |id| id.0);
        let token = request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .filter(|(name, _)| name.eq_ignore_ascii_case(scheme))
            .and_then(|(_, token)| STANDARD.decode(token.trim()).ok());
        let acceptors = acceptors.clone();
        let data = data.clone();
        Box::pin(async move {
            let challenge = |value: String| {
                let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
                result
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, value.parse().unwrap());
                result
            };
            let Some(token) = token else {
                return challenge(offers.to_owned());
            };
            let mut all = acceptors.lock().unwrap();
            let acceptor = all
                .entry(connection)
                .or_insert_with(|| Acceptor::new(scheme, &data));
            let (status, out) = acceptor.accept(&token);
            statuses.lock().unwrap().push(status);
            if status == SEC_I_CONTINUE_NEEDED {
                return challenge(format!("{scheme} {}", STANDARD.encode(out)));
            }
            if status == 0 {
                let mut result = response(200, GitService::UploadPackAdvertisement, Bytes::new());
                if !out.is_empty() {
                    result.headers_mut().insert(
                        WWW_AUTHENTICATE,
                        format!("{scheme} {}", STANDARD.encode(out))
                            .parse()
                            .unwrap(),
                    );
                }
                return result;
            }
            // Refused (SEC_E_BAD_BINDINGS for a wrong binding): a 401 with no continuation token.
            challenge(scheme.to_owned())
        })
    })
}

/// The product's native caller over the production worker, built from the sibling `gwz-sspi` checkout with the
/// synthetic fingerprint its own native tests use, and the supervisor to settle at the end.
pub(crate) fn real_caller() -> (NativeCaller, Arc<gwz_sspi::Supervisor>) {
    let source = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("src")).unwrap();
    let manifest = source
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("gwz-sspi/Cargo.toml");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .expect("external target required");
    let worker_target = target.parent().unwrap().join("sspi-worker");
    let status = std::process::Command::new("cargo")
        .args(["+1.95.0", "build", "--manifest-path"])
        .arg(&manifest)
        .args([
            "--bin",
            "gwz-sspi-worker",
            "--features",
            "worker-bin",
            "--locked",
            "--offline",
        ])
        .env("CARGO_TARGET_DIR", &worker_target)
        .env("GWZ_SSPI_BUILD_FINGERPRINT", "42".repeat(32))
        .status()
        .unwrap();
    assert!(status.success(), "the production worker did not build");
    let worker = worker_target.join("debug").join("gwz-sspi-worker.exe");
    let host = NativeHost::new(Ok(
        gwz_sspi::WorkerExecutable::new(worker, FINGERPRINT).unwrap()
    ));
    let supervisor = host.supervisor().clone().unwrap();
    let caller = host.caller();
    (caller, supervisor)
}

/// Opens the advertisement under `policy` and says how it ended.
async fn open(
    server: &Server,
    caller: &NativeCaller,
    policy: AuthPolicy,
) -> (Endpoint, Result<Prepared, Failure>) {
    let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
    endpoint.client.set_native(caller.clone());
    let mut request = input(server, GitService::UploadPackAdvertisement);
    request.policy = policy;
    let (result, _) = endpoint
        .client
        .prepare_attempt(
            request,
            &CancellationToken::new(),
            &mut endpoint.client.budget(),
            &mut None,
        )
        .await;
    (endpoint, result)
}

async fn settle(mut endpoint: Endpoint, result: Result<Prepared, Failure>) {
    drop(result);
    endpoint.client.finish_operation("operation");
    // A refused exchange cancels its worker session, whose process is confirmed gone a moment later.
    let until = std::time::Instant::now() + Duration::from_secs(20);
    while endpoint.client.reap_cleanup(Duration::from_secs(1)).await != 0
        && std::time::Instant::now() < until
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(endpoint.shutdown(Duration::from_secs(5)).await, 0);
}

/// The worker process is gone and nothing it owned is outstanding once the supervisor is shut down.
pub(crate) async fn settle_worker(supervisor: Arc<gwz_sspi::Supervisor>) {
    let outcome = supervisor
        .shutdown(gwz_sspi::Deadline::new(
            std::time::Instant::now() + Duration::from_secs(20),
        ))
        .await;
    assert!(outcome.outstanding.is_empty(), "{:?}", outcome.outstanding);
}

fn run<F: std::future::Future<Output = ()>>(work: F) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(work);
}

#[test]
fn the_logon_session_authenticates_to_an_enforcing_server_over_ntlm_and_negotiate_b11_b12_b16() {
    run(async {
        let (caller, supervisor) = real_caller();
        for (offers, scheme, policy) in [
            ("NTLM", "NTLM", AuthPolicy::WindowsDefault),
            ("NTLM", "NTLM", AuthPolicy::WindowsConfigured),
            ("Negotiate", "Negotiate", AuthPolicy::WindowsDefault),
            ("Negotiate", "Negotiate", AuthPolicy::WindowsConfigured),
            // B13 and B14: the logon session wins beside Basic and beside Digest, and Digest is ignored.
            ("NTLM, Basic realm=x", "NTLM", AuthPolicy::WindowsConfigured),
            (
                "Negotiate, Basic realm=x",
                "Negotiate",
                AuthPolicy::WindowsConfigured,
            ),
            (
                "Digest realm=x, NTLM",
                "NTLM",
                AuthPolicy::WindowsConfigured,
            ),
        ] {
            let statuses = Arc::new(Mutex::new(Vec::new()));
            let server = Server::start(acceptor_handler(
                offers,
                scheme,
                Expects::Right,
                statuses.clone(),
            ))
            .await;
            let (endpoint, result) = open(&server, &caller, policy).await;
            let prepared = result
                .as_ref()
                .unwrap_or_else(|failure| panic!("{offers} {policy:?}: {failure:?}"));
            let facts = &prepared.opened.facts;
            assert_eq!(facts.authenticated, Some(true), "{offers} {policy:?}");
            assert!(facts.credential_offered, "{offers} {policy:?}");
            let native = facts.native.as_ref().unwrap();
            assert_eq!(
                native.source,
                NativeSource::CurrentLogon,
                "{offers} {policy:?}"
            );
            assert!(native.authoritative, "{offers} {policy:?}");
            // The enforcing server accepted the client's binding: its last status is success.
            assert_eq!(
                statuses.lock().unwrap().last(),
                Some(&0),
                "{offers} {policy:?}"
            );
            settle(endpoint, result).await;
        }
        settle_worker(supervisor).await;
    });
}

#[test]
fn a_server_that_expects_other_binding_bytes_refuses_the_logon_identity_b16_control() {
    run(async {
        let (caller, supervisor) = real_caller();
        for (scheme, policy) in [
            ("NTLM", AuthPolicy::WindowsDefault),
            ("Negotiate", AuthPolicy::WindowsConfigured),
        ] {
            let statuses = Arc::new(Mutex::new(Vec::new()));
            let server = Server::start(acceptor_handler(
                scheme,
                scheme,
                Expects::Wrong,
                statuses.clone(),
            ))
            .await;
            let (endpoint, result) = open(&server, &caller, policy).await;
            let failure = result
                .as_ref()
                .err()
                .unwrap_or_else(|| panic!("{scheme}: authenticated against a wrong binding"));
            assert_eq!(failure.code, ErrorCode::Authentication, "{scheme}");
            let facts = failure.facts.as_ref().unwrap();
            assert_eq!(facts.authenticated, Some(false), "{scheme}");
            assert_eq!(facts.method, AuthMethod::Sspi, "{scheme}");
            // The OS refused the binding, not some other check, and the refusal ended the operation.
            assert_eq!(
                statuses.lock().unwrap().last(),
                Some(&SEC_E_BAD_BINDINGS),
                "{scheme}"
            );
            settle(endpoint, result).await;
        }
        settle_worker(supervisor).await;
    });
}

/// Step 1.6 (plan §4; contract §7.2; amendment 2 §3.18): a connection the logon session authenticated serves only the
/// operation binding that authenticated it. Binding A authenticates the advertisement with the real logon session;
/// binding B, of the same operation name (which shares A's route, the step's finding), sends its service request
/// under the logon-session policy. None of B's requests may reach A's authenticated connection: B is refused instead,
/// or served on a connection of its own.
#[test]
fn p1_6_a_logon_session_connection_is_not_reused_by_another_binding_real_sspi() {
    run(async {
        let (caller, supervisor) = real_caller();
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let seen: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
        let inner = acceptor_handler("NTLM", "NTLM", Expects::Right, statuses.clone());
        let records = seen.clone();
        let handler: Handler = Arc::new(move |request| {
            let connection = request
                .extensions()
                .get::<ConnectionId>()
                .map_or(0, |id| id.0);
            records.lock().unwrap().push(connection);
            inner(request)
        });
        let server = Server::start(handler).await;
        let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
        endpoint.client.set_native(caller.clone());
        let mut a = input(&server, GitService::UploadPackAdvertisement);
        a.policy = AuthPolicy::WindowsDefault;
        a.session = "binding-a".into();
        let (prepared, _) = endpoint
            .client
            .prepare_attempt(
                a,
                &CancellationToken::new(),
                &mut endpoint.client.budget(),
                &mut None,
            )
            .await;
        let prepared = prepared.unwrap_or_else(|failure| panic!("binding A: {failure:?}"));
        assert_eq!(prepared.opened.facts.authenticated, Some(true));
        // A's exchange ends in order, so its authenticated connection goes back to idle, A's.
        let (stream, task) = crate::git::endpoint::https_fixture::attach(prepared);
        stream.end_write().await.unwrap();
        let mut bytes = [0; 64];
        while stream.read(&mut bytes).await.unwrap() != 0 {}
        stream.close().await.unwrap();
        task.await.unwrap();
        let authenticated_on = *seen.lock().unwrap().last().unwrap();
        let before = seen.lock().unwrap().len();
        let mut b = input(&server, GitService::UploadPackExchange);
        b.policy = AuthPolicy::WindowsDefault;
        b.session = "binding-b".into();
        let (result, _) = endpoint
            .client
            .prepare_attempt(
                b,
                &CancellationToken::new(),
                &mut endpoint.client.budget(),
                &mut None,
            )
            .await;
        let after = seen.lock().unwrap()[before..].to_vec();
        assert!(
            after
                .iter()
                .all(|connection| *connection != authenticated_on),
            "binding B's request reached binding A's logon-session connection {authenticated_on}: {after:?}"
        );
        settle(endpoint, result).await;
        settle_worker(supervisor).await;
    });
}
