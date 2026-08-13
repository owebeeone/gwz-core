//! One retained-root authority for capability, path, and collision preflight.

use std::io::{self, Cursor};

use super::{
    CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1, LosslessIndexEntry,
    PrivateControlDomain, SupportedFilesystemProfile, TrackedWorktreeEntry,
};
use crate::checked_artifact::protocol::generated;
use crate::checked_artifact::protocol::{ProtocolCodecErrorV1, ProtocolRecordKindV1};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum PreCatalogRootKindV1 {
    Workspace,
    GitDirectory,
}

/// One non-composable capability issued by one retained-root transaction.
pub(in crate::checked_artifact) struct PreCatalogPermitV1<RetainedRoot> {
    retained_root: RetainedRoot,
    support_profile: SupportedFilesystemProfile,
    root_identity: DurableObjectIdentityV1,
    root_invocation_identity: Vec<u8>,
    rename_domain: Vec<u8>,
    path_profile: CanonicalPathIdentityV1,
    collision_domain_digest: [u8; 32],
    lease_binding: [u8; 32],
    root_kind: PreCatalogRootKindV1,
}

impl<RetainedRoot> PreCatalogPermitV1<RetainedRoot> {
    #[allow(
        clippy::too_many_arguments,
        reason = "one transaction binds every pre-catalog fact"
    )]
    fn try_new(
        retained_root: RetainedRoot,
        support_profile: SupportedFilesystemProfile,
        root_identity: DurableObjectIdentityV1,
        root_invocation_identity: Vec<u8>,
        rename_domain: Vec<u8>,
        path_profile: CanonicalPathIdentityV1,
        collision_domain_digest: [u8; 32],
        lease_binding: [u8; 32],
        root_kind: PreCatalogRootKindV1,
    ) -> Result<Self, CheckedFsError> {
        if root_identity.support_profile() != support_profile {
            return Err(CheckedFsError::ambiguous(
                "filesystem support profile",
                "durable root identity does not match the claimed profile",
            ));
        }
        if root_invocation_identity.is_empty() || rename_domain.is_empty() {
            return Err(CheckedFsError::ambiguous(
                "retained root",
                "root invocation identity and rename domain must be nonempty",
            ));
        }
        let first = path_profile
            .components()
            .first()
            .ok_or_else(|| CheckedFsError::ambiguous("retained path", "path walk is empty"))?;
        if first.parent_durable_identity() != &root_identity
            || first.parent_invocation_identity() != root_invocation_identity
            || first.rename_domain() != rename_domain
            || path_profile
                .components()
                .iter()
                .any(|part| part.parent_durable_identity().support_profile() != support_profile)
        {
            return Err(CheckedFsError::ambiguous(
                "retained path",
                "path walk is not bound to the retained root and support profile",
            ));
        }
        Ok(Self {
            retained_root,
            support_profile,
            root_identity,
            root_invocation_identity,
            rename_domain,
            path_profile,
            collision_domain_digest,
            lease_binding,
            root_kind,
        })
    }

    pub(in crate::checked_artifact) fn support_profile(&self) -> SupportedFilesystemProfile {
        self.support_profile
    }

    pub(in crate::checked_artifact) fn root_identity(&self) -> &DurableObjectIdentityV1 {
        &self.root_identity
    }

    pub(in crate::checked_artifact) fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        &self.path_profile
    }

    pub(in crate::checked_artifact) fn retained_root(&self) -> &RetainedRoot {
        &self.retained_root
    }

    pub(in crate::checked_artifact) fn root_invocation_identity(&self) -> &[u8] {
        &self.root_invocation_identity
    }

    pub(in crate::checked_artifact) fn rename_domain(&self) -> &[u8] {
        &self.rename_domain
    }

    pub(in crate::checked_artifact) const fn collision_domain_digest(&self) -> [u8; 32] {
        self.collision_domain_digest
    }

    pub(in crate::checked_artifact) const fn lease_binding(&self) -> [u8; 32] {
        self.lease_binding
    }

    pub(in crate::checked_artifact) const fn root_kind(&self) -> PreCatalogRootKindV1 {
        self.root_kind
    }
}

#[cfg(test)]
#[allow(
    clippy::too_many_arguments,
    reason = "test setup mirrors the sealed transaction fields"
)]
pub(in crate::checked_artifact) fn synthetic_pre_catalog_permit<RetainedRoot>(
    retained_root: RetainedRoot,
    support_profile: SupportedFilesystemProfile,
    root_identity: DurableObjectIdentityV1,
    root_invocation_identity: Vec<u8>,
    rename_domain: Vec<u8>,
    path_profile: CanonicalPathIdentityV1,
    collision_domain_digest: [u8; 32],
    lease_binding: [u8; 32],
    root_kind: PreCatalogRootKindV1,
) -> Result<PreCatalogPermitV1<RetainedRoot>, CheckedFsError> {
    PreCatalogPermitV1::try_new(
        retained_root,
        support_profile,
        root_identity,
        root_invocation_identity,
        rename_domain,
        path_profile,
        collision_domain_digest,
        lease_binding,
        root_kind,
    )
}

/// The provider observes capability, retained path, and the complete collision
/// domain in one transaction. No separately issued proof can be combined.
pub(in crate::checked_artifact) trait PreCatalogPreflightV1<Root: ?Sized> {
    type RetainedRoot;

    #[allow(
        clippy::type_complexity,
        reason = "the tuple is private input to the sealing default"
    )]
    fn inspect_and_scan(
        &self,
        root: &Root,
        root_kind: PreCatalogRootKindV1,
        domain: &PrivateControlDomain,
        index: &[LosslessIndexEntry],
        worktree: &[TrackedWorktreeEntry],
    ) -> Result<
        (
            Self::RetainedRoot,
            SupportedFilesystemProfile,
            DurableObjectIdentityV1,
            Vec<u8>,
            Vec<u8>,
            CanonicalPathIdentityV1,
        ),
        CheckedFsError,
    >;

    fn preflight(
        &self,
        root: &Root,
        root_kind: PreCatalogRootKindV1,
        lease_binding: [u8; 32],
        domain: &PrivateControlDomain,
        index: &[LosslessIndexEntry],
        worktree: &[TrackedWorktreeEntry],
    ) -> Result<PreCatalogPermitV1<Self::RetainedRoot>, CheckedFsError> {
        let (
            retained_root,
            support_profile,
            root_identity,
            invocation_identity,
            rename_domain,
            path_profile,
        ) = self.inspect_and_scan(root, root_kind, domain, index, worktree)?;
        PreCatalogPermitV1::try_new(
            retained_root,
            support_profile,
            root_identity,
            invocation_identity,
            rename_domain,
            path_profile,
            domain.version_digest(),
            lease_binding,
            root_kind,
        )
    }

    fn revalidate(
        &self,
        root: &Root,
        permit: &PreCatalogPermitV1<Self::RetainedRoot>,
    ) -> Result<(), CheckedFsError>;
}

pub(in crate::checked_artifact) fn read_canonical_path_identity(
    reader: impl io::Read,
) -> Result<CanonicalPathIdentityV1, ProtocolCodecErrorV1> {
    crate::checked_artifact::protocol::read_bounded_value(
        ProtocolRecordKindV1::CanonicalPathIdentity,
        reader,
        |bytes| {
            CanonicalPathIdentityV1::decode_canonical(bytes)
                .map_err(|_| ProtocolCodecErrorV1::Invalid("invalid canonical path identity"))
        },
        CanonicalPathIdentityV1::encode_canonical,
    )
}

pub(in crate::checked_artifact) fn decode_canonical_path_value(
    value: generated::CheckedCanonicalPathIdentityV1,
) -> Result<CanonicalPathIdentityV1, ProtocolCodecErrorV1> {
    read_canonical_path_identity(Cursor::new(crate::cbor::encode(&value.to_cbor())))
}
