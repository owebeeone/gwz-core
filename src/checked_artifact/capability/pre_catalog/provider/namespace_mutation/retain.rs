use super::*;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableIdentityProvider, DurableObjectIdentityV1, PathEquivalenceProvider,
};
use crate::filesystem::FsDirectory as Dir;
use std::ffi::OsString;

/// The one retained action directory a namespace backend owns for its whole
/// life: opened once through an identity-proved no-follow hop from the
/// permit-retained completed catalog, and held across every observation,
/// publication, retirement, barrier and revalidation.
pub(in crate::checked_artifact) struct RetainedActionNamespaceV1 {
    pub(crate) parent: Dir,
    pub(crate) leaf: OsString,
    pub(crate) handle: Dir,
    pub(crate) identity: DurableObjectIdentityV1,
    pub(crate) path_profile: CanonicalPathIdentityV1,
    pub(crate) reservation: RecordDigestV1,
}

/// Retains the deterministic final action directory of an admitted action.
///
/// `final_directory` is the permit's own retained completed-catalog capability,
/// so this is the single no-follow hop the audit's provenance rule allows.
pub(in crate::checked_artifact::capability::pre_catalog::provider) fn retain_action_namespace(
    final_directory: &Dir,
    action_leaf: &str,
    expected_identity: &DurableObjectIdentityV1,
    reservation: RecordDigestV1,
) -> Result<RetainedActionNamespaceV1, CheckedFsError> {
    let leaf = OsString::from(action_leaf);
    let handle = final_directory
        .retained_child(&leaf)
        .map_err(|source| CheckedFsError::io("open admitted action directory", source))?;
    let fact = super::super::super::HostPlatform.dir_identity(&handle)?;
    if fact.durable() != expected_identity {
        return Err(CheckedFsError::ambiguous(
            "action namespace",
            "admitted action directory identity changed",
        ));
    }
    let parent_fact = super::super::super::HostPlatform.dir_identity(final_directory)?;
    let path_profile = CanonicalPathIdentityV1::new(vec![CanonicalComponent::try_bound(
        AsciiComponent::parse(action_leaf.as_bytes())?,
        super::super::HostPlatform.parent_mode(final_directory)?,
        parent_fact.durable().clone(),
        parent_fact.invocation().clone(),
        super::super::HostPlatform.rename_domain(final_directory)?,
    )?])?;
    let parent = final_directory.clone_handle().map_err(|source| {
        CheckedFsError::io("retain completed catalog for revalidation", source)
    })?;
    Ok(RetainedActionNamespaceV1 {
        parent,
        leaf,
        handle,
        identity: fact.durable().clone(),
        path_profile,
        reservation,
    })
}
