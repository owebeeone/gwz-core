//! Evidence-bound successor transitions for managed-parent intent records.

use super::*;
use crate::checked_artifact::namespace::{InstalledManagedComponentV1, RetiredManagedMarkerV1};
use crate::checked_artifact::protocol::ActionSlotV1;

impl ManagedParentBootstrapIntentV1 {
    pub(in crate::checked_artifact) fn successor_after_component(
        &self,
        evidence: &InstalledManagedComponentV1,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if self.phase != ManagedBootstrapPhaseV1::InstallComponents
            || self.cursor >= self.components.len()
            || !self.matches_installed_component(evidence)
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "installed-component evidence does not close current component",
            ));
        }
        let marker = evidence.marker();
        let mut components = self.components.clone();
        components[self.cursor].ownership_marker_id = Some(marker.marker_id());
        components[self.cursor].ownership_marker_intent_id = Some(self.intent_id);
        components[self.cursor].installed_identity = Some(evidence.installed_identity().clone());
        components[self.cursor].installed_mode = Some(evidence.installed_mode());
        components[self.cursor].installed_path = Some(evidence.installed_path().clone());
        components[self.cursor].ownership_marker_object_identity =
            Some(evidence.marker_object_identity().clone());
        let next_cursor = self.cursor + 1;
        let next_phase = if next_cursor == components.len() {
            ManagedBootstrapPhaseV1::RetireMarkers
        } else {
            ManagedBootstrapPhaseV1::InstallComponents
        };
        self.successor(
            evidence.installed_identity().clone(),
            evidence.installed_mode(),
            evidence.installed_path().clone(),
            components,
            next_phase,
            if next_phase == ManagedBootstrapPhaseV1::RetireMarkers {
                0
            } else {
                next_cursor
            },
        )
    }

    pub(in crate::checked_artifact) fn successor_after_marker_retirement(
        &self,
        evidence: &RetiredManagedMarkerV1,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        if self.phase != ManagedBootstrapPhaseV1::RetireMarkers
            || self.cursor >= self.components.len()
            || !self.matches_retired_marker(evidence)
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "retired-marker evidence does not close current component",
            ));
        }
        let next_cursor = self.cursor + 1;
        let phase = if next_cursor == self.components.len() {
            ManagedBootstrapPhaseV1::Complete
        } else {
            ManagedBootstrapPhaseV1::RetireMarkers
        };
        self.successor(
            self.retained_parent_identity.clone(),
            self.retained_parent_mode,
            self.retained_parent_path.clone(),
            self.components.clone(),
            phase,
            next_cursor,
        )
    }

    fn successor(
        &self,
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_mode: PathComponentMode,
        retained_parent_path: CanonicalPathIdentityV1,
        components: Vec<ManagedBootstrapComponentRecordV1>,
        phase: ManagedBootstrapPhaseV1,
        cursor: usize,
    ) -> Result<Self, ProtocolCodecErrorV1> {
        let next_generation = self.generation_ordinal.index() + 1;
        Self::from_fields(
            self.action_digest,
            self.request_owner_binding,
            self.reservation_digest,
            self.schedule_digest,
            self.spec_digest,
            self.purpose,
            self.managed_plan_digest,
            self.bootstrap_ordinal,
            BootstrapGenerationV1::new(next_generation)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("generation range exhausted"))?,
            self.generation_start,
            self.component_start,
            retained_parent_identity,
            retained_parent_mode,
            retained_parent_path,
            components,
            self.ownership_token,
            Some(self.intent_id),
            phase,
            cursor,
        )
    }

    fn matches_installed_component(&self, evidence: &InstalledManagedComponentV1) -> bool {
        let component = &self.components[self.cursor];
        evidence.action_digest() == self.action_digest
            && evidence.reservation_digest() == self.reservation_digest
            && evidence.bootstrap_ordinal() == self.bootstrap_ordinal
            && evidence.component_ordinal() == component.global_component_ordinal
            && evidence.staging_leaf() == &component.staging_name
            && evidence.final_leaf() == &component.final_name
            && evidence.marker().matches_component(self, self.cursor)
            && evidence.installed_identity().support_profile()
                == self.retained_parent_identity.support_profile()
            && evidence.marker_object_identity().support_profile()
                == self.retained_parent_identity.support_profile()
            && evidence.installed_path().components().len()
                == self.retained_parent_path.components().len() + 1
            && evidence.installed_path().components()
                [..self.retained_parent_path.components().len()]
                == self.retained_parent_path.components()[..]
            && evidence
                .installed_path()
                .components()
                .last()
                .is_some_and(|path_component| {
                    path_component.original() == &component.final_name
                        && path_component.parent_durable_identity()
                            == &self.retained_parent_identity
                        && path_component.parent_mode() == self.retained_parent_mode
                })
    }

    fn matches_retired_marker(&self, evidence: &RetiredManagedMarkerV1) -> bool {
        let component = &self.components[self.cursor];
        let expected_retirement =
            ActionSlotV1::RetiredBootstrapMarker(component.global_component_ordinal.index() as u8)
                .name(self.action_digest);
        evidence.action_digest() == self.action_digest
            && evidence.reservation_digest() == self.reservation_digest
            && evidence.bootstrap_ordinal() == self.bootstrap_ordinal
            && evidence.component_ordinal() == component.global_component_ordinal
            && evidence.marker_retirement_leaf().as_bytes() == expected_retirement.as_bytes()
            && self.components[self.cursor].ownership_marker_id
                == Some(evidence.marker().marker_id())
            && evidence
                .marker()
                .matches_static_component(self, self.cursor)
            && component.ownership_marker_object_identity.as_ref()
                == Some(evidence.retired_marker_identity())
            && component.installed_identity.as_ref() == Some(evidence.installed_parent_identity())
            && component.installed_mode == Some(evidence.installed_parent_mode())
            && component.installed_path.as_ref() == Some(evidence.installed_parent_path())
    }
}
