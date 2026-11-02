//! The identity of the caller a [`super::NativeCaller`] was captured for, as an operation's `AuthScope` folds it (R3b-win;
//! the HTTPS model draft r5 section 3.2 item 3). Only a digest of non-secret identifiers is kept: on Windows the process
//! primary token's user SID, logon-session LUID and session id (the identity the native route verifies and the one a
//! credential helper runs as); on Unix the effective uid (the identity the helper runs as). When the capture cannot read
//! the identity, the value is unique to that capture, never a constant, so it merges no callers.

/// What the identity's digest is of, so that no other digest in gwz can equal it.
const IDENTITY_DOMAIN: &[u8] = b"gwz-caller-identity/1";

/// A caller's identity as a scope reads it: the SHA-256 of its identifiers, or a value unique to its capture. It names no
/// account and carries no secret, and it has no `Debug`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct CallerIdentity([u8; 32]);

impl CallerIdentity {
    /// The identity of the process's caller, read now (on the operation's original entry).
    pub(crate) fn current() -> Self {
        Self::of(platform::read())
    }

    /// `read` when the capture read the identity, and a value unique to this capture when it could not.
    pub(crate) fn of(read: Option<[u8; 32]>) -> Self {
        match read {
            Some(digest) => Self(digest),
            None => Self(unique()),
        }
    }

    /// The bytes the scope folds.
    pub(crate) fn digest(&self) -> &[u8; 32] {
        &self.0
    }
}

/// The digest of `fields`, each length-prefixed, so that no two lists of identifiers share one.
pub(crate) fn fold(fields: &[&[u8]]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for field in std::iter::once(&IDENTITY_DOMAIN).chain(fields) {
        digest.update((field.len() as u64).to_le_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

/// A value no other capture has: 32 bytes from the operating system's random source. Only the Windows reader can yield
/// no identity (the Unix one always reads it), and there `getrandom` 0.3 draws from `ProcessPrng`, which does not fail
/// on Windows 10 or later. Should it fail anyway, the capture fails closed by panicking: the capture has no error to
/// return (`NativeHost::caller` and `CallerIdentity::current` are infallible), and no weaker value stands in.
fn unique() -> [u8; 32] {
    let mut value = [0u8; 32];
    getrandom::fill(&mut value).expect(
        "the operating system's random source gives an unreadable caller identity a unique value",
    );
    value
}

// The token parsing is plain bytes, tested on every platform; only the Windows reader uses it.
#[allow(dead_code)]
pub(crate) mod token_layout;

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        pub(crate) mod windows;
        pub(crate) use windows as platform;
    } else {
        /// The caller's identity on Unix: the effective uid, which the credential helper runs as and which owns the
        /// files it reads. `geteuid` cannot fail, so the identity is always read here.
        pub(crate) mod platform {
            /// The digest of this process's effective uid.
            pub(crate) fn read() -> Option<[u8; 32]> {
                // SAFETY: `geteuid` takes nothing, has no failure and touches no memory of ours.
                let uid = unsafe { libc::geteuid() };
                Some(super::fold(&[&uid.to_le_bytes()]))
            }
        }
    }
}
