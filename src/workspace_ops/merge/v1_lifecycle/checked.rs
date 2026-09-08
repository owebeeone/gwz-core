use std::path::{Path, PathBuf};

use serde_yaml::Value;
use sha2::{Digest, Sha256};

#[cfg(test)]
use super::super::model::v1::validate_v1_record;
use super::super::model::v1::{MergeOperationRecordV1, ValidatedV1Record};
use super::super::record_wire::UnknownFieldManifest;
#[cfg(test)]
use crate::filesystem::{FileSystem, make_filesystem};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::WorkspaceMutatorLock;
use crate::operation_context::OperationContext;

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
    context: OperationContext,
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

    #[cfg(test)]
    pub(super) fn from_open_bytes(root: &Path, path: &Path, bytes: &[u8]) -> ModelResult<Self> {
        Self::from_open_bytes_in(&OperationContext::existing(), root, path, bytes)
    }

    pub(super) fn context(&self) -> &OperationContext {
        &self.context
    }

    pub(super) fn from_open_bytes_in(
        context: &OperationContext,
        root: &Path,
        path: &Path,
        bytes: &[u8],
    ) -> ModelResult<Self> {
        let root = context
            .filesystem()
            .canonical_path(root)
            .map_err(io_error)?;
        let expected_parent = root.join(".gwz/merge");
        if path.parent() != Some(expected_parent.as_path()) {
            return Err(unreadable(
                "v1 record is not at its canonical open location",
            ));
        }
        let decoded = super::super::record_wire::decode_production_v1(bytes)
            .map_err(|error| unreadable(format!("v1 decode failed: {error:?}")))?;
        let expected_id = path.file_stem().and_then(|value| value.to_str());
        if expected_id != Some(decoded.record().merge_id.as_str()) {
            return Err(unreadable("v1 record id does not match its file name"));
        }
        Ok(Self {
            typed: decoded.validated,
            context: context.clone(),
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
        let root = make_filesystem().canonical_path(root).map_err(io_error)?;
        let raw = serde_yaml::to_value(&record).map_err(io_error)?;
        let bytes = serde_yaml::to_string(&raw).map_err(io_error)?.into_bytes();
        let unknown_fields =
            UnknownFieldManifest::extract_v1(&raw).map_err(|error| unreadable(error.detail))?;
        let merge_id = record.merge_id.clone();
        Ok(Self {
            typed: validate_v1_record(record)?,
            context: OperationContext::existing(),
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
    guard: WorkspaceMutatorLock,
    workspace_root: PathBuf,
    context: OperationContext,
}

impl V1MutationLease {
    pub(super) fn context(&self) -> &OperationContext {
        &self.context
    }
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
    #[cfg(test)]
    pub(super) fn acquire(root: &Path) -> ModelResult<Self> {
        Self::acquire_in(&OperationContext::existing(), root)
    }

    pub(super) fn acquire_in(context: &OperationContext, root: &Path) -> ModelResult<Self> {
        let workspace_root = context
            .filesystem()
            .canonical_path(root)
            .map_err(io_error)?;
        let guard = WorkspaceMutatorLock::acquire_in(context, &workspace_root)?;
        Ok(Self {
            guard,
            workspace_root,
            context: context.clone(),
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
    #[cfg(test)]
    pub(super) fn acquire_activated(root: &Path) -> ModelResult<Self> {
        Self::acquire_activated_in(&OperationContext::existing(), root)
    }

    pub(super) fn acquire_activated_in(
        context: &OperationContext,
        root: &Path,
    ) -> ModelResult<Self> {
        let lease = Self::acquire_in(context, root)?;
        crate::checked_artifact::entry::activate_workspace_catalog(
            lease.guard.catalog_mutation_lease(),
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
    #[cfg(test)]
    pub(super) fn acquire_for_merge_start(root: &Path, workspace_id: &str) -> ModelResult<Self> {
        Self::acquire_for_merge_start_in(&OperationContext::existing(), root, workspace_id)
    }

    pub(super) fn acquire_for_merge_start_in(
        context: &OperationContext,
        root: &Path,
        workspace_id: &str,
    ) -> ModelResult<Self> {
        let lease = Self::acquire_in(context, root)?;
        crate::checked_artifact::entry::bootstrap_merge_start_parents(
            workspace_id,
            lease.guard.catalog_mutation_lease(),
            lease.guard.catalog_mutation_lease(),
        )?;
        Ok(lease)
    }

    /// DR-1 ship (1) W3 — the CATALOG-FREE creation lease
    /// (`GwzM5-8DR1-WarnOrRefuse-Charter.md` §3.1, 2026-09-03).
    ///
    /// Taken when `entry.rs::crash_recovery_decision` answered `Unsupported`
    /// and `--filesystem-strict` was absent: crash recovery is a capability, not
    /// a gate, so the merge runs — but on a volume whose identity the catalog
    /// cannot bind there is no catalog to activate, and asking for one is the
    /// refusal this ship removes.
    ///
    /// It is `acquire` plus row `:273`'s two parents, and nothing else. The
    /// parents still go through the CHECKED boundary — `entry.rs`'s
    /// `prepare_merge_start_parents_uncatalogued`, itself two
    /// `CheckedArtifact::prepare_parent` calls — so "both parents durable before
    /// record" holds on this path exactly as it does on
    /// `acquire_for_merge_start`'s; only the managed-parent PROVIDER, which
    /// needs the catalog, is out of the picture. No `create_dir_all` anywhere,
    /// and nothing inside the seam `r2d_seam_freeze.rs` freezes. `create_open`
    /// is unchanged and still publishes the record through the checked boundary
    /// (charter §4.1).
    pub(super) fn acquire_for_merge_start_uncatalogued_in(
        context: &OperationContext,
        root: &Path,
    ) -> ModelResult<Self> {
        let lease = Self::acquire_in(context, root)?;
        crate::checked_artifact::entry::prepare_merge_start_parents_uncatalogued(
            lease.context().filesystem(),
            root,
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

#[cfg(test)]
mod context_tests {
    use super::*;
    use crate::filesystem::FsKind;
    use crate::git::TestRepoSpec;
    use crate::operation_context::TestWorld;

    #[test]
    fn context_catalog_activation_reopens_and_releases_its_lease() {
        let world = TestWorld::selected();
        let context = world.context();
        let workspace = context.filesystem().test_workspace().unwrap();
        let root = workspace.path();
        context
            .repository()
            .test_init_repo(root, &TestRepoSpec::default())
            .unwrap();
        let catalog = root.join(".gwz/catalog-final");
        assert!(context.filesystem().kind(&catalog).is_err());
        let lease = V1MutationLease::acquire_activated_in(&context, root).unwrap();
        assert!(
            WorkspaceMutatorLock::try_acquire_in(&world.context(), root)
                .unwrap()
                .is_none()
        );
        let directory = context.filesystem().open_directory(&catalog).unwrap();
        let identity = directory
            .filesystem()
            .persistent_directory_identity(&directory)
            .unwrap();
        drop(lease);
        drop(context);
        let reopened = world.context();
        let lease = V1MutationLease::acquire_activated_in(&reopened, root).unwrap();
        let directory = reopened.filesystem().open_directory(&catalog).unwrap();
        assert_eq!(
            identity,
            directory
                .filesystem()
                .persistent_directory_identity(&directory)
                .unwrap()
        );
        drop(lease);
        let _creation =
            V1MutationLease::acquire_for_merge_start_in(&reopened, root, "ws_context").unwrap();
        for path in [".gwz/merge", crate::stash::STASH_BUNDLE_DIR] {
            assert_eq!(
                reopened.filesystem().kind(&root.join(path)).unwrap(),
                FsKind::Directory
            );
        }
        let mut record = crate::workspace_ops::merge::model::v1::test_record();
        record.state = crate::workspace_ops::merge::OperationState::Aborted;
        record.participants.get_mut("mem_a").unwrap().state =
            crate::workspace_ops::merge::ParticipantState::Aborted;
        let store = super::super::store::CheckedV1Store::default();
        let published = store.create_open(&_creation, root, &record, None).unwrap();
        drop(_creation);
        drop(reopened);
        let reopened = world.context();
        let observed = store
            .load_open_in(&reopened, root, &record.merge_id)
            .unwrap();
        assert!(published.same_source_as(&observed));
        assert!(observed.same_source_as(&store.reload_unchanged(&observed).unwrap()));
        let lease = V1MutationLease::acquire_in(&reopened, root).unwrap();
        assert_eq!(
            store.archive(&lease, &observed).unwrap(),
            super::super::store::ArchiveOutcome::Published
        );
        drop(lease);
        drop(reopened);
        let reopened = world.context();
        let lease = V1MutationLease::acquire_in(&reopened, root).unwrap();
        assert_eq!(
            store.archive(&lease, &observed).unwrap(),
            super::super::store::ArchiveOutcome::ReconciledDestination
        );
        assert!(
            reopened
                .filesystem()
                .kind(observed.location().path())
                .is_err()
        );
    }
}
