//! Plan-derived, reservation-bound managed-parent bootstrap contracts.

mod owner;
mod plan;
/// R2-D Phase 3 Step 3.1 — the production `ManagedParentBootstrap` provider.
#[allow(
    dead_code,
    reason = "Step 3.1 lands the provider; plan §4 Step 3.3 wires its production consumer"
)]
mod provider;

#[cfg(test)]
mod tests_intent_matrix;
#[cfg(test)]
mod tests_provider;

#[allow(
    unused_imports,
    reason = "R1 freezes the execution owner before R2 production conversion"
)]
pub(in crate::checked_artifact) use owner::*;
pub(in crate::checked_artifact) use plan::*;
#[allow(
    unused_imports,
    reason = "Step 3.1 lands the provider; plan §4 Step 3.3 wires its production consumer"
)]
pub(in crate::checked_artifact) use provider::*;

use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalPathIdentityV1, CheckedFsError, DurableObjectIdentityV1,
    PathComponentMode, PlatformCapability,
};
use crate::checked_artifact::protocol::{
    MAX_MANAGED_PARENT_BOOTSTRAPS, MAX_MANAGED_PARENT_COMPONENTS,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::checked_artifact) enum ManagedParentPurpose {
    MergeStore,
    MergeArchive,
    PreservationBundles,
    RootPreservationMarkers,
}

impl ManagedParentPurpose {
    pub(in crate::checked_artifact) const ALL: &'static [Self] = &[
        Self::MergeStore,
        Self::MergeArchive,
        Self::PreservationBundles,
        Self::RootPreservationMarkers,
    ];

    const fn code(self) -> u8 {
        match self {
            Self::MergeStore => 0,
            Self::MergeArchive => 1,
            Self::PreservationBundles => 2,
            Self::RootPreservationMarkers => 3,
        }
    }

    fn path(self) -> &'static [&'static [u8]] {
        match self {
            Self::MergeStore => &[b".gwz", b"merge"],
            Self::MergeArchive => &[b".gwz", b"merge", b"done"],
            Self::PreservationBundles => &[b".gwz", b"stash", b"bundles"],
            Self::RootPreservationMarkers => &[b"gwz.conf", b"markers"],
        }
    }

    const fn minimum_retained_parent_count(self) -> usize {
        match self {
            Self::MergeStore | Self::PreservationBundles | Self::RootPreservationMarkers => 1,
            Self::MergeArchive => 2,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedParentSpec {
    purpose: ManagedParentPurpose,
    components: Vec<AsciiComponent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedParentBootstrapRequest {
    specs: Vec<ManagedParentSpec>,
    authority: ManagedParentRequestAuthorityV1,
}

impl ManagedParentBootstrapRequest {
    /// The one reproducible pre-record request. Its fixed purpose set prevents
    /// callers from deriving a different resident action after a crash.
    pub(in crate::checked_artifact) fn for_merge_start() -> Self {
        Self::try_from_purposes(
            &[
                ManagedParentPurpose::MergeStore,
                ManagedParentPurpose::PreservationBundles,
            ],
            ManagedParentRequestAuthorityV1::MergeStart,
        )
        .expect("fixed merge-start purposes are valid")
    }

    /// Record-owned merge work may prepare bundle and root-marker parents.
    /// Archive creation uses its separate prerequisite-bearing constructor.
    pub(in crate::checked_artifact) fn try_for_durable_merge(
        purposes: &[ManagedParentPurpose],
    ) -> Result<Self, CheckedFsError> {
        if purposes.iter().any(|purpose| {
            !matches!(
                purpose,
                ManagedParentPurpose::PreservationBundles
                    | ManagedParentPurpose::RootPreservationMarkers
            )
        }) {
            return Err(request_mismatch(
                "durable merge request contains a purpose owned by another constructor",
            ));
        }
        Self::try_from_purposes(purposes, ManagedParentRequestAuthorityV1::DurableMerge)
    }

    pub(in crate::checked_artifact) fn for_archive(prerequisite: ValidatedArchiveSourceV1) -> Self {
        Self::try_from_purposes(
            &[ManagedParentPurpose::MergeArchive],
            ManagedParentRequestAuthorityV1::Archive(prerequisite),
        )
        .expect("fixed archive purpose is valid")
    }

    fn try_from_purposes(
        purposes: &[ManagedParentPurpose],
        authority: ManagedParentRequestAuthorityV1,
    ) -> Result<Self, CheckedFsError> {
        let specs = purposes
            .iter()
            .copied()
            .map(ManagedParentSpec::for_purpose)
            .collect::<Vec<_>>();
        Self::try_new(specs, authority)
    }

    fn try_new(
        specs: Vec<ManagedParentSpec>,
        authority: ManagedParentRequestAuthorityV1,
    ) -> Result<Self, CheckedFsError> {
        if specs.is_empty() || specs.len() > MAX_MANAGED_PARENT_BOOTSTRAPS {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::ManagedParentBootstrap,
                format!(
                    "managed-parent bootstrap requires 1..={MAX_MANAGED_PARENT_BOOTSTRAPS} declared purposes"
                ),
            ));
        }
        if specs
            .windows(2)
            .any(|pair| pair[0].purpose().code() >= pair[1].purpose().code())
        {
            return Err(CheckedFsError::ambiguous(
                "managed-parent bootstrap",
                "managed-parent purposes must be unique and in canonical order",
            ));
        }
        let allowed = match &authority {
            ManagedParentRequestAuthorityV1::MergeStart => specs.iter().all(|spec| {
                matches!(
                    spec.purpose(),
                    ManagedParentPurpose::MergeStore | ManagedParentPurpose::PreservationBundles
                )
            }),
            ManagedParentRequestAuthorityV1::DurableMerge => specs.iter().all(|spec| {
                matches!(
                    spec.purpose(),
                    ManagedParentPurpose::PreservationBundles
                        | ManagedParentPurpose::RootPreservationMarkers
                )
            }),
            ManagedParentRequestAuthorityV1::Archive(_) => {
                specs.len() == 1 && specs[0].purpose() == ManagedParentPurpose::MergeArchive
            }
            #[cfg(test)]
            ManagedParentRequestAuthorityV1::Unrestricted => true,
        };
        if !allowed {
            return Err(request_mismatch(
                "managed-parent purposes do not match their sealed owner",
            ));
        }
        Ok(Self { specs, authority })
    }

    pub(in crate::checked_artifact) fn specs(&self) -> &[ManagedParentSpec] {
        &self.specs
    }

    fn validate_authority(
        &self,
        request_owner_binding: crate::checked_artifact::protocol::RequestOwnerBindingV1,
    ) -> Result<(), CheckedFsError> {
        if let ManagedParentRequestAuthorityV1::Archive(prerequisite) = &self.authority
            && (prerequisite.request_owner_binding != request_owner_binding
                || prerequisite.source_record_sha256 == [0; 32])
        {
            return Err(request_mismatch(
                "archive source prerequisite belongs to another durable merge owner",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ManagedParentRequestAuthorityV1 {
    MergeStart,
    DurableMerge,
    Archive(ValidatedArchiveSourceV1),
    #[cfg(test)]
    Unrestricted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ManagedParentAuthorityClassV1 {
    MergeStart,
    DurableMerge,
    Archive,
    #[cfg(test)]
    Unrestricted,
}

impl ManagedParentBootstrapRequest {
    pub(in crate::checked_artifact) const fn authority_class(
        &self,
    ) -> ManagedParentAuthorityClassV1 {
        match self.authority {
            ManagedParentRequestAuthorityV1::MergeStart => {
                ManagedParentAuthorityClassV1::MergeStart
            }
            ManagedParentRequestAuthorityV1::DurableMerge => {
                ManagedParentAuthorityClassV1::DurableMerge
            }
            ManagedParentRequestAuthorityV1::Archive(_) => ManagedParentAuthorityClassV1::Archive,
            #[cfg(test)]
            ManagedParentRequestAuthorityV1::Unrestricted => {
                ManagedParentAuthorityClassV1::Unrestricted
            }
        }
    }

    pub(in crate::checked_artifact) fn archive_prerequisite(
        &self,
    ) -> Option<&ValidatedArchiveSourceV1> {
        match &self.authority {
            ManagedParentRequestAuthorityV1::Archive(value) => Some(value),
            _ => None,
        }
    }
}

/// Opaque result of terminal/source archive arbitration. There is deliberately
/// no general constructor; the R2 coordinator will issue it inside this module
/// from an exact durable merge-store observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ValidatedArchiveSourceV1 {
    request_owner_binding: crate::checked_artifact::protocol::RequestOwnerBindingV1,
    source_record_sha256: [u8; 32],
}

impl ValidatedArchiveSourceV1 {
    pub(in crate::checked_artifact) fn from_exact_record_owner(
        request_owner_binding: crate::checked_artifact::protocol::RequestOwnerBindingV1,
        source_record_sha256: [u8; 32],
    ) -> Result<Self, CheckedFsError> {
        if source_record_sha256 == [0; 32] {
            return Err(request_mismatch(
                "archive source prerequisite has an empty record digest",
            ));
        }
        Ok(Self {
            request_owner_binding,
            source_record_sha256,
        })
    }

    pub(in crate::checked_artifact) const fn owner_binding(
        &self,
    ) -> crate::checked_artifact::protocol::RequestOwnerBindingV1 {
        self.request_owner_binding
    }

    pub(in crate::checked_artifact) const fn source_record_sha256(&self) -> [u8; 32] {
        self.source_record_sha256
    }
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_archive_source_prerequisite(
    request_owner_binding: crate::checked_artifact::protocol::RequestOwnerBindingV1,
    source_record_sha256: [u8; 32],
) -> ValidatedArchiveSourceV1 {
    ValidatedArchiveSourceV1 {
        request_owner_binding,
        source_record_sha256,
    }
}

#[cfg(test)]
pub(in crate::checked_artifact) fn synthetic_managed_parent_request(
    purposes: &[ManagedParentPurpose],
    authority: SyntheticManagedParentAuthorityV1,
) -> Result<ManagedParentBootstrapRequest, CheckedFsError> {
    let authority = match authority {
        SyntheticManagedParentAuthorityV1::MergeStart => {
            ManagedParentRequestAuthorityV1::MergeStart
        }
        SyntheticManagedParentAuthorityV1::DurableMerge => {
            ManagedParentRequestAuthorityV1::DurableMerge
        }
        SyntheticManagedParentAuthorityV1::Archive(owner, source_record_sha256) => {
            ManagedParentRequestAuthorityV1::Archive(ValidatedArchiveSourceV1 {
                request_owner_binding: owner,
                source_record_sha256,
            })
        }
        SyntheticManagedParentAuthorityV1::Unrestricted => {
            ManagedParentRequestAuthorityV1::Unrestricted
        }
    };
    ManagedParentBootstrapRequest::try_from_purposes(purposes, authority)
}

#[cfg(test)]
pub(in crate::checked_artifact) enum SyntheticManagedParentAuthorityV1 {
    MergeStart,
    DurableMerge,
    Archive(
        crate::checked_artifact::protocol::RequestOwnerBindingV1,
        [u8; 32],
    ),
    Unrestricted,
}

impl ManagedParentSpec {
    pub(in crate::checked_artifact) fn for_purpose(purpose: ManagedParentPurpose) -> Self {
        let components = purpose
            .path()
            .iter()
            .map(|value| AsciiComponent::parse(value).expect("fixed managed path is valid"))
            .collect();
        Self {
            purpose,
            components,
        }
    }

    pub(in crate::checked_artifact) fn purpose(&self) -> ManagedParentPurpose {
        self.purpose
    }

    pub(in crate::checked_artifact) fn components(&self) -> &[AsciiComponent] {
        &self.components
    }
}

fn request_mismatch(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("managed-parent request", detail)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedParentObservationV1 {
    purpose: ManagedParentPurpose,
    retained_existing_parent_count: usize,
    retained_parent_identity: DurableObjectIdentityV1,
    retained_parent_mode: PathComponentMode,
    retained_parent_path: CanonicalPathIdentityV1,
}

impl ManagedParentObservationV1 {
    pub(in crate::checked_artifact) fn new(
        purpose: ManagedParentPurpose,
        retained_existing_parent_count: usize,
        retained_parent_identity: DurableObjectIdentityV1,
        retained_parent_mode: PathComponentMode,
        retained_parent_path: CanonicalPathIdentityV1,
    ) -> Result<Self, CheckedFsError> {
        if retained_existing_parent_count > MAX_MANAGED_PARENT_COMPONENTS {
            return Err(CheckedFsError::ambiguous(
                "managed-parent preflight",
                "retained parent count exceeds the managed-parent bound",
            ));
        }
        Ok(Self {
            purpose,
            retained_existing_parent_count,
            retained_parent_identity,
            retained_parent_mode,
            retained_parent_path,
        })
    }
}
