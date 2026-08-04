use std::collections::BTreeMap;

use serde::Deserialize;
use serde_yaml::Value;

use crate::artifact::ArtifactSourceKind;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct AcceptedWorkspaceV1 {
    pub(crate) operation_baseline_lock_sha256: String,
    pub(crate) metadata_base: AcceptedMetadataBaseV1,
    pub(crate) lock: AcceptedLockV1,
    pub(crate) member_audit: BTreeMap<String, MemberAcceptanceV1>,
    pub(crate) root: RootPublicationInputV1,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct AcceptedMetadataBaseV1 {
    pub(crate) source: AcceptedMetadataSourceV1,
    pub(crate) manifest_exact_yaml: String,
    pub(crate) manifest_sha256: String,
    pub(crate) lock_exact_yaml: String,
    pub(crate) lock_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AcceptedMetadataSourceV1 {
    OperationBaseline,
    SelectedRootResult { commit: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct AcceptedLockV1 {
    pub(crate) exact_yaml: String,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MemberAcceptanceV1 {
    Selected {
        integration: AcceptedIntegrationRefV1,
        final_checkout: AcceptedAttachedCheckoutV1,
        lock_member: AcceptedLockMemberV1,
    },
    UnselectedPresent {
        lock_member: AcceptedLockMemberV1,
    },
    Absent,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct AcceptedIntegrationRefV1 {
    pub(crate) branch: String,
    pub(crate) before_commit: String,
    pub(crate) resulting_commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct AcceptedAttachedCheckoutV1 {
    pub(crate) branch: String,
    pub(crate) commit: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct AcceptedLockMemberV1 {
    pub(crate) path: String,
    pub(crate) source_id: String,
    pub(crate) source_kind: ArtifactSourceKind,
    pub(crate) commit: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) detached: Option<bool>,
    pub(crate) upstream: Option<String>,
    pub(crate) dirty: Option<bool>,
    pub(crate) materialized: Option<bool>,
    #[serde(default, flatten)]
    pub(crate) extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct RootPublicationInputV1 {
    pub(crate) base: AcceptedRootBaseV1,
    pub(crate) publication_branch: Option<String>,
    pub(crate) baseline_artifact_hashes: RootArtifactHashesV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AcceptedRootBaseV1 {
    BornAttached {
        commit: String,
        symbolic_branch: String,
    },
    BornDetached {
        commit: String,
    },
    UnbornAttached {
        symbolic_branch: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct RootArtifactHashesV1 {
    pub(crate) lock_worktree_sha256: String,
    pub(crate) manifest_worktree_sha256: String,
    pub(crate) lock_commit_sha256: Option<String>,
    pub(crate) manifest_commit_sha256: Option<String>,
}
