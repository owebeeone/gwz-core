//! Target-bound pre-catalog authority types.
//!
//! R2-C0 freezes the live/durable type boundary, C1 supplies the pure aggregate
//! classification, and C2 consumes these permits through the sealed catalog
//! owner. No caller receives a raw filesystem writer.

use super::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
    SupportedFilesystemProfile,
};
use crate::checked_artifact::bootstrap::CatalogLeaseTargetWitnessV1;
use crate::checked_artifact::catalog::CatalogScratchNameV1;
use crate::checked_artifact::catalog::{
    CatalogAttemptBindingV1, CatalogClassificationV1, CatalogOwnerEdgeV1, classify_catalog_attempt,
};
#[cfg(test)]
use crate::checked_artifact::protocol::CatalogBootstrapOwnershipTokenV1;
use crate::checked_artifact::protocol::{
    ActionAdmissionEdgeV1, ActionAdmissionObservationV1, ActionCapacityReservationV1,
    AdmittedActionV1, CatalogBootstrapRecordV1, RecordDigestV1,
};

mod provider;

#[cfg(test)]
pub(in crate::checked_artifact) use provider::handle_probe_is_unavailable;
#[cfg(test)]
pub(in crate::checked_artifact) use provider::retain_managed_parent_at_for_test;
#[cfg(test)]
pub(crate) use provider::with_handle_probe_unavailable;
/// R2-D Phase 2 Step 2.2 — the retained action-namespace capability and the
/// role-typed edge selector the `namespace` owner drives it with.
///
/// R2-E Phase E1 Step E1.1 adds the cleanup worklist's lifecycle surface over
/// that same capability (DECISION C-2, amendment §2.2): the worklist's scratch
/// write, the published row's proof, each alias row's physical fact pair, each
/// retirement's post-edge proof, and the whole-worklist completion proof.
pub(in crate::checked_artifact) use provider::{
    ActionNamespaceEdgeV1, ObservedNamespaceObjectV1, RetainedActionNamespaceV1,
    observe_cleanup_completion, observe_cleanup_retirement, observe_cleanup_row_facts,
    observe_cleanup_worklist_row, write_cleanup_worklist_scratch,
};
/// R2-E Phase E2 — the scheduled barrier family's owner-private lifecycle
/// surface, plus O6's owner-minted roaming-anchor-home observation. The witness
/// type is nameable by the protocol record that consumes it and by the
/// `namespace` owner that forwards it; its constructor stays inside this owner,
/// so no caller can assert the facts it carries.
pub(in crate::checked_artifact) use provider::{
    AliasRetirementEntryV1, BarrierIntentRowV1, RoamingAnchorHomeWitnessV1,
    TargetAnchorAliasStateV1, barrier_target_parent, converge_target_anchor_alias,
    create_target_anchor_alias, observe_barrier_completion, observe_barrier_intent_row,
    retire_target_anchor_alias, write_barrier_intent_scratch,
};
pub(in crate::checked_artifact) use provider::{HostPlatform, VolumeDescription};
#[cfg(test)]
pub(crate) use provider::{InjectedVolumeDescription, with_identity_unavailable};
/// R2-D Phase 2 Step 2.3 — the retained managed-parent capability the
/// `namespace` owner drives edges E15 and E16 with, plus R2-D Phase 3 Step
/// 3.1's managed-prefix observation.
///
/// Step 2.3's `allow(unused_imports)` is discharged here rather than carried:
/// its one unused item was `retain_managed_parent`. A managed parent can only be
/// retained under a `&Dir`, and no `Dir` leaves the provider owner, so
/// re-exporting the constructor to `crate::checked_artifact` never made it
/// callable; Step 3.1 removes the re-export instead of keeping an allow alive
/// for an unreachable item. (Step-3.1 review [P3-1]: the production route is
/// `provider::retain_managed_prefix`, which calls `retain_managed_child`, not
/// the removed wrapper — an earlier revision of this comment said otherwise.)
///
/// R2-D Phase 3 Step 3.1b adds the managed intent record's lifecycle surface.
pub(in crate::checked_artifact) use provider::{
    ManagedInstalledFactsV1, ManagedIntentEdgeV1, ManagedPrefixObservationV1,
    ManagedRetiredFactsV1, ObservedManagedObjectV1, RetainedManagedParentV1,
    observe_managed_intent_row, read_managed_intent_row, write_managed_intent_scratch,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::checked_artifact) enum PreCatalogRootKindV1 {
    Workspace,
    GitDirectory,
}

macro_rules! digest_newtype {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(in crate::checked_artifact) struct $name([u8; 32]);

        impl $name {
            pub(in crate::checked_artifact) const fn owner_issue(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub(in crate::checked_artifact) const fn bytes(self) -> [u8; 32] {
                self.0
            }
        }
    };
}

digest_newtype!(FreshObservationDigestV1);
digest_newtype!(DurableCatalogTargetDigestV1);
digest_newtype!(HistoricalCollisionDigestV1);
digest_newtype!(MissingParentObservationDigestV1);

/// Closed result of the retained pre-catalog transaction.
pub(in crate::checked_artifact) enum CatalogPreflightV1<'lease> {
    MissingGitPrivateParent(Box<MissingCatalogParentPermitV1<'lease>>),
    Ready(Box<CatalogPermitV1<'lease>>),
}

pub(in crate::checked_artifact) fn preflight_catalog_target(
    target: CatalogLeaseTargetWitnessV1<'_>,
) -> Result<CatalogPreflightV1<'_>, CheckedFsError> {
    let bound = provider::inspect_bound_catalog_target(target)?;
    match bound.observation.ready_digests {
        Some(digests) => Ok(CatalogPreflightV1::Ready(Box::new(
            CatalogPermitV1::owner_issue(bound, digests.fresh, digests.target, digests.historical)?,
        ))),
        None => {
            let digest = bound.observation.missing_parent_digest.ok_or_else(|| {
                CheckedFsError::ambiguous(
                    "catalog preflight state",
                    "observation is neither a ready target nor a missing Git parent",
                )
            })?;
            Ok(CatalogPreflightV1::MissingGitPrivateParent(Box::new(
                MissingCatalogParentPermitV1::owner_issue(bound, digest)?,
            )))
        }
    }
}

/// A ready, live-only permit. Its target cannot be substituted because the
/// lease is carried inside the permit rather than supplied beside it.
pub(in crate::checked_artifact) struct CatalogPermitV1<'lease> {
    _catalog_target: CatalogLeaseTargetWitnessV1<'lease>,
    _retained_root: provider::RetainedPlatformRoot,
    _support_profile: SupportedFilesystemProfile,
    _root_identity: DurableObjectIdentityV1,
    _root_invocation_identity: Vec<u8>,
    _rename_domain: Vec<u8>,
    _path_profile: CanonicalPathIdentityV1,
    _raw_roles: provider::RawCatalogRoleObservationV1,
    _fresh_observation_digest: FreshObservationDigestV1,
    _durable_target_digest: DurableCatalogTargetDigestV1,
    _historical_collision_digest: HistoricalCollisionDigestV1,
    _attempt_binding: CatalogAttemptBindingV1,
}

/// Exact completed catalog retained with the same target-bound lease.
pub(in crate::checked_artifact) struct CompletedCatalogPermitV1<'lease> {
    catalog_target: CatalogLeaseTargetWitnessV1<'lease>,
    retained_root: provider::RetainedPlatformRoot,
    completed: provider::RetainedCompletedCatalogV1,
}

impl CompletedCatalogPermitV1<'_> {
    pub(in crate::checked_artifact) fn revalidate(&self) -> Result<(), CheckedFsError> {
        provider::revalidate_lease_root_binding(&self.catalog_target, &self.retained_root)?;
        self.completed.revalidate(&self.retained_root)
    }

    /// R2-D Phase 1 admission observation. Revalidating first is the
    /// `ready_edge_prologue` discipline applied to the completed permit: every
    /// admission observation and every admission edge re-proves the lease/root
    /// binding and the exact retained catalog before it looks at anything.
    pub(in crate::checked_artifact) fn observe_admission(
        &self,
        expected: &ActionCapacityReservationV1,
    ) -> Result<ActionAdmissionObservationV1, CheckedFsError> {
        self.revalidate()?;
        self.completed.observe_admission(expected)
    }

    /// One bounded durable admission edge.
    pub(in crate::checked_artifact) fn execute_admission_edge(
        &self,
        edge: ActionAdmissionEdgeV1<'_>,
        expected: &ActionCapacityReservationV1,
    ) -> Result<(), CheckedFsError> {
        self.revalidate()?;
        self.completed.execute_admission_edge(edge, expected)
    }

    /// R2-D Phase 2 Step 2.2. Retains the admitted action's namespace under the
    /// same `ready_edge_prologue` discipline the admission entry points use:
    /// the lease/root binding and the exact retained catalog are re-proved
    /// before the single no-follow hop that opens the action directory.
    pub(in crate::checked_artifact) fn retain_action_namespace(
        &self,
        admitted: &AdmittedActionV1,
    ) -> Result<provider::RetainedActionNamespaceV1, CheckedFsError> {
        self.revalidate()?;
        self.completed.retain_action_namespace(admitted)
    }

    /// R2-E E3.1 — the admitted action directory's terminal retirement, under
    /// the same `ready_edge_prologue` discipline every other admission entry
    /// point uses: the lease/root binding and the exact retained catalog are
    /// re-proved before anything is observed or moved.
    pub(in crate::checked_artifact) fn retire_admitted_action(
        &self,
        admitted: &AdmittedActionV1,
    ) -> Result<(), CheckedFsError> {
        self.revalidate()?;
        self.completed
            .retire_admitted_action(&self.retained_root, admitted)
    }

    /// R2-E Phase E2 (O6) — the roaming anchor's home, observed inside the
    /// completed-catalog owner. Revalidating first is the same
    /// `ready_edge_prologue` discipline the admission and namespace entry
    /// points use, and here it *is* the refusal arm: the witness is mintable
    /// only from a retained catalog that still revalidates, so the seam refuses
    /// typed and mints nothing where it does not
    /// (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §5.4).
    pub(in crate::checked_artifact) fn observe_roaming_anchor_home(
        &self,
    ) -> Result<provider::RoamingAnchorHomeWitnessV1, CheckedFsError> {
        self.revalidate()?;
        Ok(self.completed.observe_roaming_anchor_home())
    }

    /// R2-D Phase 3 Step 3.1 — the bounded, read-only managed-parent prefix
    /// observation, under the same `ready_edge_prologue` discipline: the
    /// lease/root binding and the exact retained catalog are re-proved before
    /// the walk, so a preflight can never read through a substituted root.
    pub(in crate::checked_artifact) fn observe_managed_prefix(
        &self,
        components: &[AsciiComponent],
    ) -> Result<provider::ManagedPrefixObservationV1, CheckedFsError> {
        self.revalidate()?;
        provider::observe_managed_prefix(&self.retained_root, components)
    }

    /// R2-D Phase 3 Step 3.1 — the retained managed parent at one declared
    /// depth of the same prefix, revalidated first for the same reason. The
    /// caller receives the retained capability only — never the root handle and
    /// never a path.
    pub(in crate::checked_artifact) fn retain_managed_prefix(
        &self,
        components: &[AsciiComponent],
        depth: usize,
        reservation: RecordDigestV1,
    ) -> Result<provider::RetainedManagedParentV1, CheckedFsError> {
        self.revalidate()?;
        provider::retain_managed_prefix(&self.retained_root, components, depth, reservation)
    }

    /// R2-D Phase 3 Step 3.1 — the managed-parent provider's instance binding,
    /// derived inside the owner from the retained root it already proved.
    pub(in crate::checked_artifact) fn managed_provider_instance(
        &self,
    ) -> Result<[u8; 32], CheckedFsError> {
        self.revalidate()?;
        Ok(provider::managed_provider_instance(&self.retained_root))
    }
}

/// A disjoint, live-only authorization for the one fixed Git `gwz` parent
/// creation edge. It contains no ready-catalog digest or catalog authority.
pub(in crate::checked_artifact) struct MissingCatalogParentPermitV1<'lease> {
    _catalog_target: CatalogLeaseTargetWitnessV1<'lease>,
    _retained_root: provider::RetainedPlatformRoot,
    _missing_parent_observation_digest: MissingParentObservationDigestV1,
}

impl<'lease> CatalogPermitV1<'lease> {
    #[allow(dead_code, reason = "R2-C2 revalidates before every physical edge")]
    pub(in crate::checked_artifact) fn revalidate_target_binding(
        &self,
    ) -> Result<(), CheckedFsError> {
        provider::revalidate_lease_root_binding(&self._catalog_target, &self._retained_root)
    }

    pub(in crate::checked_artifact) fn revalidate_observation(&self) -> Result<(), CheckedFsError> {
        provider::revalidate_ready_observation(
            &self._catalog_target,
            &self._retained_root,
            self._fresh_observation_digest,
        )
    }

    #[allow(dead_code, reason = "R2-C1 consumes the frozen ready-permit fields")]
    pub(in crate::checked_artifact) const fn support_profile(&self) -> SupportedFilesystemProfile {
        self._support_profile
    }

    #[allow(dead_code, reason = "R2-C1 consumes the frozen ready-permit fields")]
    pub(in crate::checked_artifact) fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        &self._path_profile
    }

    pub(in crate::checked_artifact) const fn digests(
        &self,
    ) -> (
        FreshObservationDigestV1,
        DurableCatalogTargetDigestV1,
        HistoricalCollisionDigestV1,
    ) {
        (
            self._fresh_observation_digest,
            self._durable_target_digest,
            self._historical_collision_digest,
        )
    }

    pub(in crate::checked_artifact) fn attempt_binding(&self) -> &CatalogAttemptBindingV1 {
        &self._attempt_binding
    }

    pub(in crate::checked_artifact) fn classify_observed(&self) -> CatalogClassificationV1 {
        classify_catalog_attempt(
            &self._attempt_binding,
            provider::outer_aggregate_facts(&self._attempt_binding, &self._raw_roles),
        )
    }

    /// Common Ready-edge mutation prologue: revalidate the retained
    /// observation, then issue the idempotent containing-root dirent
    /// barrier (DirentBarrier [P3-1] correction (a)). Every Ready owner
    /// edge passes through here, so a resume drive that re-enters after
    /// the scratch edge still anchors the private parent's dirent before
    /// any later durable role: `Complete` is unreachable without a root
    /// barrier issued by the completing process. Preflight stays
    /// read-only; the barrier lives only inside owner mutation edges.
    fn ready_edge_prologue(&self) -> Result<(), CheckedFsError> {
        self.revalidate_observation()?;
        provider::finish_ready_edge_root_barrier(&self._retained_root)
    }

    pub(in crate::checked_artifact) fn execute_owner_scratch(
        self: Box<Self>,
        edge: CatalogOwnerEdgeV1,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        let token = edge.require_scratch_token()?;
        self.ready_edge_prologue()?;
        let scratch = CatalogScratchNameV1::new(
            self._durable_target_digest,
            self._historical_collision_digest,
            token,
        );
        let record = self
            ._attempt_binding
            .record_from_scratch(&scratch)
            .map_err(|_| {
                CheckedFsError::ambiguous(
                    "catalog scratch",
                    "scratch values do not match the retained attempt binding",
                )
            })?;
        let classification = self.classify_observed();
        let create_new = classification.expected_record().is_none();
        if classification.decision()
            != crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch
            || classification
                .expected_record()
                .is_some_and(|expected| expected != &record)
        {
            return Err(CheckedFsError::ambiguous(
                "catalog scratch",
                "ready permit does not authorize this scratch edge",
            ));
        }
        provider::write_or_rewrite_scratch(
            &self._retained_root,
            &self._raw_roles,
            &scratch,
            &record,
            create_new,
        )?;
        Ok(self.into_target())
    }

    pub(in crate::checked_artifact) fn execute_owner_publish_active(
        self: Box<Self>,
        edge: CatalogOwnerEdgeV1,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        edge.require_publish_active()?;
        self.ready_edge_prologue()?;
        let classification = self.classify_observed();
        if classification.decision()
            != crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1::PublishActive
        {
            return Err(CheckedFsError::ambiguous(
                "catalog active publication",
                "ready permit does not authorize active publication",
            ));
        }
        let record = classification.expected_record().ok_or_else(|| {
            CheckedFsError::ambiguous(
                "catalog active publication",
                "published scratch has no expected record",
            )
        })?;
        let scratch = CatalogScratchNameV1::new(
            record.durable_target_digest(),
            record.historical_collision_digest(),
            record.bootstrap_ownership_token(),
        );
        provider::publish_active_record(&self._retained_root, &self._raw_roles, &scratch, record)?;
        Ok(self.into_target())
    }

    pub(in crate::checked_artifact) fn execute_owner_prepare_or_rewrite_staging(
        self: Box<Self>,
        edge: CatalogOwnerEdgeV1,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        edge.require_prepare_or_rewrite_staging()?;
        self.ready_edge_prologue()?;
        let expected = self.expected_for(
            crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1::PrepareOrRewriteStaging,
            "catalog staging",
        )?;
        provider::prepare_or_rewrite_staging(&self._retained_root, &self._raw_roles, &expected)?;
        Ok(self.into_target())
    }

    pub(in crate::checked_artifact) fn execute_owner_publish_final(
        self: Box<Self>,
        edge: CatalogOwnerEdgeV1,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        edge.require_publish_final()?;
        self.ready_edge_prologue()?;
        let expected = self.expected_for(
            crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1::PublishFinal,
            "catalog final publication",
        )?;
        provider::publish_final_directory(&self._retained_root, &self._raw_roles, &expected)?;
        Ok(self.into_target())
    }

    pub(in crate::checked_artifact) fn execute_owner_retire_active(
        self: Box<Self>,
        edge: CatalogOwnerEdgeV1,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        edge.require_retire_active()?;
        self.ready_edge_prologue()?;
        let expected = self.expected_for(
            crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1::RetireActive,
            "catalog active retirement",
        )?;
        provider::retire_active_record(&self._retained_root, &self._raw_roles, &expected)?;
        Ok(self.into_target())
    }

    pub(in crate::checked_artifact) fn execute_owner_complete(
        self: Box<Self>,
        edge: CatalogOwnerEdgeV1,
    ) -> Result<CompletedCatalogPermitV1<'lease>, CheckedFsError> {
        edge.require_complete()?;
        self.ready_edge_prologue()?;
        let expected = self.expected_for(
            crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1::Complete,
            "completed catalog",
        )?;
        let completed =
            provider::retain_completed_catalog(&self._retained_root, &self._raw_roles, &expected)?;
        let Self {
            _catalog_target,
            _retained_root,
            _support_profile: _,
            _root_identity: _,
            _root_invocation_identity: _,
            _rename_domain: _,
            _path_profile: _,
            _raw_roles: _,
            _fresh_observation_digest: _,
            _durable_target_digest: _,
            _historical_collision_digest: _,
            _attempt_binding: _,
        } = *self;
        Ok(CompletedCatalogPermitV1 {
            catalog_target: _catalog_target,
            retained_root: _retained_root,
            completed,
        })
    }

    fn expected_for(
        &self,
        decision: crate::checked_artifact::protocol::CatalogBootstrapRecoveryDecisionV1,
        fact: &'static str,
    ) -> Result<CatalogBootstrapRecordV1, CheckedFsError> {
        let classification = self.classify_observed();
        if classification.decision() != decision {
            return Err(CheckedFsError::ambiguous(
                fact,
                "ready permit does not authorize this owner edge",
            ));
        }
        classification.expected_record().cloned().ok_or_else(|| {
            CheckedFsError::ambiguous(fact, "authorized owner edge has no expected record")
        })
    }

    fn into_target(self: Box<Self>) -> CatalogLeaseTargetWitnessV1<'lease> {
        let Self {
            _catalog_target,
            _retained_root: _,
            _support_profile: _,
            _root_identity: _,
            _root_invocation_identity: _,
            _rename_domain: _,
            _path_profile: _,
            _raw_roles: _,
            _fresh_observation_digest: _,
            _durable_target_digest: _,
            _historical_collision_digest: _,
            _attempt_binding: _,
        } = *self;
        _catalog_target
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn record_for_test(
        &self,
        token: CatalogBootstrapOwnershipTokenV1,
    ) -> CatalogBootstrapRecordV1 {
        self._attempt_binding
            .record_from_scratch(&CatalogScratchNameV1::new(
                self._durable_target_digest,
                self._historical_collision_digest,
                token,
            ))
            .expect("permit digests match its attempt binding")
    }
}

impl<'lease> CatalogPermitV1<'lease> {
    #[allow(
        dead_code,
        reason = "R2-C1 derives the typed digests and issues the ready permit"
    )]
    fn owner_issue(
        bound: provider::LeaseBoundPreCatalogObservationV1<'lease>,
        fresh_observation_digest: FreshObservationDigestV1,
        durable_target_digest: DurableCatalogTargetDigestV1,
        historical_collision_digest: HistoricalCollisionDigestV1,
    ) -> Result<Self, CheckedFsError> {
        provider::revalidate_bound_observation(&bound)?;
        if !provider::has_private_parent(&bound) {
            return Err(CheckedFsError::ambiguous(
                "catalog preflight state",
                "ready permit requires the retained private parent",
            ));
        }
        let attempt_binding =
            provider::attempt_binding(&bound, durable_target_digest, historical_collision_digest)?;
        let provider::LeaseBoundPreCatalogObservationV1 {
            target,
            observation,
        } = bound;
        Ok(Self {
            _catalog_target: target,
            _retained_root: observation.retained_root,
            _support_profile: observation.support_profile,
            _root_identity: observation.root_identity,
            _root_invocation_identity: observation.root_invocation_identity,
            _rename_domain: observation.rename_domain,
            _path_profile: observation.path_profile,
            _raw_roles: observation.raw_roles,
            _fresh_observation_digest: fresh_observation_digest,
            _durable_target_digest: durable_target_digest,
            _historical_collision_digest: historical_collision_digest,
            _attempt_binding: attempt_binding,
        })
    }
}

impl<'lease> MissingCatalogParentPermitV1<'lease> {
    #[allow(dead_code, reason = "R2-C2 revalidates the one missing-parent edge")]
    pub(in crate::checked_artifact) fn revalidate_target_binding(
        &self,
    ) -> Result<(), CheckedFsError> {
        provider::revalidate_lease_root_binding(&self._catalog_target, &self._retained_root)
    }

    pub(in crate::checked_artifact) fn revalidate_observation(&self) -> Result<(), CheckedFsError> {
        provider::revalidate_missing_observation(
            &self._catalog_target,
            &self._retained_root,
            self._missing_parent_observation_digest,
        )
    }

    pub(in crate::checked_artifact) const fn observation_digest(
        &self,
    ) -> MissingParentObservationDigestV1 {
        self._missing_parent_observation_digest
    }

    pub(in crate::checked_artifact) fn execute_owner_create_and_retry(
        self: Box<Self>,
        edge: CatalogOwnerEdgeV1,
    ) -> Result<CatalogLeaseTargetWitnessV1<'lease>, CheckedFsError> {
        edge.require_create_private_parent()?;
        self.revalidate_observation()?;
        provider::create_git_private_parent(&self._retained_root)?;
        let Self {
            _catalog_target,
            _retained_root: _,
            _missing_parent_observation_digest: _,
        } = *self;
        Ok(_catalog_target)
    }
}

impl<'lease> MissingCatalogParentPermitV1<'lease> {
    #[allow(dead_code, reason = "R2-C1 issues the disjoint missing-parent permit")]
    fn owner_issue(
        bound: provider::LeaseBoundPreCatalogObservationV1<'lease>,
        missing_parent_observation_digest: MissingParentObservationDigestV1,
    ) -> Result<Self, CheckedFsError> {
        provider::revalidate_bound_observation(&bound)?;
        if bound.target.facts()?.root_kind() != PreCatalogRootKindV1::GitDirectory
            || provider::has_private_parent(&bound)
        {
            return Err(CheckedFsError::ambiguous(
                "catalog preflight state",
                "missing-parent permit requires one lease-bound Git target with no private parent",
            ));
        }
        let provider::LeaseBoundPreCatalogObservationV1 {
            target,
            observation,
        } = bound;
        Ok(Self {
            _catalog_target: target,
            _retained_root: observation.retained_root,
            _missing_parent_observation_digest: missing_parent_observation_digest,
        })
    }
}
