//! The operation's credential record (HTTPS model draft r5 section 3(b), decision b2; contract section 10, A8): which
//! keys of this operation needed a credential and, for a request-scoped one, the credential itself, in the
//! operation's memory. It is a field of [`super::OperationAuth`] and of nothing else: in no profile, pool key,
//! instance, `Scopes`, connection or adapter-shared state.
//!
//! No credential leaves the lock: a `Basic` answer is a [`SecretHeader`] built under it, and a credential's bytes are
//! wiped when the last entry holding it is replaced or rejected. Keys and rejection fingerprints are digests, and the
//! `Debug` is counts.
use super::{OUTPUT_LIMIT, Secret, SecretHeader};
use crate::git::endpoint::{https_destination::Destination, https_policy};
use gwz_transport::{
    pool::Key,
    protocol::{ErrorCode, GitService},
};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    mem::size_of,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

/// Entries of the key table, in any state.
pub(crate) const KEYS: usize = 512;
/// Distinct credentials retained.
pub(crate) const CREDENTIALS: usize = 64;
/// One credential, username and password: the helper's output limit, so a lookup cannot return more.
pub(crate) const CREDENTIAL_BYTES: usize = OUTPUT_LIMIT;
/// All retained credential bytes.
pub(crate) const RETAINED_BYTES: usize = 256 * 1024;
/// Rejected credentials' fingerprints; the next distinct one makes the record `Exhausted`.
pub(crate) const FINGERPRINTS: usize = 64;
/// The most the record retains, in bytes, with every bound full: the credentials' bytes, then per credential its
/// owner and shared count (and the username's terminator slot and allocator rounding of its two buffers), per key
/// and per fingerprint the table slot at twice its size (a `Vec` holds at most twice its length). `OperationAuth`'s
/// `AuthContext` charge adds it (contract section 10; row H1a-8).
pub(crate) const CHARGE: usize = size_of::<Needs>()
    + RETAINED_BYTES
    + CREDENTIALS * (size_of::<Held>() + 2 * size_of::<usize>() + 1 + 2 * 16)
    + KEYS * 2 * size_of::<(NeedKey, Need)>()
    + FINGERPRINTS * 2 * size_of::<Print>();
// About 350 KiB: the 72 admissions of a host, 8 active and 64 waiting, stay under its 64 MiB cap (contract section 10).
const _: () = assert!(CHARGE < 400 * 1024 && 72 * CHARGE < 64 << 20);

type Print = [u8; 32];
const KEY_DOMAIN: &[u8] = b"gwz-https-need-key/1";
const ORIGIN_DOMAIN: &[u8] = b"gwz-https-need-origin/1";
const FINGERPRINT_DOMAIN: &[u8] = b"gwz-https-need-fingerprint/1";

fn field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

/// One repository and service class at one origin (draft 3(a)): a digest over the pool key (scheme, username, host,
/// port), the repository path and the class, and a separate digest of the origin (scheme, host, port).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NeedKey {
    key: Print,
    origin: Print,
}
impl NeedKey {
    /// The key of `service` on the connections of `key` for the repository `destination` names. The repository path
    /// is the request path with its service tail removed, so an advertisement and its exchange share a key; it is
    /// read from the exchange request, whose tail is one segment, after `request` has dropped a trailing empty
    /// segment (`/repo` and `/repo/` are one key, `/repo.git` another).
    pub(crate) fn new(key: &Key, destination: &Destination, service: GitService) -> Self {
        let mut origin = Sha256::new();
        field(&mut origin, ORIGIN_DOMAIN);
        field(&mut origin, &key.scheme.wire().to_le_bytes());
        field(&mut origin, key.host.as_bytes());
        field(&mut origin, &key.port.to_le_bytes());
        let origin: Print = origin.finalize().into();
        let request = destination.request(GitService::UploadPackExchange);
        let repository = request.path().rsplit_once('/').map_or("", |(path, _)| path);
        let mut digest = Sha256::new();
        field(&mut digest, KEY_DOMAIN);
        field(&mut digest, &origin);
        match &key.username {
            Some(username) => {
                field(&mut digest, &[1]);
                field(&mut digest, username.as_bytes());
            }
            None => {
                field(&mut digest, &[0]);
            }
        }
        field(&mut digest, repository.as_bytes());
        field(
            &mut digest,
            &[u8::from(https_policy::receive_pack(service))],
        );
        Self {
            key: digest.finalize().into(),
            origin,
        }
    }
}

/// How a connection-bound need was learned (draft 3.1).
#[allow(
    dead_code,
    reason = "the connection-bound learning of HTTPS model draft r5 sub-step H3a records it, which removes this"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Evidence {
    /// The logon session answered the key's challenge (G8, RA1).
    Challenged,
    /// The key was served unchallenged on a `LogonSession` connection (G2): this proves nothing about its need.
    OnBound,
}

/// A retained credential, shared by every key of its origin that recorded equal bytes.
struct Held {
    secret: Secret,
    fingerprint: Print,
    bytes: usize,
}

enum Need {
    Basic(Arc<Held>),
    #[allow(
        dead_code,
        reason = "the connection-bound learning of HTTPS model draft r5 sub-step H3a records it, which removes this"
    )]
    ConnectionBound {
        observed: u64,
        evidence: Evidence,
    },
    Rejected,
    Unretained,
    /// A key of an `Exhausted` record that was not `Rejected`: its digest, and nothing else.
    Tombstone,
}

/// What the record says of one key. It holds no credential: `Basic` is the header, built under the record's lock.
#[derive(Debug)]
pub(crate) enum Recorded {
    Unrecorded,
    Basic(SecretHeader),
    #[allow(
        dead_code,
        reason = "the re-authentication of HTTPS model draft r5 sub-step H3b reads them, which removes this"
    )]
    ConnectionBound {
        observed: u64,
        evidence: Evidence,
    },
    Rejected,
    Unretained,
    Tombstone,
}

/// The record's sizes; nothing in it is a secret but `retained`, the credentials' total length, which `Debug` omits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) keys: usize,
    pub(crate) credentials: usize,
    pub(crate) retained: usize,
    pub(crate) fingerprints: usize,
    pub(crate) exhausted: bool,
}

struct Table {
    /// The per-operation HMAC key; `None` once the record is `Exhausted` (or when randomness failed at creation).
    mac: Option<Print>,
    entries: Vec<(NeedKey, Need)>,
    credentials: usize,
    retained: usize,
    rejected: Vec<Print>,
    probe: Probe,
}
impl Table {
    fn find(&self, key: &NeedKey) -> Option<usize> {
        self.entries.iter().position(|(entry, _)| entry == key)
    }
    fn need(&self, key: &NeedKey) -> Option<&Need> {
        self.find(key).map(|index| &self.entries[index].1)
    }
    /// Replaces `index`'s state, releasing a credential no other entry holds.
    fn replace(&mut self, index: usize, need: Need) {
        if let Need::Basic(held) = std::mem::replace(&mut self.entries[index].1, need)
            && Arc::strong_count(&held) == 1
        {
            self.credentials -= 1;
            self.retained -= held.bytes;
        }
    }
    /// Sets `key`'s state; a new key is stored only below the key bound.
    fn set(&mut self, key: &NeedKey, need: Need) -> Result<(), ErrorCode> {
        match self.find(key) {
            Some(index) => {
                self.replace(index, need);
            }
            None if self.entries.len() >= KEYS => {
                return Err(ErrorCode::Capacity);
            }
            None => {
                self.entries.push((*key, need));
            }
        }
        Ok(())
    }
    /// `Exhausted` (X1): every credential wiped and every entry but a `Rejected` one a tombstone.
    fn exhaust(&mut self) {
        self.mac = None;
        for index in 0..self.entries.len() {
            if !matches!(self.entries[index].1, Need::Rejected) {
                self.replace(index, Need::Tombstone);
            }
        }
    }
}

/// The parts of a credential the header carries: the username without its SSH terminator, and the password.
fn parts(secret: &Secret) -> (&[u8], &[u8]) {
    let username = secret
        .username
        .strip_suffix(&[0])
        .unwrap_or(&secret.username);
    (username, &secret.password)
}

/// HMAC-SHA-256, under the operation's random key, of the credential at `origin`: equal only for equal bytes at one
/// origin, and no secret. The constant tail leaves none of the credential's bytes in the hasher's block buffer.
fn fingerprint(mac: &Print, origin: &Print, secret: &Secret) -> Print {
    let (username, password) = parts(secret);
    let mut pad = [0u8; 64];
    pad[..32].copy_from_slice(mac);
    let mut inner = Sha256::new();
    inner.update(pad.map(|byte| byte ^ 0x36));
    field(&mut inner, FINGERPRINT_DOMAIN);
    field(&mut inner, origin);
    field(&mut inner, username);
    field(&mut inner, password);
    inner.update([0u8; 64]);
    let mut outer = Sha256::new();
    outer.update(pad.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// The operation's record (draft 3(b), 3(d)).
pub(crate) struct Needs {
    table: Mutex<Table>,
}
impl Needs {
    /// An empty record under a fresh random key; with no randomness it is `Exhausted` from the start.
    pub(crate) fn new() -> Self {
        let mut mac = [0u8; 32];
        Self::keyed(getrandom::fill(&mut mac).ok().map(|()| mac))
    }
    pub(super) fn keyed(mac: Option<Print>) -> Self {
        Self {
            table: Mutex::new(Table {
                mac,
                entries: Vec::new(),
                credentials: 0,
                retained: 0,
                rejected: Vec::new(),
                probe: Probe::new(),
            }),
        }
    }
    /// The table, and whether this call found its lock poisoned, which makes the record `Exhausted` first.
    fn table(&self) -> (MutexGuard<'_, Table>, bool) {
        match self.table.lock() {
            Ok(table) => (table, false),
            Err(poisoned) => {
                let mut table = poisoned.into_inner();
                table.exhaust();
                self.table.clear_poison();
                (table, true)
            }
        }
    }
    pub(crate) fn counts(&self) -> Counts {
        let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        Counts {
            keys: table.entries.len(),
            credentials: table.credentials,
            retained: table.retained,
            fingerprints: table.rejected.len(),
            exhausted: table.mac.is_none(),
        }
    }
    /// Whether the record is `Exhausted` (X1-X4): no lookup is started and nothing is answered from it.
    pub(crate) fn exhausted(&self) -> bool {
        self.table().0.mac.is_none()
    }
    /// What the record says of `key`; a poisoned lock is the budget code.
    pub(crate) fn answer(&self, key: &NeedKey) -> Result<Recorded, ErrorCode> {
        let (table, poisoned) = self.table();
        if poisoned {
            return Err(ErrorCode::Capacity);
        }
        Ok(match table.need(key) {
            None => Recorded::Unrecorded,
            Some(Need::Basic(held)) => Recorded::Basic(held.secret.header()),
            Some(Need::ConnectionBound { observed, evidence }) => Recorded::ConnectionBound {
                observed: *observed,
                evidence: *evidence,
            },
            Some(Need::Rejected) => Recorded::Rejected,
            Some(Need::Unretained) => Recorded::Unretained,
            Some(Need::Tombstone) => Recorded::Tombstone,
        })
    }
    /// Whether a fresh lookup's `secret` may be sent for `key` (G7): not for a `Rejected` key, nor when its
    /// fingerprint was rejected at the key's origin, which records the key `Rejected`.
    pub(crate) fn vet(&self, key: &NeedKey, secret: &Secret) -> Result<(), ErrorCode> {
        let (mut table, poisoned) = self.table();
        let Some(mac) = table.mac.filter(|_| !poisoned) else {
            return Err(ErrorCode::Capacity);
        };
        if matches!(table.need(key), Some(Need::Rejected)) {
            return Err(ErrorCode::Authentication);
        }
        if table
            .rejected
            .contains(&fingerprint(&mac, &key.origin, secret))
        {
            let _ = table.set(key, Need::Rejected);
            return Err(ErrorCode::Authentication);
        }
        Ok(())
    }
    /// Records the credential a fresh lookup answered `key`'s challenge with, after the authenticated 200 (G4). Equal
    /// bytes at the origin share one owner and the fresh copy is wiped. A `Rejected` key stays so and an earlier
    /// `Basic` insert wins; past a bound the key is `Unretained` (or, with the key table full, not stored) and the
    /// result is the budget code.
    pub(crate) fn record(&self, key: &NeedKey, mut secret: Secret) -> Result<(), ErrorCode> {
        let (mut table, poisoned) = self.table();
        let Some(mac) = table.mac.filter(|_| !poisoned) else {
            return Err(ErrorCode::Capacity);
        };
        match table.need(key) {
            Some(Need::Rejected | Need::Basic(_)) => {
                return Ok(());
            }
            None if table.entries.len() >= KEYS => {
                return Err(ErrorCode::Capacity);
            }
            _ => {}
        }
        let fingerprint = fingerprint(&mac, &key.origin, &secret);
        if table.rejected.contains(&fingerprint) {
            table.set(key, Need::Rejected)?;
            return Err(ErrorCode::Authentication);
        }
        let shared = table.entries.iter().find_map(|(_, need)| match need {
            Need::Basic(held) if held.fingerprint == fingerprint => Some(Arc::clone(held)),
            _ => None,
        });
        let held = match shared {
            Some(held) => held,
            None => {
                let (username, password) = parts(&secret);
                let bytes = username.len() + password.len();
                if bytes > CREDENTIAL_BYTES
                    || table.credentials >= CREDENTIALS
                    || table.retained + bytes > RETAINED_BYTES
                {
                    table.set(key, Need::Unretained)?;
                    return Err(ErrorCode::Capacity);
                }
                table.credentials += 1;
                table.retained += bytes;
                table.probe.attach(&mut secret);
                Arc::new(Held {
                    secret,
                    fingerprint,
                    bytes,
                })
            }
        };
        table.set(key, Need::Basic(held))
    }
    /// Records a connection-bound need (G2, G8, RA1): `Challenged` replaces any state but `Rejected` and an equal
    /// earlier insert; `OnBound` is stored only for a key with no entry. An `Exhausted` record stores nothing and
    /// gives the budget code, as does a full key table for a new key.
    #[allow(
        dead_code,
        reason = "the connection-bound learning of HTTPS model draft r5 sub-step H3a records it, which removes this"
    )]
    pub(crate) fn bound(
        &self,
        key: &NeedKey,
        observed: u64,
        evidence: Evidence,
    ) -> Result<(), ErrorCode> {
        let (mut table, poisoned) = self.table();
        if poisoned || table.mac.is_none() {
            return Err(ErrorCode::Capacity);
        }
        let need = Need::ConnectionBound { observed, evidence };
        match (table.find(key), evidence) {
            (None, _) => table.set(key, need),
            (Some(index), Evidence::Challenged) => {
                if !matches!(
                    table.entries[index].1,
                    Need::Rejected
                        | Need::ConnectionBound {
                            evidence: Evidence::Challenged,
                            ..
                        }
                ) {
                    table.replace(index, need);
                }
                Ok(())
            }
            (Some(_), Evidence::OnBound) => Ok(()),
        }
    }
    /// A 401 (not a 407) to a request of `key` that carried a credential, `fresh` when a fresh lookup's, else the
    /// record's own; with none (a logon-session answer) only the key is rejected. Every entry of the origin holding
    /// that credential becomes `Rejected` at once and its bytes are wiped; its fingerprint is kept, and one past the
    /// bound makes the record `Exhausted`. The result is the code the exchange ends with.
    pub(crate) fn reject(&self, key: &NeedKey, fresh: Option<&Secret>) -> ErrorCode {
        let (mut table, poisoned) = self.table();
        if poisoned {
            return ErrorCode::Capacity;
        }
        let print = match (table.mac, fresh) {
            (Some(mac), Some(secret)) => Some(fingerprint(&mac, &key.origin, secret)),
            (Some(_), None) => match table.need(key) {
                Some(Need::Basic(held)) => Some(held.fingerprint),
                _ => None,
            },
            (None, _) => None,
        };
        let _ = table.set(key, Need::Rejected);
        if let Some(print) = print {
            for index in 0..table.entries.len() {
                if matches!(&table.entries[index].1, Need::Basic(held) if held.fingerprint == print)
                {
                    table.replace(index, Need::Rejected);
                }
            }
            if !table.rejected.contains(&print) {
                if table.rejected.len() < FINGERPRINTS {
                    table.rejected.push(print);
                } else {
                    table.exhaust();
                }
            }
        }
        ErrorCode::Authentication
    }
}
impl fmt::Debug for Needs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let counts = self.counts();
        formatter
            .debug_struct("Needs")
            .field("keys", &counts.keys)
            .field("credentials", &counts.credentials)
            .field("fingerprints", &counts.fingerprints)
            .field("exhausted", &counts.exhausted)
            .finish()
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        use super::secret::SecretWipes as Wipes;
        /// Where every retained credential's `Drop` reports what its own overwrite left (row H1a-4).
        #[derive(Default)]
        struct Probe(Wipes);
        impl Probe {
            fn new() -> Self {
                Self::default()
            }
            /// Has `secret`'s `Drop` report to the probe; it does no wiping itself.
            fn attach(&self, secret: &mut Secret) {
                secret.observe_wipe(Arc::clone(&self.0));
            }
        }
        impl Needs {
            /// What each retained credential's `Drop` found in its buffers (rows H1a-4, H1c-3).
            pub(crate) fn wipes(&self) -> Wipes {
                Arc::clone(&self.table.lock().unwrap().probe.0)
            }
            /// Whether `a` and `b` hold one credential owner.
            pub(super) fn shares(&self, a: &NeedKey, b: &NeedKey) -> bool {
                let table = self.table.lock().unwrap();
                matches!(
                    (table.need(a), table.need(b)),
                    (Some(Need::Basic(x)), Some(Need::Basic(y))) if Arc::ptr_eq(x, y)
                )
            }
            /// Whether any byte string the record stores (credentials, fingerprints, key digests) contains `needle`.
            pub(super) fn contains(&self, needle: &[u8]) -> bool {
                let table = self.table.lock().unwrap();
                let found = |bytes: &[u8]| bytes.windows(needle.len()).any(|window| window == needle);
                table.rejected.iter().any(|print| found(print))
                    || table.entries.iter().any(|(key, need)| {
                        found(&key.key)
                            || found(&key.origin)
                            || matches!(need, Need::Basic(held)
                                if found(&held.secret.username) || found(&held.secret.password))
                    })
            }
            /// The bytes the record holds now: its own size, its tables' allocations and each distinct owner.
            pub(super) fn byte_sum(&self) -> usize {
                let table = self.table.lock().unwrap();
                let mut owners: Vec<*const Held> = Vec::new();
                let mut sum = size_of::<Needs>()
                    + table.entries.capacity() * size_of::<(NeedKey, Need)>()
                    + table.rejected.capacity() * size_of::<Print>();
                for (_, need) in &table.entries {
                    if let Need::Basic(held) = need
                        && !owners.contains(&Arc::as_ptr(held))
                    {
                        owners.push(Arc::as_ptr(held));
                        sum += size_of::<Held>()
                            + 2 * size_of::<usize>()
                            + held.secret.username.capacity()
                            + held.secret.password.capacity();
                    }
                }
                sum
            }
            /// Poisons the lock, as a panic under it would.
            pub(super) fn poison(&self) {
                let _ = std::thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            let _table = self.table.lock().unwrap();
                            panic!("poisoning the record's lock");
                        })
                        .join()
                });
            }
        }
    } else {
        struct Probe;
        impl Probe {
            fn new() -> Self {
                Self
            }
            fn attach(&self, _secret: &mut Secret) {}
        }
    }
}
