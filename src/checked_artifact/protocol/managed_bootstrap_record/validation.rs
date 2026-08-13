//! Closed record-shape and generation-chain validation.

use super::{
    MARKER_NAME, ManagedBootstrapComponentRecordV1, ManagedBootstrapPhaseV1, managed_staging_name,
};
use crate::checked_artifact::protocol::codec::ProtocolCodecErrorV1;
use crate::checked_artifact::protocol::ownership_marker_record::OwnershipMarkerV1;
use crate::checked_artifact::protocol::schedule::{
    ActionDigestV1, BootstrapComponentOrdinalV1, BootstrapGenerationV1, BootstrapOrdinalV1,
    RequestOwnerBindingV1, ScheduleDigestV1,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_shape(
    action_digest: ActionDigestV1,
    request_owner_binding: RequestOwnerBindingV1,
    schedule_digest: ScheduleDigestV1,
    bootstrap_ordinal: BootstrapOrdinalV1,
    generation_ordinal: BootstrapGenerationV1,
    generation_start: usize,
    component_start: usize,
    components: &[ManagedBootstrapComponentRecordV1],
    ownership_token: [u8; 32],
    predecessor_intent_id: Option<[u8; 32]>,
    phase: ManagedBootstrapPhaseV1,
    cursor: usize,
) -> Result<(), ProtocolCodecErrorV1> {
    if components.is_empty()
        || components.len() > crate::checked_artifact::protocol::MAX_MANAGED_PARENT_COMPONENTS
        || ownership_token == [0; 32]
        || component_start + components.len()
            > crate::checked_artifact::protocol::MAX_MANAGED_PARENT_COMPONENTS
    {
        return Err(ProtocolCodecErrorV1::Invalid(
            "invalid managed bootstrap bounds",
        ));
    }
    for (local, component) in components.iter().enumerate() {
        let expected = BootstrapComponentOrdinalV1::new(component_start + local)
            .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid component ordinal"))?;
        if component.component_ascii != component.final_name
            || component.global_component_ordinal != expected
            || component.staging_name != managed_staging_name(action_digest, expected.index())?
            || component.marker_name.as_bytes() != MARKER_NAME
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "managed component binding mismatch",
            ));
        }
        match (
            component.ownership_marker_id,
            component.ownership_marker_intent_id,
        ) {
            (None, None) => {}
            (Some(marker_id), Some(marker_intent_id))
                if marker_id
                    == OwnershipMarkerV1::derived_id_for_component(
                        action_digest,
                        request_owner_binding,
                        schedule_digest,
                        marker_intent_id,
                        bootstrap_ordinal,
                        local,
                        component,
                        ownership_token,
                    )? => {}
            _ => {
                return Err(ProtocolCodecErrorV1::Invalid(
                    "managed component marker binding mismatch",
                ));
            }
        }
    }
    let component_count = components.len();
    let (expected_generation, expected_markers) = match phase {
        ManagedBootstrapPhaseV1::InstallComponents if cursor < component_count => {
            (generation_start + cursor, cursor)
        }
        ManagedBootstrapPhaseV1::RetireMarkers if cursor < component_count => {
            (generation_start + component_count + cursor, component_count)
        }
        ManagedBootstrapPhaseV1::Complete if cursor == component_count => {
            (generation_start + 2 * component_count, component_count)
        }
        _ => {
            return Err(ProtocolCodecErrorV1::Invalid(
                "invalid bootstrap phase cursor",
            ));
        }
    };
    if generation_ordinal.index() != expected_generation
        || predecessor_intent_id.is_some() != (expected_generation != generation_start)
        || components[..expected_markers].iter().any(|component| {
            component.ownership_marker_id.is_none()
                || component.ownership_marker_intent_id.is_none()
        })
        || components[expected_markers..].iter().any(|component| {
            component.ownership_marker_id.is_some()
                || component.ownership_marker_intent_id.is_some()
        })
    {
        return Err(ProtocolCodecErrorV1::Invalid(
            "managed bootstrap transition mismatch",
        ));
    }
    Ok(())
}
