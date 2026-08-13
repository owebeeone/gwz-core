use std::cell::Cell;
use std::io::Cursor;

use super::super::bootstrap::{
    BoundManagedParentPlanV1, ManagedParentBootstrap, ManagedParentBootstrapOwnerV1,
    ManagedParentBootstrapRequest, ManagedParentObservationV1, ManagedParentPlanV1,
    ManagedParentPurpose, ManagedParentSpec,
};
use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, PathComponentMode,
};
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1,
    ActionDirectoryObservationV1, ActionScheduleV1, BootstrapGenerationV1, CleanupAliasSetV1,
    ManagedBootstrapInputV1, ManagedParentBootstrapIntentV1, RecordObservationV1,
    RequestOwnerBindingV1, admit_observed_action, read_and_bind_managed_bootstrap_intent,
};

fn identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
}

fn path(byte: u8) -> CanonicalPathIdentityV1 {
    CanonicalPathIdentityV1::new(vec![CanonicalComponent::new(
        AsciiComponent::parse(&[b'p', b'0' + byte]).unwrap(),
        PathComponentMode::Sensitive,
    )])
    .unwrap()
}

fn request(purposes: &[ManagedParentPurpose]) -> ManagedParentBootstrapRequest {
    ManagedParentBootstrapRequest::try_new(
        purposes
            .iter()
            .copied()
            .map(ManagedParentSpec::for_purpose)
            .collect(),
    )
    .unwrap()
}

struct Provider {
    instance: [u8; 32],
    retained_counts: Vec<usize>,
    current: Cell<bool>,
    executions: Cell<usize>,
}

impl Provider {
    fn new(instance: u8, retained_counts: Vec<usize>) -> Self {
        Self {
            instance: [instance; 32],
            retained_counts,
            current: Cell::new(true),
            executions: Cell::new(0),
        }
    }
}

impl ManagedParentBootstrap for Provider {
    type RetainedParents = usize;

    fn provider_instance_id(&self) -> [u8; 32] {
        self.instance
    }

    fn observe_preflight(
        &self,
        request: &ManagedParentBootstrapRequest,
    ) -> Result<Vec<ManagedParentObservationV1>, CheckedFsError> {
        request
            .specs()
            .iter()
            .zip(&self.retained_counts)
            .enumerate()
            .map(|(index, (spec, retained_count))| {
                ManagedParentObservationV1::new(
                    spec.purpose(),
                    *retained_count,
                    identity(index as u8 + 1),
                    PathComponentMode::Sensitive,
                    path(index as u8 + 1),
                )
            })
            .collect()
    }

    fn revalidate_plan(&self, _plan: &ManagedParentPlanV1) -> Result<bool, CheckedFsError> {
        Ok(self.current.get())
    }

    fn execute_bound(
        &self,
        _plan: &BoundManagedParentPlanV1,
    ) -> Result<Self::RetainedParents, CheckedFsError> {
        self.executions.set(self.executions.get() + 1);
        Ok(self.executions.get())
    }
}

fn admitted(
    plan: &ManagedParentPlanV1,
    action: ActionDigestV1,
    owner: RequestOwnerBindingV1,
) -> super::super::protocol::AdmittedActionV1 {
    let schedule = ActionScheduleV1::try_from_managed_plan(
        1,
        plan.schedule_inputs(),
        CleanupAliasSetV1::all(),
    )
    .unwrap();
    let reservation = ActionCapacityReservationV1::new(action, owner, schedule);
    admit_observed_action(
        &ActionDirectoryAdmissionV1::idle(),
        &reservation,
        &ActionDirectoryObservationV1::Missing,
        &ActionDirectoryObservationV1::exact(
            identity(9),
            RecordObservationV1::Exact(reservation.clone()),
        ),
    )
    .unwrap()
}

#[test]
fn preflight_derives_an_immutable_complete_plan_and_its_schedule() {
    let provider = Provider::new(7, vec![1, 2]);
    let owner = ManagedParentBootstrapOwnerV1::new(&provider);
    let action = ActionDigestV1::new([1; 32]);
    let request_owner = RequestOwnerBindingV1::new([2; 32]);
    let plan = owner
        .preflight(
            &request(&[
                ManagedParentPurpose::MergeStore,
                ManagedParentPurpose::MergeArchive,
            ]),
            action,
            request_owner,
        )
        .unwrap();

    assert_eq!(plan.action_digest(), action);
    assert_eq!(plan.request_owner_binding(), request_owner);
    assert_eq!(plan.rows().len(), 2);
    assert_eq!(plan.rows()[0].declared_order(), 0);
    assert_eq!(plan.rows()[0].components().len(), 2);
    assert_eq!(plan.rows()[0].retained_existing_parent_count(), 1);
    assert_eq!(plan.rows()[0].missing_suffix().len(), 1);
    assert_eq!(plan.rows()[1].components().len(), 3);
    assert_eq!(plan.rows()[1].retained_existing_parent_count(), 2);
    assert_eq!(plan.rows()[1].missing_suffix().len(), 1);

    let schedule = ActionScheduleV1::try_from_managed_plan(
        4,
        plan.schedule_inputs(),
        CleanupAliasSetV1::all(),
    )
    .unwrap();
    assert_eq!(schedule.managed_plan_digest(), plan.digest());
    assert_eq!(
        ActionScheduleV1::test_decode_canonical(&schedule.encode_canonical()).unwrap(),
        schedule
    );
    assert_eq!(schedule.bootstrap_rows().len(), plan.rows().len());
    for (plan_row, scheduled) in plan.rows().iter().zip(schedule.bootstrap_rows()) {
        assert_eq!(scheduled.spec_digest(), plan_row.spec_digest());
        assert_eq!(
            scheduled.component_range().len(),
            plan_row.missing_suffix().len()
        );
    }
}

#[test]
fn preflight_rejects_empty_suffix_and_noncanonical_observation_order() {
    let fully_present = Provider::new(7, vec![2]);
    assert!(
        ManagedParentBootstrapOwnerV1::new(&fully_present)
            .preflight(
                &request(&[ManagedParentPurpose::MergeStore]),
                ActionDigestV1::new([1; 32]),
                RequestOwnerBindingV1::new([2; 32]),
            )
            .is_err()
    );

    struct Reordered(Provider);
    impl ManagedParentBootstrap for Reordered {
        type RetainedParents = usize;

        fn provider_instance_id(&self) -> [u8; 32] {
            self.0.provider_instance_id()
        }

        fn observe_preflight(
            &self,
            request: &ManagedParentBootstrapRequest,
        ) -> Result<Vec<ManagedParentObservationV1>, CheckedFsError> {
            let mut rows = self.0.observe_preflight(request)?;
            rows.reverse();
            Ok(rows)
        }

        fn revalidate_plan(&self, plan: &ManagedParentPlanV1) -> Result<bool, CheckedFsError> {
            self.0.revalidate_plan(plan)
        }

        fn execute_bound(
            &self,
            plan: &BoundManagedParentPlanV1,
        ) -> Result<Self::RetainedParents, CheckedFsError> {
            self.0.execute_bound(plan)
        }
    }
    let reordered = Reordered(Provider::new(7, vec![1, 2]));
    assert!(
        ManagedParentBootstrapOwnerV1::new(&reordered)
            .preflight(
                &request(&[
                    ManagedParentPurpose::MergeStore,
                    ManagedParentPurpose::MergeArchive,
                ]),
                ActionDigestV1::new([1; 32]),
                RequestOwnerBindingV1::new([2; 32]),
            )
            .is_err()
    );
}

#[test]
fn only_the_exact_resident_plan_binds_and_executes() {
    let provider = Provider::new(7, vec![1, 2]);
    let owner = ManagedParentBootstrapOwnerV1::new(&provider);
    let action = ActionDigestV1::new([1; 32]);
    let request_owner = RequestOwnerBindingV1::new([2; 32]);
    let plan = owner
        .preflight(
            &request(&[
                ManagedParentPurpose::MergeStore,
                ManagedParentPurpose::MergeArchive,
            ]),
            action,
            request_owner,
        )
        .unwrap();
    let admitted = admitted(&plan, action, request_owner);
    let bound = owner.bind(&admitted, &plan).unwrap();
    assert_eq!(bound.reservation(), admitted.reservation());
    assert_eq!(bound.plan().digest(), plan.digest());
    assert_eq!(bound.rows().len(), 2);
    assert_eq!(owner.execute(&bound).unwrap(), 1);

    let empty = ActionCapacityReservationV1::new(
        action,
        request_owner,
        ActionScheduleV1::try_new(1, Vec::new(), CleanupAliasSetV1::all()).unwrap(),
    );
    let empty_admitted = admit_observed_action(
        &ActionDirectoryAdmissionV1::idle(),
        &empty,
        &ActionDirectoryObservationV1::Missing,
        &ActionDirectoryObservationV1::exact(
            identity(9),
            RecordObservationV1::Exact(empty.clone()),
        ),
    )
    .unwrap();
    assert!(owner.bind(&empty_admitted, &plan).is_err());

    let partial_schedule = ActionScheduleV1::try_new(
        1,
        vec![
            ManagedBootstrapInputV1::new(
                plan.rows()[0].spec_digest(),
                plan.rows()[0].missing_suffix().len(),
            )
            .unwrap(),
        ],
        CleanupAliasSetV1::all(),
    )
    .unwrap();
    let partial_reservation =
        ActionCapacityReservationV1::new(action, request_owner, partial_schedule);
    let partial_admitted = admit_observed_action(
        &ActionDirectoryAdmissionV1::idle(),
        &partial_reservation,
        &ActionDirectoryObservationV1::Missing,
        &ActionDirectoryObservationV1::exact(
            identity(8),
            RecordObservationV1::Exact(partial_reservation.clone()),
        ),
    )
    .unwrap();
    assert!(owner.bind(&partial_admitted, &plan).is_err());
}

#[test]
fn durable_intent_is_derived_from_and_rebinds_only_to_the_bound_plan_row() {
    let provider = Provider::new(7, vec![1]);
    let owner = ManagedParentBootstrapOwnerV1::new(&provider);
    let action = ActionDigestV1::new([1; 32]);
    let request_owner = RequestOwnerBindingV1::new([2; 32]);
    let plan = owner
        .preflight(
            &request(&[ManagedParentPurpose::MergeStore]),
            action,
            request_owner,
        )
        .unwrap();
    let bound = owner
        .bind(&admitted(&plan, action, request_owner), &plan)
        .unwrap();
    let intent = ManagedParentBootstrapIntentV1::try_initial(
        &bound,
        ManagedParentPurpose::MergeStore,
        [4; 32],
    )
    .unwrap();
    let generation = BootstrapGenerationV1::new(
        bound
            .scheduled_row(ManagedParentPurpose::MergeStore)
            .unwrap()
            .generation_range()
            .start,
    )
    .unwrap();
    assert_eq!(
        read_and_bind_managed_bootstrap_intent(
            Cursor::new(intent.encode_canonical()),
            &bound,
            ManagedParentPurpose::MergeStore,
            generation,
            None,
        )
        .unwrap()
        .value(),
        &intent
    );

    let other_provider = Provider::new(8, vec![1]);
    let other_owner = ManagedParentBootstrapOwnerV1::new(&other_provider);
    let other_plan = other_owner
        .preflight(
            &request(&[ManagedParentPurpose::MergeStore]),
            action,
            request_owner,
        )
        .unwrap();
    let other_bound = other_owner
        .bind(&admitted(&other_plan, action, request_owner), &other_plan)
        .unwrap();
    assert!(
        read_and_bind_managed_bootstrap_intent(
            Cursor::new(intent.encode_canonical()),
            &other_bound,
            ManagedParentPurpose::MergeStore,
            generation,
            None,
        )
        .is_err()
    );
}

#[test]
fn cross_action_owner_provider_and_stale_plans_reject_before_execution() {
    let provider = Provider::new(7, vec![1]);
    let owner = ManagedParentBootstrapOwnerV1::new(&provider);
    let action = ActionDigestV1::new([1; 32]);
    let request_owner = RequestOwnerBindingV1::new([2; 32]);
    let plan = owner
        .preflight(
            &request(&[ManagedParentPurpose::MergeStore]),
            action,
            request_owner,
        )
        .unwrap();

    assert!(
        owner
            .bind(
                &admitted(&plan, ActionDigestV1::new([3; 32]), request_owner),
                &plan,
            )
            .is_err()
    );
    assert!(
        owner
            .bind(
                &admitted(&plan, action, RequestOwnerBindingV1::new([4; 32])),
                &plan,
            )
            .is_err()
    );
    let other_provider = Provider::new(8, vec![1]);
    assert!(
        ManagedParentBootstrapOwnerV1::new(&other_provider)
            .bind(&admitted(&plan, action, request_owner), &plan)
            .is_err()
    );

    provider.current.set(false);
    assert!(
        owner
            .bind(&admitted(&plan, action, request_owner), &plan)
            .is_err()
    );
    assert_eq!(provider.executions.get(), 0);
}

#[test]
fn plan_digest_binds_retained_facts_not_only_schedule_shape() {
    let action = ActionDigestV1::new([1; 32]);
    let request_owner = RequestOwnerBindingV1::new([2; 32]);
    let first = ManagedParentBootstrapOwnerV1::new(&Provider::new(7, vec![1]))
        .preflight(
            &request(&[ManagedParentPurpose::MergeStore]),
            action,
            request_owner,
        )
        .unwrap();
    let second = ManagedParentBootstrapOwnerV1::new(&Provider::new(7, vec![0]))
        .preflight(
            &request(&[ManagedParentPurpose::MergeStore]),
            action,
            request_owner,
        )
        .unwrap();
    assert_ne!(first.digest(), second.digest());
    assert_ne!(
        first.rows()[0].missing_suffix(),
        second.rows()[0].missing_suffix()
    );
}
