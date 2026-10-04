//! Exclusive native HTTPS authentication; physical ownership stays in Prepared.
use super::*;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hyper::header::{HeaderMap, WWW_AUTHENTICATE};

/// Initialized fixed output; secrets never grow or move through a String.
pub(super) struct Header {
    bytes: Vec<u8>,
    probe: audit::Probe,
}
impl Header {
    fn zeroed(size: usize, probe: audit::Probe) -> Self {
        Self {
            bytes: vec![0; size],
            probe,
        }
    }
    fn token(scheme: NativeScheme, token: &[u8]) -> Result<Self, ErrorCode> {
        let prefix = prefix(scheme)?;
        let size = base64::encoded_len(token.len(), true).ok_or(ErrorCode::InvalidRequest)?;
        if size + prefix.len() > 65536 {
            return Err(ErrorCode::InvalidRequest);
        }
        Self::encode(
            prefix,
            token,
            Self::zeroed(prefix.len() + size, audit::Probe::inert()),
        )
    }
    fn encode(prefix: &[u8], token: &[u8], mut owner: Self) -> Result<Self, ErrorCode> {
        let output = owner
            .bytes
            .get_mut(..prefix.len())
            .ok_or(ErrorCode::InvalidRequest)?;
        output.copy_from_slice(prefix);
        STANDARD
            .encode_slice(token, &mut owner.bytes[prefix.len()..])
            .map_err(|_| ErrorCode::Protocol)?;
        Ok(owner)
    }
    fn copy(source: &[u8], probe: audit::Probe) -> Self {
        let mut owner = Self::zeroed(source.len(), probe);
        owner.bytes.copy_from_slice(source);
        owner
    }
    fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl Drop for Header {
    fn drop(&mut self) {
        crate::session_host::environment::overwrite(&mut self.bytes);
        self.probe.observe(&self.bytes);
    }
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod audit {
            use std::sync::{Arc, Mutex};
            #[derive(Clone, Default)] pub(super) struct Probe(pub(super) Option<Arc<Mutex<Vec<(usize, bool)>>>>);
            impl Probe {
                pub(super) fn inert() -> Self { Self::default() }
                pub(super) fn observe(&self, bytes: &[u8]) {
                    if let Some(records) = &self.0 { records.lock().unwrap().push((bytes.len(), bytes.iter().all(|byte| *byte == 0))); }
                }
            }
        }
    } else {
        mod audit {
            #[derive(Clone, Default)] pub(super) struct Probe;
            impl Probe { pub(super) fn inert() -> Self { Self } pub(super) fn observe(&self, _: &[u8]) {} }
        }
    }
}
fn prefix(scheme: NativeScheme) -> Result<&'static [u8], ErrorCode> {
    match scheme {
        NativeScheme::Negotiate => Ok(b"Negotiate "),
        NativeScheme::Ntlm => Ok(b"NTLM "),
        NativeScheme::Digest => Err(ErrorCode::UnsupportedOperation),
    }
}
fn raw_limit(header_bytes: usize, scheme: NativeScheme) -> Result<gwz_sspi::TokenLimit, ErrorCode> {
    let space = header_bytes
        .checked_sub(prefix(scheme)?.len())
        .ok_or(ErrorCode::InvalidRequest)?;
    let limit = (space / 4 * 3).min(65536);
    gwz_sspi::TokenLimit::new(limit as u32).map_err(|_| ErrorCode::InvalidRequest)
}
fn decode_challenge(bytes: &[u8], limit: usize) -> Result<gwz_sspi::SecretBytes, ErrorCode> {
    decode_owned(bytes, limit, audit::Probe::inert())
}
fn decode_owned(
    bytes: &[u8],
    limit: usize,
    probe: audit::Probe,
) -> Result<gwz_sspi::SecretBytes, ErrorCode> {
    if bytes.is_empty()
        || !bytes.len().is_multiple_of(4)
        || bytes.len() > base64::encoded_len(limit, true).unwrap_or(0)
    {
        return Err(ErrorCode::Protocol);
    }
    // decode_slice writes only into initialized fixed zeroizing storage.
    let mut storage = Header::zeroed(bytes.len() / 4 * 3, probe);
    let n = STANDARD
        .decode_slice(bytes, &mut storage.bytes)
        .map_err(|_| ErrorCode::Protocol)?;
    if n == 0 || n > limit {
        return Err(ErrorCode::Protocol);
    }
    Ok(gwz_sspi::SecretBytes::new(&storage.bytes[..n]))
}
struct Offer {
    scheme: NativeScheme,
    token: Header,
}
impl Offer {
    fn initial(&self) -> Result<(), ErrorCode> {
        if !self.token.bytes().is_empty() || self.scheme == NativeScheme::Digest {
            return Err(ErrorCode::UnsupportedOperation);
        }
        Ok(())
    }
}
struct Offers {
    native: Vec<Offer>,
    basic: bool,
    helper: bool,
}
impl Offers {
    fn parse(headers: &HeaderMap) -> Result<Self, ErrorCode> {
        Self::parse_owned(headers, audit::Probe::inert())
    }
    fn parse_owned(headers: &HeaderMap, probe: audit::Probe) -> Result<Self, ErrorCode> {
        let mut result = Self {
            native: Vec::new(),
            basic: false,
            helper: false,
        };
        let mut total = 0usize;
        for value in headers.get_all(WWW_AUTHENTICATE) {
            total = total
                .checked_add(value.as_bytes().len())
                .ok_or(ErrorCode::Protocol)?;
            if total > 65536 {
                return Err(ErrorCode::Protocol);
            }
            // Commas inside quoted Basic/Digest parameters are not offer boundaries.
            let text = value.to_str().map_err(|_| ErrorCode::Protocol)?;
            let mut quoted = false;
            let mut escaped = false;
            let mut start = 0;
            for (i, byte) in text.bytes().enumerate() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if quoted && byte == b'\\' {
                    escaped = true;
                    continue;
                }
                if byte == b'"' {
                    quoted = !quoted;
                }
                if byte == b',' && !quoted {
                    result.part(&text[start..i], probe.clone())?;
                    start = i + 1;
                }
            }
            if quoted || escaped {
                return Err(ErrorCode::Protocol);
            }
            result.part(&text[start..], probe.clone())?;
        }
        Ok(result)
    }
    fn part(&mut self, text: &str, probe: audit::Probe) -> Result<(), ErrorCode> {
        let text = text.trim();
        let (name, token) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        let scheme = if name.eq_ignore_ascii_case("Negotiate") {
            Some(NativeScheme::Negotiate)
        } else if name.eq_ignore_ascii_case("NTLM") {
            self.helper = true;
            Some(NativeScheme::Ntlm)
        } else if name.eq_ignore_ascii_case("Digest") {
            self.helper = true;
            Some(NativeScheme::Digest)
        } else {
            if name.eq_ignore_ascii_case("Basic") {
                self.basic = true;
                self.helper = true;
            }
            None
        };
        if let Some(scheme) = scheme {
            if self.native.iter().any(|offer| offer.scheme == scheme) {
                return Err(ErrorCode::Protocol);
            }
            let token = token.trim();
            self.native.push(Offer {
                scheme,
                token: Header::copy(token.as_bytes(), probe),
            });
        }
        Ok(())
    }
    fn helper_allowed(&self) -> bool {
        self.helper
    }
    fn select(&self, configured: bool) -> Option<&Offer> {
        [
            NativeScheme::Negotiate,
            NativeScheme::Ntlm,
            NativeScheme::Digest,
        ]
        .into_iter()
        .filter(|scheme| configured || *scheme != NativeScheme::Digest)
        .find_map(|scheme| self.native.iter().find(|offer| offer.scheme == scheme))
    }
}
struct History {
    facts: Facts,
    complete: bool,
}
impl History {
    fn new(source: NativeSource, scheme: NativeScheme) -> Self {
        Self {
            facts: Facts {
                method: AuthMethod::Sspi,
                native: Some(NativeFacts {
                    source,
                    scheme,
                    observation: NativeObservation::NotStarted,
                    mechanism: None,
                    authoritative: false,
                }),
                ..Default::default()
            },
            complete: false,
        }
    }
    fn observe(&mut self, step: &gwz_sspi::TokenStep) -> Result<(), ErrorCode> {
        let native = self.facts.native.as_mut().unwrap();
        if native.scheme == NativeScheme::Ntlm
            && !matches!(
                step.observation,
                gwz_sspi::MechanismObservation::Selected {
                    mechanism: gwz_sspi::Mechanism::Ntlm,
                    ..
                }
            )
        {
            return Err(ErrorCode::Protocol);
        }
        match step.observation {
            gwz_sspi::MechanismObservation::Unresolved => {
                if native.mechanism.is_some() {
                    return Err(ErrorCode::Protocol);
                }
                native.observation = NativeObservation::Unresolved;
                native.mechanism = None;
            }
            gwz_sspi::MechanismObservation::Selected {
                mechanism,
                authoritative,
            } => {
                let mechanism = match mechanism {
                    gwz_sspi::Mechanism::Kerberos => NativeMechanism::Kerberos,
                    gwz_sspi::Mechanism::Ntlm => NativeMechanism::Ntlm,
                    _ => return Err(ErrorCode::Protocol),
                };
                if native.mechanism.is_some_and(|old| old != mechanism)
                    || (native.authoritative && !authoritative)
                {
                    return Err(ErrorCode::Protocol);
                }
                native.observation = NativeObservation::Selected;
                native.mechanism = Some(mechanism);
                native.authoritative = authoritative;
            }
        }
        self.complete = step.status == gwz_sspi::TokenStatus::Complete;
        Ok(())
    }

    fn reject(&mut self, status: u16) {
        self.facts.http_status = Some(i64::from(status));
        if status == 401 && self.facts.credential_offered {
            self.facts.authenticated = Some(false);
        }
    }
    fn accept(&mut self, status: u16) -> Result<(), ErrorCode> {
        if status != 200
            || !self.complete
            || !self.facts.credential_offered
            || !self.facts.native.as_ref().unwrap().authoritative
        {
            return Err(ErrorCode::Protocol);
        }
        self.facts.http_status = Some(200);
        self.facts.authenticated = Some(true);
        Ok(())
    }
}

/// A host captures this on the original entry, before any fanout or detach.
/// Availability is retained so anonymous, Basic and SSH do not require SSPI.
#[derive(Clone)]
pub struct NativeCaller {
    port: Result<Arc<dyn Port>, gwz_sspi::ErrorKind>,
    qualification_direct: Option<bool>,
}
impl NativeCaller {
    cfg_if::cfg_if! { if #[cfg(test)] {
        pub(crate) fn refused_for_test(kind: gwz_sspi::ErrorKind) -> Self { Self { port: Err(kind), qualification_direct: None } }
    } }

    pub(crate) fn qualification_direct(&self) -> Option<bool> {
        self.qualification_direct
    }

    pub fn capture(supervisor: &Result<Arc<gwz_sspi::Supervisor>, gwz_sspi::ErrorKind>) -> Self {
        let port = supervisor
            .as_ref()
            .map_err(|kind| *kind)
            .and_then(|supervisor| {
                supervisor
                    .capture_caller()
                    .map(|caller| {
                        Arc::new(NativePort {
                            supervisor: supervisor.clone(),
                            caller,
                        }) as Arc<dyn Port>
                    })
                    .map_err(|error| error.kind())
            });
        let qualification_direct =
            crate::transport_host::endpoint_environment::capture_qualification_proxy();
        Self {
            port,
            qualification_direct,
        }
    }
}
type Work<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
trait Probe: Send {
    fn confirmed(&self) -> bool;
}
struct NativeProbe {
    supervisor: Arc<gwz_sspi::Supervisor>,
    id: gwz_sspi::RecordId,
}
impl Probe for NativeProbe {
    fn confirmed(&self) -> bool {
        self.supervisor.cleanup_status(self.id.clone()) == gwz_sspi::CleanupStatus::Confirmed
    }
}
struct BridgeError {
    code: ErrorCode,
    pending: Option<Box<dyn Probe>>,
}
trait Session: Send {
    fn step(
        &mut self,
        challenge: Option<gwz_sspi::SecretBytes>,
    ) -> Work<'_, Result<gwz_sspi::TokenStep, BridgeError>>;
    fn finish(self: Box<Self>) -> Work<'static, Result<(), BridgeError>>;
    fn cancel(self: Box<Self>) -> Option<Box<dyn Probe>>;
}
trait Port: Send + Sync {
    fn start(
        &self,
        request: gwz_sspi::AuthRequest,
        deadline: Instant,
        cancel: gwz_sspi::Cancellation,
    ) -> Work<'static, Result<Box<dyn Session>, BridgeError>>;
}
struct NativePort {
    supervisor: Arc<gwz_sspi::Supervisor>,
    caller: gwz_sspi::CallerCapture,
}
fn native_code(kind: gwz_sspi::ErrorKind) -> ErrorCode {
    match kind {
        gwz_sspi::ErrorKind::Timeout => ErrorCode::Timeout,
        gwz_sspi::ErrorKind::Cancelled | gwz_sspi::ErrorKind::Closed => ErrorCode::Cancelled,
        gwz_sspi::ErrorKind::IdentityMismatch => ErrorCode::Authentication,
        gwz_sspi::ErrorKind::UnsupportedPlatform => ErrorCode::UnsupportedOperation,
        gwz_sspi::ErrorKind::InvalidRequest => ErrorCode::InvalidRequest,
        gwz_sspi::ErrorKind::WorkerUnavailable | gwz_sspi::ErrorKind::WorkerMismatch => {
            ErrorCode::Unavailable
        }
        _ => ErrorCode::Protocol,
    }
}
fn bridge_error(error: gwz_sspi::Failure, supervisor: &Arc<gwz_sspi::Supervisor>) -> BridgeError {
    let pending = error
        .record_id()
        .filter(|_| error.cleanup_status() != gwz_sspi::CleanupStatus::Confirmed)
        .map(|id| {
            Box::new(NativeProbe {
                supervisor: supervisor.clone(),
                id: id.clone(),
            }) as Box<dyn Probe>
        });
    BridgeError {
        code: native_code(error.kind()),
        pending,
    }
}
impl Port for NativePort {
    fn start(
        &self,
        request: gwz_sspi::AuthRequest,
        deadline: Instant,
        cancel: gwz_sspi::Cancellation,
    ) -> Work<'static, Result<Box<dyn Session>, BridgeError>> {
        let work = self.supervisor.start_captured(
            &self.caller,
            request,
            gwz_sspi::Deadline::new(deadline.into_std()),
            cancel,
        );
        let supervisor = self.supervisor.clone();
        Box::pin(async move {
            work.await
                .map(|conversation| {
                    Box::new(NativeSession {
                        supervisor: supervisor.clone(),
                        conversation,
                    }) as Box<dyn Session>
                })
                .map_err(|error| bridge_error(error, &supervisor))
        })
    }
}
struct NativeSession {
    supervisor: Arc<gwz_sspi::Supervisor>,
    conversation: gwz_sspi::Conversation,
}
impl Session for NativeSession {
    fn step(
        &mut self,
        challenge: Option<gwz_sspi::SecretBytes>,
    ) -> Work<'_, Result<gwz_sspi::TokenStep, BridgeError>> {
        Box::pin(async move {
            self.conversation
                .step(challenge)
                .await
                .map_err(|error| bridge_error(error, &self.supervisor))
        })
    }
    fn finish(self: Box<Self>) -> Work<'static, Result<(), BridgeError>> {
        Box::pin(async move {
            self.conversation
                .finish()
                .await
                .map_err(|error| bridge_error(error, &self.supervisor))
        })
    }
    fn cancel(self: Box<Self>) -> Option<Box<dyn Probe>> {
        let NativeSession {
            supervisor,
            conversation,
        } = *self;
        let receipt = conversation.cancel();
        (receipt.cleanup != gwz_sspi::CleanupStatus::Confirmed).then(|| {
            Box::new(NativeProbe {
                supervisor,
                id: receipt.record_id,
            }) as Box<dyn Probe>
        })
    }
}
pub(super) struct Pending {
    probe: Box<dyn Probe>,
    _operation: super::super::https_operation::Dependency,
    _slot: OwnedSemaphorePermit,
}
#[derive(Default)]
pub(super) struct CleanupState {
    records: Vec<Pending>,
    checking: usize,
}
pub(super) type Cleanup = Arc<Mutex<CleanupState>>;
pub(super) fn reap(owner: &Cleanup) -> usize {
    // Metadata checks and final dependency drops occur outside the owner lock.
    let (records, claimed) = {
        let mut state = owner.lock().unwrap_or_else(|p| p.into_inner());
        let records = std::mem::take(&mut state.records);
        let claimed = records.len();
        state.checking += claimed;
        (records, claimed)
    };
    let mut pending = Vec::new();
    for record in records {
        if !record.probe.confirmed() {
            pending.push(record);
        }
    }
    let mut state = owner.lock().unwrap_or_else(|p| p.into_inner());
    state.records.extend(pending);
    state.checking -= claimed;
    state.records.len() + state.checking
}
// The owned Finish stays pollable after its enclosing task disappears. Poll and
// receipt checks happen outside this shared holder's lock; the endpoint reaper
// supplies subsequent polls without introducing another process/runtime owner.
struct FinishState {
    work: Option<Work<'static, Result<(), BridgeError>>>,
    outcome: Option<Result<(), BridgeError>>,
}
#[derive(Clone)]
struct Finishing(Arc<Mutex<FinishState>>);
impl Finishing {
    fn new(work: Work<'static, Result<(), BridgeError>>) -> Self {
        Self(Arc::new(Mutex::new(FinishState {
            work: Some(work),
            outcome: None,
        })))
    }
    fn advance(&self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), ErrorCode>> {
        let work = self.0.lock().unwrap_or_else(|p| p.into_inner()).work.take();
        if let Some(mut work) = work {
            let polled = work.as_mut().poll(cx);
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            match polled {
                std::task::Poll::Pending => state.work = Some(work),
                std::task::Poll::Ready(result) => state.outcome = Some(result),
            }
        }
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match &state.outcome {
            Some(Ok(())) => std::task::Poll::Ready(Ok(())),
            Some(Err(error)) => std::task::Poll::Ready(Err(error.code)),
            None => std::task::Poll::Pending,
        }
    }
}
impl std::future::Future for Finishing {
    type Output = Result<(), ErrorCode>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.advance(cx)
    }
}
impl Probe for Finishing {
    fn confirmed(&self) -> bool {
        let _ = self.advance(&mut std::task::Context::from_waker(std::task::Waker::noop()));
        let outcome = self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .outcome
            .take();
        let confirmed = match &outcome {
            Some(Ok(())) => true,
            Some(Err(error)) => error.pending.as_ref().is_none_or(|probe| probe.confirmed()),
            None => false,
        };
        self.0.lock().unwrap_or_else(|p| p.into_inner()).outcome = outcome;
        confirmed
    }
}
// Start owns registration/launch/Hello even after enclosing preparation Drop.
// Late conversations are cancelled; no publication is available to the reaper.
struct StartState {
    work: Option<Work<'static, Result<Box<dyn Session>, BridgeError>>>,
    outcome: Option<Result<Box<dyn Session>, BridgeError>>,
    cleanup: Option<Box<dyn Probe>>,
    disposing: bool,
    disposed: bool,
}
#[derive(Clone)]
struct Starting(Arc<Mutex<StartState>>);
impl Starting {
    fn new(work: Work<'static, Result<Box<dyn Session>, BridgeError>>) -> Self {
        Self(Arc::new(Mutex::new(StartState {
            work: Some(work),
            outcome: None,
            cleanup: None,
            disposing: false,
            disposed: false,
        })))
    }
    fn advance(&self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), ErrorCode>> {
        let work = self.0.lock().unwrap_or_else(|p| p.into_inner()).work.take();
        if let Some(mut work) = work {
            let result = work.as_mut().poll(cx);
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            match result {
                std::task::Poll::Pending => state.work = Some(work),
                std::task::Poll::Ready(outcome) => state.outcome = Some(outcome),
            }
        }
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match &state.outcome {
            Some(Ok(_)) => std::task::Poll::Ready(Ok(())),
            Some(Err(error)) => std::task::Poll::Ready(Err(error.code)),
            None => std::task::Poll::Pending,
        }
    }
    fn take_session(&self) -> Box<dyn Session> {
        match self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .outcome
            .take()
        {
            Some(Ok(session)) => session,
            _ => unreachable!("Start admitted a session"),
        }
    }
}
impl std::future::Future for Starting {
    type Output = Result<(), ErrorCode>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.advance(cx)
    }
}
impl Probe for Starting {
    fn confirmed(&self) -> bool {
        let _ = self.advance(&mut std::task::Context::from_waker(std::task::Waker::noop()));
        let outcome = {
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            if state.disposing {
                return false;
            }
            let outcome = state.outcome.take();
            if outcome.is_some() {
                state.disposing = true;
            }
            outcome
        };
        if let Some(outcome) = outcome {
            let cleanup = match outcome {
                Ok(session) => session.cancel(),
                Err(error) => error.pending,
            };
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            state.cleanup = cleanup;
            state.disposing = false;
            state.disposed = true;
        }
        let (disposed, cleanup) = {
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            if state.disposing {
                return false;
            }
            state.disposing = true;
            (state.disposed, state.cleanup.take())
        };
        let confirmed = disposed && cleanup.as_ref().is_none_or(|probe| probe.confirmed());
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.cleanup = cleanup;
        state.disposing = false;
        confirmed
    }
}
struct Guard {
    starting: Option<Starting>,
    session: Option<Box<dyn Session>>,
    finishing: Option<Finishing>,
    cancellation: gwz_sspi::Cancellation,
    cleanup: Cleanup,
    operation: Option<super::super::https_operation::Dependency>,
    slot: Option<OwnedSemaphorePermit>,
}
impl Guard {
    fn retain(&mut self, pending: Option<Box<dyn Probe>>) {
        if let Some(probe) = pending
            && let Some(operation) = self.operation.take()
        {
            self.cleanup
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .records
                .push(Pending {
                    probe,
                    _operation: operation,
                    _slot: self.slot.take().expect("charged native cleanup"),
                });
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(starting) = self.starting.take() {
            self.retain(Some(Box::new(starting)));
        }
        if let Some(session) = self.session.take() {
            self.retain(session.cancel());
        }
        if let Some(finishing) = self.finishing.take() {
            self.retain(Some(Box::new(finishing)));
        }
    }
}
/// Route publication authorizes only this scoped physical generation.
pub(crate) struct Authenticated {
    pub(super) generation: String,
    pub(super) scope: String,
    pub(super) facts: Facts,
    pool: pool::Pool,
    revoked: AtomicBool,
}
impl Authenticated {
    pub(super) fn usable(&self, lease: &HttpLease) -> bool {
        !self.revoked.load(Ordering::Acquire)
            && self.generation == lease.id
            && !lease.cancel.is_cancelled()
    }
    pub(crate) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.pool.retire_https_scope(&self.scope);
    }
}
impl Drop for Authenticated {
    fn drop(&mut self) {
        self.revoke();
    }
}
fn deadline(budget: &Budget) -> Result<Instant, ErrorCode> {
    let deadline = budget
        .logical_deadline
        .ok_or(ErrorCode::UnsupportedOperation)?;
    if Instant::now() >= deadline {
        return Err(ErrorCode::Timeout);
    }
    Ok(deadline)
}
fn control(cancel: &CancellationToken, lease: &HttpLease, until: Instant) -> Result<(), ErrorCode> {
    if cancel.is_cancelled() || lease.cancel.is_cancelled() {
        return Err(ErrorCode::Cancelled);
    }
    if Instant::now() >= until {
        return Err(ErrorCode::Timeout);
    }
    Ok(())
}
impl Client {
    pub(crate) fn set_native(&mut self, caller: NativeCaller) {
        self.native = Some(caller);
    }
    pub(super) async fn authenticate(
        &self,
        prepared: &mut Prepared,
        mut response: Response<Incoming>,
        key: &RouteKey,
        budget: &mut Budget,
        cancel: &CancellationToken,
    ) -> Result<Response<Incoming>, Failure> {
        let offers = Offers::parse(response.headers()).map_err(failure)?;
        // OD10: only NTLM/Basic/Digest trigger a helper; Negotiate alone does not.
        let credential =
            if prepared.input.policy == AuthPolicy::WindowsConfigured && offers.helper_allowed() {
                let fixed = budget.logical_deadline;
                let lookup = self.credential(key, &prepared.destination, budget, cancel);
                let result = match fixed {
                    Some(until) => tokio::time::timeout_at(until, lookup)
                        .await
                        .map_err(|_| failure(ErrorCode::Timeout))?,
                    None => lookup.await,
                };
                match result {
                    Ok(credential) => credential.has_native_identity().then_some(credential),
                    // Windows parity/TR1.6 §4: unusable helper output and absent
                    // or unstartable git are absence before credential publication.
                    // Io (including CleanupPending), timeout and cancel stay terminal.
                    Err(error)
                        if error.code == ErrorCode::Unavailable
                            || (error.code == ErrorCode::Authentication
                                && error.facts.as_ref().is_some_and(|facts| {
                                    !facts.credential_offered && facts.authenticated != Some(false)
                                })) =>
                    {
                        None
                    }
                    Err(error) => return Err(error),
                }
            } else {
                None
            };
        let source = if credential.is_some() {
            NativeSource::Configured
        } else {
            NativeSource::CurrentLogon
        };
        let Some(offer) = offers.select(credential.is_some()) else {
            if offers.basic
                && let Some(credential) = credential
            {
                prepared.opened.facts = Facts {
                    method: AuthMethod::Gh,
                    ..Default::default()
                };
                prepared.lease.as_ref().unwrap().scope(&credential.scope)?;
                prepared.authorization = Some(credential.header());
                prepared.credential = Some(credential);
                let response = self.auth_send(prepared, response, budget, cancel).await?;
                prepared.opened.facts.credential_offered = true;
                if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                    // Basic follows TR1.6 OQ4(a): validated discovery at the new
                    // location, with no forwarded header and a new helper lookup.
                    return Ok(response);
                }
                if response.status() != 200 {
                    prepared
                        .credential
                        .as_ref()
                        .unwrap()
                        .rejected
                        .store(true, Ordering::Release);
                    prepared.opened.facts.authenticated = Some(false);
                    return Err(with_facts(
                        ErrorCode::Authentication,
                        Effect::None,
                        &prepared.opened.facts,
                    ));
                }
                validate_content(&response, prepared.input.service)
                    .map_err(|code| with_facts(code, Effect::None, &prepared.opened.facts))?;
                self.routes
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .basic_install(key)
                    .map_err(failure)?;
                return Ok(response);
            }
            return Err(with_facts(
                ErrorCode::Authentication,
                Effect::None,
                &prepared.opened.facts,
            ));
        };
        let mut history = History::new(source, offer.scheme);
        prepared.opened.facts = history.facts.clone();
        let until =
            deadline(budget).map_err(|code| with_facts(code, Effect::None, &history.facts))?;
        offer
            .initial()
            .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
        let url = prepared.destination.request(prepared.input.service);
        let overhead = url[url::Position::BeforePath..].len() + prepared.destination.authority().len()
            + https_policy::response_type(prepared.input.service).len()
            + b"GET  HTTP/1.1\r\nHost: \r\nAccept: \r\nAuthorization: \r\nTransfer-Encoding: chunked\r\n\r\n".len();
        let available = 65536usize
            .checked_sub(overhead)
            .ok_or_else(|| failure(ErrorCode::InvalidRequest))?;
        let limit = raw_limit(available, offer.scheme).map_err(failure)?;
        let caller = self
            .native
            .as_ref()
            .ok_or_else(|| with_facts(ErrorCode::Unavailable, Effect::None, &history.facts))?;
        let port = caller
            .port
            .as_ref()
            .map_err(|kind| with_facts(native_code(*kind), Effect::None, &history.facts))?;
        let connection = prepared
            .lease
            .as_ref()
            .unwrap()
            .connection
            .as_ref()
            .unwrap()
            .clone();
        let binding = connection
            .lock()
            .await
            .binding
            .as_ref()
            .map(|bytes| gwz_sspi::SecretBytes::new(bytes.as_bytes()))
            .filter(|bytes| !bytes.as_bytes().is_empty())
            .ok_or_else(|| {
                with_facts(
                    ErrorCode::UnsupportedOperation,
                    Effect::None,
                    &history.facts,
                )
            })?;
        let request = gwz_sspi::AuthRequest {
            package: if offer.scheme == NativeScheme::Negotiate {
                gwz_sspi::Package::Negotiate
            } else {
                gwz_sspi::Package::Ntlm
            },
            target: gwz_sspi::SecretText::new(&format!("HTTP/{}", prepared.destination.host()))
                .map_err(|_| failure(ErrorCode::InvalidRequest))?,
            identity: credential
                .as_ref()
                .map_or(Ok(gwz_sspi::Identity::CurrentLogon), |credential| {
                    credential.native_identity()
                })
                .map_err(failure)?,
            channel_binding: binding,
            token_limit: limit,
            digest: None,
        };
        let mut guard = Guard {
            starting: None,
            session: None,
            finishing: None,
            cancellation: gwz_sspi::Cancellation::new(),
            cleanup: self.native_cleanup.clone(),
            operation: Some(self.operation(&prepared.input.operation).map_err(failure)?),
            slot: prepared._slot.take(),
        };
        let starting = Starting::new(port.start(request, until, guard.cancellation.clone()));
        guard.starting = Some(starting.clone());
        let result = tokio::select! {
            result = starting => result,
            _ = cancel.cancelled() => Err(ErrorCode::Cancelled),
            _ = prepared.lease.as_ref().unwrap().cancel.cancelled() => Err(ErrorCode::Cancelled),
            _ = tokio::time::sleep_until(until) => Err(ErrorCode::Timeout),
        };
        if let Err(code) = result {
            return Err(with_facts(code, Effect::None, &history.facts));
        }
        guard.session = Some(guard.starting.as_ref().unwrap().take_session());
        guard.starting = None;
        let scope = self.ids.unique().to_string();
        prepared.lease.as_ref().unwrap().scope(&scope)?;
        let mut challenge = None;
        for _ in 0..8 {
            control(cancel, prepared.lease.as_ref().unwrap(), until)
                .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
            let step = guard.session.as_mut().unwrap().step(challenge.take());
            let result = tokio::select! {
                result = step => result,
                _ = cancel.cancelled() => return Err(with_facts(ErrorCode::Cancelled, Effect::None, &history.facts)),
                _ = prepared.lease.as_ref().unwrap().cancel.cancelled() => return Err(with_facts(ErrorCode::Cancelled, Effect::None, &history.facts)),
                _ = tokio::time::sleep_until(until) => return Err(with_facts(ErrorCode::Timeout, Effect::None, &history.facts)),
            };
            let step = match result {
                Ok(step) => step,
                Err(error) => {
                    guard.retain(error.pending);
                    return Err(with_facts(error.code, Effect::None, &history.facts));
                }
            };
            history
                .observe(&step)
                .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
            // Complete with an empty final token may still consume the prior response.
            if !step.payload.as_bytes().is_empty() {
                let header =
                    Header::token(offer.scheme, step.payload.as_bytes()).map_err(failure)?;
                prepared.authorization = Some(https_auth::SecretHeader::from_bytes(header.bytes()));
                prepared.opened.facts = history.facts.clone();
                response = self.auth_send(prepared, response, budget, cancel).await?;
                history.facts = prepared.opened.facts.clone();
            }
            control(cancel, prepared.lease.as_ref().unwrap(), until)
                .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
            let status = response.status().as_u16();
            history.reject(status);
            prepared.opened.facts = history.facts.clone();
            if status == 200 {
                validate_content(&response, prepared.input.service)
                    .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
                if !history.complete {
                    let final_offers = Offers::parse(response.headers())
                        .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
                    let final_offer = final_offers
                        .native
                        .iter()
                        .find(|next| next.scheme == offer.scheme)
                        .ok_or_else(|| {
                            with_facts(ErrorCode::Protocol, Effect::None, &history.facts)
                        })?;
                    challenge = Some(
                        decode_challenge(final_offer.token.bytes(), limit.raw_bytes() as usize)
                            .map_err(|code| with_facts(code, Effect::None, &history.facts))?,
                    );
                    continue;
                }
                history
                    .accept(status)
                    .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
                let finishing = Finishing::new(guard.session.take().unwrap().finish());
                guard.finishing = Some(finishing.clone());
                let result = tokio::select! {
                    result = finishing => result,
                    _ = cancel.cancelled() => { guard.cancellation.cancel(); Err(ErrorCode::Cancelled) },
                    _ = prepared.lease.as_ref().unwrap().cancel.cancelled() => { guard.cancellation.cancel(); Err(ErrorCode::Cancelled) },
                    _ = tokio::time::sleep_until(until) => { guard.cancellation.cancel(); Err(ErrorCode::Timeout) },
                };
                if let Err(code) = result {
                    return Err(with_facts(code, Effect::None, &history.facts));
                }
                guard.finishing = None;
                control(cancel, prepared.lease.as_ref().unwrap(), until)
                    .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
                prepared.opened.facts = history.facts.clone();
                let authenticated = Arc::new(Authenticated {
                    generation: prepared.lease.as_ref().unwrap().id.clone(),
                    scope,
                    facts: history.facts,
                    pool: self.pool.pool.clone(),
                    revoked: AtomicBool::new(false),
                });
                self.routes
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .native_install(key, authenticated.clone())
                    .map_err(failure)?;
                prepared.native_route = Some(authenticated);
                prepared._slot = guard.slot.take();
                return Ok(response);
            }
            if status != 401 || history.complete {
                return Err(with_facts(
                    ErrorCode::Authentication,
                    Effect::None,
                    &history.facts,
                ));
            }
            let next = Offers::parse(response.headers())
                .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
            let offer = next
                .native
                .iter()
                .find(|next| next.scheme == offer.scheme)
                .ok_or_else(|| {
                    with_facts(ErrorCode::Authentication, Effect::None, &history.facts)
                })?;
            challenge = Some(
                decode_challenge(offer.token.bytes(), limit.raw_bytes() as usize)
                    .map_err(|code| with_facts(code, Effect::None, &history.facts))?,
            );
        }
        Err(with_facts(
            ErrorCode::Protocol,
            Effect::None,
            &history.facts,
        ))
    }
    async fn auth_send(
        &self,
        prepared: &mut Prepared,
        response: Response<Incoming>,
        budget: &mut Budget,
        cancel: &CancellationToken,
    ) -> Result<Response<Incoming>, Failure> {
        let connection = prepared
            .lease
            .as_ref()
            .unwrap()
            .connection
            .as_ref()
            .unwrap()
            .clone();
        let fixed = budget.logical_deadline;
        let draining = prepare::clean_challenge(
            response,
            &connection,
            cancel,
            &prepared.lease.as_ref().unwrap().cancel,
            budget,
        );
        let clean = match fixed {
            Some(until) => tokio::time::timeout_at(until, draining)
                .await
                .map_err(|_| {
                    with_facts(ErrorCode::Timeout, Effect::None, &prepared.opened.facts)
                })?,
            None => draining.await,
        }
        .map_err(|code| with_facts(code, Effect::None, &prepared.opened.facts))?;
        if !clean {
            return Err(with_facts(
                ErrorCode::Authentication,
                Effect::None,
                &prepared.opened.facts,
            ));
        }
        let (tx, body) = body_channel();
        drop(tx);
        let request = prepared.request(body)?;
        let mut connection = connection.lock().await;
        let started = Instant::now();
        let result = tokio::select! {
            _ = cancel.cancelled() => Err(ErrorCode::Cancelled),
            _ = prepared.lease.as_ref().unwrap().cancel.cancelled() => Err(ErrorCode::Cancelled),
            result = async {
                if let Some(until) = budget.logical_deadline { control(cancel, prepared.lease.as_ref().unwrap(), until)?; }
                prepared.opened.facts.credential_offered |= request.headers().contains_key(AUTHORIZATION);
                let send = connection.sender.send_request(request);
                let send = async {
                    match budget.network {
                        Some(remaining) => tokio::time::timeout(remaining, send).await.map_err(|_| ErrorCode::Timeout)?,
                        None => send.await,
                    }.map_err(|error| classify_hyper_error(&error))
                };
                match budget.logical_deadline {
                    Some(until) => tokio::time::timeout_at(until, send).await.map_err(|_| ErrorCode::Timeout)?,
                    None => send.await,
                }
            } => result,
        };
        if let Some(remaining) = budget.network.as_mut() {
            *remaining = remaining.saturating_sub(started.elapsed());
        }
        prepared.io_ms = budget.network.map_or(0, prepare::duration_ms);
        let response =
            result.map_err(|code| with_facts(code, Effect::None, &prepared.opened.facts))?;
        prepared.opened.facts.http_status = Some(response.status().as_u16() as i64);
        if prepared.protocol_error.load(Ordering::Acquire) {
            return Err(with_facts(
                ErrorCode::Protocol,
                Effect::None,
                &prepared.opened.facts,
            ));
        }
        Ok(response)
    }
}

cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
    pub(crate) fn publication_fixture() -> NativeCaller { tests::publication_caller() }
    impl Authenticated {
        pub(crate) fn revoked_for_test(&self) -> bool { self.revoked.load(Ordering::Acquire) }
    }
} }
cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
mod tests {
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
    fn production_pool_wait_cannot_extend_fixed_deadline() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let server = Server::start(Arc::new(|_| Box::pin(async { response(200, GitService::UploadPackAdvertisement, Bytes::new()) }))).await;
            let endpoint = Endpoint::new(server.config(), None, pool::Config { total: 1, per_host: 1, per_user_host: 1,
                allocation_timeout_ms: 500, connect_timeout_ms: 500, ..Default::default() }).unwrap();
            let (held, _) = endpoint.client.prepare_attempt(input(&server, GitService::UploadPackAdvertisement), &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
            let held = held.unwrap();
            let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
            let started = Instant::now(); let mut budget = endpoint.client.budget(); budget.connect = Some(Duration::from_millis(30));
            let (result, connect) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut budget, &mut None).await;
            let error = result.err().unwrap(); assert_eq!(error.code, ErrorCode::Timeout); assert!(error.setup_cause.is_none());
            assert_eq!(connect, FirstConnect::None); assert!(started.elapsed() < Duration::from_millis(200)); drop(held);
        });
    }
    #[test]
    fn production_source_matrix_uses_current_logon_only_when_no_identity() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            for (offer, policy, answer, source) in [
                ("Negotiate", AuthPolicy::WindowsConfigured, "exit 17", NativeSource::CurrentLogon),
                ("Basic realm=x, NTLM, Negotiate", AuthPolicy::WindowsDefault, "exit 17", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "printf 'username=fixture@example.test\npassword=fixture\n\n'", NativeSource::Configured),
                ("NTLM", AuthPolicy::WindowsConfigured, "printf 'username=\npassword=fixture\n\n'", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "printf '\n'", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "NO_EXECUTABLE", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "exit 17", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "printf 'username=a\nusername=b\npassword=x\n\n'", NativeSource::CurrentLogon),
                ("NTLM", AuthPolicy::WindowsConfigured, "UNSTARTABLE", NativeSource::CurrentLogon),
            ] {
                let server = Server::start(Arc::new(move |request| Box::pin(async move {
                    let mut result = response(if request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                    if result.status() == 401 { result.headers_mut().insert(WWW_AUTHENTICATE, offer.parse().unwrap()); } result
                }))).await;
                let dir = tempfile::tempdir().unwrap(); let helper = dir.path().join("git");
                crate::git::endpoint::helper_script::write_git_fixture(&helper, answer);
                if answer == "UNSTARTABLE" { std::fs::write(&helper, b"invalid executable fixture").unwrap(); }
                let auth = (answer != "NO_EXECUTABLE").then_some(https_auth::Config { executable: helper, environment: Vec::new() });
                let mut endpoint = Endpoint::new(server.config(), auth, pool::Config::default()).unwrap();
                let (caller, port) = fake(true, 2); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = policy;
                let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                let prepared = prepared.unwrap(); assert_eq!(prepared.opened.facts.native.as_ref().unwrap().source, source);
                assert_eq!(port.starts.load(Ordering::Acquire), 1); drop(prepared); endpoint.client.finish_operation("operation");
                assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
            }
        });
    }
    #[test]
    fn production_round_cap_and_http_allowance_do_not_reset_native_deadline() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            for (io, delay, expected) in [(9000, 0, ErrorCode::Protocol), (60, 40, ErrorCode::Timeout)] {
                let server = Server::start(Arc::new(move |request| Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
                    result.headers_mut().insert(WWW_AUTHENTICATE, if request.headers().contains_key(AUTHORIZATION) { "NTLM YQ==" } else { "NTLM" }.parse().unwrap()); result
                }))).await;
                let mut endpoint = Endpoint::new_with_io_timeout(server.config(), None, pool::Config::default(), io).unwrap();
                let (caller, port) = fake(false, 2); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
                let mut budget = endpoint.client.budget();
                let (result, connect) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut budget, &mut None).await;
                let error = result.err().unwrap(); assert_eq!(error.code, expected); assert!(error.setup_cause.is_none()); assert_eq!(connect, FirstConnect::Connected);
                let facts = error.facts.unwrap(); assert!(facts.native.unwrap().authoritative); assert!(facts.credential_offered);
                assert_eq!(port.deadlines.lock().unwrap().as_slice(), &[budget.logical_deadline.unwrap()]);
                assert_eq!(port.starts.load(Ordering::Acquire), 1);
                if expected == ErrorCode::Protocol { assert_eq!(port.steps.load(Ordering::Acquire), 8); }
            }
        });
    }
    #[test]
    fn production_native_exchange_keeps_facts_and_refuses_replacement_before_post() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response, attach};
            for replace in [false, true] {
                let posts = Arc::new(AtomicUsize::new(0)); let observed = posts.clone();
                let server = Server::start(Arc::new(move |request| {
                    let posts = observed.clone(); Box::pin(async move {
                        let post = request.method() == "POST";
                        if post { posts.fetch_add(1, Ordering::Relaxed); }
                        let mut result = response(if post || request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 },
                            if post { GitService::UploadPackExchange } else { GitService::UploadPackAdvertisement }, Bytes::new());
                        if result.status() == 401 { result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); } result
                    })
                })).await;
                let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
                let (caller, _) = fake(true, 2); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
                let (prepared, _) = endpoint.client.prepare_attempt(request.clone(), &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                let prepared = prepared.unwrap(); let auth = prepared.native_route.clone().unwrap();
                let (stream, task) = attach(prepared); stream.end_write().await.unwrap(); let mut bytes = [0; 1];
                assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
                if replace { auth.revoke(); }
                request.service = GitService::UploadPackExchange;
                let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                if replace {
                    assert_eq!(prepared.err().unwrap().code, ErrorCode::Authentication); assert_eq!(posts.load(Ordering::Acquire), 0);
                } else {
                    let prepared = prepared.unwrap(); assert!(prepared.opened.facts.credential_offered); assert_eq!(prepared.opened.facts.authenticated, Some(true));
                    let (stream, task) = attach(prepared); stream.end_write().await.unwrap();
                    assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
                    assert_eq!(posts.load(Ordering::Acquire), 1); assert_eq!(server.connections.load(Ordering::Acquire), 1);
                }
            }
        });
    }
    #[test]
    fn production_final_remote_token_completes_without_extra_request() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response, attach};
            let requests = Arc::new(AtomicUsize::new(0)); let observed = requests.clone();
            let server = Server::start(Arc::new(move |request| {
                observed.fetch_add(1, Ordering::Relaxed); Box::pin(async move {
                    let authorized = request.headers().contains_key(AUTHORIZATION);
                    let mut result = response(if authorized { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::from_static(b"accepted"));
                    result.headers_mut().insert(WWW_AUTHENTICATE, if authorized { "NTLM YQ==" } else { "NTLM" }.parse().unwrap()); result
                })
            })).await;
            let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
            let (caller, port) = fake(false, 4); endpoint.client.set_native(caller);
            let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
            let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
            let prepared = prepared.unwrap(); assert_eq!(prepared.opened.facts.authenticated, Some(true));
            assert_eq!(port.steps.load(Ordering::Acquire), 2); assert_eq!(requests.load(Ordering::Acquire), 2);
            let (stream, task) = attach(prepared); stream.end_write().await.unwrap(); let mut bytes = [0; 8];
            assert_eq!(stream.read(&mut bytes).await.unwrap(), 8); assert_eq!(&bytes, b"accepted");
            assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
        });
    }
    #[test]
    fn production_helper_crossing_deadline_or_cancel_never_falls_back() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            for cancel_request in [false, true] {
                let server = Server::start(Arc::new(|_| Box::pin(async {
                    let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::new());
                    result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); result
                }))).await;
                let dir = tempfile::tempdir().unwrap(); let helper = dir.path().join("git"); let entered = dir.path().join("entered");
                let script = format!("touch '{}'; sleep 1; printf 'username=fixture\npassword=fixture\n\n'", entered.display());
                crate::git::endpoint::helper_script::write_git_fixture(&helper, &script);
                let mut endpoint = Endpoint::new(server.config(), Some(https_auth::Config { executable: helper, environment: Vec::new() }), pool::Config::default()).unwrap();
                let (caller, port) = fake(true, 2); endpoint.client.set_native(caller);
                let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsConfigured;
                let client = endpoint.client.clone(); let cancel = CancellationToken::new(); let signal = cancel.clone();
                let task = tokio::spawn(async move {
                    let mut budget = client.budget(); budget.connect = Some(Duration::from_millis(if cancel_request { 1000 } else { 250 }));
                    client.prepare_attempt(request, &cancel, &mut budget, &mut None).await
                });
                while !entered.exists() { tokio::time::sleep(Duration::from_millis(1)).await; }
                if cancel_request { signal.cancel(); }
                let (result, connect) = task.await.unwrap(); let error = result.err().unwrap();
                assert_eq!(error.code, if cancel_request { ErrorCode::Cancelled } else { ErrorCode::Timeout });
                assert_eq!(connect, FirstConnect::Connected); assert_eq!(port.starts.load(Ordering::Acquire), 0);
                endpoint.client.finish_operation("operation"); assert_eq!(endpoint.shutdown(Duration::from_secs(2)).await, 0);
            }
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
    #[test]
    fn production_configured_basic_redirect_requeries_without_forwarding_credentials() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let arrivals = Arc::new(AtomicUsize::new(0)); let seen = arrivals.clone();
            let target = Server::start(Arc::new(move |request| {
                seen.fetch_add(1, Ordering::Relaxed);
                let authorized = request.headers().contains_key(AUTHORIZATION);
                if seen.load(Ordering::Relaxed) == 1 { assert!(!authorized); }
                Box::pin(async move {
                    let mut result = response(if authorized { 200 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                    if !authorized { result.headers_mut().insert(WWW_AUTHENTICATE, "Basic realm=x".parse().unwrap()); }
                    result
                })
            })).await;
            let location = format!("{}/info/refs", target.url);
            let source = Server::start(Arc::new(move |request| {
                let location = location.clone(); Box::pin(async move {
                    let authorized = request.headers().contains_key(AUTHORIZATION);
                    let mut result = response(if authorized { 302 } else { 401 }, GitService::UploadPackAdvertisement, Bytes::new());
                    if authorized { result.headers_mut().insert(hyper::header::LOCATION, location.parse().unwrap()); }
                    else { result.headers_mut().insert(WWW_AUTHENTICATE, "Basic realm=x".parse().unwrap()); }
                    result
                })
            })).await;
            let dir = tempfile::tempdir().unwrap(); let helper = dir.path().join("git");
            crate::git::endpoint::helper_script::write_git_fixture(&helper, "printf 'username=fixture\npassword=fixture\n\n'");
            let endpoint = Endpoint::new(source.config(), Some(https_auth::Config { executable: helper, environment: Vec::new() }), pool::Config::default()).unwrap();
            let mut request = input(&source, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsConfigured;
            let mut budget = endpoint.client.budget();
            let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut budget, &mut None).await;
            let prepared = prepared.unwrap(); assert!(prepared.opened.facts.credential_offered);
            assert_eq!(prepared.opened.facts.authenticated, None); assert!(prepared.opened.facts.native.is_none());
            assert_eq!(arrivals.load(Ordering::Acquire), 2); assert_eq!(budget.redirect_hops, 1);
        });
    }
    #[test]
    fn production_anonymous_advertisement_and_post_do_not_require_native_availability() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response, attach};
            for policy in [AuthPolicy::WindowsDefault, AuthPolicy::WindowsConfigured] {
                let server = Server::start(Arc::new(|request| Box::pin(async move {
                    assert!(!request.headers().contains_key(AUTHORIZATION));
                    response(200, if request.method() == "POST" { GitService::UploadPackExchange } else { GitService::UploadPackAdvertisement }, Bytes::new())
                }))).await;
                let endpoint = Endpoint::new(server.config(), None, pool::Config { connect_timeout_ms: 0, ..Default::default() }).unwrap();
                for service in [GitService::UploadPackAdvertisement, GitService::UploadPackExchange] {
                    let mut request = input(&server, service); request.policy = policy;
                    let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                    let prepared = prepared.unwrap(); assert!(prepared.opened.facts.native.is_none()); assert!(!prepared.opened.facts.credential_offered);
                    let (stream, task) = attach(prepared); stream.end_write().await.unwrap(); let mut bytes = [0; 1];
                    assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
                }
            }
        });
    }
    #[test]
    fn production_configured_basic_stays_usable_without_native_availability() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response, attach};
            let post = Arc::new(AtomicUsize::new(0)); let observed = post.clone();
            let server = Server::start(Arc::new(move |request| {
                let post = observed.clone(); Box::pin(async move {
                    let service = if request.method() == "POST" { post.fetch_add(1, Ordering::Relaxed); GitService::UploadPackExchange } else { GitService::UploadPackAdvertisement };
                    response(if request.headers().contains_key(AUTHORIZATION) { 200 } else { 401 }, service, Bytes::new())
                })
            })).await;
            let dir = tempfile::tempdir().unwrap(); let helper = dir.path().join("git");
            crate::git::endpoint::helper_script::write_git_fixture(&helper, "printf 'username=fixture\\npassword=fixture\\n\\n'");
            let endpoint = Endpoint::new(server.config(), Some(https_auth::Config { executable: helper, environment: Vec::new() }), pool::Config::default()).unwrap();
            for service in [GitService::UploadPackAdvertisement, GitService::UploadPackExchange] {
                let mut request = input(&server, service); request.policy = AuthPolicy::WindowsConfigured;
                let (prepared, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
                let prepared = prepared.unwrap(); assert_eq!(prepared.opened.facts.method, AuthMethod::Gh);
                let (stream, task) = attach(prepared); stream.end_write().await.unwrap();
                let mut bytes = [0; 1]; assert_eq!(stream.read(&mut bytes).await.unwrap(), 0); stream.close().await.unwrap(); task.await.unwrap();
            }
            assert_eq!(post.load(Ordering::Acquire), 1);
        });
    }

    #[test]
    fn production_initial_nonempty_and_zero_refuse_without_publication() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input};
            for (timeout, token) in [(30000, "Negotiate c3ludGhldGlj"), (0, "Negotiate")] {
                let server = Server::start(Arc::new(move |_| Box::pin(async move {
                    hyper::Response::builder().status(401).header("WWW-Authenticate", token)
                        .body(http_body_util::Full::new(Bytes::new())).unwrap()
                }))).await;
                let endpoint = Endpoint::new(server.config(), None, pool::Config { connect_timeout_ms: timeout, ..Default::default() }).unwrap();
                let mut input = input(&server, GitService::UploadPackAdvertisement);
                input.policy = AuthPolicy::WindowsDefault;
                let mut budget = endpoint.client.budget();
                let (result, _) = endpoint.client.prepare_attempt(input, &CancellationToken::new(), &mut budget, &mut None).await;
                let failed = result.err().expect("native refused");
                assert_eq!(failed.code, ErrorCode::UnsupportedOperation);
                let facts = failed.facts.unwrap();
                assert_eq!(facts.native.unwrap().observation, NativeObservation::NotStarted);
                assert!(!facts.credential_offered);
            }
        });
    }
    #[test]
    fn storage_wipes_before_deallocation_on_encoding_decode_parse_and_drop() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let probe = audit::Probe(Some(records.clone()));
        assert!(Header::encode(b"NTLM ", b"synthetic", Header::zeroed(6, probe.clone())).is_err());
        assert!(decode_owned(b"YR==", 2, probe.clone()).is_err());
        let mut headers = HeaderMap::new(); headers.insert(WWW_AUTHENTICATE, "NTLM YQ==, NTLM Yg==".parse().unwrap());
        assert!(Offers::parse_owned(&headers, probe.clone()).is_err());
        drop(Header::copy(b"synthetic", probe));
        let mut publication = https_auth::SecretHeader::from_bytes(b"synthetic");
        publication.observe_wipe(records.clone()); drop(publication);
        let records = records.lock().unwrap(); assert_eq!(records.len(), 5);
        assert!(records.iter().all(|(length, zero)| *length > 0 && *zero));
    }
    #[test]
    fn bounded_initial_token_refuses_before_native_begin_and_publication() {
        let mut headers = HeaderMap::new();
        headers.insert(WWW_AUTHENTICATE, "Negotiate c3ludGhldGlj".parse().unwrap());
        let offers = Offers::parse(&headers).unwrap();
        let selected = offers.select(false).unwrap();
        assert_eq!(selected.scheme, NativeScheme::Negotiate);
        let facts = History::new(NativeSource::CurrentLogon, selected.scheme).facts;
        assert_eq!(facts.native.as_ref().unwrap().observation, NativeObservation::NotStarted);
        assert!(!facts.credential_offered);
        assert_eq!(selected.initial().unwrap_err(), ErrorCode::UnsupportedOperation);
    }
    #[test]
    fn source_order_and_challenge_decode_are_closed_and_bounded() {
        let mut headers = HeaderMap::new();
        headers.insert(WWW_AUTHENTICATE, "Basic realm=\"x\", NTLM, Negotiate".parse().unwrap());
        let offers = Offers::parse(&headers).unwrap();
        assert!(offers.helper_allowed());
        assert_eq!(offers.select(true).unwrap().scheme, NativeScheme::Negotiate);
        assert_eq!(offers.select(false).unwrap().scheme, NativeScheme::Negotiate);
        assert_eq!(decode_challenge(b"YQ==", 1).unwrap().as_bytes(), b"a");
        assert!(decode_challenge(b"YQ==", 0).is_err());
        for invalid in [b"YQ".as_slice(), b"YR==", b" YQ==", b"YQ== ", b""] {
            assert!(decode_challenge(invalid, 8).is_err());
        }
    }
    #[test]
    fn authority_continue_is_preserved_but_does_not_authorize_remote_success() {
        let mut history = History::new(NativeSource::CurrentLogon, NativeScheme::Ntlm);
        let step = gwz_sspi::TokenStep {
            status: gwz_sspi::TokenStatus::Continue, attributes: 0,
            observation: gwz_sspi::MechanismObservation::Selected { mechanism: gwz_sspi::Mechanism::Ntlm, authoritative: true },
            payload: gwz_sspi::SecretBytes::new(b"synthetic"),
        };
        history.observe(&step).unwrap();
        assert!(history.facts.native.as_ref().unwrap().authoritative);
        assert_eq!(history.accept(200), Err(ErrorCode::Protocol));
        assert_eq!(history.facts.authenticated, None);
        history.facts.credential_offered = true;
        history.reject(401);
        assert_eq!(history.facts.authenticated, Some(false));
    }
    #[test]
    fn direct_ntlm_observation_cannot_be_unresolved_or_kerberos() {
        for observation in [gwz_sspi::MechanismObservation::Unresolved,
            gwz_sspi::MechanismObservation::Selected { mechanism: gwz_sspi::Mechanism::Kerberos, authoritative: true }] {
            let mut history = History::new(NativeSource::CurrentLogon, NativeScheme::Ntlm);
            let step = gwz_sspi::TokenStep { status: gwz_sspi::TokenStatus::Continue, attributes: 0,
                observation, payload: gwz_sspi::SecretBytes::new(b"synthetic") };
            assert_eq!(history.observe(&step), Err(ErrorCode::Protocol));
        }
    }
    #[test]
    fn provisional_resolution_cannot_switch_or_become_unresolved() {
        for observation in [gwz_sspi::MechanismObservation::Unresolved,
            gwz_sspi::MechanismObservation::Selected { mechanism: gwz_sspi::Mechanism::Kerberos, authoritative: false }] {
            let mut history = History::new(NativeSource::CurrentLogon, NativeScheme::Negotiate);
            let mut step = gwz_sspi::TokenStep { status: gwz_sspi::TokenStatus::Continue, attributes: 0,
                observation: gwz_sspi::MechanismObservation::Selected { mechanism: gwz_sspi::Mechanism::Ntlm, authoritative: false },
                payload: gwz_sspi::SecretBytes::new(b"synthetic") };
            history.observe(&step).unwrap(); step.observation = observation;
            assert_eq!(history.observe(&step), Err(ErrorCode::Protocol));
        }
    }
    #[test]
    fn production_drain_refusal_never_claims_publication() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use crate::git::endpoint::https_fixture::{Server, input, response};
            let sends = Arc::new(AtomicUsize::new(0)); let observed = sends.clone();
            let server = Server::start(Arc::new(move |request| {
                if request.headers().contains_key(AUTHORIZATION) { observed.fetch_add(1, Ordering::Relaxed); }
                Box::pin(async { let mut result = response(401, GitService::UploadPackAdvertisement, Bytes::from(vec![0; 65537]));
                    result.headers_mut().insert(WWW_AUTHENTICATE, "NTLM".parse().unwrap()); result })
            })).await;
            let mut endpoint = Endpoint::new(server.config(), None, pool::Config::default()).unwrap();
            let (caller, _) = fake(true, 2); endpoint.client.set_native(caller);
            let mut request = input(&server, GitService::UploadPackAdvertisement); request.policy = AuthPolicy::WindowsDefault;
            let (result, _) = endpoint.client.prepare_attempt(request, &CancellationToken::new(), &mut endpoint.client.budget(), &mut None).await;
            let error = result.err().unwrap(); assert_eq!(error.code, ErrorCode::Authentication);
            assert!(!error.facts.unwrap().credential_offered); assert_eq!(sends.load(Ordering::Acquire), 0);
        });
    }
    #[test]
    fn header_cap_accounts_for_scheme_and_base64_before_native_work() {
        assert_eq!(raw_limit(18, NativeScheme::Ntlm).unwrap().raw_bytes(), 9);
        assert!(raw_limit(8, NativeScheme::Negotiate).is_err());
        let header = Header::token(NativeScheme::Ntlm, b"synthetic").unwrap();
        assert_eq!(header.bytes(), b"NTLM c3ludGhldGlj");
    }
}
} }
