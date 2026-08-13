//! One retained-root authority for capability, path, and collision preflight.

use std::io::{self, Cursor};

use super::{
    CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1, LosslessIndexEntry,
    PrivateControlDomain, SupportedFilesystemProfile, TrackedWorktreeEntry,
};
use crate::checked_artifact::protocol::generated;
use crate::checked_artifact::protocol::{ProtocolCodecErrorV1, ProtocolRecordKindV1};

mod provider;

#[cfg(test)]
pub(in crate::checked_artifact) use provider::{
    SyntheticPreCatalogProbeV1, synthetic_pre_catalog_owner,
};

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

/// A one-shot view created only after the capability owner has re-observed the
/// retained root and every retained path component. Its borrow prevents it
/// from surviving the owner transaction that performs catalog bootstrap.
pub(in crate::checked_artifact) struct RevalidatedPreCatalogPermitV1<'permit, RetainedRoot> {
    permit: &'permit PreCatalogPermitV1<RetainedRoot>,
}

impl<'permit, RetainedRoot> RevalidatedPreCatalogPermitV1<'permit, RetainedRoot> {
    fn new(permit: &'permit PreCatalogPermitV1<RetainedRoot>) -> Self {
        Self { permit }
    }

    pub(in crate::checked_artifact) fn support_profile(&self) -> SupportedFilesystemProfile {
        self.permit.support_profile()
    }

    pub(in crate::checked_artifact) fn root_identity(&self) -> &DurableObjectIdentityV1 {
        self.permit.root_identity()
    }

    pub(in crate::checked_artifact) fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        self.permit.path_profile()
    }

    pub(in crate::checked_artifact) fn retained_root(&self) -> &RetainedRoot {
        self.permit.retained_root()
    }

    pub(in crate::checked_artifact) fn root_invocation_identity(&self) -> &[u8] {
        self.permit.root_invocation_identity()
    }

    pub(in crate::checked_artifact) fn rename_domain(&self) -> &[u8] {
        self.permit.rename_domain()
    }

    pub(in crate::checked_artifact) const fn collision_domain_digest(&self) -> [u8; 32] {
        self.permit.collision_domain_digest()
    }

    pub(in crate::checked_artifact) const fn lease_binding(&self) -> [u8; 32] {
        self.permit.lease_binding()
    }

    pub(in crate::checked_artifact) const fn root_kind(&self) -> PreCatalogRootKindV1 {
        self.permit.root_kind()
    }
}

/// The only checked-artifact entry point that can turn raw pre-catalog
/// observations into catalog mutations. Construction is owner-private; the
/// raw provider trait is private to this module subtree.
pub(in crate::checked_artifact) struct PreCatalogOwnerV1<Root: ?Sized, RetainedRoot> {
    provider: Box<dyn provider::RawPreCatalogProviderV1<Root, RetainedRoot>>,
}

impl<Root: ?Sized, RetainedRoot> PreCatalogOwnerV1<Root, RetainedRoot> {
    fn from_provider(
        provider: impl provider::RawPreCatalogProviderV1<Root, RetainedRoot> + 'static,
    ) -> Self {
        Self {
            provider: Box::new(provider),
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "one owner call binds every pre-catalog input to bootstrap"
    )]
    pub(in crate::checked_artifact) fn recover_or_create<Bootstrap>(
        &self,
        root: &Root,
        root_kind: PreCatalogRootKindV1,
        lease_binding: [u8; 32],
        domain: &PrivateControlDomain,
        index: &[LosslessIndexEntry],
        worktree: &[TrackedWorktreeEntry],
        bootstrap: &Bootstrap,
    ) -> Result<Bootstrap::Catalog, CheckedFsError>
    where
        Bootstrap: crate::checked_artifact::bootstrap::CatalogBootstrapV1<RetainedRoot>,
    {
        let observation = self
            .provider
            .inspect_and_scan(root, root_kind, domain, index, worktree)?;
        let permit = PreCatalogPermitV1::try_new(
            observation.retained_root,
            observation.support_profile,
            observation.root_identity,
            observation.root_invocation_identity,
            observation.rename_domain,
            observation.path_profile,
            domain.version_digest(),
            lease_binding,
            root_kind,
        )?;

        // Keep these calls adjacent: this is the mutation boundary. There is
        // deliberately no callback or returned permit on which a caller could
        // interpose unrelated work after revalidation.
        self.provider.revalidate(root, &permit)?;
        bootstrap.recover_or_create(RevalidatedPreCatalogPermitV1::new(&permit))
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
