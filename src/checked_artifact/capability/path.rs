//! Component-wise, identity-bound canonical path records.

use super::{CheckedFsError, DurableObjectIdentityV1, PlatformCapability};
use crate::checked_artifact::protocol::generated;

pub(in crate::checked_artifact) const MAX_PATH_COMPONENT_BYTES: usize = 255;
pub(in crate::checked_artifact) const MAX_CANONICAL_PATH_IDENTITY_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::checked_artifact) struct AsciiComponent(Vec<u8>);

impl AsciiComponent {
    pub(in crate::checked_artifact) fn parse(value: &[u8]) -> Result<Self, CheckedFsError> {
        if value.is_empty()
            || value.len() > MAX_PATH_COMPONENT_BYTES
            || matches!(value, b"." | b"..")
            || !value.is_ascii()
            || value.contains(&0)
            || value.contains(&b'/')
            || value.contains(&b'\\')
        {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::AsciiProtocolPath,
                "path component must be nonempty normal ASCII and at most 255 bytes",
            ));
        }
        Ok(Self(value.to_vec()))
    }

    pub(in crate::checked_artifact) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::checked_artifact) enum PathComponentMode {
    Sensitive,
    AsciiCaseFold,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CanonicalComponent {
    original_ascii: AsciiComponent,
    parent_mode: PathComponentMode,
    canonical_ascii: Vec<u8>,
    parent_durable_identity: DurableObjectIdentityV1,
    parent_invocation_identity: Vec<u8>,
    rename_domain: Vec<u8>,
}

impl CanonicalComponent {
    pub(in crate::checked_artifact) fn try_bound(
        original_ascii: AsciiComponent,
        parent_mode: PathComponentMode,
        parent_durable_identity: DurableObjectIdentityV1,
        parent_invocation_identity: Vec<u8>,
        rename_domain: Vec<u8>,
    ) -> Result<Self, CheckedFsError> {
        if parent_invocation_identity.is_empty()
            || parent_invocation_identity.len() > 256
            || rename_domain.is_empty()
            || rename_domain.len() > 256
        {
            return Err(CheckedFsError::ambiguous(
                "canonical path identity",
                "parent invocation identity and rename domain must each be 1..=256 bytes",
            ));
        }
        let canonical_ascii = match parent_mode {
            PathComponentMode::Sensitive => original_ascii.as_bytes().to_vec(),
            PathComponentMode::AsciiCaseFold => original_ascii
                .as_bytes()
                .iter()
                .map(u8::to_ascii_lowercase)
                .collect(),
        };
        Ok(Self {
            original_ascii,
            parent_mode,
            canonical_ascii,
            parent_durable_identity,
            parent_invocation_identity,
            rename_domain,
        })
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn new(
        original_ascii: AsciiComponent,
        parent_mode: PathComponentMode,
    ) -> Self {
        Self::try_bound(
            original_ascii,
            parent_mode,
            DurableObjectIdentityV1::linux_ext4([1; 16], 1, vec![1]).unwrap(),
            vec![1],
            vec![1],
        )
        .unwrap()
    }

    pub(in crate::checked_artifact) fn original(&self) -> &AsciiComponent {
        &self.original_ascii
    }

    pub(in crate::checked_artifact) fn parent_mode(&self) -> PathComponentMode {
        self.parent_mode
    }

    pub(in crate::checked_artifact) fn canonical_ascii(&self) -> &[u8] {
        &self.canonical_ascii
    }

    pub(in crate::checked_artifact) fn parent_durable_identity(&self) -> &DurableObjectIdentityV1 {
        &self.parent_durable_identity
    }

    pub(in crate::checked_artifact) fn parent_invocation_identity(&self) -> &[u8] {
        &self.parent_invocation_identity
    }

    pub(in crate::checked_artifact) fn rename_domain(&self) -> &[u8] {
        &self.rename_domain
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CanonicalPathIdentityV1 {
    components: Vec<CanonicalComponent>,
}

impl CanonicalPathIdentityV1 {
    pub(in crate::checked_artifact) fn new(
        components: Vec<CanonicalComponent>,
    ) -> Result<Self, CheckedFsError> {
        if components.is_empty() {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PathEquivalence,
                "canonical path identity is empty",
            ));
        }
        let value = Self { components };
        if value.encode_canonical().len() > MAX_CANONICAL_PATH_IDENTITY_BYTES {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PathEquivalence,
                "canonical path identity exceeds 4 KiB",
            ));
        }
        Ok(value)
    }

    pub(in crate::checked_artifact) fn components(&self) -> &[CanonicalComponent] {
        &self.components
    }

    pub(in crate::checked_artifact) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn decode_canonical(bytes: &[u8]) -> Result<Self, CheckedFsError> {
        let cbor = crate::cbor::try_decode(bytes).map_err(|_| {
            CheckedFsError::ambiguous("canonical path identity", "invalid taut encoding")
        })?;
        let wire = generated::CheckedCanonicalPathIdentityV1::from_cbor(&cbor).map_err(|_| {
            CheckedFsError::ambiguous("canonical path identity", "invalid taut record shape")
        })?;
        let mut components = Vec::new();
        components
            .try_reserve_exact(wire.components.len())
            .map_err(|_| {
                CheckedFsError::unsupported(
                    PlatformCapability::PathEquivalence,
                    "canonical path component allocation failed",
                )
            })?;
        for value in wire.components {
            let mode = match value.parent_mode {
                generated::CheckedPathComponentMode::Sensitive => PathComponentMode::Sensitive,
                generated::CheckedPathComponentMode::AsciiCaseFold => {
                    PathComponentMode::AsciiCaseFold
                }
            };
            let original = AsciiComponent::parse(&value.original_ascii)?;
            let parent_identity = DurableObjectIdentityV1::decode_canonical(&crate::cbor::encode(
                &value.parent_durable_identity.to_cbor(),
            ))?;
            let component = CanonicalComponent::try_bound(
                original,
                mode,
                parent_identity,
                value.parent_invocation_identity,
                value.rename_domain,
            )?;
            if value.canonical_ascii != component.canonical_ascii() {
                return Err(CheckedFsError::ambiguous(
                    "canonical path identity",
                    "stored component is not canonical",
                ));
            }
            components.push(component);
        }
        let value = Self::new(components)?;
        if value.encode_canonical() != bytes {
            return Err(CheckedFsError::ambiguous(
                "canonical path identity",
                "noncanonical encoding",
            ));
        }
        Ok(value)
    }

    pub(in crate::checked_artifact) fn to_generated(
        &self,
    ) -> generated::CheckedCanonicalPathIdentityV1 {
        generated::CheckedCanonicalPathIdentityV1 {
            components: self
                .components
                .iter()
                .map(|component| generated::CheckedCanonicalComponentV1 {
                    original_ascii: component.original_ascii.as_bytes().to_vec(),
                    parent_mode: match component.parent_mode {
                        PathComponentMode::Sensitive => {
                            generated::CheckedPathComponentMode::Sensitive
                        }
                        PathComponentMode::AsciiCaseFold => {
                            generated::CheckedPathComponentMode::AsciiCaseFold
                        }
                    },
                    canonical_ascii: component.canonical_ascii.clone(),
                    parent_durable_identity: component.parent_durable_identity.to_generated(),
                    parent_invocation_identity: component.parent_invocation_identity.clone(),
                    rename_domain: component.rename_domain.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
impl CanonicalPathIdentityV1 {
    pub(in crate::checked_artifact) fn decode_canonical_for_test(
        bytes: &[u8],
    ) -> Result<Self, CheckedFsError> {
        Self::decode_canonical(bytes)
    }
}
