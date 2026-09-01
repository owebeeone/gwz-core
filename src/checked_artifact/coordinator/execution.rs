//! Coordinator execution glue: schedule, admission binding, and the two gates
//! a writer must pass.
//!
//! R2-D Phase 3 Step 3.3 (`GwzM5-8R2D-Plan.md` §4): "schedule +
//! `AdmittedActionV1` binding so that *replacement/removal executes only after
//! an admitted action and an owner-private coherent authority observation*
//! (`GwzM5-8R4bR2ConsumerCheckpoint.md` §8 :239-240); writers receive only the
//! opaque retained-parent proof (:264-266)".
//!
//! **This step wires machinery; it does not convert consumers** (plan §4 Step
//! 3.3; conversion is R2-E), and the reachability statement that follows from
//! that is worth being exact about.
//!
//! What this owner *does* change: the Phase-1 admission owner
//! (`ActionAdmissionOwnerV1`), the Phase-3 managed-parent provider
//! (`RetainedManagedParentProviderV1`) and the coordinator's own schedule facade
//! (`derive_new_reservation`) now have production callers — here. Before this
//! step each was reachable only from tests.
//!
//! What it does **not** change: nothing here is reachable from an entry point.
//! The whole owner is `pub(in crate::checked_artifact)`, no consumer calls it,
//! and `entry.rs` still routes parent preparation through
//! `CheckedArtifact::prepare_parent`. Production *catalog activation* also
//! remains forbidden outright for the whole of R2-D (plan §5 item 2). So the
//! gate stays shut, deliberately, and R2-E's conversion is what opens it.
//!
//! A note for whoever audits those allows, **corrected at Phase 4 Step 5.1**
//! because Step 4.3's narrowing falsified the previous version of it. The
//! subtrees still carrying a blanket `#[allow(dead_code)]` at
//! `checked_artifact/mod.rs` are `bootstrap`, `capability`, `entry`, `fault_v1`,
//! `leaf`, `namespace` and `protocol` — **`coordinator` is not among them any
//! more**: Step 4.3 (settle item 7) deleted its subtree blanket and moved the
//! cover inward onto `mod identity`, which is where the family's remaining
//! frozen surface actually lives. So the `#[allow(dead_code)]` on `mod provider`
//! and `mod host` is inert only where an *enclosing* blanket still covers it,
//! while the one on **this** module is live and load-bearing: with the
//! coordinator blanket gone, it is the only thing suppressing the lint over
//! `execution`. Do not read it as decoration and delete it.
//!
//! Three properties are structural rather than advisory.
//!
//! * **The schedule and the admitted action are one binding.** An
//!   [`AdmittedCheckedActionV1`] exists only when the admission owner returned
//!   an `AdmittedActionV1` whose reservation *is* the one this checked request
//!   derived — compared, not assumed. A caller cannot pair one request's
//!   schedule with another action's admission.
//! * **Replacement and removal need both gates, and the second is checked
//!   against observed provenance.** [`RetainedWriteAuthorityV1`] is minted only
//!   from an [`AdmittedCheckedActionV1`] *and* a `CheckedAuthorityObservationV1`
//!   whose payloads were streamed under **this** admitted action's own retained
//!   directory. The observation's four reservation fields are copied from
//!   whatever reservation its issuer was handed, so they say only what the
//!   caller named; its `retained_parent_identity` is minted from the capability
//!   the payloads actually went through, so that is what the gate compares — see
//!   [`AdmittedCheckedActionV1::authorize_write`]. Checking the restated fields
//!   alone would accept an observation streamed under another action, which is
//!   the defect the Step-3.3 review's [P1-1] found in the first landing of this
//!   file and which the suite now drives directly.
//! * **Writers never hold a path.** A writer receives
//!   [`ManagedParentFacadeV1`], which carries the opaque retained-parent proof
//!   and no path string, and whose one operation re-observes the durable prefix
//!   before it yields anything. A successful `exists()` is not parent authority
//!   here because no `exists()` is available to the holder.

use super::schedule::{CoordinatorScheduleDecisionV1, derive_new_reservation};
use super::{
    CheckedActionOperationV1, CheckedActionRequestV1, CheckedLeafFactV1, CheckedManagedActionV1,
};
use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::{
    ManagedParentBootstrapOwnerV1, ManagedParentPlanV1, ManagedParentPurpose,
    RetainedManagedParentProviderV1, RetainedManagedParentRowV1, RetainedManagedParentsV1,
};
use crate::checked_artifact::capability::{
    CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1, PathComponentMode,
};
use crate::checked_artifact::catalog::OpaqueRetainedCatalogV1;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, AdmittedActionV1, CheckedAuthorityObservationV1,
    CheckedAuthorityRecordV1,
};

/// What the coordinator's schedule half decided for one checked request.
pub(in crate::checked_artifact) enum CheckedExecutionPlanV1 {
    /// The request reserves no capacity and admits no action: an observation, a
    /// no-op replacement, or a parent-only action whose managed plan is already
    /// proof-only. It can never reach a write authority, which is the point of
    /// keeping it a distinct arm rather than an empty reservation.
    ProofOnly,
    Scheduled(Box<ScheduledCheckedActionV1>),
}

/// R2-D Step 3.3 — the schedule half of the binding.
///
/// This is `derive_new_reservation`'s production caller: the coordinator's
/// private schedule facade was frozen in R2 "before production consumers are
/// converted", and this is the consumer inside the crate boundary.
pub(in crate::checked_artifact) fn schedule_checked_action(
    request: &CheckedActionRequestV1,
    managed_plan: Option<&ManagedParentPlanV1>,
) -> Result<CheckedExecutionPlanV1, CheckedFsError> {
    match derive_new_reservation(request, managed_plan)? {
        CoordinatorScheduleDecisionV1::ProofOnly => Ok(CheckedExecutionPlanV1::ProofOnly),
        CoordinatorScheduleDecisionV1::Reserve(reservation) => Ok(
            CheckedExecutionPlanV1::Scheduled(Box::new(ScheduledCheckedActionV1 {
                request: request.clone(),
                reservation: *reservation,
            })),
        ),
    }
}

/// R2-E Step E4.2 — the first merge record's parent half, admitted.
///
/// The admission session of ConsumerCheckpoint §10 row `:273`: preflight the
/// sealed merge-start action against durable state, schedule it, admit it. It
/// returns `None` on a proof-only plan, which is precisely the row's "when
/// missing" qualifier — both prefixes already fully resident, nothing to create.
/// The action is rebuilt from `workspace_id` rather than carried, so execution
/// re-derives the identical sealed request instead of trusting a handed one.
pub(in crate::checked_artifact) fn admit_merge_start_managed_parents(
    workspace_id: &str,
    catalog: OpaqueRetainedCatalogV1<'_>,
) -> Result<Option<AdmittedCheckedActionV1>, CheckedFsError> {
    let action = CheckedManagedActionV1::for_merge_start(workspace_id)?;
    let plan = {
        let provider = RetainedManagedParentProviderV1::from_retained_catalog(&catalog)?;
        ManagedParentBootstrapOwnerV1::new(&provider).preflight_checked(&action)?
    };
    match schedule_checked_action(action.checked(), Some(&plan))? {
        CheckedExecutionPlanV1::ProofOnly => Ok(None),
        CheckedExecutionPlanV1::Scheduled(scheduled) => scheduled.admit(catalog).map(Some),
    }
}

/// The execution session of the same row: install the missing prefixes, then
/// re-prove each through its own facade before the caller may write.
///
/// **The re-proof is not decoration.** E7.2's two scope clauses say a settled
/// barrier ordinal does not imply ordered parent dirents and a converged restart
/// does not imply a flush, so the row's durability rests on the provider's own
/// install-and-reobserve — each generation made durable before the next, then a
/// reproof of the whole declared path — never on this caller's observation.
pub(in crate::checked_artifact) fn execute_merge_start_managed_parents(
    workspace_id: &str,
    admitted: &AdmittedCheckedActionV1,
    catalog: &OpaqueRetainedCatalogV1<'_>,
) -> Result<Vec<ManagedParentPurpose>, CheckedFsError> {
    let action = CheckedManagedActionV1::for_merge_start(workspace_id)?;
    admitted
        .bootstrap_managed_parents(catalog, &action)?
        .iter()
        .map(|facade| facade.revalidate(catalog).map(|proved| proved.purpose()))
        .collect()
}

/// One checked request bound to the reservation it derives, before admission.
pub(in crate::checked_artifact) struct ScheduledCheckedActionV1 {
    request: CheckedActionRequestV1,
    reservation: ActionCapacityReservationV1,
}

impl ScheduledCheckedActionV1 {
    pub(in crate::checked_artifact) const fn reservation(&self) -> &ActionCapacityReservationV1 {
        &self.reservation
    }

    /// R2-D Step 3.3 — the `AdmittedActionV1` binding.
    ///
    /// This is the Phase-1 admission owner's production caller. The owner
    /// consumes the opaque retained catalog, exactly as its frozen seam requires
    /// (`GwzM5-8R2DInterfaceFreeze.md` §3.1), so admission is a session of its
    /// own and execution re-acquires the lease afterwards — which is the lease
    /// model, not a limitation of this glue.
    ///
    /// The returned handoff is checked against the reservation this request
    /// derived before it is bound. `resume_or_admit` already refuses an action
    /// directory that is not exactly this reservation's, so the comparison is
    /// belt-and-braces at the seam the coordinator owns rather than a
    /// restatement of the driver's own rule.
    pub(in crate::checked_artifact) fn admit(
        &self,
        catalog: OpaqueRetainedCatalogV1<'_>,
    ) -> Result<AdmittedCheckedActionV1, CheckedFsError> {
        let admitted = ActionAdmissionOwnerV1::from_retained_catalog(catalog)
            .resume_or_admit(&self.reservation)?;
        if admitted.reservation() != &self.reservation {
            return Err(execution_error(
                "the admitted action is not the one this checked request scheduled",
            ));
        }
        Ok(AdmittedCheckedActionV1 {
            request: self.request.clone(),
            admitted,
        })
    }
}

/// A checked request whose action is admitted: gate one, passed.
pub(in crate::checked_artifact) struct AdmittedCheckedActionV1 {
    request: CheckedActionRequestV1,
    admitted: AdmittedActionV1,
}

impl AdmittedCheckedActionV1 {
    pub(in crate::checked_artifact) const fn admitted(&self) -> &AdmittedActionV1 {
        &self.admitted
    }

    pub(in crate::checked_artifact) fn action_digest(&self) -> ActionDigestV1 {
        self.admitted.reservation().action_digest()
    }

    /// R2-D Step 3.3 — the managed-parent provider's production caller.
    ///
    /// Preflight, bind and execute, in the owner's own order, against this
    /// admitted action. The plan is re-derived here rather than carried from the
    /// scheduling session because `revalidate_plan` must run against the durable
    /// state *this* session observes; the owner then refuses a plan that does not
    /// reproduce the resident schedule, so a stale plan cannot execute.
    ///
    /// **The result is the opaque facade set, and nothing else** (Step-3.3 review
    /// [P2-1]). The provider's own `RetainedManagedParentsV1` rows expose
    /// `path()`, which the provider's internals legitimately need — the plan's
    /// clause governs what *writers* receive, not what the owner computes. So the
    /// rows are consumed here and never leave this method: a caller of the
    /// coordinator surface can obtain a managed parent only as a
    /// [`ManagedParentFacadeV1`], which exposes no path at all. "Writers receive
    /// only the opaque retained-parent proof" is thereby a property of this
    /// signature rather than a convention a caller may decline to follow.
    pub(in crate::checked_artifact) fn bootstrap_managed_parents(
        &self,
        catalog: &OpaqueRetainedCatalogV1<'_>,
        action: &CheckedManagedActionV1,
    ) -> Result<Vec<ManagedParentFacadeV1>, CheckedFsError> {
        if action.checked().action_digest() != self.request.action_digest()
            || action.checked().owner_binding() != self.request.owner_binding()
        {
            return Err(execution_error(
                "the managed request belongs to another checked action",
            ));
        }
        let provider = RetainedManagedParentProviderV1::from_retained_catalog(catalog)?;
        let owner = ManagedParentBootstrapOwnerV1::new(&provider);
        let plan = owner.preflight_checked(action)?;
        let bound = owner.bind(&self.admitted, &plan)?;
        Ok(ManagedParentFacadeV1::all(&owner.execute(&bound)?))
    }

    /// R2-D Step 3.3 — gate two.
    ///
    /// `CheckedAuthorityObservationV1` is the owner-private coherent observation
    /// of ConsumerCheckpoint §8: R1 mints it only through the authority-facts
    /// issuer, and Step 2.4 made the only route to that issuer a
    /// `StreamedPayloadProofV1` taken under one retained action directory.
    ///
    /// **Three checks, and why the obvious one is not enough** (Step-3.3 review
    /// [P1-1]). The observation carries two kinds of field, and only one kind is
    /// evidence:
    ///
    /// * *Restated* — the action digest, owner binding, schedule digest and
    ///   reservation digest are **copied from the reservation the caller handed
    ///   the issuer** (`CheckedAuthorityObservationV1::owner_issue`). Comparing
    ///   them against this action's reservation, which is what
    ///   `matches_reservation` does, therefore only proves the caller *named*
    ///   this action. It cannot see an observation whose payloads were streamed
    ///   somewhere else, and `request_owner_binding` does not save it: that is
    ///   per merge owner, so every action of one workspace shares it.
    /// * *Observed* — `retained_parent_identity` is minted by
    ///   `observe_streamed_payloads` from the retained action directory the
    ///   payloads were actually streamed through, and carried forward untouched.
    ///   It is the observation's own provenance.
    ///
    /// So the decisive check is the third one below: the observation's observed
    /// provenance must be **this** admitted action's own retained directory,
    /// whose durable identity the handoff independently carries. Two admitted
    /// actions never share a directory — their names are derived from distinct
    /// action digests — so an observation streamed under another action is
    /// refused, typed and fail-closed, rather than minting authority here. This
    /// is the same principle as Step 2.4's own closure: the proof carries its
    /// provenance, and nothing trusts a caller's restatement of it.
    ///
    /// The fourth check binds the *leaf* facts: the record's expected and goal
    /// digests must be the ones this checked request's action digest was derived
    /// over, so an observation of the right directory but the wrong content
    /// cannot authorize either.
    ///
    /// Only `Replace` and `Remove` reach any of it. `ParentOnly` writes no leaf
    /// and therefore needs no leaf authority, and `Observe` never scheduled an
    /// action at all; both are refused here rather than silently granted a
    /// capability they have no use for.
    pub(in crate::checked_artifact) fn authorize_write(
        &self,
        observation: &CheckedAuthorityObservationV1,
    ) -> Result<RetainedWriteAuthorityV1, CheckedFsError> {
        if !matches!(
            self.request.operation(),
            CheckedActionOperationV1::Replace | CheckedActionOperationV1::Remove
        ) {
            return Err(execution_error(
                "only a replacement or removal takes a leaf write authority",
            ));
        }
        let record = CheckedAuthorityRecordV1::issue(observation)
            .map_err(|_| execution_error("the authority observation is not coherent"))?;
        if !record.matches_reservation(self.admitted.reservation()) {
            return Err(execution_error(
                "the authority observation was issued against another admitted action",
            ));
        }
        if record.retained_parent_identity() != self.admitted.directory_identity() {
            return Err(execution_error(
                "the authority observation was streamed under another action's retained directory",
            ));
        }
        require_leaf_digest(
            record.expected_sha256(),
            self.request.expected(),
            "expected",
        )?;
        require_leaf_digest(record.goal_sha256(), self.request.goal(), "goal")?;
        Ok(RetainedWriteAuthorityV1 {
            action: self.action_digest(),
            record_id: record.record_id(),
            retained_parent_identity: record.retained_parent_identity().clone(),
            expected_sha256: record.expected_sha256(),
            goal_sha256: record.goal_sha256(),
        })
    }
}

/// One leaf digest of the authority record against the checked request's own
/// canonical fact.
///
/// A `Missing` fact has no digest to compare — a removal's goal and a creating
/// replacement's expected are both `Missing` — so those pass this check and rest
/// on the three before it. That is a floor, deliberately: it narrows an
/// observation of the right directory and wrong content, and it is not the check
/// that closes cross-action minting.
fn require_leaf_digest(
    record_digest: [u8; 32],
    fact: CheckedLeafFactV1,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    match fact {
        CheckedLeafFactV1::Exact { sha256, .. } if sha256 != record_digest => {
            Err(execution_error(match label {
                "expected" => "the authority observation binds a different expected content",
                _ => "the authority observation binds a different goal content",
            }))
        }
        _ => Ok(()),
    }
}

/// Leaf write authority: proof that every §8 gate was passed, in order.
///
/// It carries no path, no handle and no mutation surface. What it *does* carry
/// is every fact a downstream consumer needs to re-check the grant rather than
/// take it on trust: the action, the authority record's own id, the retained
/// directory the payloads were observed through, and the expected and goal
/// digests. The Step-3.3 review noted that a token of `{action, record_id}`
/// alone forecloses the provenance check being made later; this shape does not.
///
/// **Deliberately neither `Copy` nor `Clone`** (Step-3.3 review [P3-3]). An
/// authority token that duplicates itself silently is one whose single-grant
/// reading is a convention; moving it is the only way to pass it on, so a
/// consumer that needs two must pass two gates. R2-E's consumers take it as the
/// argument that says "this replacement may proceed"; nothing in R2-D consumes
/// it yet, which is what "wires machinery, does not convert consumers" means for
/// this type.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct RetainedWriteAuthorityV1 {
    action: ActionDigestV1,
    record_id: [u8; 32],
    retained_parent_identity: DurableObjectIdentityV1,
    expected_sha256: [u8; 32],
    goal_sha256: [u8; 32],
}

impl RetainedWriteAuthorityV1 {
    pub(in crate::checked_artifact) const fn action(&self) -> ActionDigestV1 {
        self.action
    }

    pub(in crate::checked_artifact) const fn record_id(&self) -> [u8; 32] {
        self.record_id
    }

    /// The retained action directory the authorized payloads were observed
    /// through — the observation's provenance, carried so a consumer can re-prove
    /// the grant against its own capability.
    pub(in crate::checked_artifact) const fn retained_parent_identity(
        &self,
    ) -> &DurableObjectIdentityV1 {
        &self.retained_parent_identity
    }

    pub(in crate::checked_artifact) const fn expected_sha256(&self) -> [u8; 32] {
        self.expected_sha256
    }

    pub(in crate::checked_artifact) const fn goal_sha256(&self) -> [u8; 32] {
        self.goal_sha256
    }
}

/// R2-D Step 3.3 — the writer's view of one bootstrapped managed parent
/// (ConsumerCheckpoint §9 :264-266).
///
/// A writer holds this and nothing else. It exposes no path, so the checkpoint's
/// "a path string or successful `exists()` check is not parent authority" is a
/// property of the type rather than a rule to remember, and its one operation
/// re-observes the durable prefix before yielding a proof a writer may act on.
pub(in crate::checked_artifact) struct ManagedParentFacadeV1 {
    purpose: ManagedParentPurpose,
    identity: DurableObjectIdentityV1,
    mode: PathComponentMode,
    path: CanonicalPathIdentityV1,
}

impl ManagedParentFacadeV1 {
    /// The only constructor: one row of the provider's own retained-parent
    /// proof. There is no route from a path, a name, or an `exists()` check.
    ///
    /// Private to this owner, so the *only* way a caller obtains a facade is
    /// through [`AdmittedCheckedActionV1::bootstrap_managed_parents`], which
    /// consumes the path-bearing rows rather than returning them.
    fn from_retained_row(row: &RetainedManagedParentRowV1) -> Self {
        Self {
            purpose: row.purpose(),
            identity: row.identity().clone(),
            mode: row.mode(),
            path: row.path().clone(),
        }
    }

    /// Every facade of a retained-parents proof, in the proof's own row order.
    fn all(retained: &RetainedManagedParentsV1) -> Vec<ManagedParentFacadeV1> {
        retained
            .rows()
            .iter()
            .map(Self::from_retained_row)
            .collect()
    }

    pub(in crate::checked_artifact) const fn purpose(&self) -> ManagedParentPurpose {
        self.purpose
    }

    /// The facade operation ConsumerCheckpoint §9 (:266) requires: a write may
    /// happen only through an operation that **revalidates the proof**.
    ///
    /// The prefix is re-observed through the opaque catalog — the same bounded,
    /// identity-proved walk the provider planned with — and the durable facts
    /// must still be the ones the proof carries. Drift of identity, of the
    /// child mode, of the canonical path, or of depth is a typed refusal, so a
    /// parent that was replaced between the bootstrap and the write cannot be
    /// written through.
    pub(in crate::checked_artifact) fn revalidate(
        &self,
        catalog: &OpaqueRetainedCatalogV1<'_>,
    ) -> Result<RevalidatedManagedParentV1, CheckedFsError> {
        let components = self
            .path
            .components()
            .iter()
            .map(|component| component.original().clone())
            .collect::<Vec<_>>();
        let observed = catalog.observe_managed_prefix(&components)?;
        let facts = observed
            .at(components.len())
            .ok_or_else(|| execution_error("the managed parent is no longer resident"))?;
        if facts.identity() != &self.identity
            || facts.mode() != self.mode
            || facts.path() != &self.path
        {
            return Err(execution_error(
                "the managed parent changed since its retained proof was issued",
            ));
        }
        Ok(RevalidatedManagedParentV1 {
            purpose: self.purpose,
            identity: self.identity.clone(),
        })
    }
}

/// A managed parent proved still durable at the moment of the write.
///
/// Re-proving is the only way to obtain another one, and the missing `Clone` is
/// what makes that true rather than merely intended (Step-3.3 review [P3-3]): it
/// borrows nothing and carries no handle, so a holder who needs a second proof
/// must go back through [`ManagedParentFacadeV1::revalidate`] and re-observe.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct RevalidatedManagedParentV1 {
    purpose: ManagedParentPurpose,
    identity: DurableObjectIdentityV1,
}

impl RevalidatedManagedParentV1 {
    pub(in crate::checked_artifact) const fn purpose(&self) -> ManagedParentPurpose {
        self.purpose
    }

    pub(in crate::checked_artifact) const fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }
}

fn execution_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("checked action execution", detail)
}
