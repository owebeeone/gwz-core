
use crate::model;
use crate::runtime::clock::TimestampMs;



pub(crate) fn attribution_from_protocol(
    value: &crate::OperationAttribution,
) -> model::ModelResult<model::OperationAttribution> {
    let attribution = model::OperationAttribution {
        actor: value.actor.as_ref().map(|actor| model::OperationActor {
            actor_id: actor.actor_id.clone(),
            display_name: actor.display_name.clone(),
            email: actor.email.clone(),
            authority: actor.authority.clone(),
        }),
        git_author: value.git_author.as_ref().map(git_identity_from_protocol),
        git_committer: value.git_committer.as_ref().map(git_identity_from_protocol),
        credential_ref: value.credential_ref.clone(),
    };
    attribution.validate()?;
    Ok(attribution)
}

pub(crate) fn git_identity_from_protocol(value: &crate::GitObjectIdentity) -> model::GitObjectIdentity {
    model::GitObjectIdentity {
        name: value.name.clone(),
        email: value.email.clone(),
        time_ms: value.time_ms.map(TimestampMs),
        timezone_offset_minutes: value.timezone_offset_minutes,
    }
}

