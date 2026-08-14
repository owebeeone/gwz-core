//! Closed record-shape and generation-chain validation.

use super::{
    ManagedBootstrapComponentRecordV1, ManagedBootstrapPhaseV1, managed_marker_name,
    managed_staging_name,
};
use crate::checked_artifact::capability::{
    DurableObjectIdentityV1, DurablePathV1, PathComponentMode,
};
use crate::checked_artifact::protocol::codec::ProtocolCodecErrorV1;
use crate::checked_artifact::protocol::codec::path_matches_profile;
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
    retained_parent_identity: &DurableObjectIdentityV1,
    retained_parent_mode: PathComponentMode,
    retained_parent_path: &DurablePathV1,
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
            || component.marker_name != managed_marker_name()
        {
            return Err(ProtocolCodecErrorV1::Invalid(
                "managed component binding mismatch",
            ));
        }
        let installed = match (
            component.ownership_marker_id,
            component.ownership_marker_intent_id,
            component.installed_identity.as_ref(),
            component.installed_mode,
            component.installed_path.as_ref(),
            component.ownership_marker_object_identity.as_ref(),
        ) {
            (None, None, None, None, None, None) => false,
            (
                Some(marker_id),
                Some(marker_intent_id),
                Some(installed_identity),
                Some(_),
                Some(installed_path),
                Some(marker_object_identity),
            ) if marker_id
                == OwnershipMarkerV1::derived_id_for_component(
                    action_digest,
                    request_owner_binding,
                    schedule_digest,
                    marker_intent_id,
                    bootstrap_ordinal,
                    local,
                    component,
                    ownership_token,
                )?
                && path_matches_profile(installed_path, installed_identity.support_profile())
                && marker_object_identity.support_profile()
                    == installed_identity.support_profile() =>
            {
                true
            }
            _ => {
                return Err(ProtocolCodecErrorV1::Invalid(
                    "managed component marker binding mismatch",
                ));
            }
        };
        if installed {
            let path = component
                .installed_path
                .as_ref()
                .expect("complete installed tuple was checked");
            let last = path
                .components()
                .last()
                .expect("canonical path is nonempty");
            if last.original() != &component.final_name {
                return Err(ProtocolCodecErrorV1::Invalid(
                    "installed component path leaf mismatch",
                ));
            }
            if local > 0 {
                let previous = &components[local - 1];
                let previous_path =
                    previous
                        .installed_path
                        .as_ref()
                        .ok_or(ProtocolCodecErrorV1::Invalid(
                            "installed component chain is not a prefix",
                        ))?;
                if path.components().len() != previous_path.components().len() + 1
                    || path.components()[..previous_path.components().len()]
                        != previous_path.components()[..]
                    || Some(last.parent_durable_identity()) != previous.installed_identity.as_ref()
                    || last.parent_mode()
                        != previous
                            .installed_mode
                            .ok_or(ProtocolCodecErrorV1::Invalid(
                                "installed component mode is missing",
                            ))?
                {
                    return Err(ProtocolCodecErrorV1::Invalid(
                        "installed component chain binding mismatch",
                    ));
                }
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
                || component.installed_identity.is_none()
                || component.installed_mode.is_none()
                || component.installed_path.is_none()
                || component.ownership_marker_object_identity.is_none()
        })
        || components[expected_markers..].iter().any(|component| {
            component.ownership_marker_id.is_some()
                || component.ownership_marker_intent_id.is_some()
                || component.installed_identity.is_some()
                || component.installed_mode.is_some()
                || component.installed_path.is_some()
                || component.ownership_marker_object_identity.is_some()
        })
    {
        return Err(ProtocolCodecErrorV1::Invalid(
            "managed bootstrap transition mismatch",
        ));
    }
    if let Some(last_installed) = components[..expected_markers].last()
        && (last_installed.installed_identity.as_ref() != Some(retained_parent_identity)
            || last_installed.installed_mode != Some(retained_parent_mode)
            || last_installed.installed_path.as_ref() != Some(retained_parent_path))
    {
        return Err(ProtocolCodecErrorV1::Invalid(
            "retained parent does not equal the installed component chain",
        ));
    }
    Ok(())
}
