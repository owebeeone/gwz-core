use std::ffi::OsStr;

use super::authority::{CheckedArtifactAuthority, RetainedSource, authority_name};
use super::fault::{CheckedArtifactFault, fault};
use super::{CheckedArtifact, CheckedArtifactFact, ParentState, error, io_error};
use crate::model::ModelResult;

impl CheckedArtifact {
    pub(super) fn finish_replace(
        &self,
        authority: &CheckedArtifactAuthority,
        expected: &CheckedArtifactFact,
        goal: &[u8],
    ) -> ModelResult<()> {
        self.finish(authority, expected, Some(goal))
    }

    pub(super) fn finish_remove(
        &self,
        authority: &CheckedArtifactAuthority,
        expected: &CheckedArtifactFact,
    ) -> ModelResult<()> {
        self.finish(authority, expected, None)
    }

    fn finish(
        &self,
        authority: &CheckedArtifactAuthority,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
    ) -> ModelResult<()> {
        let ParentState::Open {
            identity: parent_identity,
            ..
        } = &self.parent
        else {
            return Err(error(
                self.code,
                &self.label,
                "canonical parent is missing or invalid during cleanup",
            ));
        };
        if !self.parent_is_current(parent_identity)?
            || parent_identity.durable != authority.retained_parent_identity
        {
            return Err(error(
                self.code,
                &self.label,
                "retained parent changed before checked-artifact cleanup",
            ));
        }
        let desired = goal.map_or(CheckedArtifactFact::Missing, |bytes| {
            CheckedArtifactFact::Bytes(bytes.to_vec())
        });
        if self.observe_durable()? != desired {
            return Err(error(
                self.code,
                &self.label,
                "managed destination is not the exact durable goal",
            ));
        }
        let private = self.open_private(false)?.ok_or_else(|| {
            error(
                self.code,
                &self.label,
                "private authority disappeared before cleanup",
            )
        })?;
        let residue = self.inspect_family(expected, goal)?;
        if residue.foreign || residue.authority.as_ref() != Some(authority) {
            return Err(error(
                self.code,
                &self.label,
                "family authority changed before cleanup",
            ));
        }
        if let Some(staged) = residue.goal {
            let managed = self.observe_leaf_exact_current()?;
            if managed.identity.as_ref() != Some(&staged.identity) {
                return Err(error(
                    self.code,
                    &self.label,
                    "staged goal is a different-identity duplicate",
                ));
            }
            private
                .remove_file(&staged.name)
                .map_err(|cause| io_error(self.code, &self.label, cause))?;
            super::platform::private_barrier(
                &private,
                super::platform::DirentBarrierClass::AnchoredPrivateArea,
                self.code,
                &self.label,
            )?;
        }
        if let Some(source) = residue.source {
            let RetainedSource::Existing(expected_identity) = &authority.retained_source else {
                return Err(error(
                    self.code,
                    &self.label,
                    "missing-source authority unexpectedly owns source residue",
                ));
            };
            if source.identity.durable != *expected_identity {
                return Err(error(
                    self.code,
                    &self.label,
                    "quarantined source identity changed before cleanup",
                ));
            }
            fault(
                CheckedArtifactFault::BeforeSourceCleanup,
                self.code,
                &self.label,
            )?;
            let rechecked = self.inspect_family(expected, goal)?;
            if rechecked.foreign
                || rechecked.authority.as_ref() != Some(authority)
                || rechecked.source.as_ref().is_none_or(|value| {
                    value.name != source.name || value.identity != source.identity
                })
                || self.observe_durable()? != desired
                || !self.parent_is_current(parent_identity)?
            {
                return Err(error(
                    self.code,
                    &self.label,
                    "cleanup evidence changed before source retirement",
                ));
            }
            private
                .remove_file(&source.name)
                .map_err(|cause| io_error(self.code, &self.label, cause))?;
            super::platform::private_barrier(
                &private,
                super::platform::DirentBarrierClass::AnchoredPrivateArea,
                self.code,
                &self.label,
            )?;
            fault(
                CheckedArtifactFault::AfterSourceCleanup,
                self.code,
                &self.label,
            )?;
        }
        // E0.2 §7.1's PER-CONVERTED-CONSUMER `finish()`-REACHABILITY RECORD,
        // taken at R2-E E4.7 (2026-09-02) and re-measured against this tree.
        //
        // The condition (`GwzM5-8R2E-SemanticsAmendment-DRAFT.md` §7.1, and the
        // A-1 rider whose travelling record is `GwzM5-8R2E-E7-Acceptance.md`'s
        // O12 row at `:181` — NOT the `:179` that the capability-free amendment
        // §4 and the plan cite, which is now the O10 row): decision A-1 rests on
        // the legacy compensating observation continuing to run wherever the
        // legacy `finish()` still executes, and REOPENS if any converted
        // consumer bypasses BOTH the checked retirement and the legacy recheck.
        // That recheck is these lines plus the `rebarrier_exact` below
        // (`residue.rs:578`), and it is UNCONDITIONAL within `finish()`: the
        // source-retirement block above is guarded by
        // `if let Some(source) = residue.source`, this is not — it runs on every
        // call, a `Missing -> Bytes` creation included.
        //
        // Phase E4's converted consumers are E4.1 and E4.2, and there are no
        // others (operator ruling (a), 2026-09-02, quoted in full at
        // `workspace_ops/merge/v1_lifecycle/finalization/execute.rs`):
        //
        //  * E4.1 — `entry.rs::activate_workspace_catalog` via
        //    `catalog::recover_or_create`. `finish()` NOT reachable: the path
        //    constructs no `CheckedArtifact` at all and publishes through the
        //    pre-catalog permit's `publish_verified_no_replace`. It takes the
        //    CHECKED retirement instead, so it bypasses only one of the two.
        //    DOES NOT REOPEN.
        //  * E4.2 parent half — `entry.rs::bootstrap_merge_start_parents` via
        //    `coordinator::execution`. `finish()` NOT reachable, same reason;
        //    the checked retirement is the provider's source-associated rename
        //    (`namespace_mutation.rs`: `require_absent` at `:284`, then
        //    `PublicationSourceV1::regular_file` at `:306`). DOES NOT REOPEN.
        //  * E4.2 record half — `entry.rs::create_merge_store_record` ends at
        //    `replace_exact(&CheckedArtifactFact::Missing, goal)`, so
        //    `transition.rs:105-106` runs `publish_goal` then `finish_replace`
        //    and `finish()` IS reachable. Because `expected` is `Missing` the
        //    detach is skipped and the source-retirement block above is inert —
        //    but THESE lines execute, and `authority_name` below is used only
        //    after they have proved the family closed. This is the strongest
        //    form of the A-1 answer: the compensating observation runs on the
        //    first converted production write. DOES NOT REOPEN.
        //
        // The charter prep's fourth row — E4.5-B's marker arm, written
        // conditionally — is VACATED rather than answered: E4.5-B does not open
        // and `finalization/execute.rs:45` stays raw. There is no fourth
        // converted consumer, now or in R2-E.
        //
        // VERDICT: no converted consumer bypasses both mechanisms. The A-1
        // rider's E4.7 reopen condition is CHECKED and NOT MET; DECISION A-1
        // STANDS and the `authority_name` rename it rejected stays rejected.
        // DR-1 inherits the question only through the legacy in-place writer's
        // retirement (`GwzM5-8R2E-CapabilityFreeAmendment.md` §4), not through
        // A-1. Cite drift re-pointed at this tree: A-1's
        // `namespace_mutation.rs:280-288`/`:263-265` are `:306`/`:284`, and
        // `residue.rs:570-596` is `:578`; these lines are UNMOVED since A-1 was
        // written.
        let rechecked = self.inspect_family(expected, goal)?;
        if rechecked.foreign
            || rechecked.authority.as_ref() != Some(authority)
            || rechecked.source.is_some()
            || rechecked.goal.is_some()
            || self.observe_durable()? != desired
        {
            return Err(error(
                self.code,
                &self.label,
                "family is not closed for authority retirement",
            ));
        }
        let authority_name = authority_name(&authority.family_key, &authority.action_key);
        self.rebarrier_exact(&private, OsStr::new(&authority_name))?;
        fault(
            CheckedArtifactFault::BeforeAuthorityCleanup,
            self.code,
            &self.label,
        )?;
        private
            .remove_file(&authority_name)
            .map_err(|cause| io_error(self.code, &self.label, cause))?;
        super::platform::private_barrier(
            &private,
            super::platform::DirentBarrierClass::AnchoredPrivateArea,
            self.code,
            &self.label,
        )?;
        fault(
            CheckedArtifactFault::AfterAuthorityCleanup,
            self.code,
            &self.label,
        )
    }
}
