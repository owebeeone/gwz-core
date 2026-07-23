use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::ModelResult;

/// Additive coordinated-merge evidence carried by an ordinary commit marker.
///
/// The containing root commit is the composition commit; it is deliberately
/// absent here so the marker never attempts to refer to its own commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MarkerMergeArtifact {
    pub merge_id: String,
    pub operation_id: String,
    pub source_ref: String,
    pub selected_targets: Vec<String>,
    pub participants: BTreeMap<String, MarkerMergeParticipantArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_merge_commit: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MarkerMergeParticipantArtifact {
    pub target_kind: MarkerMergeTargetKind,
    pub target_branch: String,
    pub before_commit: String,
    pub source_commit: String,
    pub resulting_commit: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerMergeTargetKind {
    Member,
    Root,
}

impl MarkerMergeArtifact {
    pub(crate) fn validate(&self) -> ModelResult<()> {
        super::require_non_empty("merge.merge_id", &self.merge_id)?;
        super::require_non_empty("merge.operation_id", &self.operation_id)?;
        super::require_non_empty("merge.source_ref", &self.source_ref)?;
        if self.selected_targets.is_empty() {
            return Err(super::invalid(
                "merge.selected_targets must contain at least one target",
            ));
        }
        let mut selected = BTreeSet::new();
        for target in &self.selected_targets {
            validate_merge_target("merge selected target", target)?;
            if !selected.insert(target.as_str()) {
                return Err(super::invalid(format!(
                    "duplicate merge selected target '{target}'"
                )));
            }
            let participant = self.participants.get(target).ok_or_else(|| {
                super::invalid(format!(
                    "merge selected target '{target}' has no participant evidence"
                ))
            })?;
            let expected = if target == "@root" {
                MarkerMergeTargetKind::Root
            } else {
                MarkerMergeTargetKind::Member
            };
            if participant.target_kind != expected {
                return Err(super::invalid(format!(
                    "merge participant '{target}' has the wrong target kind"
                )));
            }
        }
        for (target, participant) in &self.participants {
            validate_merge_target("merge participant", target)?;
            if !selected.contains(target.as_str()) {
                return Err(super::invalid(format!(
                    "merge participant '{target}' is not a selected target"
                )));
            }
            participant.validate()?;
        }
        super::validate_optional_text("merge.root_merge_commit", &self.root_merge_commit)?;
        match (
            self.participants.get("@root"),
            self.root_merge_commit.as_deref(),
        ) {
            (Some(root), Some(commit)) if root.resulting_commit == commit => Ok(()),
            (Some(_), Some(_)) => Err(super::invalid(
                "merge.root_merge_commit does not match the @root result",
            )),
            (Some(_), None) => Err(super::invalid(
                "merge.root_merge_commit is required when @root participated",
            )),
            (None, Some(_)) => Err(super::invalid(
                "merge.root_merge_commit requires an explicit @root participant",
            )),
            (None, None) => Ok(()),
        }
    }
}

impl MarkerMergeParticipantArtifact {
    fn validate(&self) -> ModelResult<()> {
        super::require_non_empty("merge participant target_branch", &self.target_branch)?;
        super::require_non_empty("merge participant before_commit", &self.before_commit)?;
        super::require_non_empty("merge participant source_commit", &self.source_commit)?;
        super::require_non_empty("merge participant resulting_commit", &self.resulting_commit)
    }
}

fn validate_merge_target(field: &str, target: &str) -> ModelResult<()> {
    if target == "@root" {
        Ok(())
    } else {
        super::parse_id(field, "mem_", target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::MarkerArtifact;

    fn participant(kind: MarkerMergeTargetKind) -> MarkerMergeParticipantArtifact {
        MarkerMergeParticipantArtifact {
            target_kind: kind,
            target_branch: "main".into(),
            before_commit: "before".into(),
            source_commit: "source".into(),
            resulting_commit: "result".into(),
        }
    }

    fn merge() -> MarkerMergeArtifact {
        MarkerMergeArtifact {
            merge_id: "merge_1".into(),
            operation_id: "op_1".into(),
            source_ref: "feature/x".into(),
            selected_targets: vec!["mem_a".into()],
            participants: [("mem_a".into(), participant(MarkerMergeTargetKind::Member))].into(),
            root_merge_commit: None,
        }
    }

    #[test]
    fn additive_merge_round_trips_and_old_marker_remains_compatible() {
        let old = "{schema: gwz.marker/v0, gwz_commit_id: 01987b0c-2f75-7c4a-9a32-8fd22f7d7c91, workspace_id: ws_01, created_at: now, created_by: {actor_id: agent_01}, root: {path: .}, selected_targets: [], committed_targets: [], members: {}}";
        assert!(MarkerArtifact::from_yaml(old).unwrap().merge.is_none());
        let value = merge();
        let decoded: MarkerMergeArtifact =
            serde_yaml::from_str(&serde_yaml::to_string(&value).unwrap()).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn validation_rejects_inconsistent_targets_and_root_evidence() {
        let mut value = merge();
        value.selected_targets.clear();
        value.participants.clear();
        assert!(value.validate().is_err());

        let mut value = merge();
        value.selected_targets.push("mem_a".into());
        assert!(value.validate().unwrap_err().message.contains("duplicate"));

        let mut value = merge();
        value.participants.clear();
        assert!(value.validate().is_err());

        let mut value = merge();
        value
            .participants
            .insert("@root".into(), participant(MarkerMergeTargetKind::Root));
        assert!(value.validate().is_err());

        value.selected_targets = vec!["@root".into()];
        value.participants.remove("mem_a");
        assert!(value.validate().is_err());
        value.root_merge_commit = Some("wrong".into());
        assert!(
            value
                .validate()
                .unwrap_err()
                .message
                .contains("does not match")
        );
        value.root_merge_commit = Some("result".into());
        assert!(value.validate().is_ok());
        value.participants.get_mut("@root").unwrap().target_kind = MarkerMergeTargetKind::Member;
        assert!(value.validate().is_err());
    }
}
