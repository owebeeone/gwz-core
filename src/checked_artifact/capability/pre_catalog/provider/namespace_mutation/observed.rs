use crate::checked_artifact::capability::DurableObjectIdentityV1;

/// One exact regular-file namespace object, observed and closed.
///
/// The bytes and the encoded identity are the primitive's own source-association
/// expectation. The observation handle is dropped before it is returned: on
/// Windows the primitive reopens the source with `DELETE` access, which a
/// surviving caller handle opened without `FILE_SHARE_DELETE` would refuse, and
/// the sealed primitive re-establishes identity through its own capability
/// anyway (`admission_mutation.rs:269-275`, the `publish_final_directory`
/// precedent it cites).
pub(in crate::checked_artifact) struct ObservedNamespaceObjectV1 {
    pub(crate) identity: DurableObjectIdentityV1,
    pub(crate) encoded_identity: Vec<u8>,
    pub(crate) bytes: Vec<u8>,
}

impl ObservedNamespaceObjectV1 {
    pub(in crate::checked_artifact) const fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }
}
