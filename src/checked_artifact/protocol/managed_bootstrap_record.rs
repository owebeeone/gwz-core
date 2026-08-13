//! Closed durable state chain for one scheduled managed-parent bootstrap.

use sha2::{Digest, Sha256};

use super::ActionCapacityReservationV1;
use super::codec::{ProtocolCodecErrorV1, decode_ascii};
use super::schedule::{
    ActionDigestV1, BootstrapComponentOrdinalV1, BootstrapGenerationV1, BootstrapOrdinalV1,
    RecordDigestV1, RequestOwnerBindingV1, ScheduleDigestV1,
};
use crate::checked_artifact::bootstrap::{BoundManagedParentPlanV1, ManagedParentPurpose};
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, DurableObjectIdentityV1, PathComponentMode,
};

mod codec;
#[allow(
    unused_imports,
    reason = "R1 exports bounded recovery interfaces before R2 consumers are converted"
)]
pub(in crate::checked_artifact) use codec::*;
mod validation;
use validation::validate_shape;
mod transition;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ManagedBootstrapPhaseV1 {
    InstallComponents,
    RetireMarkers,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedBootstrapComponentRecordV1 {
    component_ascii: AsciiComponent,
    staging_name: AsciiComponent,
    final_name: AsciiComponent,
    marker_name: AsciiComponent,
    global_component_ordinal: BootstrapComponentOrdinalV1,
    ownership_marker_id: Option<[u8; 32]>,
    ownership_marker_intent_id: Option<[u8; 32]>,
    installed_identity: Option<DurableObjectIdentityV1>,
    installed_mode: Option<PathComponentMode>,
    installed_path: Option<CanonicalPathIdentityV1>,
    ownership_marker_object_identity: Option<DurableObjectIdentityV1>,
}

impl ManagedBootstrapComponentRecordV1 {
    pub(in crate::checked_artifact) fn final_name(&self) -> &AsciiComponent {
        &self.final_name
    }

    pub(in crate::checked_artifact) fn staging_name(&self) -> &AsciiComponent {
        &self.staging_name
    }

    pub(in crate::checked_artifact) const fn global_component_ordinal(
        &self,
    ) -> BootstrapComponentOrdinalV1 {
        self.global_component_ordinal
    }

    pub(in crate::checked_artifact) const fn ownership_marker_id(&self) -> Option<[u8; 32]> {
        self.ownership_marker_id
    }

    pub(in crate::checked_artifact) fn installed_identity(
        &self,
    ) -> Option<&DurableObjectIdentityV1> {
        self.installed_identity.as_ref()
    }

    pub(in crate::checked_artifact) const fn installed_mode(&self) -> Option<PathComponentMode> {
        self.installed_mode
    }

    pub(in crate::checked_artifact) fn installed_path(&self) -> Option<&CanonicalPathIdentityV1> {
        self.installed_path.as_ref()
    }

    pub(in crate::checked_artifact) fn ownership_marker_object_identity(
        &self,
    ) -> Option<&DurableObjectIdentityV1> {
        self.ownership_marker_object_identity.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedParentBootstrapIntentV1 {
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    reservation_digest: RecordDigestV1,
    schedule_digest: ScheduleDigestV1,
    spec_digest: [u8; 32],
    purpose: ManagedParentPurpose,
    managed_plan_digest: [u8; 32],
    bootstrap_ordinal: BootstrapOrdinalV1,
    generation_ordinal: BootstrapGenerationV1,
    generation_start: usize,
    component_start: usize,
    retained_parent_identity: DurableObjectIdentityV1,
    retained_parent_mode: PathComponentMode,
    retained_parent_path: CanonicalPathIdentityV1,
    components: Vec<ManagedBootstrapComponentRecordV1>,
    ownership_token: [u8; 32],
    predecessor_intent_id: Option<[u8; 32]>,
    phase: ManagedBootstrapPhaseV1,
    cursor: usize,
    intent_id: [u8; 32],
}

impl ManagedParentBootstrapIntentV1 {
    pub(in crate::checked_artifact) fn try_initial(
        bound_plan: &BoundManagedParentPlanV1,
        purpose: ManagedParentPurpose,
        ownership_token: [u8; 32],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let row = bound_plan
            .scheduled_row(purpose)
            .ok_or(ProtocolCodecErrorV1::Invalid(
                "managed purpose is not present in the bound plan",
            ))?;
        if bound_plan.plan().digest() != bound_plan.reservation().schedule().managed_plan_digest() {
            return Err(ProtocolCodecErrorV1::Invalid(
                "managed plan digest does not match resident schedule",
            ));
        }
        Self::try_initial_fields(
            bound_plan.reservation(),
            row.spec_digest(),
            purpose,
            row.bootstrap_ordinal(),
            row.retained_parent_identity().clone(),
            row.retained_parent_mode(),
            row.retained_parent_path().clone(),
            row.missing_suffix().to_vec(),
            ownership_token,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::checked_artifact) fn try_initial_for_test(
        reservation: &ActionCapacityReservationV1,
        spec_digest: [u8; 32],
        bootstrap_ordinal: BootstrapOrdinalV1,
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_mode: PathComponentMode,
        retained_parent_path: CanonicalPathIdentityV1,
        missing_components: Vec<AsciiComponent>,
        ownership_token: [u8; 32],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        Self::try_initial_fields(
            reservation,
            spec_digest,
            ManagedParentPurpose::MergeStore,
            bootstrap_ordinal,
            retained_parent_identity,
            retained_parent_mode,
            retained_parent_path,
            missing_components,
            ownership_token,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn try_initial_fields(
        reservation: &ActionCapacityReservationV1,
        spec_digest: [u8; 32],
        purpose: ManagedParentPurpose,
        bootstrap_ordinal: BootstrapOrdinalV1,
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_mode: PathComponentMode,
        retained_parent_path: CanonicalPathIdentityV1,
        missing_components: Vec<AsciiComponent>,
        ownership_token: [u8; 32],
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if ownership_token == [0; 32] {
            return Err(ProtocolCodecErrorV1::Invalid(
                "managed bootstrap ownership token must be nonzero",
            ));
        }
        let row = reservation
            .schedule()
            .bootstrap_rows()
            .get(bootstrap_ordinal.index())
            .ok_or(ProtocolCodecErrorV1::Invalid(
                "bootstrap ordinal is not reserved",
            ))?;
        let component_range = row.component_range();
        let generation_range = row.generation_range();
        if row.ordinal() != bootstrap_ordinal
            || row.spec_digest() != spec_digest
            || missing_components.len() != component_range.len()
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "managed bootstrap plan does not match reservation",
            ));
        }
        let mut components = Vec::new();
        components
            .try_reserve_exact(missing_components.len())
            .map_err(|_| ProtocolCodecErrorV1::Invalid("component allocation failed"))?;
        for (local, final_name) in missing_components.into_iter().enumerate() {
            let global = BootstrapComponentOrdinalV1::new(component_range.start + local)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid component ordinal"))?;
            components.push(ManagedBootstrapComponentRecordV1 {
                component_ascii: final_name.clone(),
                staging_name: managed_staging_name(reservation.action_digest(), global.index())?,
                final_name,
                marker_name: managed_marker_name(),
                global_component_ordinal: global,
                ownership_marker_id: None,
                ownership_marker_intent_id: None,
                installed_identity: None,
                installed_mode: None,
                installed_path: None,
                ownership_marker_object_identity: None,
            });
        }
        Self::from_fields(
            reservation.action_digest(),
            reservation.request_owner_binding(),
            reservation.record_digest(),
            reservation.schedule().digest(),
            spec_digest,
            purpose,
            reservation.schedule().managed_plan_digest(),
            bootstrap_ordinal,
            BootstrapGenerationV1::new(generation_range.start)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid generation ordinal"))?,
            generation_range.start,
            component_range.start,
            retained_parent_identity,
            retained_parent_mode,
            retained_parent_path,
            components,
            ownership_token,
            None,
            ManagedBootstrapPhaseV1::InstallComponents,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_fields(
        action_digest: ActionDigestV1,
        request_owner_binding: RequestOwnerBindingV1,
        reservation_digest: RecordDigestV1,
        schedule_digest: ScheduleDigestV1,
        spec_digest: [u8; 32],
        purpose: ManagedParentPurpose,
        managed_plan_digest: [u8; 32],
        bootstrap_ordinal: BootstrapOrdinalV1,
        generation_ordinal: BootstrapGenerationV1,
        generation_start: usize,
        component_start: usize,
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_mode: PathComponentMode,
        retained_parent_path: CanonicalPathIdentityV1,
        components: Vec<ManagedBootstrapComponentRecordV1>,
        ownership_token: [u8; 32],
        predecessor_intent_id: Option<[u8; 32]>,
        phase: ManagedBootstrapPhaseV1,
        cursor: usize,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let profile = retained_parent_identity.support_profile();
        if !super::codec::path_matches_profile(&retained_parent_path, profile) {
            return Err(ProtocolCodecErrorV1::Invalid(
                "managed bootstrap parent identities use different support profiles",
            ));
        }
        validate_shape(
            action_digest,
            request_owner_binding,
            schedule_digest,
            bootstrap_ordinal,
            generation_ordinal,
            generation_start,
            component_start,
            &retained_parent_identity,
            retained_parent_mode,
            &retained_parent_path,
            &components,
            ownership_token,
            predecessor_intent_id,
            phase,
            cursor,
        )?;
        let mut value = Self {
            action_digest,
            request_owner_binding,
            reservation_digest,
            schedule_digest,
            spec_digest,
            purpose,
            managed_plan_digest,
            bootstrap_ordinal,
            generation_ordinal,
            generation_start,
            component_start,
            retained_parent_identity,
            retained_parent_mode,
            retained_parent_path,
            components,
            ownership_token,
            predecessor_intent_id,
            phase,
            cursor,
            intent_id: [0; 32],
        };
        value.intent_id = Sha256::digest(value.digest_material()).into();
        Ok(value)
    }

    pub(in crate::checked_artifact) const fn intent_id(&self) -> [u8; 32] {
        self.intent_id
    }

    pub(in crate::checked_artifact) const fn phase(&self) -> ManagedBootstrapPhaseV1 {
        self.phase
    }

    pub(in crate::checked_artifact) const fn cursor(&self) -> usize {
        self.cursor
    }

    pub(in crate::checked_artifact) fn components(&self) -> &[ManagedBootstrapComponentRecordV1] {
        &self.components
    }

    pub(in crate::checked_artifact) fn is_complete(&self) -> bool {
        self.phase == ManagedBootstrapPhaseV1::Complete && self.cursor == self.components.len()
    }

    pub(in crate::checked_artifact) const fn generation_ordinal(&self) -> BootstrapGenerationV1 {
        self.generation_ordinal
    }

    pub(in crate::checked_artifact) const fn predecessor_intent_id(&self) -> Option<[u8; 32]> {
        self.predecessor_intent_id
    }

    pub(in crate::checked_artifact) fn matches_reservation(
        &self,
        reservation: &ActionCapacityReservationV1,
    ) -> bool {
        let Some(row) = reservation
            .schedule()
            .bootstrap_rows()
            .get(self.bootstrap_ordinal.index())
        else {
            return false;
        };
        self.action_digest == reservation.action_digest()
            && self.request_owner_binding == reservation.request_owner_binding()
            && self.reservation_digest == reservation.record_digest()
            && self.schedule_digest == reservation.schedule().digest()
            && self.managed_plan_digest == reservation.schedule().managed_plan_digest()
            && self.spec_digest == row.spec_digest()
            && self.bootstrap_ordinal == row.ordinal()
            && self.generation_start == row.generation_range().start
            && self.component_start == row.component_range().start
            && self.components.len() == row.component_range().len()
            && self.generation_ordinal.index() < row.generation_range().end
    }

    pub(in crate::checked_artifact) fn matches_component_parent(
        &self,
        component_index: usize,
        identity: &DurableObjectIdentityV1,
        path: &CanonicalPathIdentityV1,
    ) -> bool {
        let Some(component) = self.components.get(component_index) else {
            return false;
        };
        let Some(installed_path) = component.installed_path.as_ref() else {
            return component_index == self.cursor
                && self.retained_parent_identity == *identity
                && self.retained_parent_path == *path;
        };
        installed_path.components().len() == path.components().len() + 1
            && installed_path.components()[..path.components().len()] == path.components()[..]
            && installed_path
                .components()
                .last()
                .is_some_and(|installed| installed.parent_durable_identity() == identity)
    }

    fn matches_bound_plan(
        &self,
        bound_plan: &BoundManagedParentPlanV1,
        purpose: ManagedParentPurpose,
    ) -> bool {
        let Some(row) = bound_plan.scheduled_row(purpose) else {
            return false;
        };
        self.matches_reservation(bound_plan.reservation())
            && self.purpose == purpose
            && self.managed_plan_digest == bound_plan.plan().digest()
            && self.spec_digest == row.spec_digest()
            && self.bootstrap_ordinal == row.bootstrap_ordinal()
            && self.generation_start == row.generation_range().start
            && self.component_start == row.component_range().start
            && self.matches_initial_parent(
                row.retained_parent_identity(),
                row.retained_parent_mode(),
                row.retained_parent_path(),
            )
            && self
                .components
                .iter()
                .map(|component| &component.final_name)
                .eq(row.missing_suffix().iter())
    }

    fn matches_initial_parent(
        &self,
        identity: &DurableObjectIdentityV1,
        mode: PathComponentMode,
        path: &CanonicalPathIdentityV1,
    ) -> bool {
        let Some(first_installed_path) = self
            .components
            .first()
            .and_then(|component| component.installed_path.as_ref())
        else {
            return self.retained_parent_identity == *identity
                && self.retained_parent_mode == mode
                && self.retained_parent_path == *path;
        };
        first_installed_path.components().len() == path.components().len() + 1
            && first_installed_path.components()[..path.components().len()] == path.components()[..]
            && first_installed_path
                .components()
                .last()
                .is_some_and(|component| {
                    component.parent_durable_identity() == identity
                        && component.parent_mode() == mode
                })
    }

    pub(super) const fn action_digest(&self) -> ActionDigestV1 {
        self.action_digest
    }

    pub(super) const fn request_owner_binding(&self) -> RequestOwnerBindingV1 {
        self.request_owner_binding
    }

    pub(super) const fn schedule_digest(&self) -> ScheduleDigestV1 {
        self.schedule_digest
    }

    pub(super) const fn bootstrap_ordinal(&self) -> BootstrapOrdinalV1 {
        self.bootstrap_ordinal
    }

    pub(super) const fn ownership_token(&self) -> [u8; 32] {
        self.ownership_token
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn retained_parent_path_for_test(
        &self,
    ) -> &CanonicalPathIdentityV1 {
        &self.retained_parent_path
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn retained_parent_identity_for_test(
        &self,
    ) -> &DurableObjectIdentityV1 {
        &self.retained_parent_identity
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) const fn retained_parent_mode_for_test(
        &self,
    ) -> PathComponentMode {
        self.retained_parent_mode
    }
}

pub(in crate::checked_artifact) fn managed_staging_name(
    action: ActionDigestV1,
    component: usize,
) -> Result<AsciiComponent, ProtocolCodecErrorV1> {
    let component = BootstrapComponentOrdinalV1::new(component)
        .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid component ordinal"))?;
    let name = format!(
        "gwz-bootstrap-{}-{:02}-staging-v1",
        action.hex(),
        component.index()
    );
    decode_ascii(name.as_bytes())
}

pub(in crate::checked_artifact) fn managed_marker_name() -> AsciiComponent {
    decode_ascii(b"gwz-bootstrap-owner-v1").expect("fixed managed marker name is valid")
}
