use std::collections::BTreeMap;
use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace::WORKSPACE_MANIFEST;

use super::super::publication::RootEvidenceObservation;
use super::super::{MergeTargetKind, OperationDriftKind, OperationState, participant_semantics};
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalStatusSourceIdentity {
    kind: super::super::record_wire::CanonicalRecordKind,
    path: std::path::PathBuf,
    digest: super::super::record_wire::Sha256Digest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum CanonicalStatusSource {
    Open,
    Archived,
}

pub(in crate::workspace_ops::merge) fn select_canonical_status_source(
    locations: &super::super::record_wire::CanonicalMergeLocations,
    merge_id: &str,
    open_is_terminal: Option<bool>,
) -> ModelResult<CanonicalStatusSource> {
    match (locations.open().exact(), locations.archived().exact()) {
        (None, None) => Err(ModelError::new(
            ErrorCode::OperationNotFound,
            format!("merge record '{merge_id}' was not found"),
        )),
        (Some(_), None) => Ok(CanonicalStatusSource::Open),
        (None, Some(_)) => Ok(CanonicalStatusSource::Archived),
        (Some((_, open, _)), Some((_, archived, _)))
            if open_is_terminal == Some(true) && open == archived =>
        {
            Ok(CanonicalStatusSource::Open)
        }
        (Some(_), Some(_)) => Err(status_contradiction(merge_id)),
    }
}

fn status_contradiction(merge_id: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("open and archived copies of merge record '{merge_id}' are contradictory"),
    )
}

fn status_contention(merge_id: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("merge record '{merge_id}' changed during both status attempts"),
    )
}

/// `gwz merge --status [<id>]` when nothing is open.
///
/// **M5d (`GwzM5-8M5d-Charter.md` §1, §2).** The dispatch classifies the open
/// record's envelope BEFORE this is reached: a v1 record goes to the v1
/// lifecycle's own status, and a v0 envelope is the §2 refusal. So the only
/// records this serves are archived ones, over both envelopes — I2 §7's
/// projection, which charter §5 retains.
pub(crate) fn handle_status(
    root: &Path,
    merge_id: Option<&str>,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let Some(requested) = merge_id else {
        return super::super::response::idle_status_response(context);
    };
    let locations = super::super::record_wire::acquire_canonical_merge_locations(root, requested)?;
    match select_canonical_status_source(&locations, requested, None)? {
        CanonicalStatusSource::Archived => {
            let (_, bytes, _) = locations
                .archived()
                .exact()
                .ok_or_else(|| status_contention(requested))?;
            let archived =
                super::super::record_wire::decode_archived(bytes.as_slice(), requested)?;
            super::super::response::archived_status_response(
                requested,
                archived.projection(),
                context,
            )
        }
        // An open record appeared between the dispatch's envelope
        // classification and this read. It is not this function's to serve.
        CanonicalStatusSource::Open => Err(status_contention(requested)),
    }
}

pub(in crate::workspace_ops::merge) struct MergeStatusViewObservation {
    pub(in crate::workspace_ops::merge) participants:
        BTreeMap<String, super::super::MergeParticipantObservation>,
    pub(in crate::workspace_ops::merge) operation_drift: Vec<super::super::OperationDrift>,
    pub(in crate::workspace_ops::merge) interrupted_root_rollback: bool,
}

pub(in crate::workspace_ops::merge) fn observe_status_view<B: GitBackend>(
    backend: &B,
    root: &Path,
    view: super::MergeStatusRecordView<'_>,
) -> ModelResult<MergeStatusViewObservation> {
    // Validate the entire durable path set before the first repository access;
    // a corrupt unselected row must not become a later filesystem escape.
    for (target_id, participant) in view.participants() {
        validated_participant_path(root, target_id, participant)?;
    }
    let mut participants = BTreeMap::new();
    for target_id in view.selected_targets() {
        let participant = view.participants().get(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        participants.insert(
            target_id.clone(),
            observe_participant(backend, root, target_id, participant)?,
        );
    }

    let root_attempted = view.participants().values().any(|participant| {
        participant.target_kind == MergeTargetKind::Root
            && participant_semantics::status::status_policy(participant.state).root_attempted_role
                == participant_semantics::status::RootAttemptedRole::Attempted
    });
    let mut operation_drift = view.operation_drift().to_vec();
    match view
        .publication()
        .and_then(|publication| publication.candidate.as_ref())
    {
        Some(_) => {
            let prefix =
                super::super::publication::classify_candidate_publication_view(root, view)?;
            if prefix.is_none()
                || !super::super::publication::publication_prefix_allowed_view(
                    view,
                    prefix.expect("candidate prefix was checked"),
                )?
            {
                push_operation_drift(
                    &mut operation_drift,
                    OperationDriftKind::RootCandidateStateChanged,
                    "workspace root candidate artifacts do not match an allowed publication prefix",
                );
            }
        }
        None if !root_attempted => {
            compare_digest(
                root,
                artifact::LOCK_PATH,
                &view.baseline().lock_sha256,
                OperationDriftKind::BaselineLockChanged,
                &mut operation_drift,
            );
        }
        None => {}
    }
    if !root_attempted {
        compare_digest(
            root,
            WORKSPACE_MANIFEST,
            &view.baseline().manifest_sha256,
            OperationDriftKind::BaselineManifestChanged,
            &mut operation_drift,
        );
    }
    if view.state() == OperationState::Finalizing
        && let Some(publication) = view.publication()
        && (publication.candidate.is_some() || !root_attempted)
    {
        let root_matches = if publication.candidate.is_some() {
            match super::super::publication::observe_root_evidence_view(backend, root, view)? {
                Some(RootEvidenceObservation::Baseline) => publication.composition_commit.is_none(),
                Some(RootEvidenceObservation::Composition(result)) => publication
                    .composition_commit
                    .as_deref()
                    .is_none_or(|recorded| recorded == result.commit),
                None => false,
            }
        } else {
            let root_head = backend.head(root)?;
            !root_head.is_detached
                && root_head.commit == view.baseline().root_head
                && view
                    .baseline()
                    .root_branch
                    .as_deref()
                    .is_none_or(|branch| root_head.branch.as_deref() == Some(branch))
        };
        if !root_matches {
            push_operation_drift(
                &mut operation_drift,
                OperationDriftKind::RootCandidateStateChanged,
                "workspace root HEAD does not match the recorded merge publication state",
            );
        }
    }
    if super::super::root::root_finalization_is_exact_view(backend, root, view)?
        && let Some(root) = participants.get_mut("@root")
    {
        participant_semantics::status::apply_exact_root_finalization_override(root);
    }
    let interrupted_root_rollback =
        super::super::root::interrupted_evidence_rollback_is_exact_view(backend, root, view)?;
    if interrupted_root_rollback {
        normalize_interrupted_root_rollback(view, &mut participants, &mut operation_drift)?;
    }
    Ok(MergeStatusViewObservation {
        participants,
        operation_drift,
        interrupted_root_rollback,
    })
}

fn normalize_interrupted_root_rollback(
    view: super::MergeStatusRecordView<'_>,
    participants: &mut BTreeMap<String, super::super::MergeParticipantObservation>,
    operation_drift: &mut Vec<super::super::OperationDrift>,
) -> ModelResult<()> {
    let participant = view.selected_root_participant()?.ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root evidence exists without a durable root participant",
        )
    })?;
    let observation = participants.get_mut("@root").ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "root evidence exists without a root status observation",
        )
    })?;
    operation_drift.retain(|drift| drift.kind != OperationDriftKind::RootCandidateStateChanged);
    observation.live_commit = participant.resulting_commit.clone();
    observation.conflict_paths.clear();
    observation.drift.clear();
    observation.abort_eligibility.eligible = true;
    observation.abort_eligibility.blockers.clear();
    Ok(())
}
