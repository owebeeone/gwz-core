//! Restart-stable durable path descriptors.

use super::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
    MAX_CANONICAL_PATH_IDENTITY_BYTES, PathComponentMode, PlatformCapability,
};
use crate::checked_artifact::protocol::generated;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct DurablePathComponentV1 {
    original_ascii: AsciiComponent,
    parent_mode: PathComponentMode,
    canonical_ascii: Vec<u8>,
    parent_durable_identity: DurableObjectIdentityV1,
}

impl DurablePathComponentV1 {
    fn from_live(value: &super::CanonicalComponent) -> Self {
        Self {
            original_ascii: value.original().clone(),
            parent_mode: value.parent_mode(),
            canonical_ascii: value.canonical_ascii().to_vec(),
            parent_durable_identity: value.parent_durable_identity().clone(),
        }
    }

    fn try_from_generated(
        value: generated::CheckedDurablePathComponentV1,
    ) -> Result<Self, CheckedFsError> {
        let original_ascii = AsciiComponent::parse(&value.original_ascii)?;
        let parent_mode = match value.parent_mode {
            generated::CheckedPathComponentMode::Sensitive => PathComponentMode::Sensitive,
            generated::CheckedPathComponentMode::AsciiCaseFold => PathComponentMode::AsciiCaseFold,
        };
        let expected_canonical = match parent_mode {
            PathComponentMode::Sensitive => original_ascii.as_bytes().to_vec(),
            PathComponentMode::AsciiCaseFold => original_ascii
                .as_bytes()
                .iter()
                .map(u8::to_ascii_lowercase)
                .collect(),
        };
        if value.canonical_ascii != expected_canonical {
            return Err(CheckedFsError::ambiguous(
                "durable path",
                "stored component is not canonical for its parent mode",
            ));
        }
        let parent_durable_identity = DurableObjectIdentityV1::decode_canonical(
            &crate::cbor::encode(&value.parent_durable_identity.to_cbor()),
        )?;
        Ok(Self {
            original_ascii,
            parent_mode,
            canonical_ascii: expected_canonical,
            parent_durable_identity,
        })
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

    fn to_generated(&self) -> generated::CheckedDurablePathComponentV1 {
        generated::CheckedDurablePathComponentV1 {
            original_ascii: self.original_ascii.as_bytes().to_vec(),
            parent_mode: match self.parent_mode {
                PathComponentMode::Sensitive => generated::CheckedPathComponentMode::Sensitive,
                PathComponentMode::AsciiCaseFold => {
                    generated::CheckedPathComponentMode::AsciiCaseFold
                }
            },
            canonical_ascii: self.canonical_ascii.clone(),
            parent_durable_identity: self.parent_durable_identity.to_generated(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct DurablePathV1 {
    components: Vec<DurablePathComponentV1>,
}

impl DurablePathV1 {
    pub(in crate::checked_artifact) fn from_live(
        value: &CanonicalPathIdentityV1,
    ) -> Result<Self, CheckedFsError> {
        Self::try_new(
            value
                .components()
                .iter()
                .map(DurablePathComponentV1::from_live)
                .collect(),
        )
    }

    fn try_new(components: Vec<DurablePathComponentV1>) -> Result<Self, CheckedFsError> {
        if components.is_empty() {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PathEquivalence,
                "durable path is empty",
            ));
        }
        let support_profile = components[0].parent_durable_identity.support_profile();
        if components
            .iter()
            .any(|component| component.parent_durable_identity.support_profile() != support_profile)
        {
            return Err(CheckedFsError::ambiguous(
                "durable path",
                "component filesystem support profiles do not match",
            ));
        }
        let value = Self { components };
        if value.encode_canonical().len() > MAX_CANONICAL_PATH_IDENTITY_BYTES {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PathEquivalence,
                "durable path exceeds 4 KiB",
            ));
        }
        Ok(value)
    }

    pub(in crate::checked_artifact) fn components(&self) -> &[DurablePathComponentV1] {
        &self.components
    }

    pub(in crate::checked_artifact) fn encode_canonical(&self) -> Vec<u8> {
        crate::cbor::encode(&self.to_generated().to_cbor())
    }

    pub(in crate::checked_artifact) fn decode_canonical(
        bytes: &[u8],
    ) -> Result<Self, CheckedFsError> {
        let cbor = crate::cbor::try_decode(bytes)
            .map_err(|_| CheckedFsError::ambiguous("durable path", "invalid taut encoding"))?;
        let wire = generated::CheckedDurablePathV1::from_cbor(&cbor)
            .map_err(|_| CheckedFsError::ambiguous("durable path", "invalid taut record shape"))?;
        let mut components = Vec::new();
        components
            .try_reserve_exact(wire.components.len())
            .map_err(|_| {
                CheckedFsError::unsupported(
                    PlatformCapability::PathEquivalence,
                    "durable path component allocation failed",
                )
            })?;
        for component in wire.components {
            components.push(DurablePathComponentV1::try_from_generated(component)?);
        }
        let value = Self::try_new(components)?;
        if value.encode_canonical() != bytes {
            return Err(CheckedFsError::ambiguous(
                "durable path",
                "noncanonical encoding",
            ));
        }
        Ok(value)
    }

    pub(in crate::checked_artifact) fn to_generated(&self) -> generated::CheckedDurablePathV1 {
        generated::CheckedDurablePathV1 {
            components: self
                .components
                .iter()
                .map(DurablePathComponentV1::to_generated)
                .collect(),
        }
    }

    #[cfg(test)]
    pub(in crate::checked_artifact) fn to_live_for_test(&self) -> CanonicalPathIdentityV1 {
        CanonicalPathIdentityV1::new(
            self.components
                .iter()
                .map(|component| {
                    super::CanonicalComponent::try_bound(
                        component.original_ascii.clone(),
                        component.parent_mode,
                        component.parent_durable_identity.clone(),
                        vec![0xa5; 16],
                        vec![0x5a; 16],
                    )
                    .expect("durable component is valid live test material")
                })
                .collect(),
        )
        .expect("durable path is valid live test material")
    }
}
