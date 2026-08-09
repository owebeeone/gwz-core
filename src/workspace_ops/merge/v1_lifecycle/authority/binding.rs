use serde::Serialize;
use serde_yaml::Value;
use sha2::{Digest, Sha256};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::checked::{RecordDigest, StoredV1Record};

#[derive(Debug, Eq, PartialEq)]
struct ProofBinding {
    source_digest: RecordDigest,
    workspace_id: String,
    merge_id: String,
    operation_id: String,
    owner: String,
    action: String,
    phase: String,
    payload_sha256: [u8; 32],
}

impl ProofBinding {
    fn new<T: Serialize>(
        current: &StoredV1Record,
        owner: &str,
        action: &str,
        phase: &str,
        payload: &T,
    ) -> ModelResult<Self> {
        let record = current.record();
        Ok(Self {
            source_digest: current.source_digest(),
            workspace_id: record.workspace_id.clone(),
            merge_id: record.merge_id.clone(),
            operation_id: record.operation_id.clone(),
            owner: owner.into(),
            action: action.into(),
            phase: phase.into(),
            payload_sha256: payload_hash(payload)?,
        })
    }

    fn same_record(&self, current: &StoredV1Record) -> bool {
        let record = current.record();
        self.source_digest == current.source_digest()
            && self.workspace_id == record.workspace_id
            && self.merge_id == record.merge_id
            && self.operation_id == record.operation_id
    }
}

#[derive(Debug)]
pub(super) struct BoundValue<T> {
    binding: ProofBinding,
    pub(super) value: T,
}

pub(super) struct AuthorityIssuer<'a> {
    current: &'a StoredV1Record,
}

impl<'a> AuthorityIssuer<'a> {
    pub(super) fn for_observer(current: &'a StoredV1Record) -> Self {
        Self { current }
    }

    pub(super) fn bind<T: Serialize>(
        &self,
        owner: &str,
        action: &str,
        phase: &str,
        value: T,
    ) -> ModelResult<BoundValue<T>> {
        BoundValue::new(self.current, owner, action, phase, value)
    }
}

impl<T: Serialize> BoundValue<T> {
    pub(super) fn new(
        current: &StoredV1Record,
        owner: &str,
        action: &str,
        phase: &str,
        value: T,
    ) -> ModelResult<Self> {
        Ok(Self {
            binding: ProofBinding::new(current, owner, action, phase, &value)?,
            value,
        })
    }

    pub(super) fn matches(
        &self,
        current: &StoredV1Record,
        owner: &str,
        action: &str,
        phase: &str,
    ) -> bool {
        self.binding.same_record(current)
            && self.binding.owner == owner
            && self.binding.action == action
            && self.binding.phase == phase
            && payload_hash(&self.value).is_ok_and(|payload| self.binding.payload_sha256 == payload)
    }
}

pub(super) fn payload_hash<T: Serialize>(value: &T) -> ModelResult<[u8; 32]> {
    let value = serde_yaml::to_value(value).map_err(binding_error)?;
    let mut framed = b"gwz/v1/authority-payload\0".to_vec();
    frame_value(&value, &mut framed)?;
    Ok(Sha256::digest(framed).into())
}

fn frame_value(value: &Value, out: &mut Vec<u8>) -> ModelResult<()> {
    match value {
        Value::Null => out.push(0),
        Value::Bool(value) => out.extend([1, u8::from(*value)]),
        Value::Number(value) => frame_segment(2, value.to_string().as_bytes(), out),
        Value::String(value) => frame_segment(3, value.as_bytes(), out),
        Value::Sequence(values) => {
            out.push(4);
            out.extend((values.len() as u64).to_be_bytes());
            for value in values {
                let mut framed = Vec::new();
                frame_value(value, &mut framed)?;
                frame_segment(0, &framed, out);
            }
        }
        Value::Mapping(values) => {
            let mut entries = Vec::with_capacity(values.len());
            for (key, value) in values {
                let (mut framed_key, mut framed_value) = (Vec::new(), Vec::new());
                frame_value(key, &mut framed_key)?;
                frame_value(value, &mut framed_value)?;
                entries.push((framed_key, framed_value));
            }
            entries.sort();
            out.push(5);
            out.extend((entries.len() as u64).to_be_bytes());
            for (key, value) in entries {
                frame_segment(0, &key, out);
                frame_segment(0, &value, out);
            }
        }
        Value::Tagged(value) => {
            frame_segment(6, value.tag.to_string().as_bytes(), out);
            frame_value(&value.value, out)?;
        }
    }
    Ok(())
}

fn frame_segment(domain: u8, bytes: &[u8], out: &mut Vec<u8>) {
    out.push(domain);
    out.extend((bytes.len() as u64).to_be_bytes());
    out.extend(bytes);
}

fn binding_error(error: impl std::fmt::Display) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("cannot canonically bind v1 transition payload: {error}"),
    )
}
