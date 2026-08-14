//! Component-wise, identity-bound live path observations.

use super::{CheckedFsError, DurableObjectIdentityV1, PlatformCapability};

mod durable;

pub(in crate::checked_artifact) use durable::*;

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
        DurablePathV1::from_live(&value)?;
        Ok(value)
    }

    pub(in crate::checked_artifact) fn components(&self) -> &[CanonicalComponent] {
        &self.components
    }

    /// Deterministic same-process digest material. This includes invocation
    /// identities and rename domains but is never a durable wire record.
    pub(in crate::checked_artifact) fn fresh_digest_material(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        frame(&mut bytes, b"gwz-live-canonical-path-v1\0");
        frame(&mut bytes, &(self.components.len() as u64).to_be_bytes());
        for component in &self.components {
            frame(&mut bytes, component.original_ascii.as_bytes());
            frame(
                &mut bytes,
                &[match component.parent_mode {
                    PathComponentMode::Sensitive => 0,
                    PathComponentMode::AsciiCaseFold => 1,
                }],
            );
            frame(&mut bytes, &component.canonical_ascii);
            frame(
                &mut bytes,
                &component.parent_durable_identity.encode_canonical(),
            );
            frame(&mut bytes, &component.parent_invocation_identity);
            frame(&mut bytes, &component.rename_domain);
        }
        bytes
    }
}

fn frame(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value);
}
