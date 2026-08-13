//! Test-only facade for exhaustive synthetic admission observations.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ActionDirectoryObservationV1 {
    Missing,
    Exact {
        identity: DurableObjectIdentityV1,
        reservation: Box<RecordObservationV1<ActionCapacityReservationV1>>,
        extra_children: usize,
    },
    Other,
}

impl ActionDirectoryObservationV1 {
    pub(in crate::checked_artifact) fn exact(
        identity: DurableObjectIdentityV1,
        reservation: RecordObservationV1<ActionCapacityReservationV1>,
    ) -> Self {
        Self::exact_with_children(identity, reservation, 0)
    }

    fn exact_with_children(
        identity: DurableObjectIdentityV1,
        reservation: RecordObservationV1<ActionCapacityReservationV1>,
        extra_children: usize,
    ) -> Self {
        Self::Exact {
            identity,
            reservation: Box::new(reservation),
            extra_children,
        }
    }

    fn to_owner(&self) -> owner::ObservedActionDirectoryV1 {
        match self {
            Self::Missing => owner::ObservedActionDirectoryV1::Missing,
            Self::Exact {
                identity,
                reservation,
                extra_children,
            } => owner::ObservedActionDirectoryV1::exact(
                identity.clone(),
                reservation.as_ref().clone(),
                *extra_children,
            ),
            Self::Other => owner::ObservedActionDirectoryV1::Other,
        }
    }
}

pub(in crate::checked_artifact) fn classify_handoff(
    admission: &ActionDirectoryAdmissionV1,
    expected: &ActionCapacityReservationV1,
    staging: &ActionDirectoryObservationV1,
    final_directory: &ActionDirectoryObservationV1,
) -> AdmissionHandoffDecisionV1 {
    owner::CatalogAdmissionOwnerV1::new().classify_handoff(
        admission,
        expected,
        &staging.to_owner(),
        &final_directory.to_owner(),
    )
}

pub(in crate::checked_artifact) fn admit_observed_action(
    admission: &ActionDirectoryAdmissionV1,
    expected: &ActionCapacityReservationV1,
    staging: &ActionDirectoryObservationV1,
    final_directory: &ActionDirectoryObservationV1,
) -> Option<AdmittedActionV1> {
    owner::CatalogAdmissionOwnerV1::new().admit(
        admission,
        expected,
        &staging.to_owner(),
        &final_directory.to_owner(),
    )
}

pub(in crate::checked_artifact) struct CatalogAdmissionOwnerTestV1;

impl CatalogAdmissionOwnerTestV1 {
    pub(in crate::checked_artifact) const fn new() -> Self {
        Self
    }

    pub(in crate::checked_artifact) const fn observe_missing(
        &self,
    ) -> ActionDirectoryObservationV1 {
        ActionDirectoryObservationV1::Missing
    }

    pub(in crate::checked_artifact) fn observe_exact(
        &self,
        identity: DurableObjectIdentityV1,
        reservation: RecordObservationV1<ActionCapacityReservationV1>,
        extra_children: usize,
    ) -> ActionDirectoryObservationV1 {
        ActionDirectoryObservationV1::exact_with_children(identity, reservation, extra_children)
    }

    pub(in crate::checked_artifact) const fn observe_other(&self) -> ActionDirectoryObservationV1 {
        ActionDirectoryObservationV1::Other
    }

    pub(in crate::checked_artifact) fn classify_handoff(
        &self,
        admission: &ActionDirectoryAdmissionV1,
        expected: &ActionCapacityReservationV1,
        staging: &ActionDirectoryObservationV1,
        final_directory: &ActionDirectoryObservationV1,
    ) -> AdmissionHandoffDecisionV1 {
        classify_handoff(admission, expected, staging, final_directory)
    }

    pub(in crate::checked_artifact) fn admit(
        &self,
        admission: &ActionDirectoryAdmissionV1,
        expected: &ActionCapacityReservationV1,
        staging: &ActionDirectoryObservationV1,
        final_directory: &ActionDirectoryObservationV1,
    ) -> Option<AdmittedActionV1> {
        admit_observed_action(admission, expected, staging, final_directory)
    }
}
