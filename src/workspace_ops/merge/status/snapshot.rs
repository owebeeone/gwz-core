use std::collections::BTreeMap;
use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace::WORKSPACE_MANIFEST;

use super::super::publication::RootEvidenceObservation;
use super::super::{
    MergeOperationRecord, MergeStatusSnapshot, MergeStore, MergeTargetKind, OperationDriftKind,
    OperationState, participant_semantics,
};
use super::*;

#[allow(dead_code, reason = "P3 consumes the canonical v0 status result")]
pub(in crate::workspace_ops::merge) enum CanonicalStatusObservationV0 {
    Open {
        record: Box<MergeOperationRecord>,
        live: MergeStatusViewObservation,
    },
    Archived {
        record: Box<super::super::record_wire::ValidatedArchivedRecord>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalStatusSourceIdentity {
    kind: super::super::record_wire::CanonicalRecordKind,
    path: std::path::PathBuf,
    digest: super::super::record_wire::Sha256Digest,
}

struct CanonicalStatusAcquisitionV0 {
    locations: super::super::record_wire::CanonicalMergeLocations,
    identity: CanonicalStatusSourceIdentity,
    record: CanonicalStatusRecordV0,
}

enum CanonicalStatusRecordV0 {
    Open(Box<MergeOperationRecord>),
    Archived(Box<super::super::record_wire::ValidatedArchivedRecord>),
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

#[allow(dead_code, reason = "P3 consumes the canonical v0 status observer")]
pub(in crate::workspace_ops::merge) fn observe_canonical_status_v0<B: GitBackend>(
    backend: &B,
    root: &Path,
    merge_id: &str,
) -> ModelResult<CanonicalStatusObservationV0> {
    for attempt in 0..2 {
        let acquired = match acquire_status_v0(root, merge_id) {
            Ok(value) => value,
            Err(error) if attempt == 0 => return Err(error),
            Err(_) => return Err(status_contention(merge_id)),
        };
        let live = match &acquired.record {
            CanonicalStatusRecordV0::Open(record) => Some(observe_status_view(
                backend,
                root,
                super::MergeStatusRecordView::from_v0(record),
            )?),
            CanonicalStatusRecordV0::Archived(_) => None,
        };
        let unchanged = acquire_status_v0(root, merge_id)
            .ok()
            .is_some_and(|reread| {
                reread.locations == acquired.locations && reread.identity == acquired.identity
            });
        if unchanged {
            return Ok(match acquired.record {
                CanonicalStatusRecordV0::Open(record) => CanonicalStatusObservationV0::Open {
                    record,
                    live: live.expect("open status acquired live facts"),
                },
                CanonicalStatusRecordV0::Archived(record) => {
                    CanonicalStatusObservationV0::Archived { record }
                }
            });
        }
        if attempt == 1 {
            return Err(status_contention(merge_id));
        }
    }
    Err(status_contention(merge_id))
}

fn acquire_status_v0(root: &Path, merge_id: &str) -> ModelResult<CanonicalStatusAcquisitionV0> {
    let locations = super::super::record_wire::acquire_canonical_merge_locations(root, merge_id)?;
    let open = locations
        .open()
        .exact()
        .map(|(_, bytes, _)| decode_open_v0(bytes.as_slice(), merge_id))
        .transpose()?;
    let archived = locations
        .archived()
        .exact()
        .map(|(_, bytes, _)| {
            super::super::record_wire::decode_archived_v0(bytes.as_slice(), merge_id)
        })
        .transpose()?;
    let source = select_canonical_status_source(
        &locations,
        merge_id,
        open.as_ref().map(|record| !record.state.is_open()),
    )?;
    let (identity, record) = match source {
        CanonicalStatusSource::Open => {
            let (path, _, digest) = locations.open().exact().expect("open source exists");
            (
                source_identity(path, digest),
                CanonicalStatusRecordV0::Open(Box::new(open.expect("open source decoded"))),
            )
        }
        CanonicalStatusSource::Archived => {
            let (path, _, digest) = locations.archived().exact().expect("archive source exists");
            (
                source_identity(path, digest),
                CanonicalStatusRecordV0::Archived(Box::new(
                    archived.expect("archive source decoded"),
                )),
            )
        }
    };
    Ok(CanonicalStatusAcquisitionV0 {
        locations,
        identity,
        record,
    })
}

fn source_identity(
    path: &super::super::record_wire::CanonicalRecordPath,
    digest: super::super::record_wire::Sha256Digest,
) -> CanonicalStatusSourceIdentity {
    CanonicalStatusSourceIdentity {
        kind: path.kind(),
        path: path.as_path().to_owned(),
        digest,
    }
}

fn decode_open_v0(bytes: &[u8], merge_id: &str) -> ModelResult<MergeOperationRecord> {
    let decoded = super::super::record_wire::decode_production_v0(bytes)
        .map_err(|error| open_decode_error(merge_id, error))?;
    let (_, _, record) = decoded.into_production_parts();
    if record.merge_id != merge_id {
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("open merge record '{merge_id}' identifies a different operation"),
        ));
    }
    Ok(record)
}

fn open_decode_error(
    merge_id: &str,
    error: super::super::record_wire::RecordDecodeError,
) -> ModelError {
    use super::super::record_wire::{HeaderClassificationError, RecordDecodeError};

    match error {
        RecordDecodeError::Header(HeaderClassificationError::Unsupported {
            header,
            required_wave,
        }) => ModelError::new(
            ErrorCode::UnsupportedRecordVersion,
            format!(
                "merge record '{merge_id}' uses schema '{}' version {}; use a compatible newer GWZ",
                header.schema, header.record_schema_version
            ),
        )
        .with_record_context(crate::MergeRecordCompatibilityContext {
            merge_id: merge_id.to_owned(),
            schema: Some(header.schema),
            record_schema_version: Some(i64::from(header.record_schema_version)),
            required_wave,
            legacy_mode: None,
        }),
        _ => ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            format!("open merge record '{merge_id}' is unreadable"),
        ),
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

pub(crate) fn handle_status<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    merge_id: Option<&str>,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let Some(requested) = merge_id else {
        return match store.discover_open(root)? {
            Some(record) => snapshot_status(backend, root, record)?.to_response(context),
            None => super::super::response::idle_status_response(context),
        };
    };
    match observe_canonical_status_v0(backend, root, requested)? {
        CanonicalStatusObservationV0::Open { record, live } => MergeStatusSnapshot {
            record: *record,
            participants: live.participants,
            operation_drift: live.operation_drift,
        }
        .to_response(context),
        CanonicalStatusObservationV0::Archived { record } => {
            super::super::response::archived_status_response(
                requested,
                record.projection(),
                context,
            )
        }
    }
}

pub(crate) fn snapshot_status<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: MergeOperationRecord,
) -> ModelResult<MergeStatusSnapshot> {
    let observation = observe_status_view(
        backend,
        root,
        super::MergeStatusRecordView::from_v0(&record),
    )?;
    let snapshot = MergeStatusSnapshot {
        record,
        participants: observation.participants,
        operation_drift: observation.operation_drift,
    };
    debug_assert!(
        !observation.interrupted_root_rollback || snapshot.participants.contains_key("@root")
    );
    Ok(snapshot)
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
