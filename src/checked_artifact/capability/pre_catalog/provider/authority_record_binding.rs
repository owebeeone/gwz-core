//! Owner-private authority-record binding: the bounded record and the streamed
//! payloads, joined but never conflated.
//!
//! R2-D Phase 2 Step 2.4 (`GwzM5-8R2D-Plan.md` §4). This owner is the
//! production caller of two seams it does not own — the R1 bounded parse
//! (`protocol/authority_record.rs`) and the landed `LeafObserver`
//! (`provider/leaf_observation.rs`, interface freeze §3.3) — and its whole job
//! is to keep the two budgets apart:
//!
//! | Path | Budget | Where |
//! | --- | --- | --- |
//! | authority **record** | `ProtocolRecordKindV1::Authority` = 16 KiB, frozen | [`read_resident_authority_record`], [`install_authority_record`] |
//! | source/goal **payload** | the payload's own length, streamed in the observer's fixed window | [`observe_streamed_payloads`] |
//!
//! No function here passes a record bound to a payload read or a payload
//! length to a record read, and [`validate_terminal_relation`] asserts the
//! separation on every join rather than trusting it
//! (`GwzM5-8R4bR2ConsumerCheckpoint.md` §8 :232-237).
//!
//! # The E9 writer-class condition this caller carries
//!
//! The interface freeze's E9 activation annotation (§4.3, landed with Step
//! 2.1) states the durability property the Windows arm of the leaf flush
//! relies on, and assigns its carriage here verbatim:
//!
//! > every gwz writer reaches observed leaves through write-through or
//! > write-handle `sync_all` (verified across `mutation.rs`/
//! > `directory_mutation.rs`/`admission_mutation.rs`/`residue.rs`), so for
//! > gwz-written leaves `ExactDurable` means the same on every platform; for
//! > FOREIGN-written leaves `ExactDurable` is strictly weaker on Windows
//! > (namespace ordering via the E10/P5 anchor round-trip only, no byte-flush
//! > claim). Step 2.4's production caller binding must carry this condition.
//!
//! **One clause of that quotation is superseded** (2026-08-22, by the E10/E14
//! anchor-readiness arm). The parenthetical "namespace ordering via the E10/P5
//! anchor round-trip only" describes no production path any more: E10 and E14
//! both barrier the retained action directory, which is an *exact interior* and
//! may retain no durability anchor, so their Windows arm is the documented
//! no-op (`platform::private_barrier`, `DirentBarrierClass::ExactInterior`).
//! The residual is therefore **empty, not weaker** — on Windows a
//! foreign-written leaf's `ExactDurable` carries no byte-flush claim *and* no
//! namespace-ordering claim. That strengthens this carriage rather than
//! disturbing it: the condition was always carried as a refusal, never as a
//! reduced-strength acceptance, so the gate below is exactly as sound with the
//! residual empty as it was with the residual overstated.
//!
//! This binding carries it in both required forms. In the doc contract: the
//! payload slots this owner observes are `SourcePayload` and `GoalPayload`
//! *inside an admitted action directory*, which only gwz writers reach, so the
//! nominal case is `ObservedLeafWriterClassV1::GwzWritten` and its
//! `ExactDurable` means the same on every platform. And in code, because the
//! nominal case is a claim rather than a proof: the writer class is a required
//! argument, never a default, and [`ObservedLeafWriterClassV1::Foreign`] is
//! handled explicitly — on Windows a foreign-written leaf's `ExactDurable`
//! carries no durable proof at all, so it is refused as authority rather than
//! silently accepted at a weaker strength.
//!
//! The same annotation's negative space is carried too: `MissingDurable` is a
//! two-sided absence proof and "does not assert continuous absence across the
//! barrier window", so this owner never converts an absence into a durable
//! payload fact — an absent payload slot is a typed refusal here, because an
//! authority record binds a fingerprint and two digests and has no encoding
//! for "absent".

use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom, Write};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use super::directory_mutation::{
    ObservedFileV1, durable_write_options, sync_directory_edge, verify_named_file, verify_open_file,
};
use super::leaf_observation::HostLeafObserverV1;
use super::publication::{DestinationRecheckV1, PublicationSourceV1, publish_verified_no_replace};
use super::retained::encode_identity;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableIdentityProvider,
    DurableObjectIdentityV1, PlatformCapability,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::leaf::{
    DurableLeafExpectation, DurableLeafProof, ExpectedLeafContent, LeafObserver,
};
use crate::checked_artifact::namespace::{NamespaceProtocol, RetainedDirectory};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionSlotV1, AuthorityFactsIssuerV1,
    BaseActionSlotV1, BoundCheckedAuthorityRecordV1, CheckedAuthorityObservationV1,
    CheckedAuthorityRecordV1, DurableLeafFingerprintV1, ProtocolCodecErrorV1, ProtocolRecordKindV1,
    RequestOwnerBindingV1, RetainedAuthorityFactsV1, RetainedAuthorityRequestV1,
    read_and_bind_authority_record,
};

/// The retained action directory every operation here runs through. It is the
/// capability the caller was issued; this owner opens no name of its own and
/// derives every slot name from the admitted action's digest.
pub(in crate::checked_artifact) type RetainedActionDirectoryV1 =
    RetainedDirectory<Dir, DurableObjectIdentityV1, CanonicalPathIdentityV1>;

/// Who wrote the payload leaves an authority observation is taken over.
///
/// Required, never defaulted: it is the E9 condition's carriage, and a default
/// would make the weaker Windows case invisible at the call site.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ObservedLeafWriterClassV1 {
    /// Written by a gwz writer, so the leaf reached its durable state through
    /// write-through or a write-handle `sync_all`. `ExactDurable` means the
    /// same on every platform.
    GwzWritten,
    /// Written by something outside gwz. On Windows the observation handle is
    /// read-only so no handle flush is available, and E10's barrier over an
    /// exact interior is the documented no-op, so `ExactDurable` carries
    /// neither a byte-flush claim nor a namespace-ordering one there. The
    /// residual is empty, which is why this class is refused rather than
    /// accepted at a reduced strength.
    Foreign,
}

/// The payload facts one authority observation binds, all of them streamed,
/// **together with the provenance of the capability they were streamed under**.
///
/// The payload half is the whole payload-derived content of an authority
/// record: one fingerprint and two digests. Its size is fixed and independent
/// of the payloads', which is the structural reason a 4 GiB source cannot widen
/// a 16 KiB record.
///
/// The provenance half — `action`, `artifact_root`, `retained_parent_identity`
/// — is carried here rather than re-supplied at the transaction because only
/// [`observe_streamed_payloads`] can mint this type, and it mints these three
/// fields from the retained capability it actually streamed through. That is
/// what lets [`AuthorityTransactionV1`] take no parent argument at all: an
/// observation whose root and parent identity come from one capability while
/// its digests come from another is not merely rejected, it cannot be
/// constructed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct StreamedPayloadProofV1 {
    action: ActionDigestV1,
    artifact_root: CanonicalPathIdentityV1,
    retained_parent_identity: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
    source_length: u64,
    goal_length: u64,
}

impl StreamedPayloadProofV1 {
    /// The admitted action whose payload slots were streamed.
    pub(in crate::checked_artifact) const fn action(&self) -> ActionDigestV1 {
        self.action
    }

    /// The durable identity of the retained action directory the payloads were
    /// streamed through.
    pub(in crate::checked_artifact) const fn retained_parent_identity(
        &self,
    ) -> &DurableObjectIdentityV1 {
        &self.retained_parent_identity
    }

    pub(in crate::checked_artifact) const fn source(&self) -> &DurableLeafFingerprintV1 {
        &self.source
    }

    pub(in crate::checked_artifact) const fn expected_sha256(&self) -> [u8; 32] {
        self.expected_sha256
    }

    pub(in crate::checked_artifact) const fn goal_sha256(&self) -> [u8; 32] {
        self.goal_sha256
    }

    /// The streamed payload lengths, which the terminal relation compares
    /// against the record bound rather than against each other.
    pub(in crate::checked_artifact) const fn payload_lengths(&self) -> (u64, u64) {
        (self.source_length, self.goal_length)
    }
}

/// Streams the source and goal payload leaves through the landed observer.
///
/// Both leaves are proved across the scheduled namespace barrier, each with its
/// own ordinal, through the observer's own `observe_durable` — so this owner
/// performs no read of its own and inherits the seam's bounded/fallible,
/// one-retained-handle, two-sided properties instead of restating them.
///
/// Neither payload is materialised: `Content` is opened twice by the observer
/// and compared in its fixed window, and what comes back is a fingerprint.
pub(in crate::checked_artifact) fn observe_streamed_payloads<Source, Goal, Protocol>(
    parent: &RetainedActionDirectoryV1,
    action: ActionDigestV1,
    writer_class: ObservedLeafWriterClassV1,
    source: &Source,
    goal: &Goal,
    namespace: &mut Protocol,
    ordinals: (Protocol::BarrierOrdinal, Protocol::BarrierOrdinal),
) -> Result<StreamedPayloadProofV1, CheckedFsError>
where
    Source: ExpectedLeafContent + ?Sized,
    Goal: ExpectedLeafContent + ?Sized,
    Protocol: NamespaceProtocol<
            DirectoryHandle = Dir,
            Identity = DurableObjectIdentityV1,
            PathProfile = CanonicalPathIdentityV1,
        >,
{
    let (source_ordinal, goal_ordinal) = ordinals;
    let observed_source = stream_payload(
        parent,
        &slot(action, BaseActionSlotV1::SourcePayload)?,
        writer_class,
        source,
        namespace,
        source_ordinal,
        "authority source payload",
    )?;
    let observed_goal = stream_payload(
        parent,
        &slot(action, BaseActionSlotV1::GoalPayload)?,
        writer_class,
        goal,
        namespace,
        goal_ordinal,
        "authority goal payload",
    )?;
    Ok(StreamedPayloadProofV1 {
        // Provenance, taken from the capability these payloads were actually
        // streamed through — not from a caller, and not re-supplied later.
        action,
        artifact_root: parent.path_profile().clone(),
        retained_parent_identity: parent.identity().clone(),
        source: DurableLeafFingerprintV1::new(
            observed_source.identity,
            observed_source.length,
            observed_source.sha256,
        ),
        expected_sha256: observed_source.sha256,
        goal_sha256: observed_goal.sha256,
        source_length: observed_source.length,
        goal_length: observed_goal.length,
    })
}

/// One streamed payload fact, as the observer proved it.
struct StreamedLeafV1 {
    identity: DurableObjectIdentityV1,
    length: u64,
    sha256: [u8; 32],
}

/// One payload leaf, through the frozen seam, with the E9 condition applied to
/// the strength of the proof that comes back.
fn stream_payload<Content, Protocol>(
    parent: &RetainedActionDirectoryV1,
    leaf: &AsciiComponent,
    writer_class: ObservedLeafWriterClassV1,
    content: &Content,
    namespace: &mut Protocol,
    ordinal: Protocol::BarrierOrdinal,
    fact: &'static str,
) -> Result<StreamedLeafV1, CheckedFsError>
where
    Content: ExpectedLeafContent + ?Sized,
    Protocol: NamespaceProtocol<
            DirectoryHandle = Dir,
            Identity = DurableObjectIdentityV1,
            PathProfile = CanonicalPathIdentityV1,
        >,
{
    let proof = HostLeafObserverV1.observe_durable(
        parent,
        leaf,
        DurableLeafExpectation::Exact(content),
        namespace,
        ordinal,
    )?;
    match proof {
        DurableLeafProof::ExactDurable {
            identity,
            length,
            sha256,
        } => {
            require_authority_strength(writer_class, fact)?;
            Ok(StreamedLeafV1 {
                identity,
                length,
                sha256,
            })
        }
        // The E9 annotation's negative space: a two-sided absence proof does
        // not assert continuous absence across the barrier window, and an
        // authority record has no encoding for an absent payload. Both are
        // refusals, not weaker facts.
        DurableLeafProof::MissingDurable => Err(CheckedFsError::ambiguous(
            fact,
            "an absent payload leaf cannot carry authority",
        )),
        DurableLeafProof::Other(_) => Err(CheckedFsError::ambiguous(
            fact,
            "payload leaf is not the exact durable object the authority binds",
        )),
    }
}

/// Whether this platform's `ExactDurable` loses its byte-flush claim for a leaf
/// gwz did not write — the E9 condition reduced to the one bit that actually
/// varies by platform.
///
/// This is a `cfg!` expression rather than a `#[cfg]` arm **deliberately**. The
/// two behaviours differ only in this boolean, not in their implementation, so
/// a pair of `#[cfg]` arms would buy nothing and cost the property that matters
/// here: an arm compiled only on Windows is type-checked only on Windows, and
/// the arm that is not being built can rot unnoticed. With one always-compiled
/// body, every host builds and lints the whole condition, and the Windows
/// behaviour is reachable in a test by reading this same constant.
///
/// (Contrast `leaf_observation::flush_observed_leaf`, which is genuinely two
/// `#[cfg]` arms because the *implementations* differ — `sync_all` versus a
/// documented no-op.)
pub(super) const FOREIGN_EXACT_DURABLE_IS_WEAKER: bool = cfg!(windows);

/// The refusal the weaker platform issues, hoisted so both the gate and its
/// test name the same string.
pub(super) const FOREIGN_AUTHORITY_REFUSAL: &str =
    "a foreign-written leaf carries no durable proof on this platform";

/// The E9 writer-class condition, as a gate rather than a comment.
///
/// Where the leaf flush really executed on the observation handle,
/// `ExactDurable` is the full claim regardless of who wrote the leaf. Where it
/// did not — Windows, whose read-only observation handle cannot
/// `FlushFileBuffers` — the substituting property is the *writer's*
/// write-through, which only holds for gwz writers. A foreign leaf keeps no
/// residual at all there: E10's barrier runs over an exact interior, whose
/// Windows arm is the documented no-op, so there is no namespace ordering to
/// fall back on either. It is refused rather than downgraded, because an
/// authority record is exactly a durability claim and this platform can supply
/// none for that class.
fn require_authority_strength(
    writer_class: ObservedLeafWriterClassV1,
    fact: &'static str,
) -> Result<(), CheckedFsError> {
    if FOREIGN_EXACT_DURABLE_IS_WEAKER && writer_class == ObservedLeafWriterClassV1::Foreign {
        return Err(CheckedFsError::ambiguous(fact, FOREIGN_AUTHORITY_REFUSAL));
    }
    Ok(())
}

/// One coherent retained authority transaction.
///
/// This is the value R1's owner seam was frozen to require, and it now delivers
/// that by construction rather than by calling convention: **every** fact it
/// issues comes out of the single [`StreamedPayloadProofV1`], which only
/// [`observe_streamed_payloads`] can mint and which carries the artifact root
/// and retained parent identity of the capability it streamed through. There is
/// no parent parameter to disagree with the proof, so an observation pairing
/// one directory's root with another directory's digests is unrepresentable —
/// not merely unattempted (`GwzM5-8R4bR2ConsumerCheckpoint.md` §14 first
/// bullet).
pub(in crate::checked_artifact) struct AuthorityTransactionV1 {
    request_owner_binding: RequestOwnerBindingV1,
    proof: StreamedPayloadProofV1,
}

impl AuthorityTransactionV1 {
    pub(in crate::checked_artifact) const fn from_streamed_proof(
        request_owner_binding: RequestOwnerBindingV1,
        proof: StreamedPayloadProofV1,
    ) -> Self {
        Self {
            request_owner_binding,
            proof,
        }
    }
}

impl RetainedAuthorityRequestV1 for AuthorityTransactionV1 {
    fn observe_retained_request(
        &self,
        issue: &AuthorityFactsIssuerV1,
    ) -> Result<RetainedAuthorityFactsV1, ProtocolCodecErrorV1> {
        Ok(issue.issue(
            self.request_owner_binding,
            self.proof.artifact_root.clone(),
            self.proof.retained_parent_identity.clone(),
            self.proof.source().clone(),
            self.proof.expected_sha256(),
            self.proof.goal_sha256(),
        ))
    }
}

/// The parse/proof join — boundary `record.terminal_relation_validate`.
///
/// This is the one place the two paths meet, and it proves three things the
/// split would otherwise only assert:
///
/// 1. **The record describes these payloads.** Its source fingerprint and both
///    digests equal the streamed proof's, field by field.
/// 2. **The record stayed record-sized.** The bytes the bounded read accepted
///    are within the frozen record bound — checked against the kind, not
///    against a payload.
/// 3. **The payloads were never treated as record-sized.** The relation holds
///    with payload lengths on either side of the record bound, so a payload
///    larger than 16 KiB is ordinary rather than a refusal, and that is what
///    makes the separation observable instead of merely intended.
/// 4. **The record and the payloads came from the same place.** The proof's
///    provenance — the action digest and the durable identity of the retained
///    directory it was streamed through — must equal the capability this
///    record was read out of.
///
/// Point 4 is what closes the last place a caller can still pair two
/// capabilities. [`AuthorityTransactionV1`] takes no parent, so the observation
/// side is unrepresentable; but `install`/`read`/`retire` each take `parent` and
/// `action` alongside a proof, so a caller could stream under one action
/// directory and join under another. That is refused here, typed and
/// fail-closed, before the boundary is announced.
pub(in crate::checked_artifact) fn validate_terminal_relation(
    parent: &RetainedActionDirectoryV1,
    action: ActionDigestV1,
    bound: &BoundCheckedAuthorityRecordV1,
    proof: &StreamedPayloadProofV1,
) -> Result<(), CheckedFsError> {
    if proof.action() != action
        || proof.retained_parent_identity() != parent.identity()
        || &proof.artifact_root != parent.path_profile()
    {
        return Err(CheckedFsError::ambiguous(
            "authority terminal relation",
            "the streamed payload proof was taken under a different retained action directory",
        ));
    }
    let record = bound.value();
    if record.source() != proof.source()
        || record.expected_sha256() != proof.expected_sha256()
        || record.goal_sha256() != proof.goal_sha256()
    {
        return Err(CheckedFsError::ambiguous(
            "authority terminal relation",
            "the bound authority record does not describe the streamed payloads",
        ));
    }
    if bound.record_bytes() > ProtocolRecordKindV1::Authority.max_bytes() {
        return Err(CheckedFsError::ambiguous(
            "authority terminal relation",
            "the bound authority record exceeds its frozen protocol-record bound",
        ));
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::RecordTerminalRelationValidate,
    );
    Ok(())
}

/// Installs an authority record: write-ahead scratch, then the no-replace
/// publish onto the active slot, then reobservation of what landed.
///
/// The order is the already-admitted one (P2 write-through + flush, then P1
/// sealed no-replace publication); nothing here mints a name, and both slots
/// are derived from the admitted action's digest.
pub(in crate::checked_artifact) fn install_authority_record(
    parent: &RetainedActionDirectoryV1,
    action: ActionDigestV1,
    record: &CheckedAuthorityRecordV1,
) -> Result<DurableObjectIdentityV1, CheckedFsError> {
    let handle = parent.handle();
    let bytes = record.encode_canonical();
    if bytes.len() > ProtocolRecordKindV1::Authority.max_bytes() {
        return Err(CheckedFsError::ambiguous(
            "authority record",
            "authority record exceeds its frozen protocol-record bound",
        ));
    }
    let scratch = slot_name(action, BaseActionSlotV1::AuthorityScratch)?;
    let active = slot_name(action, BaseActionSlotV1::Authority)?;
    write_authority_scratch(handle, OsStr::new(scratch.as_str()), &bytes)?;
    publish_authority_record(
        handle,
        OsStr::new(scratch.as_str()),
        OsStr::new(active.as_str()),
        &bytes,
    )
}

/// Boundaries `record.scratch_create`, `record.scratch_write` and
/// `record.scratch_flush`: the next durable authority state is complete,
/// open-file verified, named-file verified and parent-flushed before the
/// active name is touched.
fn write_authority_scratch(parent: &Dir, name: &OsStr, bytes: &[u8]) -> Result<(), CheckedFsError> {
    let mut options = durable_write_options(false);
    options.create(true);
    let mut file = parent
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open authority scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordScratchCreate);
    file.set_len(0)
        .map_err(|source| CheckedFsError::io("truncate authority scratch", source))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind authority scratch", source))?;
    file.write_all(bytes)
        .map_err(|source| CheckedFsError::io("write authority scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordScratchWrite);
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush authority scratch", source))?;
    let identity = encode_identity(&super::HostPlatform.file_identity(&file)?);
    let written = ObservedFileV1 {
        identity: &identity,
        bytes,
    };
    verify_open_file(&mut file, written, "authority scratch")?;
    drop(file);
    verify_named_file(parent, name, written, "authority scratch")?;
    sync_directory_edge(parent, "flush authority scratch write")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordScratchFlush);
    Ok(())
}

/// Boundaries `record.active_publish` and `record.active_reobserve`.
fn publish_authority_record(
    parent: &Dir,
    scratch: &OsStr,
    active: &OsStr,
    bytes: &[u8],
) -> Result<DurableObjectIdentityV1, CheckedFsError> {
    let (identity, observed) = observed_record(parent, scratch, "authority scratch")?;
    if observed != bytes {
        return Err(CheckedFsError::ambiguous(
            "authority scratch",
            "the resident authority scratch is not the record about to be published",
        ));
    }
    publish_verified_no_replace(
        parent,
        scratch,
        parent,
        active,
        PublicationSourceV1::regular_file(&identity, &observed),
        DestinationRecheckV1::None,
        "publish authority record",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordActivePublish);
    verify_named_file(
        parent,
        active,
        ObservedFileV1 {
            identity: &identity,
            bytes: &observed,
        },
        "published authority record",
    )?;
    sync_directory_edge(parent, "flush authority record publication")?;
    let published = super::HostPlatform.file_identity(&open_record(
        parent,
        active,
        "published authority record",
    )?)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordActiveReobserve);
    Ok(published.durable().clone())
}

/// Reads the resident authority record and binds it, through the R1 bounded
/// parse owner.
///
/// The reader handed across is the opened record file itself: this owner does
/// not read it, does not size it, and does not know its bound. That is the
/// split — the bound belongs to the record kind, and it lives with the parse.
pub(in crate::checked_artifact) fn read_resident_authority_record(
    parent: &RetainedActionDirectoryV1,
    action: ActionDigestV1,
    reservation: &ActionCapacityReservationV1,
    observation: &CheckedAuthorityObservationV1,
) -> Result<BoundCheckedAuthorityRecordV1, CheckedFsError> {
    let active = slot_name(action, BaseActionSlotV1::Authority)?;
    let file = open_record(
        parent.handle(),
        OsStr::new(active.as_str()),
        "resident authority record",
    )?;
    read_and_bind_authority_record(file, reservation, observation).map_err(|_| {
        CheckedFsError::ambiguous(
            "resident authority record",
            "the resident authority record does not bind to this reservation and observation",
        )
    })
}

/// Retires an authority record onto its scheduled retired alias: reserve the
/// deterministic destination row, retire through the sealed primitive, then
/// reobserve the retired row.
pub(in crate::checked_artifact) fn retire_authority_record(
    parent: &RetainedActionDirectoryV1,
    action: ActionDigestV1,
) -> Result<DurableObjectIdentityV1, CheckedFsError> {
    let handle = parent.handle();
    let active = slot_name(action, BaseActionSlotV1::Authority)?;
    let retired = slot_name(action, BaseActionSlotV1::RetiredAuthorityAlias)?;
    let active = OsStr::new(active.as_str());
    let retired = OsStr::new(retired.as_str());

    // `record.retirement_reserve`: the deterministic retired alias must be free
    // before the edge. The sealed primitive is no-replace by construction; this
    // states the same property as a pre-edge expectation, so a resident alias
    // is a typed refusal rather than a failed rename.
    if handle.symlink_metadata(retired).is_ok() {
        return Err(CheckedFsError::ambiguous(
            "authority record retirement",
            "the scheduled retired authority alias is already resident",
        ));
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordRetirementReserve);

    let (identity, bytes) = observed_record(handle, active, "retiring authority record")?;
    publish_verified_no_replace(
        handle,
        active,
        handle,
        retired,
        PublicationSourceV1::regular_file(&identity, &bytes),
        DestinationRecheckV1::None,
        "retire authority record",
    )?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordRetireExact);

    verify_named_file(
        handle,
        retired,
        ObservedFileV1 {
            identity: &identity,
            bytes: &bytes,
        },
        "retired authority record",
    )?;
    sync_directory_edge(handle, "flush authority record retirement")?;
    let reobserved = super::HostPlatform.file_identity(&open_record(
        handle,
        retired,
        "retired authority record",
    )?)?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::RecordRetiredReobserve);
    Ok(reobserved.durable().clone())
}

/// Opens one protocol-record slot no-follow, refusing anything that is not a
/// canonical regular file.
fn open_record(
    parent: &Dir,
    name: &OsStr,
    fact: &'static str,
) -> Result<cap_std::fs::File, CheckedFsError> {
    let metadata = parent
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe the authority record", source))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Err(CheckedFsError::ambiguous(
            fact,
            "authority record is not a canonical regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    parent
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open the authority record", source))
}

/// The record's encoded identity and bytes, read under the record kind's own
/// bound. The `+ 1` read is what proves the bound was respected rather than
/// assumed.
fn observed_record(
    parent: &Dir,
    name: &OsStr,
    fact: &'static str,
) -> Result<(Vec<u8>, Vec<u8>), CheckedFsError> {
    let mut file = open_record(parent, name, fact)?;
    let identity = encode_identity(&super::HostPlatform.file_identity(&file)?);
    let limit = ProtocolRecordKindV1::Authority.max_bytes();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "authority record read allocation failed",
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind the authority record", source))?;
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read the authority record", source))?;
    if bytes.len() > limit {
        return Err(CheckedFsError::ambiguous(
            fact,
            "authority record exceeds its frozen protocol-record bound",
        ));
    }
    Ok((identity, bytes))
}

fn slot_name(action: ActionDigestV1, slot: BaseActionSlotV1) -> Result<String, CheckedFsError> {
    Ok(ActionSlotV1::Base(slot).name(action))
}

fn slot(action: ActionDigestV1, slot: BaseActionSlotV1) -> Result<AsciiComponent, CheckedFsError> {
    AsciiComponent::parse(slot_name(action, slot)?.as_bytes()).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::AsciiProtocolPath,
            "action slot name is not an ASCII path component",
        )
    })
}
