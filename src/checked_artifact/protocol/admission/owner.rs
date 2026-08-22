//! Owner-private physical observation and admission issuance.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ObservedActionDirectoryV1 {
    Missing,
    Exact {
        identity: DurableObjectIdentityV1,
        reservation: Box<RecordObservationV1<ActionCapacityReservationV1>>,
        extra_children: usize,
    },
    Other,
}

impl ObservedActionDirectoryV1 {
    pub(in crate::checked_artifact) fn exact(
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

    fn has_exact(&self, expected: &ActionCapacityReservationV1) -> bool {
        matches!(
            self,
            Self::Exact {
                reservation,
                extra_children: 0,
                ..
            } if matches!(reservation.as_ref(), RecordObservationV1::Exact(value) if value == expected)
        )
    }

    fn has_rewritable_reservation(&self) -> bool {
        matches!(
            self,
            Self::Exact {
                reservation,
                extra_children: 0,
                ..
            } if matches!(
                reservation.as_ref(),
                RecordObservationV1::Missing | RecordObservationV1::PartialExpectedPrefix
            )
        )
    }

    fn exact_identity_for(
        &self,
        expected: &ActionCapacityReservationV1,
    ) -> Option<&DurableObjectIdentityV1> {
        match self {
            Self::Exact {
                identity,
                reservation,
                extra_children: 0,
            } if matches!(reservation.as_ref(), RecordObservationV1::Exact(value) if value == expected) => {
                Some(identity)
            }
            _ => None,
        }
    }
}

/// Sole production issuer for the action-directory authority handoff.
///
/// The physical half of this classifier is the R2-D `checked_artifact/admission`
/// owner, frozen at `GwzM5-8R2DInterfaceFreeze.md` §3.1 ("The physical driver is
/// the missing half of that classifier, not a second decision surface"). Only
/// that owner and this issuer can mint an `AdmittedActionV1`: the raw
/// observations below still carry no handle and no mutation capability, so the
/// amendment §7 (:576-577) boundary is unchanged by the widened visibility.
pub(in crate::checked_artifact) struct CatalogAdmissionOwnerV1;

impl CatalogAdmissionOwnerV1 {
    pub(in crate::checked_artifact) const fn new() -> Self {
        Self
    }

    pub(in crate::checked_artifact) fn classify_handoff(
        &self,
        admission: &ActionDirectoryAdmissionV1,
        expected: &ActionCapacityReservationV1,
        staging: &ObservedActionDirectoryV1,
        final_directory: &ObservedActionDirectoryV1,
    ) -> AdmissionHandoffDecisionV1 {
        use AdmissionHandoffDecisionV1::*;
        if !admission.matches_reservation(expected) {
            return Ambiguous;
        }
        match (staging, final_directory) {
            (ObservedActionDirectoryV1::Missing, ObservedActionDirectoryV1::Missing) => {
                CreateStaging
            }
            (value, ObservedActionDirectoryV1::Missing) if value.has_rewritable_reservation() => {
                WriteOrRewriteReservation
            }
            (value, ObservedActionDirectoryV1::Missing) if value.has_exact(expected) => {
                PublishStaging
            }
            (ObservedActionDirectoryV1::Missing, value) if value.has_exact(expected) => {
                ReplacePreparingWithIdle
            }
            _ => Ambiguous,
        }
    }

    pub(in crate::checked_artifact) fn admit(
        &self,
        admission: &ActionDirectoryAdmissionV1,
        expected: &ActionCapacityReservationV1,
        staging: &ObservedActionDirectoryV1,
        final_directory: &ObservedActionDirectoryV1,
    ) -> Option<AdmittedActionV1> {
        if !admission.is_idle() || !matches!(staging, ObservedActionDirectoryV1::Missing) {
            return None;
        }
        Some(AdmittedActionV1 {
            reservation: expected.clone(),
            directory_identity: final_directory.exact_identity_for(expected)?.clone(),
        })
    }
}
