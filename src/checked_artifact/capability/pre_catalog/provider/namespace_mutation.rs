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

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Seek, SeekFrom, Write};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use sha2::{Digest, Sha256};

use super::directory_mutation::sync_directory_edge;
use super::publication::{DestinationRecheckV1, PublicationSourceV1, publish_verified_no_replace};
use super::retained::encode_identity;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableIdentityProvider, DurableObjectIdentityV1, PathEquivalenceProvider, PlatformCapability,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionSlotV1, BaseActionSlotV1,
    BoundCleanupWorklistV1, CleanupAliasV1, CleanupPhysicalFactV1, CleanupResolutionV1,
    DurableLeafFingerprintV1, ProtocolRecordKindV1, RecordDigestV1,
    decode_action_capacity_reservation,
    read_and_bind_cleanup_worklist,
};
use crate::model::ErrorCode;

/// Which side of the action namespace an edge is crossing.
///
/// The two edges are physically one move — a source-associated, no-replace
/// rename between two deterministic slot names under the same retained action
/// directory — so they share one primitive call site. The role selects the
/// stable `namespace.*` boundaries and the diagnostic label, exactly as
/// `AdmissionRecordRowV1` selects `admission.*` for the three admission rows
/// that share one write helper (`admission_mutation.rs:315-370`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ActionNamespaceEdgeV1 {
    /// Edge E12 — a scheduled scratch role published onto its active role.
    Publish,
    /// Edge E13 — a scheduled active role retired onto its retirement role.
    Retire,
}

impl ActionNamespaceEdgeV1 {
    const fn label(self) -> &'static str {
        match self {
            Self::Publish => "publish action namespace slot",
            Self::Retire => "retire action namespace slot",
        }
    }

    const fn flush_label(self) -> &'static str {
        match self {
            Self::Publish => "flush action namespace publication",
            Self::Retire => "flush action namespace retirement",
        }
    }

    /// The reserve / pre-edge / edge / post-edge boundaries this role crosses.
    #[cfg(test)]
    const fn faults(self) -> [CheckedArtifactFaultKeyV1; 4] {
        match self {
            Self::Publish => [
                CheckedArtifactFaultKeyV1::NamespaceDestinationReserve,
                CheckedArtifactFaultKeyV1::NamespacePrePublishReobserve,
                CheckedArtifactFaultKeyV1::NamespacePublishNoReplace,
                CheckedArtifactFaultKeyV1::NamespacePublishedReobserve,
            ],
            Self::Retire => [
                CheckedArtifactFaultKeyV1::NamespaceRetirementReserve,
                CheckedArtifactFaultKeyV1::NamespacePreRetireReobserve,
                CheckedArtifactFaultKeyV1::NamespaceRetireExact,
                CheckedArtifactFaultKeyV1::NamespaceRetiredReobserve,
            ],
        }
    }
}

/// One exact regular-file namespace object, observed and closed.
///
/// The bytes and the encoded identity are the primitive's own source-association
/// expectation. The observation handle is dropped before it is returned: on
/// Windows the primitive reopens the source with `DELETE` access, which a
/// surviving caller handle opened without `FILE_SHARE_DELETE` would refuse, and
/// the sealed primitive re-establishes identity through its own capability
/// anyway (`admission_mutation.rs:269-275`, the `publish_final_directory`
/// precedent it cites).
pub(in crate::checked_artifact) struct ObservedNamespaceObjectV1 {
    identity: DurableObjectIdentityV1,
    encoded_identity: Vec<u8>,
    bytes: Vec<u8>,
}

impl ObservedNamespaceObjectV1 {
    pub(in crate::checked_artifact) const fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }
}

/// The one retained action directory a namespace backend owns for its whole
/// life: opened once through an identity-proved no-follow hop from the
/// permit-retained completed catalog, and held across every observation,
/// publication, retirement, barrier and revalidation.
pub(in crate::checked_artifact) struct RetainedActionNamespaceV1 {
    parent: Dir,
    leaf: OsString,
    handle: Dir,
    identity: DurableObjectIdentityV1,
    path_profile: CanonicalPathIdentityV1,
    reservation: RecordDigestV1,
}

/// Retains the deterministic final action directory of an admitted action.
///
/// `final_directory` is the permit's own retained completed-catalog capability,
/// so this is the single no-follow hop the audit's provenance rule allows.
pub(super) fn retain_action_namespace(
    final_directory: &Dir,
    action_leaf: &str,
    expected_identity: &DurableObjectIdentityV1,
    reservation: RecordDigestV1,
) -> Result<RetainedActionNamespaceV1, CheckedFsError> {
    let leaf = OsString::from(action_leaf);
    let handle = final_directory
        .open_dir_nofollow(&leaf)
        .map_err(|source| CheckedFsError::io("open admitted action directory", source))?;
    let fact = super::HostPlatform.dir_identity(&handle)?;
    if fact.durable() != expected_identity {
        return Err(CheckedFsError::ambiguous(
            "action namespace",
            "admitted action directory identity changed",
        ));
    }
    let parent_fact = super::HostPlatform.dir_identity(final_directory)?;
    let path_profile = CanonicalPathIdentityV1::new(vec![CanonicalComponent::try_bound(
        AsciiComponent::parse(action_leaf.as_bytes())?,
        super::HostPlatform.parent_mode(final_directory)?,
        parent_fact.durable().clone(),
        parent_fact.invocation().clone(),
        super::HostPlatform.rename_domain(final_directory)?,
    )?])?;
    let parent = final_directory.try_clone().map_err(|source| {
        CheckedFsError::io("retain completed catalog for revalidation", source)
    })?;
    Ok(RetainedActionNamespaceV1 {
        parent,
        leaf,
        handle,
        identity: fact.durable().clone(),
        path_profile,
        reservation,
    })
}

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
            .open_dir_nofollow(&self.leaf)
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
        self.handle.symlink_metadata(os_name(leaf)).is_ok()
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
        // in exactly one of its two scheduled homes. No bound is applied and
        // none may be: a payload's length is never a protocol-record bound
        // (ConsumerCheckpoint §8 :236-237), so what this boundary names is the
        // rows' residency and canonical shape, never a read of their content.
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
        // every scheduled row must classify complete in the action directory's
        // own terms: the live row gone and the retired alias resident. That is
        // the durable state `cleanup.completion_reobserve` leaves, restated as
        // the terminal retirement's precondition rather than assumed from it.
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
                    .symlink_metadata(os_name(&resident))
                    .map_err(|source| CheckedFsError::io("observe terminal row", source))?;
                if !metadata.is_file() || metadata.is_symlink() {
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
    /// every scheduled row complete.
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
            if self.row_is_resident(&slot_leaf(action, live)?)
                || !self.row_is_resident(&slot_leaf(action, retired)?)
            {
                return Err(CheckedFsError::ambiguous(
                    "terminal cleanup worklist",
                    "a scheduled cleanup row has not reached its retired alias",
                ));
            }
        }
        Ok(())
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
        crate::checked_artifact::platform::private_barrier(
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
        match self.handle.symlink_metadata(name) {
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
            .symlink_metadata(&name)
            .map_err(|source| CheckedFsError::io("observe namespace object", source))?;
        if !metadata.is_file() || metadata.is_symlink() {
            // A directory source would need the §4.4 Class 1 managed
            // source-interior arm, which §4.3 assigns to Phase 2.3/3, so this
            // owner refuses rather than publishing without one.
            return Err(CheckedFsError::ambiguous(
                label,
                "namespace object is not a canonical regular file",
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = self
            .handle
            .open_with(&name, &options)
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

/// One scheduled base slot of an admitted action, as this owner's leaf type.
///
/// Every name is derived from the admitted action's own digest through the
/// frozen `ActionSlotV1` grammar; this file mints no name, exactly as its
/// header says.
fn slot_leaf(
    action: ActionDigestV1,
    slot: BaseActionSlotV1,
) -> Result<AsciiComponent, CheckedFsError> {
    AsciiComponent::parse(ActionSlotV1::Base(slot).name(action).as_bytes())
}

/// Action slot names are frozen ASCII, so this conversion is total and needs no
/// platform-specific `OsStr` construction.
fn os_name(leaf: &AsciiComponent) -> OsString {
    OsString::from(
        std::str::from_utf8(leaf.as_bytes()).expect("an ASCII component is always valid UTF-8"),
    )
}

// ---------------------------------------------------------------------------
// R2-E Phase E1 Step E1.1 — the `cleanup.*` family's eleven boundaries.
//
// DECISION C-1 (amendment §2.2): the two renames this family drives — the
// worklist publish and each alias retirement — keep announcing the already
// executed `namespace.publish_no_replace` / `namespace.retire_exact` boundaries
// of Step 2.2, because the family routes through the role-validated backend
// rather than opening the sealed primitive again. `cleanup.worklist_publish`
// and `cleanup.alias_retire` therefore name the *post-edge* states, in the same
// shape `managed_bootstrap.prior_generation_retire` does
// (`managed_mutation.rs:1039-1042`).
// ---------------------------------------------------------------------------

/// R2-E E1.1 — the cleanup worklist's scheduled scratch row (keys
/// `cleanup.worklist_scratch_create` / `_write` / `_flush`).
///
/// DECISION C-3, as simplified at E0.2b §4: the row is the shared
/// `BaseActionSlotV1::RecordScratch` slot (`protocol/slots.rs:106`/`:140`), an
/// entirely unconsumed base slot whose first and only user is this worklist. No
/// slot is minted, and there is no ordering condition — OPEN-C1 was struck when
/// the tree answered it: the authority record's own scratch is
/// `BaseActionSlotV1::AuthorityScratch` (`authority_record_binding.rs:486`), and
/// the `record.scratch_*` keys are named after the record *family*, not a slot.
///
/// **Write-or-rewrite, for the reason `write_managed_intent_scratch` is**
/// (`managed_mutation.rs:1058-1064`, `:1073-1101`). The bytes are re-derived
/// identically on every drive, so a leftover scratch from an interrupted drive
/// is this drive's own residue and must be converged on rather than wedged
/// against. The row carries no authority until the no-replace publish moves it,
/// and the post-write proof below re-reads the named row before any of that
/// happens.
pub(in crate::checked_artifact) fn write_cleanup_worklist_scratch(
    action: &RetainedActionNamespaceV1,
    scratch_leaf: &AsciiComponent,
    bytes: &[u8],
) -> Result<(), CheckedFsError> {
    let directory = &action.handle;
    let name = os_name(scratch_leaf);
    let create_new = match directory.symlink_metadata(&name) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => true,
        Err(source) => {
            return Err(CheckedFsError::io(
                "observe cleanup worklist scratch",
                source,
            ));
        }
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => false,
        Ok(_) => {
            return Err(cleanup_error(
                "cleanup worklist scratch row is not a canonical regular file",
            ));
        }
    };
    if bytes.len() > ProtocolRecordKindV1::CleanupWorklist.max_bytes() {
        return Err(cleanup_error(
            "cleanup worklist record exceeds its frozen record bound",
        ));
    }
    let options = super::directory_mutation::durable_write_options(create_new);
    let mut file = directory
        .open_with(&name, &options)
        .map_err(|source| CheckedFsError::io("open cleanup worklist scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistScratchCreate);
    if !create_new {
        file.set_len(0)
            .map_err(|source| CheckedFsError::io("truncate cleanup worklist scratch", source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind cleanup worklist scratch", source))?;
    }
    file.write_all(bytes)
        .map_err(|source| CheckedFsError::io("write cleanup worklist scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistScratchWrite);
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush cleanup worklist scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistScratchFlush);
    drop(file);
    sync_directory_edge(directory, "flush cleanup worklist scratch row")?;
    let observed = action.observe_regular_file(
        scratch_leaf,
        ProtocolRecordKindV1::CleanupWorklist,
        "cleanup worklist scratch",
    )?;
    if observed.bytes != bytes {
        return Err(cleanup_error(
            "cleanup worklist scratch is not the record just written",
        ));
    }
    Ok(())
}

/// R2-E E1.1 — the published worklist row and its two boundaries (keys
/// `cleanup.worklist_publish` / `cleanup.worklist_reobserve`).
///
/// The caller has just moved the scratch row onto `BaseActionSlotV1::CleanupWorklist`
/// through the Step-2.2 backend; the first boundary names that rename as durable
/// and unread, and the second names the bounded re-read that proves it — which
/// `read_and_bind_cleanup_worklist` (`protocol/cleanup.rs:356-367`) additionally
/// refuses when the worklist does not match the resident reservation.
pub(in crate::checked_artifact) fn observe_cleanup_worklist_row(
    action: &RetainedActionNamespaceV1,
    worklist_leaf: &AsciiComponent,
    reservation: &ActionCapacityReservationV1,
) -> Result<BoundCleanupWorklistV1, CheckedFsError> {
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistPublish);
    let bound = read_cleanup_worklist(action, worklist_leaf, reservation)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupWorklistReobserve);
    Ok(bound)
}

/// R2-E E1.1 — one worklist row's `(source, destination)` physical fact pair
/// (keys `cleanup.source_reobserve` / `cleanup.destination_reobserve`).
///
/// The pair is exactly what `BoundCleanupWorklistV1::classify`
/// (`protocol/cleanup.rs:323-333`) consumes, so a restart resolves each row from
/// durable truth rather than from the caller's own expectation.
pub(in crate::checked_artifact) fn observe_cleanup_row_facts(
    action: &RetainedActionNamespaceV1,
    source_leaf: &AsciiComponent,
    destination_leaf: &AsciiComponent,
) -> Result<(CleanupPhysicalFactV1, CleanupPhysicalFactV1), CheckedFsError> {
    let source = observe_cleanup_alias(action, source_leaf)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupSourceReobserve);
    let destination = observe_cleanup_alias(action, destination_leaf)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupDestinationReobserve);
    Ok((source, destination))
}

/// R2-E E1.1 — one alias retirement's three post-edge boundaries (keys
/// `cleanup.alias_retire`, `cleanup.retired_alias_reobserve`,
/// `cleanup.row_complete`).
///
/// The rename itself is Step 2.2's `namespace.retire_exact` (DECISION C-1); the
/// first boundary here names the state it leaves. The flush is the resident twin
/// of the unannounced `sync_directory_edge` at the tail of [`RetainedActionNamespaceV1::execute_edge`]:
/// re-flushing an already-flushed directory is idempotent, and it is what gives
/// this family its own announced completion edge without opening the sealed
/// primitive or minting a key.
pub(in crate::checked_artifact) fn observe_cleanup_retirement(
    action: &RetainedActionNamespaceV1,
    destination_leaf: &AsciiComponent,
    expected: &DurableLeafFingerprintV1,
) -> Result<(), CheckedFsError> {
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupAliasRetire);
    let retired = observe_cleanup_alias(action, destination_leaf)?;
    if retired != CleanupPhysicalFactV1::Exact(expected.clone()) {
        return Err(cleanup_error(
            "retired cleanup alias is not the fingerprint its worklist row named",
        ));
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupRetiredAliasReobserve);
    sync_directory_edge(&action.handle, "flush cleanup row completion")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupRowComplete);
    Ok(())
}

/// R2-E E1.1 — the whole-worklist proof (key `cleanup.completion_reobserve`).
///
/// The worklist is re-read bounded and every row must classify
/// `CleanupResolutionV1::Complete` (`protocol/cleanup.rs:394-398`). This is the
/// state `terminal.cleanup_reobserve` consumes.
///
/// `rows` carries each reserved alias with the two schedule-derived leaves it
/// resolves between, because this file mints no name; the aliases are matched by
/// value rather than by position, so the worklist's own canonical row order is
/// what selects them.
pub(in crate::checked_artifact) fn observe_cleanup_completion(
    action: &RetainedActionNamespaceV1,
    worklist_leaf: &AsciiComponent,
    reservation: &ActionCapacityReservationV1,
    rows: &[(CleanupAliasV1, AsciiComponent, AsciiComponent)],
) -> Result<(), CheckedFsError> {
    let bound = read_cleanup_worklist(action, worklist_leaf, reservation)?;
    for index in 0..bound.len() {
        let row = bound
            .row(index)
            .ok_or_else(|| cleanup_error("cleanup worklist row is not bound"))?;
        let Some((_, source_leaf, destination_leaf)) =
            rows.iter().find(|(alias, _, _)| *alias == row.alias())
        else {
            return Err(cleanup_error(
                "cleanup completion was not given this row's scheduled leaves",
            ));
        };
        let source = observe_cleanup_alias(action, source_leaf)?;
        let destination = observe_cleanup_alias(action, destination_leaf)?;
        if bound.classify(index, &source, &destination) != Some(CleanupResolutionV1::Complete) {
            return Err(cleanup_error(
                "a cleanup worklist row is not durably complete",
            ));
        }
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::CleanupCompletionReobserve);
    Ok(())
}

/// A bounded read of the resident worklist row, bound to the resident
/// reservation. It announces no boundary of its own: the announced pair is
/// [`observe_cleanup_worklist_row`]'s, and the completion proof's re-read is the
/// resume's own read — exactly as `read_managed_intent_row` is
/// (`managed_mutation.rs:1159-1163`).
fn read_cleanup_worklist(
    action: &RetainedActionNamespaceV1,
    worklist_leaf: &AsciiComponent,
    reservation: &ActionCapacityReservationV1,
) -> Result<BoundCleanupWorklistV1, CheckedFsError> {
    let observed = action.observe_regular_file(
        worklist_leaf,
        ProtocolRecordKindV1::CleanupWorklist,
        "cleanup worklist row",
    )?;
    read_and_bind_cleanup_worklist(observed.bytes.as_slice(), reservation).map_err(|_| {
        cleanup_error("cleanup worklist row does not bind to the resident reservation")
    })
}

/// One cleanup alias row's physical fact — absent, exactly fingerprinted, or
/// anything else — in the three-way shape `classify_cleanup_row`
/// (`protocol/cleanup.rs:383-401`) resolves.
///
/// The row is read under the **same** stated bound its retirement is read under
/// — this family's own `CleanupWorklist` record bound, whose statement of record
/// is `CleanupRetirementDestination::source_bound` (`namespace/roles.rs`). Using
/// one bound for both is what keeps the two halves coherent: a row this family
/// could not retire through `execute_edge` is refused when it is observed rather
/// than fingerprinted as though it could be.
///
/// It announces no boundary: the announced observations are
/// [`observe_cleanup_row_facts`]'s, and the completion proof's re-reads are the
/// resume's own reads.
fn observe_cleanup_alias(
    action: &RetainedActionNamespaceV1,
    leaf: &AsciiComponent,
) -> Result<CleanupPhysicalFactV1, CheckedFsError> {
    match action.handle.symlink_metadata(os_name(leaf)) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(CleanupPhysicalFactV1::Missing);
        }
        Err(source) => return Err(CheckedFsError::io("observe cleanup alias", source)),
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => {}
        Ok(_) => return Ok(CleanupPhysicalFactV1::Other),
    }
    let observed = action.observe_regular_file(
        leaf,
        ProtocolRecordKindV1::CleanupWorklist,
        "cleanup alias",
    )?;
    Ok(CleanupPhysicalFactV1::Exact(DurableLeafFingerprintV1::new(
        observed.identity,
        observed.bytes.len() as u64,
        Sha256::digest(&observed.bytes).into(),
    )))
}

fn cleanup_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("action cleanup worklist", detail)
}
