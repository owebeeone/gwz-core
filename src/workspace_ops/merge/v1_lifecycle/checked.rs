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
    /// R2-E Phase E4 Step E4.1 (O2): the checked v1 operation's prologue is
    /// where the catalog is activated, and it is this crate's only production
    /// catalog activation.
    ///
    /// **Why here.** `recover_or_create` needs a `CatalogMutationLeaseV1`,
    /// which is borrowed from a HELD `WorkspaceMutatorLock` — so the activation
    /// is structurally inside an operation already holding the lock, which is
    /// what E0.2 §5.2's relocation decision requires. Of the lock's nine
    /// production sites, that decision's frozen text keeps `repo create`,
    /// `init-from-sources`, the ordinary merge, abort, GC and the mutation
    /// guard capability-free; the v1 lifecycle's are what remain, and this one
    /// is the prologue BOTH v1 production entries take (`start.rs`'s creation
    /// lease and `service.rs`'s mutation loop), so one site covers the whole
    /// checked path. `--no-ff` is the only route into v1 at this writer floor
    /// (`model/version.rs`), so a filesystem without durable identity refuses
    /// exactly the checked feature and nothing else.
    ///
    /// **Ordering** (E0.2b §5.3 item 6): activation happens before the
    /// operation's own first durable mutation — the lease is taken before
    /// `create_open` and before the service's commit loop — so a capability
    /// refusal leaves the merge store untouched. The catalog's OWN partial
    /// state is the other arm: `recover_or_create` is a recover-or-create
    /// owner and converges on restart. Both arms are driven, not asserted.
    ///
    /// The retained catalog is dropped here: activation proves and persists
    /// it, and each consumer re-acquires under its own lease. E4.2-E4.6 are the
    /// consumers.
    pub(super) fn acquire(root: &Path) -> ModelResult<Self> {
        let workspace_root = root.canonicalize().map_err(io_error)?;
        let guard = WorkspaceMutatorLock::acquire(&workspace_root)?;
        crate::checked_artifact::entry::activate_workspace_catalog(guard.catalog_mutation_lease())?;
        Ok(Self {
            _guard: guard,
            workspace_root,
        })
    }

    pub(super) fn covers(&self, location: &OpenRecordLocation) -> bool {
        self.workspace_root == location.root && location.path.starts_with(&self.workspace_root)
    }

    #[cfg(test)]
    pub(super) fn acquire_for_test(root: &Path) -> ModelResult<Self> {
        Self::acquire(root)
    }
}

fn io_error(error: impl std::fmt::Display) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

fn unreadable(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, detail)
}
