//! Managed-parent plan issuance, admission binding, and execution owner.

use sha2::{Digest, Sha256};

use super::*;
use crate::checked_artifact::protocol::{
    ActionDigestV1, ActionScheduleV1, AdmittedActionV1, ManagedBootstrapInputV1,
    RequestOwnerBindingV1,
};

/// Provider implementation seam. Raw observations are converted to the fixed
/// plan above by `ManagedParentBootstrapOwnerV1`; execution receives only its
/// opaque admitted binding.
pub(in crate::checked_artifact) trait ManagedParentBootstrap {
    type RetainedParents;

    fn provider_instance_id(&self) -> [u8; 32];

    fn observe_preflight(
        &self,
        request: &ManagedParentBootstrapRequest,
    ) -> Result<Vec<ManagedParentObservationV1>, CheckedFsError>;

    fn revalidate_plan(&self, plan: &ManagedParentPlanV1) -> Result<bool, CheckedFsError>;

    fn execute_bound(
        &self,
        plan: &BoundManagedParentPlanV1,
    ) -> Result<Self::RetainedParents, CheckedFsError>;
}

pub(in crate::checked_artifact) struct ManagedParentBootstrapOwnerV1<'a, Provider> {
    provider: &'a Provider,
}

impl<'a, Provider: ManagedParentBootstrap> ManagedParentBootstrapOwnerV1<'a, Provider> {
    pub(in crate::checked_artifact) const fn new(provider: &'a Provider) -> Self {
        Self { provider }
    }

    pub(in crate::checked_artifact) fn preflight(
        &self,
        request: &ManagedParentBootstrapRequest,
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
    ) -> Result<ManagedParentPlanV1, CheckedFsError> {
        let provider_instance =
            ManagedParentProviderBindingV1::try_new(self.provider.provider_instance_id())?;
        let observations = self.provider.observe_preflight(request)?;
        if observations.len() != request.specs().len() {
            return Err(plan_mismatch(
                "provider returned a partial managed-parent plan",
            ));
        }

        let mut rows = Vec::new();
        rows.try_reserve_exact(request.specs().len()).map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::ManagedParentBootstrap,
                "managed-parent plan allocation failed",
            )
        })?;
        for (declared_order, (spec, observed)) in
            request.specs().iter().zip(observations).enumerate()
        {
            if spec.purpose() != observed.purpose
                || observed.retained_existing_parent_count >= spec.components().len()
            {
                return Err(plan_mismatch(
                    "provider plan is reordered, different, or has no missing suffix",
                ));
            }
            let components = spec.components().to_vec();
            let missing_suffix = components[observed.retained_existing_parent_count..].to_vec();
            let spec_digest = digest_spec(spec.purpose(), &components);
            rows.push(ManagedParentPlanRowV1 {
                declared_order,
                purpose: spec.purpose(),
                retained_existing_parent_count: observed.retained_existing_parent_count,
                retained_parent_identity: observed.retained_parent_identity,
                retained_parent_mode: observed.retained_parent_mode,
                retained_parent_path: observed.retained_parent_path,
                components,
                missing_suffix,
                spec_digest,
            });
        }
        let total_missing = rows
            .iter()
            .map(|row| row.missing_suffix.len())
            .sum::<usize>();
        if total_missing > MAX_MANAGED_PARENT_COMPONENTS {
            return Err(plan_mismatch(
                "managed-parent plan exceeds the aggregate component bound",
            ));
        }
        let digest = digest_plan(
            provider_instance,
            action_digest,
            request_owner_binding,
            &rows,
        );
        let schedule_rows = rows
            .iter()
            .map(|row| ManagedBootstrapInputV1::new(row.spec_digest, row.missing_suffix.len()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| plan_mismatch("managed-parent plan cannot be scheduled"))?;
        Ok(ManagedParentPlanV1 {
            provider_instance,
            action_digest,
            request_owner_binding,
            rows,
            digest,
            schedule_inputs: ManagedParentScheduleInputsV1 {
                plan_digest: digest,
                rows: schedule_rows,
            },
        })
    }

    pub(in crate::checked_artifact) fn bind(
        &self,
        admitted_action: &AdmittedActionV1,
        plan: &ManagedParentPlanV1,
    ) -> Result<BoundManagedParentPlanV1, CheckedFsError> {
        let provider_instance =
            ManagedParentProviderBindingV1::try_new(self.provider.provider_instance_id())?;
        let reservation = admitted_action.reservation();
        let schedule = reservation.schedule();
        if plan.provider_instance != provider_instance
            || plan.action_digest != reservation.action_digest()
            || plan.request_owner_binding != reservation.request_owner_binding()
            || plan.rows.is_empty()
            || schedule.managed_plan_digest() != plan.digest
            || schedule.bootstrap_rows().len() != plan.rows.len()
        {
            return Err(plan_mismatch(
                "managed-parent plan does not match the admitted action",
            ));
        }
        let expected = ActionScheduleV1::try_from_managed_plan(
            schedule.barrier_count(),
            plan.schedule_inputs(),
            schedule.cleanup_aliases(),
        )
        .map_err(|_| plan_mismatch("managed-parent plan cannot reproduce resident schedule"))?;
        if &expected != schedule {
            return Err(plan_mismatch(
                "managed-parent rows or assigned ranges differ from the resident schedule",
            ));
        }

        let mut rows = Vec::new();
        rows.try_reserve_exact(plan.rows.len()).map_err(|_| {
            CheckedFsError::unsupported(
                PlatformCapability::ManagedParentBootstrap,
                "bound managed-parent plan allocation failed",
            )
        })?;
        for plan_row in &plan.rows {
            let scheduled = schedule
                .bootstrap_rows()
                .iter()
                .find(|row| row.spec_digest() == plan_row.spec_digest)
                .ok_or_else(|| plan_mismatch("scheduled managed-parent row is missing"))?;
            if scheduled.component_range().len() != plan_row.missing_suffix.len() {
                return Err(plan_mismatch(
                    "scheduled managed-parent component range differs from the plan",
                ));
            }
            rows.push(BoundManagedParentPlanRowV1 {
                plan_row: plan_row.clone(),
                bootstrap_ordinal: scheduled.ordinal(),
                generation_range: scheduled.generation_range(),
                component_range: scheduled.component_range(),
            });
        }
        if !self.provider.revalidate_plan(plan)? {
            return Err(plan_mismatch(
                "managed-parent plan is stale at the admission boundary",
            ));
        }
        Ok(BoundManagedParentPlanV1 {
            provider_instance,
            admitted_action: admitted_action.clone(),
            plan: plan.clone(),
            rows,
        })
    }

    pub(in crate::checked_artifact) fn execute(
        &self,
        bound: &BoundManagedParentPlanV1,
    ) -> Result<Provider::RetainedParents, CheckedFsError> {
        let current =
            ManagedParentProviderBindingV1::try_new(self.provider.provider_instance_id())?;
        if current != bound.provider_instance || !self.provider.revalidate_plan(&bound.plan)? {
            return Err(plan_mismatch(
                "bound managed-parent plan is stale or belongs to another provider",
            ));
        }
        self.provider.execute_bound(bound)
    }
}

fn digest_spec(purpose: ManagedParentPurpose, components: &[AsciiComponent]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"gwz-managed-parent-spec-v1\0");
    digest.update([purpose.code()]);
    append_components(&mut digest, components);
    digest.finalize().into()
}

fn digest_plan(
    provider: ManagedParentProviderBindingV1,
    action: ActionDigestV1,
    owner: RequestOwnerBindingV1,
    rows: &[ManagedParentPlanRowV1],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"gwz-managed-parent-plan-v1\0");
    digest.update(provider.0);
    digest.update(action.bytes());
    digest.update(owner.bytes());
    digest.update((rows.len() as u64).to_be_bytes());
    for row in rows {
        digest.update((row.declared_order as u64).to_be_bytes());
        digest.update([row.purpose.code()]);
        digest.update((row.retained_existing_parent_count as u64).to_be_bytes());
        append_bytes(
            &mut digest,
            &row.retained_parent_identity.encode_canonical(),
        );
        digest.update([match row.retained_parent_mode {
            PathComponentMode::Sensitive => 0,
            PathComponentMode::AsciiCaseFold => 1,
        }]);
        append_bytes(&mut digest, &row.retained_parent_path.encode_canonical());
        append_components(&mut digest, &row.components);
        append_components(&mut digest, &row.missing_suffix);
        digest.update(row.spec_digest);
    }
    digest.finalize().into()
}

fn append_components(digest: &mut Sha256, components: &[AsciiComponent]) {
    digest.update((components.len() as u64).to_be_bytes());
    for component in components {
        append_bytes(digest, component.as_bytes());
    }
}

fn append_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn plan_mismatch(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("managed-parent plan", detail)
}
