//! Versioned owner and action identity derivation.

use sha2::{Digest, Sha256};

use crate::checked_artifact::bootstrap::{
    ManagedParentAuthorityClassV1, ManagedParentBootstrapRequest, ManagedParentPurpose,
    ValidatedArchiveSourceV1,
};
use crate::checked_artifact::capability::{
    AsciiComponent, CheckedFsError, MAX_CANONICAL_PATH_IDENTITY_BYTES, PreCatalogRootKindV1,
};
use crate::checked_artifact::protocol::{ActionDigestV1, RequestOwnerBindingV1};
use crate::workspace_ops::{
    CheckedArchiveSourceObservation, CheckedOwnerRecordObservation, CheckedOwnerRecordVersion,
};

const OWNER_DOMAIN: &[u8] = b"gwz-checked-owner-v1\0";
const ACTION_DOMAIN: &[u8] = b"gwz-checked-action-v1\0";
const ENCODING_VERSION: u8 = 1;
const MAX_ID_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnerVariantV1 {
    ManagedParents = 0,
    MergeRecordV0 = 1,
    MergeRecordV1 = 2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OwnerFieldsV1 {
    ManagedParents {
        workspace_id: String,
    },
    MergeRecord {
        workspace_id: String,
        merge_id: String,
        operation_id: String,
        source_record_sha256: [u8; 32],
    },
}

/// Private durable identity from which a request-owner binding is derived.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CheckedActionOwnerV1 {
    variant: OwnerVariantV1,
    fields: OwnerFieldsV1,
}

impl CheckedActionOwnerV1 {
    pub(in crate::checked_artifact) fn for_merge_start(
        workspace_id: &str,
    ) -> Result<Self, CheckedFsError> {
        validate_prefixed_id("workspace ID", "ws_", workspace_id)?;
        Ok(Self {
            variant: OwnerVariantV1::ManagedParents,
            fields: OwnerFieldsV1::ManagedParents {
                workspace_id: workspace_id.to_owned(),
            },
        })
    }

    fn for_merge_record(
        variant: OwnerVariantV1,
        workspace_id: &str,
        merge_id: &str,
        operation_id: &str,
        source_record_bytes: &[u8],
    ) -> Result<Self, CheckedFsError> {
        validate_prefixed_id("workspace ID", "ws_", workspace_id)?;
        validate_slug_id("merge ID", merge_id)?;
        validate_prefixed_id("operation ID", "op_", operation_id)?;
        if source_record_bytes.is_empty() {
            return Err(identity_error("durable source record bytes are empty"));
        }
        Ok(Self {
            variant,
            fields: OwnerFieldsV1::MergeRecord {
                workspace_id: workspace_id.to_owned(),
                merge_id: merge_id.to_owned(),
                operation_id: operation_id.to_owned(),
                source_record_sha256: Sha256::digest(source_record_bytes).into(),
            },
        })
    }

    fn from_record_observation(
        observation: &CheckedOwnerRecordObservation<'_>,
    ) -> Result<Self, CheckedFsError> {
        let variant = match observation.version() {
            CheckedOwnerRecordVersion::V0 => OwnerVariantV1::MergeRecordV0,
            CheckedOwnerRecordVersion::V1 => OwnerVariantV1::MergeRecordV1,
        };
        Self::for_merge_record(
            variant,
            observation.workspace_id(),
            observation.merge_id(),
            observation.operation_id(),
            observation.exact_bytes(),
        )
    }

    fn source_record_sha256(&self) -> Option<[u8; 32]> {
        match self.fields {
            OwnerFieldsV1::ManagedParents { .. } => None,
            OwnerFieldsV1::MergeRecord {
                source_record_sha256,
                ..
            } => Some(source_record_sha256),
        }
    }

    fn permits_managed_request(&self, request: &ManagedParentBootstrapRequest) -> bool {
        match (self.variant, request.authority_class()) {
            (OwnerVariantV1::ManagedParents, ManagedParentAuthorityClassV1::MergeStart) => true,
            (
                OwnerVariantV1::MergeRecordV0 | OwnerVariantV1::MergeRecordV1,
                ManagedParentAuthorityClassV1::DurableMerge,
            ) => true,
            (
                OwnerVariantV1::MergeRecordV0 | OwnerVariantV1::MergeRecordV1,
                ManagedParentAuthorityClassV1::Archive,
            ) => request.archive_prerequisite().is_some_and(|prerequisite| {
                prerequisite.owner_binding() == self.request_owner_binding()
                    && Some(prerequisite.source_record_sha256()) == self.source_record_sha256()
            }),
            #[cfg(test)]
            (_, ManagedParentAuthorityClassV1::Unrestricted) => true,
            _ => false,
        }
    }

    pub(in crate::checked_artifact) fn request_owner_binding(&self) -> RequestOwnerBindingV1 {
        let mut sink = HashSink::new();
        self.write_canonical(&mut sink);
        RequestOwnerBindingV1::new(sink.finish())
    }

    #[cfg(test)]
    fn canonical_preimage(&self) -> Vec<u8> {
        let mut value = Vec::new();
        self.write_canonical(&mut value);
        value
    }

    fn write_canonical(&self, sink: &mut impl CanonicalSink) {
        match &self.fields {
            OwnerFieldsV1::ManagedParents { workspace_id } => {
                write_header(sink, OWNER_DOMAIN, self.variant as u8, 2);
                write_field(sink, 0, workspace_id.as_bytes());
                write_field(sink, 1, &[0]); // MergeStart
            }
            OwnerFieldsV1::MergeRecord {
                workspace_id,
                merge_id,
                operation_id,
                source_record_sha256,
            } => {
                write_header(sink, OWNER_DOMAIN, self.variant as u8, 4);
                write_field(sink, 0, workspace_id.as_bytes());
                write_field(sink, 1, merge_id.as_bytes());
                write_field(sink, 2, operation_id.as_bytes());
                write_field(sink, 3, source_record_sha256);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CheckedActionOperationV1 {
    Observe = 0,
    Replace = 1,
    Remove = 2,
    ParentOnly = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CheckedLeafFactV1 {
    Missing,
    Exact { length: u64, sha256: [u8; 32] },
}

impl CheckedLeafFactV1 {
    const fn encoded_len(self) -> usize {
        match self {
            Self::Missing => 1,
            Self::Exact { .. } => 41,
        }
    }

    fn write_canonical(self, sink: &mut impl CanonicalSink) {
        match self {
            Self::Missing => sink.write(&[0]),
            Self::Exact { length, sha256 } => {
                sink.write(&[1]);
                sink.write(&length.to_be_bytes());
                sink.write(&sha256);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CheckedPurposeSetV1(u8);

impl CheckedPurposeSetV1 {
    const VALID_MASK: u8 = 0b1111;

    fn from_request(request: &ManagedParentBootstrapRequest) -> Self {
        let mask = request.specs().iter().fold(0, |mask, spec| {
            mask | match spec.purpose() {
                ManagedParentPurpose::MergeStore => 1,
                ManagedParentPurpose::MergeArchive => 2,
                ManagedParentPurpose::PreservationBundles => 4,
                ManagedParentPurpose::RootPreservationMarkers => 8,
            }
        });
        Self(mask)
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(in crate::checked_artifact) const fn mask(self) -> u8 {
        self.0
    }
}

/// Fully validated request material. Its fields are private so the coordinator
/// remains the only source of catalog identity and schedule inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CheckedActionRequestV1 {
    owner_binding: RequestOwnerBindingV1,
    operation: CheckedActionOperationV1,
    root_kind: PreCatalogRootKindV1,
    path: Option<Vec<AsciiComponent>>,
    expected: CheckedLeafFactV1,
    goal: CheckedLeafFactV1,
    purposes: CheckedPurposeSetV1,
}

/// Sealed owner, action identity, and managed-purpose request. Production
/// managed preflight accepts only this tuple, never independent digests or
/// purpose lists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct CheckedManagedActionV1 {
    checked: CheckedActionRequestV1,
    managed: ManagedParentBootstrapRequest,
}

impl CheckedManagedActionV1 {
    pub(in crate::checked_artifact) fn for_merge_start(
        workspace_id: &str,
    ) -> Result<Self, CheckedFsError> {
        let owner = CheckedActionOwnerV1::for_merge_start(workspace_id)?;
        Self::seal(owner, ManagedParentBootstrapRequest::for_merge_start())
    }

    pub(in crate::checked_artifact) fn for_durable_merge(
        observation: &CheckedOwnerRecordObservation<'_>,
        purposes: &[ManagedParentPurpose],
    ) -> Result<Self, CheckedFsError> {
        let owner = CheckedActionOwnerV1::from_record_observation(observation)?;
        let managed = ManagedParentBootstrapRequest::try_for_durable_merge(purposes)?;
        Self::seal(owner, managed)
    }

    pub(in crate::checked_artifact) fn for_archive(
        observation: &CheckedArchiveSourceObservation<'_>,
    ) -> Result<Self, CheckedFsError> {
        let owner = CheckedActionOwnerV1::from_record_observation(observation.owner())?;
        let prerequisite = ValidatedArchiveSourceV1::from_exact_record_owner(
            owner.request_owner_binding(),
            owner
                .source_record_sha256()
                .ok_or_else(|| identity_error("archive owner is not record-derived"))?,
        )?;
        Self::seal(
            owner,
            ManagedParentBootstrapRequest::for_archive(prerequisite),
        )
    }

    fn seal(
        owner: CheckedActionOwnerV1,
        managed: ManagedParentBootstrapRequest,
    ) -> Result<Self, CheckedFsError> {
        if !owner.permits_managed_request(&managed) {
            return Err(identity_error(
                "managed-parent authority does not match its owner class",
            ));
        }
        let checked = CheckedActionRequestV1::for_managed_parents(&owner, &managed)?;
        Ok(Self { checked, managed })
    }

    pub(in crate::checked_artifact) fn checked(&self) -> &CheckedActionRequestV1 {
        &self.checked
    }

    pub(in crate::checked_artifact) fn managed(&self) -> &ManagedParentBootstrapRequest {
        &self.managed
    }
}

impl CheckedActionRequestV1 {
    pub(in crate::checked_artifact) fn for_managed_parents(
        owner: &CheckedActionOwnerV1,
        request: &ManagedParentBootstrapRequest,
    ) -> Result<Self, CheckedFsError> {
        if !owner.permits_managed_request(request) {
            return Err(identity_error(
                "managed-parent authority does not match its owner class",
            ));
        }
        let purposes = CheckedPurposeSetV1::from_request(request);
        if purposes.is_empty() {
            return Err(identity_error("parent-only action has no purpose"));
        }
        Ok(Self {
            owner_binding: owner.request_owner_binding(),
            operation: CheckedActionOperationV1::ParentOnly,
            root_kind: PreCatalogRootKindV1::Workspace,
            path: None,
            expected: CheckedLeafFactV1::Missing,
            goal: CheckedLeafFactV1::Missing,
            purposes,
        })
    }

    fn for_leaf(
        owner: &CheckedActionOwnerV1,
        operation: CheckedActionOperationV1,
        root_kind: PreCatalogRootKindV1,
        path: Vec<AsciiComponent>,
        expected: CheckedLeafFactV1,
        goal: CheckedLeafFactV1,
        purposes: CheckedPurposeSetV1,
    ) -> Result<Self, CheckedFsError> {
        if path.is_empty() {
            return Err(identity_error("checked leaf path is empty"));
        }
        validate_path_encoding(&path)?;
        let legal = match operation {
            CheckedActionOperationV1::Observe => expected == goal,
            CheckedActionOperationV1::Replace => matches!(goal, CheckedLeafFactV1::Exact { .. }),
            CheckedActionOperationV1::Remove => {
                matches!(expected, CheckedLeafFactV1::Exact { .. })
                    && goal == CheckedLeafFactV1::Missing
            }
            CheckedActionOperationV1::ParentOnly => false,
        };
        if !legal || purposes.mask() & !CheckedPurposeSetV1::VALID_MASK != 0 {
            return Err(identity_error("illegal checked leaf request shape"));
        }
        Ok(Self {
            owner_binding: owner.request_owner_binding(),
            operation,
            root_kind,
            path: Some(path),
            expected,
            goal,
            purposes,
        })
    }

    pub(in crate::checked_artifact) fn action_digest(&self) -> ActionDigestV1 {
        let mut sink = HashSink::new();
        self.write_canonical(&mut sink);
        ActionDigestV1::new(sink.finish())
    }

    pub(in crate::checked_artifact) const fn owner_binding(&self) -> RequestOwnerBindingV1 {
        self.owner_binding
    }

    pub(in crate::checked_artifact) const fn operation(&self) -> CheckedActionOperationV1 {
        self.operation
    }

    pub(in crate::checked_artifact) const fn expected(&self) -> CheckedLeafFactV1 {
        self.expected
    }

    pub(in crate::checked_artifact) const fn goal(&self) -> CheckedLeafFactV1 {
        self.goal
    }

    pub(in crate::checked_artifact) const fn purposes(&self) -> CheckedPurposeSetV1 {
        self.purposes
    }

    #[cfg(test)]
    fn canonical_preimage(&self) -> Vec<u8> {
        let mut value = Vec::new();
        self.write_canonical(&mut value);
        value
    }

    fn write_canonical(&self, sink: &mut impl CanonicalSink) {
        write_header(sink, ACTION_DOMAIN, 0, 7);
        write_field(sink, 0, &self.owner_binding.bytes());
        write_field(sink, 1, &[self.operation as u8]);
        write_field(
            sink,
            2,
            &[match self.root_kind {
                PreCatalogRootKindV1::Workspace => 0,
                PreCatalogRootKindV1::GitDirectory => 1,
            }],
        );
        write_field_header(sink, 3, optional_path_encoded_len(self.path.as_deref()));
        write_optional_path(sink, self.path.as_deref());
        write_field_header(sink, 4, self.expected.encoded_len());
        self.expected.write_canonical(sink);
        write_field_header(sink, 5, self.goal.encoded_len());
        self.goal.write_canonical(sink);
        write_field(sink, 6, &[self.purposes.mask()]);
    }
}

trait CanonicalSink {
    fn write(&mut self, bytes: &[u8]);
}

impl CanonicalSink for Vec<u8> {
    fn write(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

struct HashSink(Sha256);

impl HashSink {
    fn new() -> Self {
        Self(Sha256::new())
    }

    fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

impl CanonicalSink for HashSink {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
}

fn write_header(sink: &mut impl CanonicalSink, domain: &[u8], variant: u8, field_count: u16) {
    sink.write(domain);
    sink.write(&[ENCODING_VERSION]);
    sink.write(&[variant]);
    sink.write(&field_count.to_be_bytes());
}

fn write_field(sink: &mut impl CanonicalSink, tag: u8, value: &[u8]) {
    write_field_header(sink, tag, value.len());
    sink.write(value);
}

fn write_field_header(sink: &mut impl CanonicalSink, tag: u8, length: usize) {
    sink.write(&[tag]);
    sink.write(&(length as u64).to_be_bytes());
}

fn write_optional_path(sink: &mut impl CanonicalSink, path: Option<&[AsciiComponent]>) {
    let Some(path) = path else {
        sink.write(&[0]);
        return;
    };
    sink.write(&[1]);
    sink.write(&(path.len() as u16).to_be_bytes());
    for component in path {
        sink.write(&(component.as_bytes().len() as u16).to_be_bytes());
        sink.write(component.as_bytes());
    }
}

fn optional_path_encoded_len(path: Option<&[AsciiComponent]>) -> usize {
    path.map_or(1, |components| {
        3 + components
            .iter()
            .map(|component| 2 + component.as_bytes().len())
            .sum::<usize>()
    })
}

fn validate_path_encoding(path: &[AsciiComponent]) -> Result<(), CheckedFsError> {
    if optional_path_encoded_len(Some(path)) > MAX_CANONICAL_PATH_IDENTITY_BYTES {
        return Err(identity_error(
            "checked leaf path identity exceeds the 4 KiB bound",
        ));
    }
    Ok(())
}

fn validate_prefixed_id(
    label: &'static str,
    prefix: &str,
    value: &str,
) -> Result<(), CheckedFsError> {
    if !value.starts_with(prefix)
        || value.len() <= prefix.len()
        || value.len() > MAX_ID_BYTES
        || !value.bytes().all(is_portable_id_byte)
        || matches!(&value[prefix.len()..], "." | "..")
    {
        return Err(identity_error(label));
    }
    Ok(())
}

fn validate_slug_id(label: &'static str, value: &str) -> Result<(), CheckedFsError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || matches!(value, "." | "..")
        || !value.bytes().all(is_portable_id_byte)
    {
        return Err(identity_error(label));
    }
    Ok(())
}

const fn is_portable_id_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

fn identity_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("checked action identity", detail)
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_record_owner_v0(
    workspace_id: &str,
    merge_id: &str,
    operation_id: &str,
    source_record_bytes: &[u8],
) -> Result<CheckedActionOwnerV1, CheckedFsError> {
    CheckedActionOwnerV1::for_merge_record(
        OwnerVariantV1::MergeRecordV0,
        workspace_id,
        merge_id,
        operation_id,
        source_record_bytes,
    )
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_record_owner_v1(
    workspace_id: &str,
    merge_id: &str,
    operation_id: &str,
    source_record_bytes: &[u8],
) -> Result<CheckedActionOwnerV1, CheckedFsError> {
    CheckedActionOwnerV1::for_merge_record(
        OwnerVariantV1::MergeRecordV1,
        workspace_id,
        merge_id,
        operation_id,
        source_record_bytes,
    )
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_leaf_request(
    owner: &CheckedActionOwnerV1,
    operation: CheckedActionOperationV1,
    root_kind: PreCatalogRootKindV1,
    path: Vec<AsciiComponent>,
    expected: CheckedLeafFactV1,
    goal: CheckedLeafFactV1,
    purpose_mask: u8,
) -> Result<CheckedActionRequestV1, CheckedFsError> {
    if purpose_mask & !CheckedPurposeSetV1::VALID_MASK != 0 {
        return Err(identity_error("purpose bitset has unknown bits"));
    }
    CheckedActionRequestV1::for_leaf(
        owner,
        operation,
        root_kind,
        path,
        expected,
        goal,
        CheckedPurposeSetV1(purpose_mask),
    )
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_owner_preimage(
    owner: &CheckedActionOwnerV1,
) -> Vec<u8> {
    owner.canonical_preimage()
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_action_preimage(
    request: &CheckedActionRequestV1,
) -> Vec<u8> {
    request.canonical_preimage()
}
