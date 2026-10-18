//! Exclusive native HTTPS authentication; physical ownership stays in Prepared.
use super::*;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hyper::header::{HeaderMap, WWW_AUTHENTICATE};

mod exchange;
mod owners;
mod sspi;

/// The request rounds one operation's native exchanges may use (design 8).
pub(super) const NATIVE_ROUNDS: usize = 8;

pub use sspi::{NativeCaller, NativeHost};

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
        // The wire enum names Digest, but 1.1.0 never answers it (design 7): no offer holds it.
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
        if !self.token.bytes().is_empty() {
            return Err(ErrorCode::UnsupportedOperation);
        }
        Ok(())
    }
}
/// What a challenge offers that this transport answers: `Negotiate` and `NTLM` with the logon session, and
/// `Basic` with a configured helper. Anything else, `Digest` included, is not an offer.
struct Offers {
    native: Vec<Offer>,
    basic: bool,
}
impl Offers {
    fn parse(headers: &HeaderMap) -> Result<Self, ErrorCode> {
        Self::parse_owned(headers, audit::Probe::inert())
    }
    fn parse_owned(headers: &HeaderMap, probe: audit::Probe) -> Result<Self, ErrorCode> {
        let mut result = Self {
            native: Vec::new(),
            basic: false,
        };
        let mut total = 0usize;
        for value in headers.get_all(WWW_AUTHENTICATE) {
            total = total
                .checked_add(value.as_bytes().len())
                .ok_or(ErrorCode::Protocol)?;
            if total > 65536 {
                return Err(ErrorCode::Protocol);
            }
            // Commas inside quoted Basic/Digest parameters are not offer boundaries. The challenge is parsed on bytes:
            // a realm may hold obs-text or UTF-8, which `HeaderValue::to_str` refuses, and only a scheme token has to
            // be ASCII (a name that is not one is no scheme this transport answers).
            let text = value.as_bytes();
            let mut quoted = false;
            let mut escaped = false;
            let mut start = 0;
            for (i, byte) in text.iter().copied().enumerate() {
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
    fn part(&mut self, text: &[u8], probe: audit::Probe) -> Result<(), ErrorCode> {
        let text = text.trim_ascii();
        let (name, token) = match text.iter().position(u8::is_ascii_whitespace) {
            Some(at) => (&text[..at], &text[at + 1..]),
            None => (text, &[][..]),
        };
        let scheme = if name.eq_ignore_ascii_case(b"Negotiate") {
            Some(NativeScheme::Negotiate)
        } else if name.eq_ignore_ascii_case(b"NTLM") {
            Some(NativeScheme::Ntlm)
        } else {
            self.basic |= name.eq_ignore_ascii_case(b"Basic");
            None
        };
        if let Some(scheme) = scheme {
            if self.native.iter().any(|offer| offer.scheme == scheme) {
                return Err(ErrorCode::Protocol);
            }
            self.native.push(Offer {
                scheme,
                token: Header::copy(token.trim_ascii(), probe),
            });
        }
        Ok(())
    }
    /// The logon session's scheme: `Negotiate` first, then `NTLM` (design 7, TD1). The helper is not asked.
    fn select(&self) -> Option<&Offer> {
        [NativeScheme::Negotiate, NativeScheme::Ntlm]
            .into_iter()
            .find_map(|scheme| self.native.iter().find(|offer| offer.scheme == scheme))
    }
}
struct History {
    facts: Facts,
    complete: bool,
}
impl History {
    fn new(scheme: NativeScheme) -> Self {
        Self {
            facts: Facts {
                method: AuthMethod::Sspi,
                native: Some(NativeFacts {
                    source: NativeSource::CurrentLogon,
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
    /// Records that the provider refused the sign-in before any mechanism was seen (decision D48): the sign-in
    /// began and its mechanism is unresolved, which no other failure of `Protocol` with nothing sent has. The wire
    /// accepts that state for Negotiate only, so an NTLM refusal leaves its facts as they were.
    fn provider_refused(&mut self) {
        let native = self.facts.native.as_mut().unwrap();
        if native.scheme == NativeScheme::Negotiate && native.mechanism.is_none() {
            native.observation = NativeObservation::Unresolved;
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

type Work<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
trait Probe: Send {
    fn confirmed(&self) -> bool;
}
struct BridgeError {
    code: ErrorCode,
    pending: Option<Box<dyn Probe>>,
    /// The native provider refused the sign-in or exceeded a bound (`ErrorKind::ProviderRejected`), which `code`
    /// (`Protocol`) shares with a worker that broke its contract. The failure's facts carry the difference (D48).
    provider_refused: bool,
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
/// Route publication authorizes only this scoped physical generation.
pub(crate) struct Authenticated {
    pub(super) generation: String,
    pub(super) scope: String,
    pub(super) facts: Facts,
    pool: HttpsPool,
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

cfg_if::cfg_if! { if #[cfg(test)] {
    pub(crate) fn publication_fixture() -> NativeCaller { tests::publication_caller() }
    /// A caller whose logon session never completes, for a server reached by `host`, and the count of the steps
    /// every session of it has taken.
    pub(crate) fn counting_fixture(host: &'static str) -> (NativeCaller, Arc<AtomicUsize>) { tests::counting_caller(host) }
    /// `counting_fixture`, and the deadline D every exchange of it was started with.
    pub(crate) fn recording_fixture(host: &'static str) -> (NativeCaller, Arc<AtomicUsize>, Arc<Mutex<Vec<Instant>>>) { tests::recording_caller(host) }
    impl Authenticated {
        pub(crate) fn revoked_for_test(&self) -> bool { self.revoked.load(Ordering::Acquire) }
    }
} }
cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
