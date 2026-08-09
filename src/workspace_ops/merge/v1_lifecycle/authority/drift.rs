use serde::Serialize;

use crate::workspace_ops::merge::{ParticipantDrift, ParticipantDriftKind};

#[derive(Clone, Debug, Serialize)]
pub(in crate::workspace_ops::merge::v1_lifecycle) struct ParticipantDriftPayload {
    pub(in crate::workspace_ops::merge::v1_lifecycle) member_id: String,
    pub(in crate::workspace_ops::merge::v1_lifecycle) identity: ParticipantDriftIdentity,
    pub(in crate::workspace_ops::merge::v1_lifecycle) drift: ParticipantDrift,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(in crate::workspace_ops::merge::v1_lifecycle) struct ParticipantDriftIdentity {
    pub(in crate::workspace_ops::merge::v1_lifecycle) kind: ParticipantDriftKind,
    pub(in crate::workspace_ops::merge::v1_lifecycle) expected_branch: Option<String>,
    pub(in crate::workspace_ops::merge::v1_lifecycle) live_branch: Option<String>,
    pub(in crate::workspace_ops::merge::v1_lifecycle) expected_head: Option<String>,
    pub(in crate::workspace_ops::merge::v1_lifecycle) live_head: Option<String>,
    pub(in crate::workspace_ops::merge::v1_lifecycle) expected_merge_head: Option<String>,
    pub(in crate::workspace_ops::merge::v1_lifecycle) live_merge_head: Option<String>,
    pub(in crate::workspace_ops::merge::v1_lifecycle) occurrence: usize,
}

impl ParticipantDriftIdentity {
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn new(
        drift: &ParticipantDrift,
        occurrence: usize,
    ) -> Self {
        Self {
            kind: drift.kind,
            expected_branch: drift.expected_branch.clone(),
            live_branch: drift.live_branch.clone(),
            expected_head: drift.expected_head.clone(),
            live_head: drift.live_head.clone(),
            expected_merge_head: drift.expected_merge_head.clone(),
            live_merge_head: drift.live_merge_head.clone(),
            occurrence,
        }
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn matches(
        &self,
        drift: &ParticipantDrift,
    ) -> bool {
        self.kind == drift.kind
            && self.expected_branch == drift.expected_branch
            && self.live_branch == drift.live_branch
            && self.expected_head == drift.expected_head
            && self.live_head == drift.live_head
            && self.expected_merge_head == drift.expected_merge_head
            && self.live_merge_head == drift.live_merge_head
    }
}
