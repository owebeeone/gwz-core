use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::filesystem::{FileSystem, RenameMode, make_filesystem};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::{MemberPath, WORKSPACE_MANIFEST};

mod conf_integrity;
mod merge_marker;

pub(crate) use conf_integrity::sha256_hex;
pub use conf_integrity::{
    CONF_BANNER, CONF_INTEGRITY_MARKER_PATH, CONF_INTEGRITY_SCHEMA, ConfIntegrityVerdict,
    GUARDED_CONF_PATHS, conf_hand_edit_error, inspect_conf_integrity,
    refresh_conf_integrity_marker,
};
pub(crate) use conf_integrity::{
    canonical_conf_integrity_marker, conf_integrity_for_bytes, inspect_conf_integrity_in,
    refresh_conf_integrity_marker_in,
};
pub use merge_marker::{
    MarkerMergeArtifact, MarkerMergeParticipantArtifact, MarkerMergeTargetKind,
};

mod durable_write;
mod encoding;
mod records;
mod store;
mod validation;

pub use durable_write::*;
pub use encoding::*;
pub use records::*;
pub use store::*;
pub(crate) use validation::*;

#[cfg(test)]
pub(crate) mod tests;

pub const WORKSPACE_SCHEMA: &str = "gwz.workspace/v0";
pub const LOCK_SCHEMA: &str = "gwz.lock/v0";
pub const SNAPSHOT_SCHEMA: &str = "gwz.snapshot/v0";
pub const MARKER_SCHEMA: &str = "gwz.marker/v0";
pub const LOCK_PATH: &str = "gwz.conf/gwz.lock.yml";
pub const SNAPSHOT_DIR: &str = "gwz.conf/snapshots";
pub const MARKER_DIR: &str = "gwz.conf/markers";

// The write_atomic family's own implementation stays in this file: the
// checked-artifact boundary package pins artifact/mod.rs by content digest
// precisely so a change to these three writers cannot slip past the
// capability-free raw-writer inventory (which counts their CALL sites).
/// The raw atomic writer deliberately does NOT touch the conf-integrity marker. The merge
/// lane publishes the lock and restores the manifest through this seam, and writing the
/// marker there dirties the root worktree at the exact moments that lane requires it
/// clean. The typed conf writers below re-record it; anything git rewrites behind gwz's
/// back is reconciled at the gate instead.
pub fn write_atomic(path: &Path, contents: impl AsRef<str>) -> ModelResult<()> {
    write_atomic_in(&make_filesystem(), path, contents)
}

pub(crate) fn write_atomic_in(
    filesystem: &dyn FileSystem,
    path: &Path,
    contents: impl AsRef<str>,
) -> ModelResult<()> {
    write_atomic_bytes_in(filesystem, path, contents.as_ref().as_bytes())
}

pub(crate) fn write_atomic_bytes_in(
    filesystem: &dyn FileSystem,
    path: &Path,
    contents: &[u8],
) -> ModelResult<()> {
    let staged = stage_durably(filesystem, path, contents)?;
    publish_staged(filesystem, &staged, path)
}
