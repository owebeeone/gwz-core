//! Exclusive native HTTPS authentication; physical ownership stays in Prepared.
use super::*;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hyper::header::{HeaderMap, WWW_AUTHENTICATE};

mod exchange;
mod owners;
mod sspi;

pub use sspi::NativeCaller;

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

type Work<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
trait Probe: Send {
    fn confirmed(&self) -> bool;
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

cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
    pub(crate) fn publication_fixture() -> NativeCaller { tests::publication_caller() }
    impl Authenticated {
        pub(crate) fn revoked_for_test(&self) -> bool { self.revoked.load(Ordering::Acquire) }
    }
} }
cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
