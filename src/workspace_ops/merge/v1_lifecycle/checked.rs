use std::path::{Path, PathBuf};

use serde_yaml::Value;
use sha2::{Digest, Sha256};

use super::super::model::v1::{MergeOperationRecordV1, ValidatedV1Record, validate_v1_record};
use super::super::record_wire::UnknownFieldManifest;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::WorkspaceMutatorLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RecordDigest([u8; 32]);

impl RecordDigest {
    pub(super) fn from_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OpenRecordLocation {
    root: PathBuf,
    path: PathBuf,
}

impl OpenRecordLocation {
    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

pub(super) struct StoredV1Record {
    typed: ValidatedV1Record,
    raw: Value,
    unknown_fields: UnknownFieldManifest,
    source_digest: RecordDigest,
    location: OpenRecordLocation,
}

impl StoredV1Record {
    pub(super) fn record(&self) -> &MergeOperationRecordV1 {
        self.typed.record()
    }

    pub(super) fn source_digest(&self) -> RecordDigest {
        self.source_digest
    }

    pub(super) fn location(&self) -> &OpenRecordLocation {
        &self.location
    }

    #[cfg(test)]
    pub(super) fn raw(&self) -> &Value {
        &self.raw
    }

    pub(super) fn unknown_fields(&self) -> &UnknownFieldManifest {
        &self.unknown_fields
    }

    pub(super) fn from_open_bytes(root: &Path, path: &Path, bytes: &[u8]) -> ModelResult<Self> {
        let root = root.canonicalize().map_err(io_error)?;
        let expected_parent = root.join(".gwz/merge");
        if path.parent() != Some(expected_parent.as_path()) {
            return Err(unreadable(
                "v1 record is not at its canonical open location",
            ));
        }
        let decoded = super::super::record_wire::decode_production_v1(bytes)
            .map_err(|error| unreadable(format!("v1 decode failed: {error:?}")))?;
        let expected_id = path.file_stem().and_then(|value| value.to_str());
        if expected_id != Some(decoded.record.merge_id.as_str()) {
            return Err(unreadable("v1 record id does not match its file name"));
        }
        Ok(Self {
            typed: validate_v1_record(decoded.record)?,
            raw: decoded.raw,
            unknown_fields: decoded.unknown_fields,
            source_digest: RecordDigest::from_bytes(bytes),
            location: OpenRecordLocation {
                root,
                path: path.into(),
            },
        })
    }

    #[cfg(test)]
    pub(super) fn for_test(root: &Path, record: MergeOperationRecordV1) -> ModelResult<Self> {
        let root = root.canonicalize().map_err(io_error)?;
        let raw = serde_yaml::to_value(&record).map_err(io_error)?;
        let bytes = serde_yaml::to_string(&raw).map_err(io_error)?.into_bytes();
        let unknown_fields =
            UnknownFieldManifest::extract_v1(&raw).map_err(|error| unreadable(error.detail))?;
        let merge_id = record.merge_id.clone();
        Ok(Self {
            typed: validate_v1_record(record)?,
            raw,
            unknown_fields,
            source_digest: RecordDigest::from_bytes(&bytes),
            location: OpenRecordLocation {
                path: root.join(".gwz/merge").join(format!("{merge_id}.yaml")),
                root,
            },
        })
    }

    pub(super) fn same_source_as(&self, other: &Self) -> bool {
        self.source_digest == other.source_digest
            && self.location == other.location
            && self.record() == other.record()
            && self.raw == other.raw
            && self.unknown_fields == other.unknown_fields
    }
}

pub(super) struct V1MutationLease {
    _guard: WorkspaceMutatorLock,
    workspace_root: PathBuf,
}

impl V1MutationLease {
    /// The plain lease: the workspace mutator lock and nothing else.
    ///
    /// **CAPABILITY-FREE, and that is the contract** (E4.1 review [P1-1]/[P2-1]
    /// cure). The v1 ABORT and PRESERVE routes take this one: abort is on
    /// E0.2 §5.2's capability-free list, so an open v1 record on a filesystem
    /// the catalog cannot use must still be abortable — that is the in-code
    /// exit every refusal below depends on existing.
    /// **Scoped by path** (2026-09-02, CapabilityFreeAmendment §6): the LEASE
    /// is capability-free unconditionally, but an abort that must re-verify a
    /// checked artifact — a preservation bundle, a selected root's manifest and
    /// lock, or the published evidence — still takes the legacy identity probe
    /// ON this lease. A dated residual shipped with A1; DR-1's (C) is the cure.
    pub(super) fn acquire(root: &Path) -> ModelResult<Self> {
        let workspace_root = root.canonicalize().map_err(io_error)?;
        let guard = WorkspaceMutatorLock::acquire(&workspace_root)?;
        Ok(Self {
            _guard: guard,
            workspace_root,
        })
    }

    /// The lease plus R2-E Step E4.1's catalog activation (O2), for the arms
    /// that mutate a record toward v1 semantics: `start.rs`'s creation lease
    /// and `service.rs`'s forward (`ResumeStart`/`Continue`) loop.
    ///
    /// **Why not in `acquire`** (review [P1-1]): that caught every arm taking
    /// the lock, abort included, and — through the A1 adapter — an ORDINARY
    /// merge resumed after a `Finalizing` interruption.
    /// `ACTIVE_WRITER_FLOOR` (`workspace_ops/merge/model/version.rs`) governs
    /// which version a START writes, not which lifecycle a record already on
    /// disk routes to. The adapter now proves viability before its durable
    /// v0->v1 rewrite; abort keeps the plain lease above.
    ///
    /// **Ordering** (E0.2b §5.3 item 6): taken before `create_open` and before
    /// the service's commit loop, so a refusal leaves the merge store
    /// untouched; the catalog's own partial state converges on restart.
    pub(super) fn acquire_activated(root: &Path) -> ModelResult<Self> {
        let lease = Self::acquire(root)?;
        crate::checked_artifact::entry::activate_workspace_catalog(
            lease._guard.catalog_mutation_lease(),
        )?;
        Ok(lease)
    }

    /// R2-E Step E4.2 — the CREATION lease: activation plus the §10 row `:273`
    /// bootstrap, in the order that row freezes.
    ///
    /// **Both parents durable before record.** They are installed and re-proved
    /// here, before `create_open` and before any Git work, so a record can only
    /// ever be published into prefixes made durable first. The bootstrap door
    /// recovers the catalog itself, so taking this lease activates it; E4.1's
    /// `acquire_activated` stays the forward SERVICE loop's, creating no parent.
    /// Two leases: admission consumes the first, execution recovers after it.
    pub(super) fn acquire_for_merge_start(root: &Path, workspace_id: &str) -> ModelResult<Self> {
        let lease = Self::acquire(root)?;
        crate::checked_artifact::entry::bootstrap_merge_start_parents(
            workspace_id,
            lease._guard.catalog_mutation_lease(),
            lease._guard.catalog_mutation_lease(),
        )?;
        Ok(lease)
    }

    pub(super) fn covers(&self, location: &OpenRecordLocation) -> bool {
        self.workspace_root == location.root && location.path.starts_with(&self.workspace_root)
    }

    #[cfg(test)]
    pub(super) fn acquire_for_test(root: &Path) -> ModelResult<Self> {
        Self::acquire(root)
    }

    #[cfg(test)]
    pub(super) fn acquire_activated_for_test(root: &Path) -> ModelResult<Self> {
        Self::acquire_activated(root)
    }

    #[cfg(test)]
    pub(super) fn acquire_for_merge_start_for_test(root: &Path, id: &str) -> ModelResult<Self> {
        Self::acquire_for_merge_start(root, id)
    }
}

fn io_error(error: impl std::fmt::Display) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

fn unreadable(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, detail)
}
