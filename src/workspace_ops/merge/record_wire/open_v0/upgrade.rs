use std::path::Path;

use serde_yaml::Value;

use super::super::decode::{DecodedV0Record, decode_production_v1};
use super::super::unknown_fields::UnknownFieldManifest;
use super::{
    OpenV0AdaptationInternal as OpenV0Adaptation, adapt_open_v0_internal as adapt_open_v0,
};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::CanonicalMergeRecord;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PreparedOpenV0Upgrade {
    ValidUnlisted,
    Eligible(Box<PreparedV1Upgrade>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreparedV1Upgrade {
    pub(crate) rule_id: String,
    pub(crate) next_action: String,
    bytes: Vec<u8>,
    canonical: CanonicalMergeRecord,
    unknown_fields: UnknownFieldManifest,
}

impl PreparedV1Upgrade {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn verify_bytes(&self, bytes: &[u8]) -> ModelResult<()> {
        let decoded = decode_production_v1(bytes)
            .map_err(|error| verification_error(format!("v1 decode failed: {error:?}")))?;
        if decoded.canonical != self.canonical || decoded.unknown_fields != self.unknown_fields {
            return Err(verification_error(
                "v1 canonical model or unknown-field manifest changed",
            ));
        }
        let next_action = crate::workspace_ops::merge::acceptance::finalization_next_action_for_v1(
            &decoded.record,
        )?;
        if next_action != self.next_action {
            return Err(verification_error(format!(
                "v1 next action '{next_action}' differs from expected '{}'",
                self.next_action
            )));
        }
        Ok(())
    }
}

pub(crate) fn prepare_upgrade<B: GitBackend>(
    backend: &B,
    root: &Path,
    decoded: &DecodedV0Record,
    writer_version: &str,
) -> ModelResult<PreparedOpenV0Upgrade> {
    let adaptation = adapt_open_v0(backend, root, decoded, writer_version)?;
    let OpenV0Adaptation::Eligible {
        rule_id,
        next_action,
        record,
        canonical,
        unknown_fields,
    } = adaptation
    else {
        return Ok(PreparedOpenV0Upgrade::ValidUnlisted);
    };
    let mut raw = serde_yaml::to_value(&*record)
        .map_err(|error| verification_error(format!("v1 serialization failed: {error}")))?;
    let overlaid = unknown_fields
        .apply_surviving(&mut raw)
        .map_err(|error| verification_error(error.detail))?;
    if overlaid != unknown_fields {
        return Err(verification_error(
            "migration retired an unknown field from a surviving container",
        ));
    }
    let bytes = serialize(raw)?;
    let prepared = PreparedV1Upgrade {
        rule_id,
        next_action,
        bytes,
        canonical: *canonical,
        unknown_fields,
    };
    prepared.verify_bytes(prepared.bytes())?;
    Ok(PreparedOpenV0Upgrade::Eligible(Box::new(prepared)))
}

fn serialize(raw: Value) -> ModelResult<Vec<u8>> {
    serde_yaml::to_string(&raw)
        .map(String::into_bytes)
        .map_err(|error| verification_error(format!("v1 encoding failed: {error}")))
}

fn verification_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!(
            "atomic merge-record upgrade verification failed: {}",
            detail.into()
        ),
    )
}
