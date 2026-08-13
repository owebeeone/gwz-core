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
        request.validate_authority(request_owner_binding)?;
        let provider_instance =
            ManagedParentProviderBindingV1::try_new(self.provider.provider_instance_id())?;
        let observations = self.provider.observe_preflight(request)?;
        if observations.len() != request.specs().len() {
            return Err(plan_mismatch(
                "provider returned a partial managed-parent plan",
            ));
        }
        let observation_digest = digest_observations(request, &observations);
        let declared_purposes = request
            .specs()
            .iter()
            .map(ManagedParentSpec::purpose)
            .collect::<Vec<_>>();

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
                || observed.retained_existing_parent_count > spec.components().len()
            {
                return Err(plan_mismatch(
                    "provider plan is reordered, different, or exceeds the fixed path",
                ));
            }
            let minimum = spec.purpose().minimum_retained_parent_count();
            if observed.retained_existing_parent_count < minimum
                || !retained_path_matches(spec, &observed)
            {
                return Err(plan_mismatch(
                    "provider retained prefix violates the purpose ownership policy",
                ));
            }
            if observed.retained_existing_parent_count == spec.components().len() {
                continue;
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
        if rows_have_overlapping_missing_edges(&rows) {
            return Err(plan_mismatch(
                "managed-parent rows claim overlapping physical components",
            ));
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
            &declared_purposes,
            observation_digest,
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
            declared_purposes,
            observation_digest,
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

fn retained_path_matches(spec: &ManagedParentSpec, observed: &ManagedParentObservationV1) -> bool {
    let retained = observed.retained_existing_parent_count;
    let path = observed.retained_parent_path.components();
    path.len() == retained
        && path
            .iter()
            .zip(&spec.components()[..retained])
            .all(|(actual, expected)| actual.original() == expected)
}

fn rows_have_overlapping_missing_edges(rows: &[ManagedParentPlanRowV1]) -> bool {
    for (left_index, left) in rows.iter().enumerate() {
        for right in &rows[left_index + 1..] {
            for left_depth in left.retained_existing_parent_count..left.components.len() {
                let left_edge = &left.components[..=left_depth];
                for right_depth in right.retained_existing_parent_count..right.components.len() {
                    let right_edge = &right.components[..=right_depth];
                    if component_prefix(left_edge, right_edge)
                        || component_prefix(right_edge, left_edge)
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn component_prefix(left: &[AsciiComponent], right: &[AsciiComponent]) -> bool {
    left.len() <= right.len() && left.iter().zip(right).all(|(left, right)| left == right)
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
    declared_purposes: &[ManagedParentPurpose],
    observation_digest: [u8; 32],
    rows: &[ManagedParentPlanRowV1],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"gwz-managed-parent-plan-v1\0");
    digest.update(provider.0);
    digest.update(action.bytes());
    digest.update(owner.bytes());
    digest.update((declared_purposes.len() as u64).to_be_bytes());
    for purpose in declared_purposes {
        digest.update([purpose.code()]);
    }
    digest.update(observation_digest);
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

fn digest_observations(
    request: &ManagedParentBootstrapRequest,
    observations: &[ManagedParentObservationV1],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"gwz-managed-parent-observations-v1\0");
    digest.update((observations.len() as u64).to_be_bytes());
    for (spec, observed) in request.specs().iter().zip(observations) {
        digest.update([spec.purpose().code()]);
        append_components(&mut digest, spec.components());
        digest.update((observed.retained_existing_parent_count as u64).to_be_bytes());
        append_bytes(
            &mut digest,
            &observed.retained_parent_identity.encode_canonical(),
        );
        digest.update([match observed.retained_parent_mode {
            PathComponentMode::Sensitive => 0,
            PathComponentMode::AsciiCaseFold => 1,
        }]);
        append_bytes(
            &mut digest,
            &observed.retained_parent_path.encode_canonical(),
        );
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
