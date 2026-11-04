//! A workspace's server keys and the names they give (split doc §1 "Unix socket name", §4.2 "One server per
//! directory", :229, :234).
//!
//! Two keys, for two jobs (operator decision 2026-11-03, 2.3f round 2 Code P2-1, option (a)):
//!
//! | Key | Hashes | Names | Why |
//! | --- | --- | --- | --- |
//! | [`ServerKey`] | the canonical absolute path of the workspace root | `ws-<key>`, `ws-<key>.lock`, `ws-<key>.log` | an address-keyed grant (split doc §5.6) names one workspace path for its whole life: a new workspace that reuses a deleted one's inode number gets another address and inherits no grant, and a remount that changes the device number voids none |
//! | [`IdentityKey`] | the root's filesystem identity ([`workspace_identity`]: 64-bit volume, 128-bit file) | `id-<key>.lock` | two canonical spellings of one directory (a macOS firmlink, a bind mount, a case-folding directory) have two `ws-<key>` names; the server holds this lock for its whole life, so a second server for the same directory refuses as held and names the first |
//!
//! Both are the first 16 hex digits of a SHA-256. Moving or renaming a workspace gives it a new address, and its
//! grants stop applying (fail closed). The identity lock carries no grant: a server writes its address into it and
//! clears it at a clean exit, so an inode number reused after that server is gone finds the lock free and inherits
//! nothing.
use crate::workspace_identity::{WorkspaceIdentity, workspace_identity};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
};

/// Hex digits of a key.
const KEY_DIGITS: usize = 16;

/// The first [`KEY_DIGITS`] hex digits of the SHA-256 of `bytes`.
fn key_of(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    hex[..KEY_DIGITS].to_owned()
}

/// The `ws-<key>` key of one workspace: the hash of its canonical root path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServerKey(String);

impl ServerKey {
    /// The key of the workspace whose canonical absolute root is `canonical` (no filesystem access).
    pub(crate) fn of_canonical(canonical: &Path) -> Self {
        Self(key_of(canonical.as_os_str().as_encoded_bytes()))
    }

    /// The socket's name, `ws-<key>` (Linux, macOS).
    pub(crate) fn socket_name(&self) -> String {
        format!("ws-{}", self.0)
    }

    /// The lock file's name, `ws-<key>.lock`.
    pub(crate) fn lock_name(&self) -> String {
        format!("ws-{}.lock", self.0)
    }

    /// The default log's name, `ws-<key>.log`, beside the lock.
    pub(crate) fn log_name(&self) -> String {
        format!("ws-{}.log", self.0)
    }
}

/// The `id-<key>` key of one directory: the hash of its filesystem identity, whatever the spelling of its path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IdentityKey(String);

impl IdentityKey {
    pub(crate) fn of_identity(identity: WorkspaceIdentity) -> Self {
        let mut bytes = [0u8; 24];
        bytes[..8].copy_from_slice(&identity.volume.to_le_bytes());
        bytes[8..].copy_from_slice(&identity.file.to_le_bytes());
        Self(key_of(&bytes))
    }

    /// The identity lock file's name, `id-<key>.lock`.
    pub(crate) fn lock_name(&self) -> String {
        format!("id-{}.lock", self.0)
    }
}

/// Both keys of one workspace root, and the canonical path the first hashes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Keys {
    pub(crate) server: ServerKey,
    pub(crate) identity: IdentityKey,
    /// The canonical absolute root, as the server records it.
    pub(crate) canonical: PathBuf,
}

impl Keys {
    /// The keys of the workspace whose root is at `root`, any spelling: `root` canonicalised (links resolved), and
    /// its identity read.
    ///
    /// # Errors
    ///
    /// The root cannot be canonicalised or has no identity (it does not exist or cannot be opened).
    pub(crate) fn of_root(root: &Path) -> io::Result<Self> {
        let canonical = std::fs::canonicalize(root)?;
        let identity = workspace_identity(&canonical)?;
        Ok(Self {
            server: ServerKey::of_canonical(&canonical),
            identity: IdentityKey::of_identity(identity),
            canonical,
        })
    }
}
