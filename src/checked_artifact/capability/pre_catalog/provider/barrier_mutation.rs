//! Owner-private physical edges of one scheduled action barrier.
//!
//! R2-E Phase E2 (`GwzM5-8R2E-Plan.md` §3 Phase E2), binding the `barrier.*`
//! activation record filed at `GwzM5-8R2DInterfaceFreeze.md` §3.5 —
//! `GwzM5-8R2E-SemanticsAmendment-DRAFT.md` §3 as amended by
//! `GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §1/§5/§6.3, **addendum
//! controlling**.
//!
//! **DECISION B-1** gives the family its own owner-private mutation file rather
//! than adding sixteen sites to `namespace_mutation.rs`: the protocol composes
//! two retained parents — the completed catalog's roaming-anchor home and a
//! barrier target parent — which is the same ground on which `managed_mutation`
//! is its own file, and `namespace_mutation.rs` is already the file the settled
//! tuple's cohesion trigger watches.
//!
//! **This file holds every `barrier.*` injection site.** The `namespace` owner
//! holds none, exactly as it holds none of `namespace.*` and none of the
//! activated `managed_bootstrap.*`: it validates capabilities and never
//! mutates, so every durable barrier edge is announced from here
//! (`interface_tests/fault_expected_keys.rs`, the driver-holds-zero rule).
//!
//! # O6 — the caller-supplied-restatement class's last shape
//!
//! `BarrierIntentV1::issue` used to accept three caller-asserted facts — the
//! catalog anchor's identity, the roaming anchor's home parent identity, and
//! the home's name — with no observation check. That is the named
//! caller-restatement class, and the Step-4.3 precedent
//! (`GwzM5-8R2DPhase4Closure.md` §4) has three parts: the *owner* observes, the
//! owner *refuses* on disagreement, and the *issuer's signature* carries the
//! derivation obligation.
//!
//! [`RoamingAnchorHomeWitnessV1`] is part (i). It is minted **inside** the
//! pre-catalog provider owner from `RetainedCompletedCatalogV1`'s own retained
//! `catalog_anchor` and `final_directory` handles, and handed out as a typed
//! value the caller cannot construct — the inverse of
//! `AuthorityFactsIssuerV1`'s argument shape and the same shape as
//! `RetainedCompletedCatalogV1::observe_admission` /
//! `retain_action_namespace`, whose in-code contract is *"The caller receives
//! the typed observation only — never a handle"* (E0.2b §5.3).
//!
//! Part (ii) lands twice: the mint refuses whenever the retained catalog fails
//! to revalidate (`CompletedCatalogPermitV1::observe_roaming_anchor_home`
//! revalidates first, so a changed catalog-anchor identity, a final directory
//! that is no longer the named catalog, or a catalog whose completion predicate
//! no longer holds mints nothing — E0.2b §5.4); and the **read** side refuses,
//! because `read_and_bind_barrier_intent` now requires the witness and compares
//! the resident record's three identity facts against it beside its five
//! existing checks (E0.2b §5.2). Without that second refusal the restatement
//! class would survive restart: `decode_canonical` rebuilds through
//! `from_bound_fields` and bypasses `issue` entirely, so a drive resuming after
//! a crash would read caller-asserted identities off disk and act on them
//! however tight `issue` became.
//!
//! Part (iii) is written on `BarrierIntentV1::issue`'s own signature and doc,
//! because that signature is the only place a future transaction author looks.
//!
//! # The alias lifecycle — DECISION B-5, copy-not-move
//!
//! The roaming anchor **never leaves home**. What travels is a *freshly
//! created, independent* regular file carrying exactly `ROAMING_ANCHOR_BYTES`,
//! written through the P2 family under the schedule-derived reserved leaf, and
//! retired onto `RetiredRoamingAnchorAlias(ordinal)` when the barrier is done.
//! The catalog's `roaming-anchor-home-v1` row is never opened for write, never
//! linked, never renamed and never removed by any edge in this file.
//!
//! Both refused shapes are recorded here because a future reader must find the
//! reason rather than re-derive it (E0.2b §1.1-§1.3):
//!
//! * **Moving the home row** is fatal by design, not only in a crash window.
//!   While the anchor is away, `interior::completed_record` sees the home row
//!   absent, returns `None`, and the recovery classifier's only non-`Ambiguous`
//!   tuple is unreachable — a permanent `Err` with no in-code exit. And once
//!   the object is retired into an action directory, restoring the home row
//!   needs a *new* inode, hence a new `DurableObjectIdentityV1`, which the
//!   resident `RetiredActionsDescriptor` and `CatalogFormat` rows no longer
//!   match.
//! * **Hard-linking the home row** — the shape that would have let the alias
//!   share the home object's durable identity — is **macOS-fatal**. macOS
//!   allocates `ATTR_CMN_OBJPERMANENTID` per hard link *and re-homes the first
//!   link onto a fresh id when the second is created*
//!   (`platform/anchor/tests.rs`,
//!   `hard_link_identity_sharing_is_what_the_retirement_rows_assume`, and the
//!   provider's own `commonattr: ATTR_CMN_OBJPERMANENTID`). Linking the home
//!   row would therefore change the *home*'s identity and fail
//!   `completed_record` and `require_named_file_identity` on the very next
//!   observation — a deterministic first-barrier catastrophe.
//!
//! What a fresh copy costs, stated rather than implied: the alias does not
//! share the home object's identity, so the two alias reobservations are
//! **residency-and-bytes** proofs, not identity proofs. That loses nothing the
//! frozen record could have expressed — `BarrierIntentV1` has no alias-identity
//! field and gains none here, so no restart could ever have re-derived the
//! alias's identity from durable state on any shape.
//!
//! **DECISION B-6:** the alias is created and disposed of on **every** platform,
//! not only Windows. The physical barrier it enables is Windows-only, but a
//! platform-conditional alias would make six of the sixteen keys vacuous rows
//! off Windows, and the alias is a 22-byte file. Uniform creation keeps every
//! matrix row a real process stop across a real durable edge on all three
//! platforms.
//!
//! # OPEN-B2, answered at E2.1: the target parent stays action-directory-pinned
//!
//! The frozen seam mints a `BarrierTarget` from the namespace backend's own
//! `retained_parent()`, and `barrier_namespace` refuses a parent that is not
//! it, so today the "target parent" *is* the retained action directory. E2.1
//! does **not** widen it to a retained managed parent. Grounds:
//!
//! 1. The coupling clause (E0.2b §10): widening makes rows #10 and #12
//!    cross-parent renames, which `RetainedActionNamespaceV1::execute_edge`
//!    (same-directory, `&self.handle` for both sides) cannot serve. Three
//!    things would then move together — `CATALOG_PUBLICATION_CALL_COUNTS` would
//!    move at E2 after all, a new sealed-primitive call site would appear in
//!    this file, and the rename-domain question would apply to a *pair* of
//!    parents rather than one.
//! 2. The evidence that would settle the widening is an E4 consumer that does
//!    not exist yet — the ConsumerCheckpoint §10 marker and
//!    `.git/info/exclude` rows — and Phase E4 is re-scheduled behind R2-F's
//!    relocation package under the operator's 2026-08-27 ruling (a), so no such
//!    row can be measured now.
//! 3. `RetainedManagedParentV1`'s first *production* caller is E4.2, which is
//!    also where the settle asked R2-E to revisit the
//!    `retain_managed_parent_at_for_test` door. Widening here would extend a
//!    seam whose only caller is that `cfg(test)` door.
//!
//! Re-owned rather than closed: if an E4 row needs a barrier over a directory
//! it may not anchor, the widening lands there with all three coupled moves.
//!
//! # OPEN-B3, answered at E2.1: the reserved leaf's grammar
//!
//! **The leaf must be a canonical `ActionSlotV1` name of this action.** The
//! gate is `ActionSlotV1::parse` returning `Valid`, applied where the barrier is
//! *bound* (`namespace/host.rs`'s `scheduled_barrier_slots`, whose doc already
//! claimed the leaf was schedule-derived and which now enforces it), so both
//! the first drive and every restart pass through one check.
//!
//! The dotted, action-scoped `.ca1-{family}-{action}-{kind}` shape Step 4.2
//! adopted for the private area's own scratch is **refused** for this leaf.
//! Grounds: wherever the target parent is catalog-owned, `interior::exact_row`
//! walks the frozen `InfrastructureSlotV1` / `RootEntryNameV1` grammar and
//! refuses everything else as an unowned child — a `.ca1-*` leaf there makes
//! the catalog unobservable, which is the `Ambiguous` dead end this whole
//! family is designed around. The action-slot grammar is ASCII, action-scoped
//! (so two actions cannot collide), and already legal in every directory the
//! frozen design lets a barrier target.
//!
//! **What the gate does not do, stated honestly: no frozen slot names the
//! *live* alias.** `RetiredRoamingAnchorAlias(ordinal)` is its retirement
//! destination, not its home, and E2.1 holds no authorization to mint a slot —
//! that would move `BASE_ACTION_SLOTS` and the `MAX_ACTION_SLOTS == 261`
//! compile-time assertion. So the leaf stays the caller's *reservation* inside
//! that closed grammar, and three physical guards carry the rest: the alias is
//! created `create_new`, so an occupied leaf is a typed refusal rather than a
//! silent replacement; the retirement is no-replace; and the intent record
//! binds the leaf durably, so a restart reads it from disk instead of
//! re-choosing.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom, Write};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use super::directory_mutation::sync_directory_edge;
use super::interior::ROAMING_ANCHOR_BYTES;
use super::namespace_mutation::RetainedActionNamespaceV1;
use crate::checked_artifact::capability::{
    AsciiComponent, CheckedFsError, DurableIdentityProvider, DurableObjectIdentityV1,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, BarrierOrdinalV1, BoundBarrierIntentV1, InfrastructureSlotV1,
    ProtocolRecordKindV1, read_and_bind_barrier_intent,
};
use crate::model::ErrorCode;

/// The barrier owner's own observation of the roaming anchor's home.
///
/// Carries the two identities the intent record binds plus the derived home
/// name. There is no public constructor: `owner_mint` is visible only inside
/// `capability::pre_catalog`, so the only route to a witness is through a
/// `RetainedCompletedCatalogV1` that revalidated.
pub(in crate::checked_artifact) struct RoamingAnchorHomeWitnessV1 {
    catalog_anchor_identity: DurableObjectIdentityV1,
    private_home_parent_identity: DurableObjectIdentityV1,
    private_home_name: AsciiComponent,
}

impl RoamingAnchorHomeWitnessV1 {
    /// Minted inside the completed-catalog owner from its own retained
    /// handles. The home *name* is not observed at all and is not a parameter:
    /// it is the frozen compile-time constant
    /// `InfrastructureSlotV1::RoamingAnchorHome.name()`, and a derived constant
    /// cannot be restated wrongly (E0.2 §3.2, the `DERIVE` row).
    pub(in crate::checked_artifact::capability::pre_catalog) fn owner_mint(
        catalog_anchor_identity: DurableObjectIdentityV1,
        private_home_parent_identity: DurableObjectIdentityV1,
    ) -> Self {
        Self {
            catalog_anchor_identity,
            private_home_parent_identity,
            private_home_name: AsciiComponent::parse(
                InfrastructureSlotV1::RoamingAnchorHome.name().as_bytes(),
            )
            .expect("the frozen roaming-anchor home name is ASCII"),
        }
    }

    /// The protocol-private semantic tests bind every persisted field of
    /// `BarrierIntentV1`, including the home name the production mint derives,
    /// so the test-only constructor takes all three. It is the
    /// `NamespaceBarrierAuthority::test_only` precedent
    /// (`namespace/mod.rs`): a sealed value with a `cfg(test)` door, never a
    /// production route.
    #[cfg(test)]
    pub(in crate::checked_artifact) const fn test_only(
        catalog_anchor_identity: DurableObjectIdentityV1,
        private_home_parent_identity: DurableObjectIdentityV1,
        private_home_name: AsciiComponent,
    ) -> Self {
        Self {
            catalog_anchor_identity,
            private_home_parent_identity,
            private_home_name,
        }
    }

    pub(in crate::checked_artifact) const fn catalog_anchor_identity(
        &self,
    ) -> &DurableObjectIdentityV1 {
        &self.catalog_anchor_identity
    }

    pub(in crate::checked_artifact) const fn private_home_parent_identity(
        &self,
    ) -> &DurableObjectIdentityV1 {
        &self.private_home_parent_identity
    }

    pub(in crate::checked_artifact) const fn private_home_name(&self) -> &AsciiComponent {
        &self.private_home_name
    }
}

/// Which scheduled row of one barrier ordinal an intent observation names.
///
/// **DECISION B-2** — the intent record's five keys follow the managed intent
/// lifecycle exactly, because the record is the same shape: a bounded canonical
/// record moved between `BarrierIntentScratch`, `BarrierIntentActive(ordinal)`
/// and `BarrierIntentRetired(ordinal)`. So the three scratch keys mirror
/// `scratch_faults()` and the publish/retire pairs mirror `edge_faults()`.
///
/// The renames themselves stay the already-executed
/// `namespace.publish_no_replace` / `namespace.retire_exact` boundaries of Step
/// 2.2, because this lifecycle routes through the role-validated backend rather
/// than opening the sealed primitive again. These keys therefore name the
/// *post-edge state* — "the rename is durable and nothing has looked at it yet"
/// — exactly as `managed_bootstrap.prior_generation_retire` does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum BarrierIntentRowV1 {
    /// The published row this ordinal drives from.
    Active,
    /// The retirement that is this ordinal's own completion record.
    Retired,
}

impl BarrierIntentRowV1 {
    const fn label(self) -> &'static str {
        match self {
            Self::Active => "publish barrier intent",
            Self::Retired => "retire barrier intent",
        }
    }

    /// The post-edge and post-observation boundaries this row crosses.
    #[cfg(test)]
    const fn faults(self) -> [CheckedArtifactFaultKeyV1; 2] {
        match self {
            Self::Active => [
                CheckedArtifactFaultKeyV1::BarrierIntentPublish,
                CheckedArtifactFaultKeyV1::BarrierIntentReobserve,
            ],
            Self::Retired => [
                CheckedArtifactFaultKeyV1::BarrierIntentRetire,
                CheckedArtifactFaultKeyV1::BarrierIntentRetiredReobserve,
            ],
        }
    }
}

/// Which entry a drive took into the one alias-retirement helper.
///
/// **DECISION B-4 — #10 and #12 are two boundaries, not one key at two sites.**
/// They share this helper but differ in durable pre-state and in what the
/// entering drive can prove: `OwnDrive` is entered by the drive that created
/// this alias moments earlier and still holds that fact; `Stranded` is entered
/// by a *fresh* process that found an alias at the reserved leaf it cannot
/// prove is its own — the crash window between the alias's creation and its
/// retirement. This is the E15-install / E15-restart split the corpus already
/// keys separately (`managed_bootstrap.marker_retire` vs `component_reobserve`),
/// not the `staging_directory_flush` one-key-two-sites case.
///
/// **What partitions them, precisely** (OPEN-B8, answered at E2.1): the
/// in-memory fact that *this* drive created the alias. The durable state alone
/// does **not** partition the two entries — `BarrierIntentActive(ordinal)` is
/// resident on both paths, because the intent is retired at key #14, after the
/// alias is gone. Exactly one of the two entries runs per ordinal: whichever
/// one disposes of the alias fills `RetiredRoamingAnchorAlias(ordinal)`, and a
/// filled retirement row routes every later drive past both.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum AliasRetirementEntryV1 {
    /// The alias this drive created, retired by the drive that created it.
    OwnDrive,
    /// An alias a previous drive stranded, retired by the restart that found it.
    Stranded,
}

impl AliasRetirementEntryV1 {
    const fn label(self) -> &'static str {
        match self {
            Self::OwnDrive => "retire this drive's roaming anchor alias",
            Self::Stranded => "retire a stranded roaming anchor alias",
        }
    }

    #[cfg(test)]
    const fn faults(self) -> [CheckedArtifactFaultKeyV1; 2] {
        match self {
            Self::OwnDrive => [
                CheckedArtifactFaultKeyV1::BarrierAnchorReturn,
                CheckedArtifactFaultKeyV1::BarrierAnchorReturnReobserve,
            ],
            Self::Stranded => [
                CheckedArtifactFaultKeyV1::BarrierTargetAliasRetire,
                CheckedArtifactFaultKeyV1::BarrierTargetAliasReobserve,
            ],
        }
    }
}

/// Keys #1-#3 — the intent record written into the action directory's scheduled
/// `BarrierIntentScratch` row.
///
/// Primitive family P2 only (write-through plus flush); the record reaches its
/// active row through the Step-2.2 backend's `publish_barrier_intent`, which is
/// why this step adds no publication call site.
///
/// **Write-or-rewrite, for the same reason `write_managed_intent_scratch` is.**
/// The scratch slot is one deterministic base slot shared by every ordinal of
/// this action, and the bytes are re-derived identically on every drive, so a
/// leftover scratch from an interrupted ordinal is this drive's own residue and
/// must be converged on rather than wedged against.
pub(in crate::checked_artifact) fn write_barrier_intent_scratch(
    action: &RetainedActionNamespaceV1,
    scratch_leaf: &AsciiComponent,
    bytes: &[u8],
) -> Result<(), CheckedFsError> {
    let directory = action.handle();
    let name = os_name(scratch_leaf);
    let create_new = match directory.symlink_metadata(&name) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => true,
        Err(source) => return Err(CheckedFsError::io("observe barrier intent scratch", source)),
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => false,
        Ok(_) => {
            return Err(barrier_error(
                "barrier intent scratch row is not a canonical regular file",
            ));
        }
    };
    if bytes.len() > ProtocolRecordKindV1::BarrierIntent.max_bytes() {
        return Err(barrier_error(
            "barrier intent record exceeds its frozen record bound",
        ));
    }
    let options = super::directory_mutation::durable_write_options(create_new);
    let mut file = directory
        .open_with(&name, &options)
        .map_err(|source| CheckedFsError::io("open barrier intent scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::BarrierIntentScratchCreate);
    if !create_new {
        file.set_len(0)
            .map_err(|source| CheckedFsError::io("truncate barrier intent scratch", source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind barrier intent scratch", source))?;
    }
    file.write_all(bytes)
        .map_err(|source| CheckedFsError::io("write barrier intent scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::BarrierIntentScratchWrite);
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush barrier intent scratch", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::BarrierIntentScratchFlush);
    drop(file);
    sync_directory_edge(directory, "flush barrier intent scratch row")?;
    let observed = read_bounded(
        directory,
        &name,
        "write barrier intent scratch",
        ProtocolRecordKindV1::BarrierIntent.max_bytes(),
    )?;
    if observed != bytes {
        return Err(barrier_error(
            "barrier intent scratch is not the record just written",
        ));
    }
    Ok(())
}

/// Keys #4/#5 (`Active`) and #14/#15 (`Retired`) — the post-edge proof of one
/// scheduled intent row, and the two boundaries around it.
///
/// The caller has just moved the row through the Step-2.2 backend; this reads
/// it back bounded, binds it to the resident reservation and ordinal, and — the
/// O6 read side — proves its three identity facts equal to the witness the
/// owner re-minted from its own retained capabilities on this very drive. So
/// the record a drive resumes from is durable truth rather than either the
/// caller's expectation or some earlier drive's assertion.
pub(in crate::checked_artifact) fn observe_barrier_intent_row(
    action: &RetainedActionNamespaceV1,
    leaf: &AsciiComponent,
    row: BarrierIntentRowV1,
    reservation: &ActionCapacityReservationV1,
    ordinal: BarrierOrdinalV1,
    home: &RoamingAnchorHomeWitnessV1,
) -> Result<BoundBarrierIntentV1, CheckedFsError> {
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(row.faults()[0]);
    let bytes = read_bounded(
        action.handle(),
        &os_name(leaf),
        row.label(),
        ProtocolRecordKindV1::BarrierIntent.max_bytes(),
    )?;
    let bound = read_and_bind_barrier_intent(
        std::io::Cursor::new(bytes),
        reservation,
        ordinal,
        home,
    )
    .map_err(|_| {
        CheckedFsError::ambiguous(
            row.label(),
            "resident barrier intent does not bind to this reservation, ordinal and anchor home",
        )
    })?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(row.faults()[1]);
    Ok(bound)
}

/// What the alias phase must do next, decided from the target parent's own
/// resident state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum TargetAnchorAliasStateV1 {
    /// No alias is resident: this drive creates one and takes the fresh path.
    Absent,
    /// An alias is resident at the reserved leaf and this drive did not create
    /// it: the restart path, `AliasRetirementEntryV1::Stranded`.
    Stranded,
}

/// The alias phase's entry decision, and the one place a drive may make it.
///
/// **This exists because branching on the reserved leaf alone is wrong, and was
/// wrong in the first landing of this family** (R2-E Phase E2 review [P2-1]).
/// The roaming barrier's Windows arm renames the alias out to an outbound name
/// this owner cannot derive — the derivation is `platform`'s — so a drive that
/// asked only "is the reserved leaf resident?" answered *no* after a
/// mid-round-trip crash, created a second object over the empty leaf, and then
/// tripped the barrier's own both-names guard. The attempt refused, the next one
/// settled the ordinal through `Stranded`, and the outbound name was left
/// **permanently** — a name no later drive returned, because a settled ordinal
/// short-circuits and no other ordinal reserves that leaf.
///
/// So the question is asked of `platform`, which owns both names:
/// `prepare_roaming_target` converges whatever window a previous drive left
/// before it answers. Its state machine is total over the two names and stated
/// at its own definition; here is what each answer means to the caller:
///
/// * `Absent` — neither name resident. Create the alias, barrier, retire it:
///   `AliasRetirementEntryV1::OwnDrive`.
/// * `Resident` — an alias is at the reserved leaf, so `Stranded`. Three
///   different histories collapse into it and all three are safe: the ordinary
///   between-drives state; a mid-round-trip crash that the return rename has
///   just **converged**, leaving nothing behind; and, only on a tree a
///   pre-remediation binary wrote, an alias resident alongside a leftover
///   outbound object, which is left as a tolerated legacy orphan rather than
///   wedging the ordinal on a permanent refusal. The reason the third is
///   tolerated rather than converged is at `prepare_roaming_target`.
///
/// **No `barrier.*` boundary is announced here, and none is owed.** The return
/// rename is inside primitive family P5's own recovery surface, exactly like the
/// resident protocol's `AnchorState::NeedsReturn` return, which announces no
/// `barrier.*` key either; key #8 names "`private_barrier` has returned", never
/// its internal renames. The census is unmoved: 165, no key minted.
pub(in crate::checked_artifact) fn converge_target_anchor_alias(
    target: &RetainedActionNamespaceV1,
    reserved_leaf: &AsciiComponent,
) -> Result<TargetAnchorAliasStateV1, CheckedFsError> {
    let state = crate::checked_artifact::platform::prepare_roaming_target(
        target.handle(),
        &os_name(reserved_leaf),
        ROAMING_ANCHOR_BYTES,
        ErrorCode::IoError,
        "converge roaming anchor alias",
    )
    .map_err(|source| CheckedFsError::ambiguous("converge roaming anchor alias", source.message))?;
    Ok(match state {
        crate::checked_artifact::platform::RoamingTargetStateV1::Absent => {
            TargetAnchorAliasStateV1::Absent
        }
        crate::checked_artifact::platform::RoamingTargetStateV1::Resident => {
            TargetAnchorAliasStateV1::Stranded
        }
    })
}

/// Keys #6-#7 — the target parent's roaming anchor alias.
///
/// A **freshly created, independent** regular file carrying exactly
/// `ROAMING_ANCHOR_BYTES`, written through the P2 family and flushed with its
/// parent (DECISION B-5). `create_new` is the collision guard: an occupied
/// reserved leaf is a typed refusal, never a replacement. The catalog's
/// `roaming-anchor-home-v1` row is not touched by this edge.
///
/// The post-write proof is **residency and bytes**, not identity, and says so:
/// the intent binds no alias identity, so no restart could re-derive one.
pub(in crate::checked_artifact) fn create_target_anchor_alias(
    target: &RetainedActionNamespaceV1,
    reserved_leaf: &AsciiComponent,
) -> Result<(), CheckedFsError> {
    let directory = target.handle();
    let name = os_name(reserved_leaf);
    let options = super::directory_mutation::durable_write_options(true);
    let mut file = directory
        .open_with(&name, &options)
        .map_err(|source| CheckedFsError::io("create roaming anchor alias", source))?;
    file.write_all(ROAMING_ANCHOR_BYTES)
        .map_err(|source| CheckedFsError::io("write roaming anchor alias", source))?;
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush roaming anchor alias", source))?;
    drop(file);
    sync_directory_edge(directory, "flush roaming anchor alias row")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::BarrierAnchorOutbound);
    require_alias_resident(directory, &name, "create roaming anchor alias")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::BarrierAnchorOutboundReobserve,
    );
    Ok(())
}

/// Keys #8-#9 — `private_barrier` over the **target parent**, with the third
/// `DirentBarrierClass` (DECISION B-3).
///
/// This is a distinct call site in a distinct file from the two that pass
/// `ExactInterior`, which is why the §4.3 E10/E14 activation annotation is
/// unaffected. On Windows the class round-trips the *supplied* alias by its
/// reserved leaf and surveys for nothing; on every other platform it is the
/// directory `fsync`, exactly as both older classes are.
///
/// The post-barrier reobservation is a **real identity check**: the intent
/// binds `target_parent_identity`, so the live target is re-proved against the
/// fact the record carries, not against anything this drive assumed.
///
/// It additionally re-proves the **alias**, which §1.5's rewritten row #9 does
/// not require (E2 review [P3-6], recorded so the row and the code are not later
/// read as divergent). §1.5 deliberately dropped a residency clause from row #9
/// because under DECISION B-5 the *home* anchor never travels and so has no
/// residency to prove at the target. The alias is a different object, it
/// genuinely is resident at this point, and proving it is what makes a barrier
/// that silently lost its lent object a typed refusal instead of a green edge.
/// Strictly stronger than the row, and non-divergent from it.
pub(in crate::checked_artifact) fn barrier_target_parent(
    target: &RetainedActionNamespaceV1,
    reserved_leaf: &AsciiComponent,
    expected_identity: &DurableObjectIdentityV1,
) -> Result<(), CheckedFsError> {
    let directory = target.handle();
    let name = os_name(reserved_leaf);
    crate::checked_artifact::platform::private_barrier(
        directory,
        crate::checked_artifact::platform::DirentBarrierClass::RoamingAnchoredTarget {
            alias: &name,
            bytes: ROAMING_ANCHOR_BYTES,
        },
        ErrorCode::IoError,
        "barrier target parent",
    )
    .map_err(|source| CheckedFsError::ambiguous("barrier target parent", source.message))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::BarrierTargetBarrier);
    if super::HostPlatform.dir_identity(directory)?.durable() != expected_identity {
        return Err(barrier_error(
            "barrier target parent is no longer the identity the intent bound",
        ));
    }
    require_alias_resident(directory, &name, "reobserve barrier target parent")?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::BarrierTargetReobserve);
    Ok(())
}

/// Keys #10-#11 (`OwnDrive`) and #12-#13 (`Stranded`) — the post-edge proof
/// that the alias has left the reserved leaf for its scheduled retirement row.
///
/// The rename itself is the already-executed `namespace.retire_exact` boundary,
/// reached through `ActionNamespace::retire_barrier_target_alias`; these keys
/// name the post-edge state, per DECISION C-1's routing rule. Under
/// copy-not-move this edge is no longer catalog-fatal on either entry: it
/// retires a derived 22-byte object, and the home row never moved.
///
/// The proof is **two-sided**, in E11's shape: the retirement row must hold the
/// alias's bytes *and* the reserved leaf must hold nothing.
pub(in crate::checked_artifact) fn retire_target_anchor_alias(
    target: &RetainedActionNamespaceV1,
    reserved_leaf: &AsciiComponent,
    retired_leaf: &AsciiComponent,
    entry: AliasRetirementEntryV1,
) -> Result<(), CheckedFsError> {
    let directory = target.handle();
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(entry.faults()[0]);
    require_alias_resident(directory, &os_name(retired_leaf), entry.label())?;
    require_absent(directory, &os_name(reserved_leaf), entry.label())?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(entry.faults()[1]);
    Ok(())
}

/// Key #16 — the whole barrier ordinal has settled.
///
/// The restart observation: the reserved leaf holds nothing, the alias is
/// durably retired at its ordinal row, and the intent is retired at its own —
/// the state a resume uses to skip an ordinal it already completed. It crosses
/// no durable edge of its own, which is why it is the one key of this family a
/// settled drive re-crosses on every attempt.
pub(in crate::checked_artifact) fn observe_barrier_completion(
    target: &RetainedActionNamespaceV1,
    reserved_leaf: &AsciiComponent,
    retired_alias_leaf: &AsciiComponent,
    retired_intent_leaf: &AsciiComponent,
) -> Result<(), CheckedFsError> {
    let directory = target.handle();
    let label = "observe settled barrier ordinal";
    require_absent(directory, &os_name(reserved_leaf), label)?;
    require_alias_resident(directory, &os_name(retired_alias_leaf), label)?;
    match directory.symlink_metadata(os_name(retired_intent_leaf)) {
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => {}
        Ok(_) => {
            return Err(barrier_error(
                "settled barrier ordinal's retired intent row is not a canonical regular file",
            ));
        }
        Err(source) => {
            return Err(CheckedFsError::io(
                "observe retired barrier intent row",
                source,
            ));
        }
    }
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(CheckedArtifactFaultKeyV1::BarrierCompletionReobserve);
    Ok(())
}

/// The alias's residency-and-bytes proof: a canonical regular file, opened
/// no-follow, whose content is exactly the frozen roaming-anchor bytes. Bounded
/// by the frozen constant's own length plus one, so a longer object is refused
/// rather than read.
fn require_alias_resident(
    directory: &Dir,
    name: &OsStr,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    let bytes = read_bounded(directory, name, label, ROAMING_ANCHOR_BYTES.len())?;
    if bytes != ROAMING_ANCHOR_BYTES {
        return Err(CheckedFsError::ambiguous(
            label,
            "roaming anchor alias does not carry the frozen anchor bytes",
        ));
    }
    Ok(())
}

fn require_absent(
    directory: &Dir,
    name: &OsStr,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    match directory.symlink_metadata(name) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(CheckedFsError::io("observe reserved target leaf", source)),
        Ok(_) => Err(CheckedFsError::ambiguous(
            label,
            "the reserved target leaf is still occupied",
        )),
    }
}

/// One canonical regular file read against a frozen bound — never against the
/// object's own length (ConsumerCheckpoint §8 :236-237).
fn read_bounded(
    directory: &Dir,
    name: &OsStr,
    label: &'static str,
    limit: usize,
) -> Result<Vec<u8>, CheckedFsError> {
    let metadata = directory
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe barrier object", source))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Err(CheckedFsError::ambiguous(
            label,
            "barrier object is not a canonical regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open barrier object no-follow", source))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit + 1).map_err(|_| {
        CheckedFsError::unsupported(
            crate::checked_artifact::capability::PlatformCapability::PrivateNamespaceCollisionScan,
            "barrier object read allocation failed",
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind barrier object", source))?;
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read barrier object", source))?;
    if bytes.len() > limit {
        return Err(CheckedFsError::ambiguous(
            label,
            "barrier object exceeds its frozen bound",
        ));
    }
    Ok(bytes)
}

fn barrier_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("action barrier", detail)
}

/// Barrier names are frozen ASCII, so this conversion is total and needs no
/// platform-specific `OsStr` construction (`namespace_mutation.rs`).
fn os_name(leaf: &AsciiComponent) -> OsString {
    OsString::from(
        std::str::from_utf8(leaf.as_bytes()).expect("an ASCII component is always valid UTF-8"),
    )
}
