//! The production `ManagedParentBootstrap` provider.
//!
//! R2-D Phase 3 Step 3.1 (`GwzM5-8R2D-Plan.md` §4): `observe_preflight` /
//! `revalidate_plan` / `execute_bound` on the retained provider, over the
//! opaque retained catalog and nothing else. The frozen seam is
//! `GwzM5-8R2DInterfaceFreeze.md` §3.4 and is implemented, not reshaped.
//!
//! Three properties are structural here rather than advisory.
//!
//! * **No path, no handle, no ambient authority.** The provider holds one
//!   `&OpaqueRetainedCatalogV1`. Every physical fact it uses comes back through
//!   that opaque catalog as a typed observation or a retained capability, and
//!   every durable edge is executed by the Step-2.2/2.3 backend behind
//!   `ActionNamespace` (freeze §3.1, §3.2; ConsumerCheckpoint §9 :264-266).
//! * **Every name is derived, never allocated.** The component names come from
//!   the bound plan's missing suffix, the staging and marker names from the
//!   frozen managed vocabulary, and the retirement rows from the resident
//!   schedule. Nothing here mints a name, a record, or a retry name
//!   (RemPlan-4 §4 R2 stop clause).
//! * **A drive resumes the resident intent, and never replans.** The row's state
//!   is the durable intent record; a restart reads it and continues from the
//!   phase and cursor it names, entering only the half of the component sequence
//!   whose durable row is absent (plan §4 Step 3.1: "restart consumes the
//!   resident intent and scheduled slots, never replans a partially completed
//!   live path").
//!
//! **Step 3.1b — what the intent record's own lifecycle adds** (plan §4 Step
//! 3.1's "durable successor" and "prior-generation retirement" items; freeze
//! §4.3 row E17). Each generation of the chain is written to the scheduled
//! `BootstrapIntentScratch` row, published onto `BootstrapIntentActive(g)`,
//! reobserved, and its predecessor retired onto `BootstrapIntentRetired(g-1)`;
//! the last generation retires as the row's completion record. Step 3.1 could
//! only *re-derive* the chain from evidence, and the only evidence that closes an
//! install is the ownership marker still inside its component — which edge E16
//! removes — so a row interrupted inside the marker-retirement phase could never
//! converge. With the record resident that interruption resumes at its cursor,
//! and the refusal Step 3.1 carried is gone.
//!
//! **Both §4.4 Class 1 arms resolve to none, and why.** Freeze §4.3 makes E17's
//! arms conditional in the same form row E16's were. Every move this lifecycle
//! makes is a *regular-file protocol record* travelling between two deterministic
//! slots of one retained action directory, through the Step-2.2 backend's
//! role-validated `publish_bootstrap_generation` / `retire_bootstrap_generation`
//! — which carry `PublicationSourceV1::regular_file` and
//! `DestinationRecheckV1::None`, exactly as E12/E13 do. No directory is
//! published, so no source-interior arm exists to add; no retirement destination
//! is re-checked, so no destination arm is either. `PublicationSourceV1` and
//! `DestinationRecheckV1` are unchanged by this step, and it adds no
//! `publish_verified_no_replace` caller.
//!
//! **The ownership token's boundary, carried forward from the Step-3.1 review
//! §9.** A resumed drive takes the token from the resident record rather than
//! re-deriving it. That read-back is *self-consistency* — "this chain is the one
//! this admitted action's plan describes" — and it is **not** an adoption or
//! exclusion proof. `read_and_bind_managed_bootstrap_intent` binds the record to
//! this bound plan, purpose, generation and predecessor; it does not, and must
//! not be read to, establish that no other writer produced the chain. Anything
//! inside the permit-retained root is the same-user boundary the E16 record
//! already declares outside scope. A later step that uses a resident record to
//! decide adoption of state this action did not create makes determinism
//! load-bearing for exclusion, and must re-litigate the token then.

use std::io::Cursor;

use sha2::{Digest, Sha256};

use super::owner::ManagedParentBootstrap;
use super::plan::{BoundManagedParentPlanRowV1, BoundManagedParentPlanV1, ManagedParentPlanV1};
use super::{ManagedParentBootstrapRequest, ManagedParentObservationV1, ManagedParentPurpose};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
    ManagedIntentEdgeV1, PathComponentMode, PlatformCapability,
};
use crate::checked_artifact::catalog::OpaqueRetainedCatalogV1;
use crate::checked_artifact::namespace::{
    ActionNamespace, BootstrapGenerationSlots, BootstrapIntentRowV1, HostActionNamespaceV1,
    retain_action_namespace,
};
use crate::checked_artifact::protocol::{
    BootstrapGenerationV1, ManagedBootstrapPhaseV1, ManagedParentBootstrapIntentV1,
    OwnershipMarkerV1, ProtocolRecordKindV1, read_and_bind_managed_bootstrap_intent,
};

/// The generation rows this drive names, spelled once so the two classification
/// sites and the four edge sites cannot drift apart.
const ROW_ACTIVE: BootstrapIntentRowV1 = BootstrapIntentRowV1::Active;
const ROW_RETIRED: BootstrapIntentRowV1 = BootstrapIntentRowV1::Retired;

/// The managed intent record's frozen protocol kind. Every bounded read and
/// every retained source of this lifecycle is budgeted against it, never against
/// a file's own length (ConsumerCheckpoint §8 :236-237).
const RECORD_KIND: ProtocolRecordKindV1 = ProtocolRecordKindV1::BootstrapIntent;

/// One bootstrapped managed parent, as the writer receives it: the opaque
/// retained-parent proof of ConsumerCheckpoint §9 (:264-266). It carries the
/// same typed durable facts the plan row does — never a path string, never a
/// handle.
pub(in crate::checked_artifact) struct RetainedManagedParentRowV1 {
    purpose: ManagedParentPurpose,
    identity: DurableObjectIdentityV1,
    mode: PathComponentMode,
    path: CanonicalPathIdentityV1,
}

impl RetainedManagedParentRowV1 {
    pub(in crate::checked_artifact) const fn purpose(&self) -> ManagedParentPurpose {
        self.purpose
    }

    pub(in crate::checked_artifact) const fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }

    pub(in crate::checked_artifact) const fn mode(&self) -> PathComponentMode {
        self.mode
    }

    pub(in crate::checked_artifact) const fn path(&self) -> &CanonicalPathIdentityV1 {
        &self.path
    }
}

/// The provider's `RetainedParents`: one proof per executed plan row, in the
/// plan's own row order.
pub(in crate::checked_artifact) struct RetainedManagedParentsV1 {
    rows: Vec<RetainedManagedParentRowV1>,
}

impl RetainedManagedParentsV1 {
    pub(in crate::checked_artifact) fn rows(&self) -> &[RetainedManagedParentRowV1] {
        &self.rows
    }

    pub(in crate::checked_artifact) fn row(
        &self,
        purpose: ManagedParentPurpose,
    ) -> Option<&RetainedManagedParentRowV1> {
        self.rows.iter().find(|row| row.purpose == purpose)
    }
}

/// The production managed-parent bootstrap provider over one retained catalog.
pub(in crate::checked_artifact) struct RetainedManagedParentProviderV1<'catalog, 'lease> {
    catalog: &'catalog OpaqueRetainedCatalogV1<'lease>,
    instance: [u8; 32],
}

impl<'catalog, 'lease> RetainedManagedParentProviderV1<'catalog, 'lease> {
    /// The only constructor. The provider instance is derived from the retained
    /// catalog's own durable identity, so it is stable across the preflight,
    /// bind and execute of one target and differs between targets — which is
    /// exactly the binding `ManagedParentBootstrapOwnerV1` re-proves before it
    /// executes a bound plan (`owner.rs` `execute`).
    pub(in crate::checked_artifact) fn from_retained_catalog(
        catalog: &'catalog OpaqueRetainedCatalogV1<'lease>,
    ) -> Result<Self, CheckedFsError> {
        let instance = catalog.managed_provider_instance()?;
        Ok(Self { catalog, instance })
    }

    /// One generation's three scheduled rows, from the resident schedule.
    fn generation_slots(
        &self,
        namespace: &ActionNamespace<HostActionNamespaceV1>,
        bootstrap_index: usize,
        generation: usize,
    ) -> Result<BootstrapGenerationSlots, CheckedFsError> {
        namespace
            .bootstrap_slots(bootstrap_index)
            .map_err(|_| provider_refusal("bootstrap row is not scheduled"))?
            .generation(generation)
            .map_err(|_| provider_refusal("bootstrap generation is not scheduled"))
    }

    /// R2-D Phase 3 Step 3.1b — the resume, read from the durable intent chain
    /// rather than re-derived from evidence.
    ///
    /// The walk is the reason the retired generations are kept at all: each
    /// record names its predecessor's `intent_id`, and
    /// `read_and_bind_managed_bootstrap_intent` refuses a record whose
    /// predecessor is not the one just read — so the chain is verified link by
    /// link from its first generation, not adopted from whichever row happens to
    /// be resident. It is bounded by the row's own scheduled generation range.
    ///
    /// Classification is exact rather than heuristic, because the publish/retire
    /// order makes the durable states disjoint: a generation is retired only
    /// after its successor is published, so "some row retired and no row active"
    /// can only be the row's completion.
    fn resume_intent(
        &self,
        namespace: &ActionNamespace<HostActionNamespaceV1>,
        plan: &BoundManagedParentPlanV1,
        row: &BoundManagedParentPlanRowV1,
    ) -> Result<IntentResumeV1, CheckedFsError> {
        let bootstrap_index = row.bootstrap_ordinal().index();
        let mut predecessor = None;
        let mut current = None;
        let mut retired_any = false;
        for generation in row.generation_range() {
            let slots = self.generation_slots(namespace, bootstrap_index, generation)?;
            let active = namespace.bootstrap_intent_row_is_resident(&slots, ROW_ACTIVE);
            let retired = namespace.bootstrap_intent_row_is_resident(&slots, ROW_RETIRED);
            if !active && !retired {
                break;
            }
            let bytes = namespace
                .read_bootstrap_intent_row(&slots, if active { ROW_ACTIVE } else { ROW_RETIRED })?;
            let ordinal = BootstrapGenerationV1::new(generation)
                .map_err(|_| provider_refusal("bootstrap generation is not scheduled"))?;
            let bound = read_and_bind_managed_bootstrap_intent(
                Cursor::new(&bytes),
                plan,
                row.purpose(),
                ordinal,
                predecessor,
            )
            .map_err(|_| {
                provider_refusal("resident managed intent does not bind this plan's chain")
            })?;
            let intent = bound.value().clone();
            predecessor = Some(intent.intent_id());
            retired_any |= retired;
            if active {
                current = Some(intent);
            }
        }
        Ok(match (current, retired_any) {
            (Some(intent), _) => IntentResumeV1::Current(Box::new(intent)),
            (None, true) => IntentResumeV1::Settled,
            (None, false) => IntentResumeV1::Fresh,
        })
    }

    /// R2-D Phase 3 Step 3.1b — one generation made durable, and its predecessor
    /// retired.
    ///
    /// Both halves are guarded by residency, so a fresh drive performs both, a
    /// restart between them performs the second only, and a restart after both
    /// performs neither — and every one of the three re-crosses the same two
    /// observation boundaries, which is what makes them repeatable rather than
    /// single-crossing. The record reaches its active row through the Step-2.2
    /// backend's role-validated `publish_bootstrap_generation`, so this step adds
    /// no publication call site.
    fn settle_generation(
        &self,
        namespace: &mut ActionNamespace<HostActionNamespaceV1>,
        row: &BoundManagedParentPlanRowV1,
        intent: &ManagedParentBootstrapIntentV1,
    ) -> Result<(), CheckedFsError> {
        let bootstrap_index = row.bootstrap_ordinal().index();
        let generation = intent.generation_ordinal().index();
        let start = row.generation_range().start;
        let edge = if generation == start {
            ManagedIntentEdgeV1::Initial
        } else {
            ManagedIntentEdgeV1::Successor
        };
        let bytes = intent.encode_canonical();
        let slots = self.generation_slots(namespace, bootstrap_index, generation)?;
        if !namespace.bootstrap_intent_row_is_resident(&slots, ROW_ACTIVE) {
            namespace.write_bootstrap_intent_scratch(&slots, &bytes, edge)?;
            let source = namespace
                .retain_scheduled_source(slots.scratch_leaf().clone(), RECORD_KIND)
                .map_err(|_| provider_refusal("managed intent scratch is not retainable"))?;
            namespace.publish_bootstrap_generation(&source, &slots)?;
        }
        if namespace.observe_bootstrap_intent_row(&slots, ROW_ACTIVE, edge)? != bytes {
            return Err(provider_refusal(
                "resident managed intent is not the generation just published",
            ));
        }
        if generation > start {
            self.retire_generation(
                namespace,
                row,
                generation - 1,
                ManagedIntentEdgeV1::PriorGeneration,
            )?;
        }
        Ok(())
    }

    /// R2-D Phase 3 Step 3.1b — one generation's active record retired onto its
    /// scheduled retirement row, then reobserved. Guarded, so a restart past the
    /// rename reaches the same two boundaries without re-entering the edge.
    fn retire_generation(
        &self,
        namespace: &mut ActionNamespace<HostActionNamespaceV1>,
        row: &BoundManagedParentPlanRowV1,
        generation: usize,
        edge: ManagedIntentEdgeV1,
    ) -> Result<(), CheckedFsError> {
        let slots =
            self.generation_slots(namespace, row.bootstrap_ordinal().index(), generation)?;
        if namespace.bootstrap_intent_row_is_resident(&slots, ROW_ACTIVE) {
            let source = namespace
                .retain_scheduled_source(slots.active_leaf().clone(), RECORD_KIND)
                .map_err(|_| provider_refusal("managed intent generation is not retainable"))?;
            namespace.retire_bootstrap_generation(&source, &slots)?;
        }
        namespace.observe_bootstrap_intent_row(&slots, ROW_RETIRED, edge)?;
        Ok(())
    }

    /// The facts of the deepest durably present ancestor of one declared
    /// managed path.
    fn observe_spec(
        &self,
        components: &[AsciiComponent],
    ) -> Result<(usize, ManagedParentFactsV1), CheckedFsError> {
        let observed = self.catalog.observe_managed_prefix(components)?;
        let depth = observed.retained_count();
        let facts = observed
            .at(depth)
            .ok_or_else(|| provider_refusal("managed parent path has no retained ancestor"))?;
        Ok((
            depth,
            ManagedParentFactsV1 {
                identity: facts.identity().clone(),
                mode: facts.mode(),
                path: facts.path().clone(),
            },
        ))
    }

    /// One component's half of the sequence, driven once, returning the
    /// successor its durable evidence closes.
    ///
    /// Each half is entered only when its own durable row is absent, so a
    /// restart on either side of a physical edge replays through the restart
    /// observation instead of re-crossing the edge.
    fn advance_one(
        &self,
        namespace: &mut ActionNamespace<HostActionNamespaceV1>,
        plan: &BoundManagedParentPlanV1,
        row: &BoundManagedParentPlanRowV1,
        intent: &ManagedParentBootstrapIntentV1,
    ) -> Result<ManagedParentBootstrapIntentV1, CheckedFsError> {
        let cursor = intent.cursor();
        let component = intent
            .components()
            .get(cursor)
            .ok_or_else(|| provider_refusal("managed intent has no current component"))?;
        let staging_leaf = component.staging_name().clone();
        let final_leaf = component.final_name().clone();
        let parent = self.catalog.retain_managed_prefix(
            row.components(),
            row.retained_existing_parent_count() + cursor,
            plan.reservation_digest(),
        )?;
        let installed_resident = parent.row_is_resident(&final_leaf);
        if intent.phase() == ManagedBootstrapPhaseV1::InstallComponents && !installed_resident {
            let marker = OwnershipMarkerV1::for_current_component(intent)
                .map_err(|_| provider_refusal("managed install intent cannot issue its marker"))?;
            parent.stage_component(&staging_leaf, &marker)?;
        }
        let slots = namespace.retain_managed_component_slots(
            parent,
            row.bootstrap_ordinal().index(),
            row.component_range().start + cursor,
            final_leaf,
        )?;
        match intent.phase() {
            ManagedBootstrapPhaseV1::InstallComponents => {
                if !installed_resident {
                    let source = namespace.retain_managed_staging_source(intent, &slots)?;
                    namespace.install_bootstrap_component(&source, intent, &slots)?;
                }
                let installed = namespace.recover_installed_bootstrap_component(intent, &slots)?;
                intent.successor_after_component(&installed)
            }
            ManagedBootstrapPhaseV1::RetireMarkers => {
                let retired = if namespace.scheduled_row_is_resident(slots.marker_retired_leaf()) {
                    namespace.recover_retired_bootstrap_marker(intent, &slots)?
                } else {
                    let source = namespace.retain_managed_marker_source(&slots)?;
                    namespace.retire_bootstrap_marker(&source, intent, &slots)?
                };
                intent.successor_after_marker_retirement(&retired)
            }
            ManagedBootstrapPhaseV1::Complete => {
                return Err(provider_refusal("managed intent is complete mid-drive"));
            }
        }
        .map_err(|_| provider_refusal("managed evidence does not close the current component"))
    }

    /// One plan row, driven to completion.
    ///
    /// **The restart story, in one place.** The row's state is the *resident
    /// intent chain*, not a re-derivation: [`Self::resume_intent`] reads it,
    /// verified link by link, and the drive continues from whatever phase and
    /// cursor that record names. Each generation is made durable before the next
    /// component's work begins, and its predecessor is retired immediately after,
    /// so at most two active records exist at once and the resume always finds
    /// the newest.
    ///
    /// This is what closes the window Step 3.1 could only refuse. Re-deriving the
    /// chain from evidence required the ownership marker to still be inside its
    /// installed component, which edge E16 removes — so a row interrupted *inside*
    /// the marker-retirement phase (any row with two or more missing components:
    /// the ordinary first-merge `.gwz/stash/bundles`) could never converge. With
    /// the record resident, that same interruption resumes at exactly its cursor.
    fn execute_row(
        &self,
        plan: &BoundManagedParentPlanV1,
        row: &BoundManagedParentPlanRowV1,
    ) -> Result<RetainedManagedParentRowV1, CheckedFsError> {
        let components = row.components();
        let mut namespace: ActionNamespace<HostActionNamespaceV1> =
            retain_action_namespace(self.catalog, plan.admitted_action().clone())?;

        let mut pending = match self.resume_intent(&namespace, plan, row)? {
            IntentResumeV1::Settled => None,
            IntentResumeV1::Current(intent) => Some(*intent),
            IntentResumeV1::Fresh => Some(
                ManagedParentBootstrapIntentV1::try_initial(
                    plan,
                    row.purpose(),
                    ownership_token(plan),
                )
                .map_err(|_| {
                    provider_refusal("managed bootstrap intent does not bind the admitted plan")
                })?,
            ),
        };

        while let Some(intent) = pending {
            self.settle_generation(&mut namespace, row, &intent)?;
            if intent.is_complete() {
                self.retire_generation(
                    &mut namespace,
                    row,
                    intent.generation_ordinal().index(),
                    ManagedIntentEdgeV1::FinalRetirement,
                )?;
                break;
            }
            pending = Some(self.advance_one(&mut namespace, plan, row, &intent)?);
        }

        // The final reproof (plan §4 Step 3.1): the whole declared path is
        // re-observed through the same bounded walk, so the proof the writer
        // receives is a fresh durable observation rather than the last edge's
        // own report.
        let (depth, facts) = self.observe_spec(components)?;
        if depth != components.len() {
            return Err(provider_refusal(
                "managed parent is not fully resident after execution",
            ));
        }
        Ok(RetainedManagedParentRowV1 {
            purpose: row.purpose(),
            identity: facts.identity,
            mode: facts.mode,
            path: facts.path,
        })
    }
}

impl ManagedParentBootstrap for RetainedManagedParentProviderV1<'_, '_> {
    type RetainedParents = RetainedManagedParentsV1;

    fn provider_instance_id(&self) -> [u8; 32] {
        self.instance
    }

    fn observe_preflight(
        &self,
        request: &ManagedParentBootstrapRequest,
    ) -> Result<Vec<ManagedParentObservationV1>, CheckedFsError> {
        let mut observations = Vec::new();
        observations
            .try_reserve_exact(request.specs().len())
            .map_err(|_| allocation_failure("managed-parent observation allocation failed"))?;
        for spec in request.specs() {
            let (depth, facts) = self.observe_spec(spec.components())?;
            observations.push(ManagedParentObservationV1::new(
                spec.purpose(),
                depth,
                facts.identity,
                facts.mode,
                facts.path,
            )?);
        }
        Ok(observations)
    }

    /// Restart-closed by construction. The plan records the prefix that existed
    /// when it was made, so a drive that has already installed components sees a
    /// *deeper* prefix — which is convergence, not staleness. What must still
    /// hold is that the recorded retained parent is durably the same object at
    /// the same depth, and that is exactly what is compared.
    fn revalidate_plan(&self, plan: &ManagedParentPlanV1) -> Result<bool, CheckedFsError> {
        for row in plan.rows() {
            let observed = self.catalog.observe_managed_prefix(row.components())?;
            let recorded = row.retained_existing_parent_count();
            let Some(facts) = observed.at(recorded) else {
                return Ok(false);
            };
            if facts.identity() != row.retained_parent_identity()
                || facts.mode() != row.retained_parent_mode()
                || facts.path() != row.retained_parent_path()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn execute_bound(
        &self,
        plan: &BoundManagedParentPlanV1,
    ) -> Result<Self::RetainedParents, CheckedFsError> {
        let mut rows = Vec::new();
        rows.try_reserve_exact(plan.rows().len())
            .map_err(|_| allocation_failure("retained managed-parent allocation failed"))?;
        for row in plan.rows() {
            rows.push(self.execute_row(plan, row)?);
        }
        Ok(RetainedManagedParentsV1 { rows })
    }
}

struct ManagedParentFactsV1 {
    identity: DurableObjectIdentityV1,
    mode: PathComponentMode,
    path: CanonicalPathIdentityV1,
}

/// What the resident intent chain says a row's next drive must do.
///
/// The three states are disjoint by construction of the publish/retire order: a
/// generation is retired only after its successor is published, so "a retired
/// generation and no active one" can only be the row's own completion.
enum IntentResumeV1 {
    /// No generation of this row is resident: the drive starts the chain.
    Fresh,
    /// The newest resident active generation, verified link by link from the
    /// row's first. Boxed because the intent record is by far the largest thing
    /// this enum carries.
    Current(Box<ManagedParentBootstrapIntentV1>),
    /// Every generation is retired: the row is complete and only owes its final
    /// reproof.
    Settled,
}

/// The intent chain's ownership token, derived rather than allocated.
///
/// It must be reproducible: the token feeds every intent id, every intent id
/// feeds the ownership marker each staged component carries, and a restart
/// re-derives the chain from the bound plan before it compares markers on disk.
/// A random token would make the second drive of an interrupted bootstrap
/// disown its own staged component — and allocating one would be the very
/// nondeterminism the R2 stop clause forbids. Every input is already durable and
/// already bound to this one admitted action.
///
/// This derivation is the **first-generation seed only**. Step 3.1b landed the
/// intent record's durable lifecycle, so every later drive of a row takes the
/// token from the resident record — `resume_intent` returns the chain and
/// `try_initial` runs only when no generation is resident at all. The boundary
/// the module header states applies to that read-back: it is self-consistency,
/// not exclusion.
fn ownership_token(plan: &BoundManagedParentPlanV1) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"gwz-managed-bootstrap-ownership-token-v1\0");
    digest.update(plan.plan().digest());
    digest.update(plan.action_digest().bytes());
    digest.update(plan.reservation_digest().bytes());
    digest.update(plan.schedule_digest().bytes());
    digest.update(plan.request_owner_binding().bytes());
    digest.finalize().into()
}

fn provider_refusal(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("managed-parent provider", detail)
}

fn allocation_failure(detail: &'static str) -> CheckedFsError {
    CheckedFsError::unsupported(PlatformCapability::ManagedParentBootstrap, detail)
}
