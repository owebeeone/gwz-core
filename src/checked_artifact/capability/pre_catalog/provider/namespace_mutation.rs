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

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Seek, SeekFrom};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use super::directory_mutation::sync_directory_edge;
use super::publication::{DestinationRecheckV1, PublicationSourceV1, publish_verified_no_replace};
use super::retained::encode_identity;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableIdentityProvider, DurableObjectIdentityV1, PathEquivalenceProvider, PlatformCapability,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::{ProtocolRecordKindV1, RecordDigestV1};
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

/// Action slot names are frozen ASCII, so this conversion is total and needs no
/// platform-specific `OsStr` construction.
fn os_name(leaf: &AsciiComponent) -> OsString {
    OsString::from(
        std::str::from_utf8(leaf.as_bytes()).expect("an ASCII component is always valid UTF-8"),
    )
}
