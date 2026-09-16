//! Owner-private physical namespace edges for one admitted action directory.
//!
//! R2-D Phase 2 Step 2.2 (`GwzM5-8R2D-Plan.md` §4): the physical half of the
//! retained-handle `ActionNamespace` backend. Edges E12 (`publish_exact`) and
//! E13 (`retire_exact`) run through the sealed source-associated publication
//! primitive (`GwzM5-8R2DInterfaceFreeze.md` §4.1 row P1, §4.3 rows E12/E13),
//! and edge E14 (`barrier`) runs through the admitted dirent-barrier family
//! (§4.1 row P5, §4.3 row E14). No raw rename is named here
//! (`GwzM5-8R2CCatalogBootstrapAmendment.md` §8.13).
//!
//! **No recheck arm is added by this file.** §4.3 assigns a §4.4 Class 1 arm to
//! rows E3, E7, E15, E16 and E17 only; E12/E13/E14 carry none. Every source
//! this owner publishes is therefore a *regular file*, which the primitive
//! verifies by identity and bytes with no interior recheck, and every
//! destination is `DestinationRecheckV1::None`. A directory source is refused
//! rather than published, because publishing one would need the managed
//! source-interior arm §4.4 assigns to Phase 2.3/3.
//!
//! Provenance is the audit's own pattern (`GwzM5-8R2C2PublicationAudit.md`
//! :39-44): the action directory is reached through exactly one identity-proved
//! no-follow hop from the permit-retained completed catalog, the caller
//! revalidates the permit immediately before the hop, and no ambient path Dir
//! ever reaches the primitive.
//!
//! Every name here is a deterministic, schedule-derived action slot supplied by
//! `namespace/roles.rs`; this file mints no name and no record.
//!
//! **R2-E Phase E1 Step E1.1** adds the eleven `cleanup.*` boundaries at the
//! foot of this file, per DECISION C-2 of the E0.2 semantics amendment
//! (`GwzM5-8R2E-SemanticsAmendment-DRAFT.md` §2.2, as amended by
//! `GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §4): every cleanup edge is
//! inside the one retained action directory this file's
//! `RetainedActionNamespaceV1` already owns, so the family needs no second
//! retained capability and mints no second owner file. They still mint no name:
//! every leaf they touch arrives as an `&AsciiComponent` derived by
//! `namespace/roles.rs` from the admitted action's own schedule.

use crate::filesystem::FsKind;
use std::ffi::OsStr;
use std::io::{self, Read, Seek, SeekFrom};

use crate::filesystem::{FsDirectory as Dir, FsOpenMode};
use sha2::{Digest, Sha256};

use super::directory_mutation::sync_directory_edge;
use super::publication::{DestinationRecheckV1, PublicationSourceV1, publish_verified_no_replace};
use super::retained::encode_identity;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableIdentityProvider,
    DurableObjectIdentityV1, PlatformCapability,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, BaseActionSlotV1, CleanupAliasV1, CleanupPhysicalFactV1,
    CleanupResolutionV1, DurableLeafFingerprintV1, ProtocolRecordKindV1, RecordDigestV1,
    decode_action_capacity_reservation, read_and_bind_cleanup_worklist,
};
use crate::model::ErrorCode;

mod cleanup;
mod edge;
mod observed;
mod retain;
mod support;

pub(crate) use cleanup::*;
pub(in crate::checked_artifact) use edge::*;
pub(in crate::checked_artifact) use observed::*;
pub(in crate::checked_artifact::capability::pre_catalog::provider) use retain::retain_action_namespace;
pub(in crate::checked_artifact) use retain::*;
pub(crate) use support::*;

impl RetainedActionNamespaceV1 {
    pub(in crate::checked_artifact) const fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }

    pub(in crate::checked_artifact) const fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        &self.path_profile
    }

    pub(in crate::checked_artifact) const fn reservation(&self) -> RecordDigestV1 {
        self.reservation
    }

    /// The retained action directory itself, for the one sibling owner that
    /// needs it as a *destination*: R2-D Step 2.3's ownership-marker retirement
    /// (edge E16) renames out of an installed managed component and into this
    /// action directory's scheduled `RetiredBootstrapMarker` row. The handle
    /// stays inside the sealed pre-catalog provider owner — `managed_mutation`
    /// is a sibling module of this one, not a consumer — so the "the real `Dir`
    /// never leaves the provider owner" rule is unweakened.
    pub(super) const fn handle(&self) -> &Dir {
        &self.handle
    }

    /// Re-proves that the retained handle is still the named action directory
    /// of the same reservation, in the shape `completed.rs:171-184` uses for the
    /// retained catalog: the name is reopened no-follow and both the freshly
    /// named identity and the retained handle's own identity must still equal
    /// the identity retained at acquisition.
    pub(in crate::checked_artifact) fn revalidate(
        &self,
        expected_identity: &DurableObjectIdentityV1,
        expected_reservation: RecordDigestV1,
    ) -> Result<(), CheckedFsError> {
        if expected_reservation != self.reservation || expected_identity != &self.identity {
            return Err(CheckedFsError::ambiguous(
                "action namespace",
                "action directory binding does not match the admitted action",
            ));
        }
        let named = self
            .parent
            .retained_child(&self.leaf)
            .map_err(|source| CheckedFsError::io("reopen named action directory", source))?;
        if super::HostPlatform.dir_identity(&named)?.durable() != &self.identity
            || super::HostPlatform.dir_identity(&self.handle)?.durable() != &self.identity
        {
            return Err(CheckedFsError::ambiguous(
                "action namespace",
                "retained action directory is no longer the named action directory",
            ));
        }
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::NamespaceParentRevalidate,
        );
        Ok(())
    }

    /// Retains one exact regular-file namespace source: no-follow open, durable
    /// identity, and bounded content read against the frozen record bound of
    /// `kind` (never the payload's own length — ConsumerCheckpoint §8 :236-237).
    pub(in crate::checked_artifact) fn retain_source(
        &self,
        leaf: &AsciiComponent,
        kind: ProtocolRecordKindV1,
    ) -> Result<ObservedNamespaceObjectV1, CheckedFsError> {
        let observed = self.observe_regular_file(leaf, kind, "action namespace source")?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::NamespaceSourceRetain);
        Ok(observed)
    }

    /// One bounded durable namespace edge: reserve the deterministic
    /// destination row, reobserve the source inside the strictest window,
    /// publish through the sealed primitive without replacement, reobserve the
    /// published row, and flush the parent.
    pub(in crate::checked_artifact) fn execute_edge(
        &self,
        edge: ActionNamespaceEdgeV1,
        source_leaf: &AsciiComponent,
        destination_leaf: &AsciiComponent,
        source: &ObservedNamespaceObjectV1,
        kind: ProtocolRecordKindV1,
    ) -> Result<DurableObjectIdentityV1, CheckedFsError> {
        let label = edge.label();
        #[cfg(test)]
        let faults = edge.faults();
        let source_name = os_name(source_leaf);
        let destination_name = os_name(destination_leaf);
        self.require_absent(&destination_name, label)?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(faults[0]);

        let fresh = self.observe_regular_file(source_leaf, kind, label)?;
        if fresh.identity != source.identity
            || fresh.encoded_identity != source.encoded_identity
            || fresh.bytes != source.bytes
        {
            return Err(CheckedFsError::ambiguous(
                label,
                "retained namespace source changed before the edge",
            ));
        }
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(faults[1]);

        publish_verified_no_replace(
            &self.handle,
            &source_name,
            &self.handle,
            &destination_name,
            PublicationSourceV1::regular_file(&source.encoded_identity, &source.bytes),
            DestinationRecheckV1::None,
            label,
        )?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(faults[2]);

        let republished = self.observe_regular_file(destination_leaf, kind, label)?;
        if republished.encoded_identity != source.encoded_identity
            || republished.bytes != source.bytes
        {
            return Err(CheckedFsError::ambiguous(
                label,
                "published namespace row is not the retained source object",
            ));
        }
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(faults[3]);
        sync_directory_edge(&self.handle, edge.flush_label())?;
        Ok(republished.identity)
    }

    /// Whether a deterministic slot row is resident. Read-only, and the only
    /// namespace question this owner answers without an edge, so a restart can
    /// tell which scheduled row it already reached.
    pub(in crate::checked_artifact) fn row_is_resident(&self, leaf: &AsciiComponent) -> bool {
        self.handle.entry_metadata(os_name(leaf)).is_ok()
    }

    /// R2-E E3.1 — `terminal.*` keys #1-#4: the four durable rows the terminal
    /// retirement is entitled to retire, re-read through the retained action
    /// directory before anything moves.
    ///
    /// **DECISION T-C′** (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §8,
    /// replacing DECISION T-C): the family's sites split by *capability*, not
    /// by family. These four are reads of the **action directory**, which is
    /// the capability this file owns — `admission_mutation::execute` takes only
    /// the catalog root and holds no action-directory handle at all — so
    /// putting them there would have needed a second capability forward for no
    /// gain. No new forward is minted by this half.
    ///
    /// All four are read-only, which is exactly why all four are repeatable
    /// boundaries: a crash at any of them leaves no durable delta.
    pub(in crate::checked_artifact) fn observe_terminal_preconditions(
        &self,
        expected: &ActionCapacityReservationV1,
    ) -> Result<(), CheckedFsError> {
        let action = expected.action_digest();

        // `terminal.authority_reobserve` — the authority row is resident in
        // exactly one of its two scheduled homes and reads inside the frozen
        // `Authority` record bound. Exactly one: a retirement is entitled to
        // retire one authority record, and both homes occupied is the
        // half-retired state a cleanup restart must resolve first, not a state
        // a terminal retirement may move over.
        self.require_single_scheduled_home(
            action,
            BaseActionSlotV1::Authority,
            BaseActionSlotV1::RetiredAuthorityAlias,
            Some(ProtocolRecordKindV1::Authority),
            "terminal authority row",
        )?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::TerminalAuthorityReobserve,
        );

        // `terminal.payload_reobserve` — the source and goal payload rows, each
        // in exactly one of its two scheduled homes, each a canonical regular
        // file. **No read of their content, and no digest relation.**
        //
        // DETERMINATION (2026-08-27, R2-E E3 remediation round; raised by the
        // E3 interior review as F3, ruled by the lane owner). E0.2 §4.3 row #2
        // announced "have been re-read and are the ones the action digest was
        // derived over". This boundary does not and must not bind that: a
        // payload's length is never a protocol-record bound
        // (`GwzM5-8R4bR2ConsumerCheckpoint.md` §8 :236-237), so there is no
        // budget in the frozen record vocabulary under which this owner could
        // read a payload leaf, and the digest relation the row names is the
        // *authority record's* — proved by `record.binding_validate` and by
        // `require_leaf_digest` (`coordinator/execution.rs`), both of which
        // need a `CheckedAuthorityObservationV1` that no terminal retirement
        // holds. **The row's semantic is amended to exactly what is bound
        // here: the source and goal payload rows are resident in exactly one
        // of their two scheduled homes, each a canonical regular file, read
        // not at all.** Flagged for the lane owner to carry into the freeze
        // §3.5 activation record in place of row #2's drafted text; the
        // announced semantic and the code now match, which is what the
        // disclosure-integrity ruling requires.
        for (live, retired) in [
            (
                BaseActionSlotV1::SourcePayload,
                BaseActionSlotV1::RetiredSourceAlias,
            ),
            (
                BaseActionSlotV1::GoalPayload,
                BaseActionSlotV1::RetiredGoalAlias,
            ),
        ] {
            self.require_single_scheduled_home(
                action,
                live,
                retired,
                None,
                "terminal payload row",
            )?;
        }
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::TerminalPayloadReobserve);

        // `terminal.cleanup_reobserve` — the join to `cleanup.*` key #11. The
        // worklist is read bounded and bound to the resident reservation, and
        // every row is then put through the **frozen classifier itself**:
        // `BoundCleanupWorklistV1::classify` must return
        // `CleanupResolutionV1::Complete` for every scheduled row. Nothing is
        // restated in this owner's own words — the rule is
        // `protocol/cleanup.rs`'s, so the announced semantic and the code are
        // the same sentence.
        self.require_completed_cleanup_worklist(expected)?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::TerminalCleanupReobserve);

        // `terminal.reservation_reobserve` — the resident reservation still
        // decodes to this exact reservation, and its record digest is still the
        // one this capability was retained against.
        let resident = self.observe_regular_file(
            &slot_leaf(action, BaseActionSlotV1::Reservation)?,
            ProtocolRecordKindV1::Capacity,
            "terminal resident reservation",
        )?;
        let decoded = decode_action_capacity_reservation(std::io::Cursor::new(&resident.bytes))
            .map_err(|_| {
                CheckedFsError::ambiguous(
                    "terminal resident reservation",
                    "the resident reservation is not a canonical capacity record",
                )
            })?;
        if &decoded != expected || decoded.record_digest() != self.reservation {
            return Err(CheckedFsError::ambiguous(
                "terminal resident reservation",
                "the resident reservation is not the admitted action's reservation",
            ));
        }
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::TerminalReservationReobserve,
        );
        Ok(())
    }

    /// R2-E E3.1 — `terminal.*` key #5: the action directory's own flush, so
    /// every row the four observations above proved is durable before the
    /// directory moves. Primitive family P2's parent flush (freeze §4.1 P2),
    /// over the capability that owns this directory (DECISION T-C′).
    pub(in crate::checked_artifact) fn flush_terminal_action_directory(
        &self,
    ) -> Result<(), CheckedFsError> {
        sync_directory_edge(&self.handle, "flush terminal action directory")?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::TerminalDirectoryFlush);
        Ok(())
    }

    /// One scheduled row of the retiring action, resident in exactly one of its
    /// two scheduled homes — live, or retired onto its cleanup alias.
    ///
    /// `bound`, when given, is the row's frozen record kind: the read is
    /// budgeted by the record kind and never by the object's own length.
    fn require_single_scheduled_home(
        &self,
        action: crate::checked_artifact::protocol::ActionDigestV1,
        live: BaseActionSlotV1,
        retired: BaseActionSlotV1,
        bound: Option<ProtocolRecordKindV1>,
        label: &'static str,
    ) -> Result<(), CheckedFsError> {
        let live = slot_leaf(action, live)?;
        let retired = slot_leaf(action, retired)?;
        let resident = match (self.row_is_resident(&live), self.row_is_resident(&retired)) {
            (true, false) => live,
            (false, true) => retired,
            _ => {
                return Err(CheckedFsError::ambiguous(
                    label,
                    "the scheduled row is not resident in exactly one of its two homes",
                ));
            }
        };
        match bound {
            Some(kind) => self.observe_regular_file(&resident, kind, label).map(drop),
            None => {
                let metadata = self
                    .handle
                    .entry_metadata(os_name(&resident))
                    .map_err(|source| CheckedFsError::io("observe terminal row", source))?;
                if metadata.kind != FsKind::File || metadata.kind == FsKind::Symlink {
                    return Err(CheckedFsError::ambiguous(
                        label,
                        "the scheduled row is not a canonical regular file",
                    ));
                }
                Ok(())
            }
        }
    }

    /// The bounded cleanup worklist, bound to the resident reservation, with
    /// **every row classified `Complete` by the frozen classifier**.
    ///
    /// The classification is `protocol/cleanup.rs`'s own — this owner supplies
    /// the two physical facts per row and `BoundCleanupWorklistV1::classify`
    /// supplies the rule, so the boundary binds the sentence §4.3 announces
    /// rather than a residency restatement of it. The E3 interior review (F3)
    /// found the first shape proving residency only while announcing the
    /// classifier; that gap is closed here rather than papered over.
    fn require_completed_cleanup_worklist(
        &self,
        expected: &ActionCapacityReservationV1,
    ) -> Result<(), CheckedFsError> {
        let action = expected.action_digest();
        let observed = self.observe_regular_file(
            &slot_leaf(action, BaseActionSlotV1::CleanupWorklist)?,
            ProtocolRecordKindV1::CleanupWorklist,
            "terminal cleanup worklist",
        )?;
        let worklist =
            read_and_bind_cleanup_worklist(std::io::Cursor::new(&observed.bytes), expected)
                .map_err(|_| {
                    CheckedFsError::ambiguous(
                        "terminal cleanup worklist",
                        "the resident cleanup worklist does not bind to this reservation",
                    )
                })?;
        for index in 0..worklist.len() {
            let row = worklist
                .row(index)
                .expect("a bounded worklist yields every row below its own length");
            let (live, retired) = match row.alias() {
                CleanupAliasV1::Source => (
                    BaseActionSlotV1::SourcePayload,
                    BaseActionSlotV1::RetiredSourceAlias,
                ),
                CleanupAliasV1::Goal => (
                    BaseActionSlotV1::GoalPayload,
                    BaseActionSlotV1::RetiredGoalAlias,
                ),
                CleanupAliasV1::Authority => (
                    BaseActionSlotV1::Authority,
                    BaseActionSlotV1::RetiredAuthorityAlias,
                ),
            };
            let source = self.cleanup_physical_fact(&slot_leaf(action, live)?, row.expected())?;
            let destination =
                self.cleanup_physical_fact(&slot_leaf(action, retired)?, row.expected())?;
            if worklist.classify(index, &source, &destination)
                != Some(CleanupResolutionV1::Complete)
            {
                return Err(CheckedFsError::ambiguous(
                    "terminal cleanup worklist",
                    "a scheduled cleanup row does not classify complete",
                ));
            }
        }
        Ok(())
    }

    /// One cleanup alias's physical fact, in the frozen
    /// [`CleanupPhysicalFactV1`] vocabulary.
    ///
    /// **The read is bounded by the worklist row's own recorded length, never
    /// by the object's.** `expected.length()` is a field of a bounded protocol
    /// record this owner has already read and bound to the resident
    /// reservation, so budgeting the stream by it keeps the
    /// "payload size is not confused with protocol-record size" rule
    /// (`GwzM5-8R4bR2ConsumerCheckpoint.md` §8 :236-237) intact: nothing here
    /// trusts a length the object itself supplies. The digest is streamed in
    /// fixed-size chunks, so a payload of any size costs constant memory, and
    /// a leaf that runs past the recorded length is `Other` — refused — rather
    /// than read further.
    fn cleanup_physical_fact(
        &self,
        leaf: &AsciiComponent,
        expected: &DurableLeafFingerprintV1,
    ) -> Result<CleanupPhysicalFactV1, CheckedFsError> {
        let name = os_name(leaf);
        let metadata = match self.handle.entry_metadata(&name) {
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(CleanupPhysicalFactV1::Missing);
            }
            Err(source) => return Err(CheckedFsError::io("observe cleanup alias", source)),
            Ok(value) => value,
        };
        if metadata.kind != FsKind::File || metadata.kind == FsKind::Symlink {
            return Ok(CleanupPhysicalFactV1::Other);
        }
        let options = FsOpenMode::Read;
        let mut file = self
            .handle
            .open_file(&name, &options)
            .map_err(|source| CheckedFsError::io("open cleanup alias no-follow", source))?;
        let fact = super::HostPlatform.file_identity(&file)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind cleanup alias", source))?;
        let limit = expected.length();
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; CLEANUP_ALIAS_STREAM_CHUNK];
        let mut length = 0_u64;
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|source| CheckedFsError::io("read cleanup alias", source))?;
            if read == 0 {
                break;
            }
            length = length.saturating_add(read as u64);
            if length > limit {
                return Ok(CleanupPhysicalFactV1::Other);
            }
            hasher.update(&buffer[..read]);
        }
        if length != limit {
            return Ok(CleanupPhysicalFactV1::Other);
        }
        Ok(CleanupPhysicalFactV1::Exact(DurableLeafFingerprintV1::new(
            fact.durable().clone(),
            length,
            hasher.finalize().into(),
        )))
    }

    /// Edge E14 — the admitted dirent-barrier family (§4.1 row P5) over the
    /// retained action directory itself.
    ///
    /// The retained action directory is an **exact interior**: its children are
    /// the admitted action's own evidence and admission refuses a nonzero
    /// `extra_children` (`protocol/admission/owner.rs:29-38`), so it may retain
    /// none of the permanent durability anchor the Windows arm of P5 renames.
    /// The class is therefore passed explicitly, and P5's Windows arm documents
    /// what stands in its place there (`platform.rs`, the writer-class-
    /// conditional arm recorded in the freeze §4.3 E9 form). On every other
    /// platform the class selects nothing: both are the same directory `fsync`.
    pub(in crate::checked_artifact) fn barrier(&self) -> Result<(), CheckedFsError> {
        crate::checked_artifact::platform::filesystem_private_barrier(
            &self.handle,
            crate::checked_artifact::platform::DirentBarrierClass::ExactInterior,
            ErrorCode::IoError,
            "action namespace barrier",
        )
        .map_err(|source| CheckedFsError::ambiguous("action namespace barrier", source.message))?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::NamespaceParentBarrier);
        Ok(())
    }

    /// The deterministic destination row must be free before the edge. The
    /// sealed primitive's hardcoded `replace=false` is what makes the move
    /// itself atomic against a racing occupant; this is the same property
    /// stated as a pre-edge expectation so a resident row is a typed refusal
    /// rather than an `EEXIST`.
    fn require_absent(&self, name: &OsStr, label: &'static str) -> Result<(), CheckedFsError> {
        match self.handle.entry_metadata(name) {
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(CheckedFsError::io("observe namespace destination", source)),
            Ok(_) => Err(CheckedFsError::ambiguous(
                label,
                "namespace destination row is already occupied",
            )),
        }
    }

    fn observe_regular_file(
        &self,
        leaf: &AsciiComponent,
        kind: ProtocolRecordKindV1,
        label: &'static str,
    ) -> Result<ObservedNamespaceObjectV1, CheckedFsError> {
        let name = os_name(leaf);
        let metadata = self
            .handle
            .entry_metadata(&name)
            .map_err(|source| CheckedFsError::io("observe namespace object", source))?;
        if metadata.kind != FsKind::File || metadata.kind == FsKind::Symlink {
            // A directory source would need the §4.4 Class 1 managed
            // source-interior arm, which §4.3 assigns to Phase 2.3/3, so this
            // owner refuses rather than publishing without one.
            return Err(CheckedFsError::ambiguous(
                label,
                "namespace object is not a canonical regular file",
            ));
        }
        let options = FsOpenMode::Read;
        let mut file = self
            .handle
            .open_file(&name, &options)
            .map_err(|source| CheckedFsError::io("open namespace object no-follow", source))?;
        let fact = super::HostPlatform.file_identity(&file)?;
        let limit = kind.max_bytes();
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(limit + 1).map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "namespace object read allocation failed",
            )
        })?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind namespace object", source))?;
        file.take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|source| CheckedFsError::io("read namespace object", source))?;
        if bytes.len() > limit {
            return Err(CheckedFsError::ambiguous(
                label,
                "namespace object exceeds its frozen record bound",
            ));
        }
        Ok(ObservedNamespaceObjectV1 {
            identity: fact.durable().clone(),
            encoded_identity: encode_identity(&fact),
            bytes,
        })
    }
}
