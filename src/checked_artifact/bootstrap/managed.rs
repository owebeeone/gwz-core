//! Plan-derived, reservation-bound managed-parent bootstrap contracts.

mod owner;
mod plan;

#[allow(
    unused_imports,
    reason = "R1 freezes the execution owner before R2 production conversion"
)]
pub(in crate::checked_artifact) use owner::*;
pub(in crate::checked_artifact) use plan::*;

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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedParentSpec {
    purpose: ManagedParentPurpose,
    components: Vec<AsciiComponent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) struct ManagedParentBootstrapRequest {
    specs: Vec<ManagedParentSpec>,
}

impl ManagedParentBootstrapRequest {
    pub(in crate::checked_artifact) fn try_new(
        specs: Vec<ManagedParentSpec>,
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
        Ok(Self { specs })
    }

    pub(in crate::checked_artifact) fn specs(&self) -> &[ManagedParentSpec] {
        &self.specs
    }
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
