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
//! * **A drive is derived from durable state, never replanned.** Each component
//!   asks the durable namespace which half of its sequence is already there and
//!   enters only the half that is not, so a restart re-derives the identical
//!   intent chain and the identical slots (plan §4 Step 3.1: "restart consumes
//!   the resident intent and scheduled slots, never replans a partially
//!   completed live path").
//!
//! **What Step 3.1 does not land, stated once.** The intent record's own
//! durable lifecycle — the initial intent publication, the per-generation
//! successor publication, the prior-generation retirement and the final intent
//! retirement (freeze §4.3 row E17 and the `managed_bootstrap.*` keys the §3.5
//! annotation reserves for them) — is not written here. In its place the intent
//! chain is re-derived deterministically from the bound plan and the *durable
//! evidence* of each completed component, which closes the restart everywhere
//! except one window: replaying an install needs the ownership marker still
//! inside its component, and edge E16 removes it, so a row whose markers are
//! only *partly* retired cannot be replayed and is refused, typed and unmutated,
//! rather than re-attempted (`execute_row`). Persisting the chain is what closes
//! that window, and it is the named follow-up, together with E17's §4.4 Class 1
//! arms. No `managed_bootstrap.*` key changes activation state in this step: the
//! plan assigns `managed_bootstrap.*` activation and its matrix to Step 3.2.

use std::ops::Range;

use sha2::{Digest, Sha256};

use super::owner::ManagedParentBootstrap;
use super::plan::{BoundManagedParentPlanRowV1, BoundManagedParentPlanV1, ManagedParentPlanV1};
use super::{ManagedParentBootstrapRequest, ManagedParentObservationV1, ManagedParentPurpose};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
    PathComponentMode, PlatformCapability,
};
use crate::checked_artifact::catalog::OpaqueRetainedCatalogV1;
use crate::checked_artifact::namespace::{
    ActionNamespace, HostActionNamespaceV1, retain_action_namespace,
};
use crate::checked_artifact::protocol::{
    ActionDigestV1, ActionSlotV1, ManagedBootstrapPhaseV1, ManagedParentBootstrapIntentV1,
    OwnershipMarkerV1,
};

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

    /// Which of the three resumable states a row's scheduled retirement rows
    /// put it in. Read-only, and derived from the schedule's own deterministic
    /// slot names, so it allocates nothing and probes nothing else.
    fn classify_resume(
        &self,
        namespace: &ActionNamespace<HostActionNamespaceV1>,
        action: ActionDigestV1,
        component_range: &Range<usize>,
    ) -> Result<RowResumeV1, CheckedFsError> {
        let mut retired = 0;
        for ordinal in component_range.clone() {
            if namespace.scheduled_row_is_resident(&retirement_leaf(action, ordinal)?) {
                retired += 1;
            }
        }
        if retired == 0 {
            Ok(RowResumeV1::FromFirstGeneration)
        } else if retired == component_range.len() {
            Ok(RowResumeV1::Settled)
        } else {
            Err(provider_refusal(
                "managed bootstrap row is partially retired and needs its resident intent to resume",
            ))
        }
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

    /// One plan row, driven to completion.
    ///
    /// The loop is the whole restart story: the intent's own phase and cursor
    /// select the next boundary, both are re-derived from durable evidence on
    /// every drive, and each half is entered only when its durable row is
    /// absent.
    ///
    /// **Where the re-derivation stops, stated exactly.** The chain is
    /// re-derived by *replaying evidence*, and the only evidence that closes an
    /// install is the ownership marker still inside the installed component —
    /// which edge E16 deliberately removes. So a row whose retirement rows are
    /// all resident is already settled and short-circuits to the final reproof
    /// (the common restart, and the whole restart for a one-component row), a
    /// row with none of them re-derives from the first generation, and a row
    /// with *some* of them is the one window a re-derivation cannot cross: it
    /// is refused, typed and unmutated, until the resident intent record lands
    /// (see the module header's named follow-up). It is refused rather than
    /// re-attempted precisely because re-attempting would ask
    /// `observe_installed` for a marker the retirement already moved, and that
    /// refusal would look like drift rather than an unfinished sequence.
    fn execute_row(
        &self,
        plan: &BoundManagedParentPlanV1,
        row: &BoundManagedParentPlanRowV1,
    ) -> Result<RetainedManagedParentRowV1, CheckedFsError> {
        let reservation = plan.reservation_digest();
        let mut intent =
            ManagedParentBootstrapIntentV1::try_initial(plan, row.purpose(), ownership_token(plan))
                .map_err(|_| {
                    provider_refusal("managed bootstrap intent does not bind the admitted plan")
                })?;
        let mut namespace: ActionNamespace<HostActionNamespaceV1> =
            retain_action_namespace(self.catalog, plan.admitted_action().clone())?;
        let bootstrap_index = row.bootstrap_ordinal().index();
        let component_range = row.component_range();
        let base_depth = row.retained_existing_parent_count();
        let components = row.components();
        let resume = self.classify_resume(&namespace, plan.action_digest(), &component_range)?;

        while resume == RowResumeV1::FromFirstGeneration && !intent.is_complete() {
            let cursor = intent.cursor();
            let component = intent
                .components()
                .get(cursor)
                .ok_or_else(|| provider_refusal("managed intent has no current component"))?;
            let staging_leaf = component.staging_name().clone();
            let final_leaf = component.final_name().clone();
            let parent =
                self.catalog
                    .retain_managed_prefix(components, base_depth + cursor, reservation)?;
            let installed_resident = parent.row_is_resident(&final_leaf);
            if intent.phase() == ManagedBootstrapPhaseV1::InstallComponents && !installed_resident {
                let marker = OwnershipMarkerV1::for_current_component(&intent).map_err(|_| {
                    provider_refusal("managed install intent cannot issue its marker")
                })?;
                parent.stage_component(&staging_leaf, &marker)?;
            }
            let slots = namespace.retain_managed_component_slots(
                parent,
                bootstrap_index,
                component_range.start + cursor,
                final_leaf,
            )?;
            intent = match intent.phase() {
                ManagedBootstrapPhaseV1::InstallComponents => {
                    if !installed_resident {
                        let source = namespace.retain_managed_staging_source(&intent, &slots)?;
                        namespace.install_bootstrap_component(&source, &intent, &slots)?;
                    }
                    let installed =
                        namespace.recover_installed_bootstrap_component(&intent, &slots)?;
                    intent.successor_after_component(&installed)
                }
                ManagedBootstrapPhaseV1::RetireMarkers => {
                    let retired =
                        if namespace.scheduled_row_is_resident(slots.marker_retired_leaf()) {
                            namespace.recover_retired_bootstrap_marker(&intent, &slots)?
                        } else {
                            let source = namespace.retain_managed_marker_source(&slots)?;
                            namespace.retire_bootstrap_marker(&source, &intent, &slots)?
                        };
                    intent.successor_after_marker_retirement(&retired)
                }
                ManagedBootstrapPhaseV1::Complete => {
                    return Err(provider_refusal("managed intent is complete mid-drive"));
                }
            }
            .map_err(|_| {
                provider_refusal("managed evidence does not close the current component")
            })?;
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

/// The two resumable states a row can be driven from. The third — partially
/// retired — is a typed refusal rather than a state, so it has no variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowResumeV1 {
    FromFirstGeneration,
    Settled,
}

/// One scheduled marker-retirement row name, derived from the frozen slot
/// grammar. Nothing is minted: this is the same name
/// `BootstrapSlots::component` binds into the component's slots.
fn retirement_leaf(
    action: ActionDigestV1,
    ordinal: usize,
) -> Result<AsciiComponent, CheckedFsError> {
    let ordinal = u8::try_from(ordinal)
        .map_err(|_| provider_refusal("managed component ordinal is not scheduled"))?;
    AsciiComponent::parse(
        ActionSlotV1::RetiredBootstrapMarker(ordinal)
            .name(action)
            .as_bytes(),
    )
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
/// When the follow-up lands the intent record's durable lifecycle the token is
/// read back from the resident record instead of re-derived; the derivation
/// stays as the first-generation seed.
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
