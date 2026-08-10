use std::fs;
use std::path::{Path, PathBuf};

use super::super::*;
use crate::artifact::LOCK_PATH;
use crate::git::{GitBackend, GitCandidateFile, GitRepositoryState};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::acceptance::{
    V1AcceptanceMetadata, V1AcceptanceRecord, build_v1_acceptance,
};
use crate::workspace_ops::merge::model::v1::{
    AcceptedRootBaseV1, MergeOperationRecordV1, RecoveryOriginStateV1,
};
use crate::workspace_ops::merge::{MergeTargetKind, OperationState, ParticipantState};

mod handoff;
mod publication;

pub(in crate::workspace_ops::merge::v1_lifecycle) use handoff::{
    RecordEvidenceOr, observe_reverse_publication_handoff,
};

pub(in crate::workspace_ops::merge::v1_lifecycle) fn observe_finalization<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    let fact = match request.kind() {
        ObservationKind::ParticipantsComplete => participants_complete(backend, current)?,
        ObservationKind::Acceptance => acceptance(backend, current)?,
        ObservationKind::Publication => publication::observe(backend, context, current)?,
        _ => {
            return Err(ModelError::new(
                ErrorCode::MergePhaseUnsupported,
                "v1 finalization runtime received a non-finalization observation",
            ));
        }
    };
    BoundExactObservation::issue(current, request, fact)
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn verify_finalization_action<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    action: PublicationPhysicalAction,
) -> ModelResult<()> {
    publication::verify_action(backend, current, action)
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn verify_finalization_recovery_origin<
    B: GitBackend,
>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<()> {
    if current.record().accepted_workspace.is_none() {
        return match acceptance(backend, current) {
            Ok(_) => Ok(()),
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::MergeDrift | ErrorCode::AcceptanceInputDrift
                ) =>
            {
                Err(ModelError::new(
                    ErrorCode::RecoveryEvidenceMismatch,
                    error.message,
                ))
            }
            Err(error) => Err(error),
        };
    }
    if publication::recovery_origin_is_exact(backend, current)? {
        Ok(())
    } else {
        Err(ModelError::new(
            ErrorCode::RecoveryEvidenceMismatch,
            "live publication state does not exactly match the recorded finalizing origin",
        ))
    }
}

fn participants_complete<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    verify_metadata_path_parent(current.location().root())?;
    verify_participants(backend, current)?;
    let proof = VerifiedParticipants::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "enter_finalizing",
        "executing",
        (),
    )?;
    Ok(completed(CompletedObservation::Participants(proof)))
}

fn acceptance<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<ExactObservationFact> {
    verify_metadata_path_parent(current.location().root())?;
    verify_participants(backend, current)?;
    let record = current.record();
    let accepted = if let Some(root) = record.participants.get("@root") {
        let commit = root
            .resulting_commit
            .as_deref()
            .ok_or_else(|| acceptance_error(record, "selected root has no resulting commit"))?;
        let manifest = committed_text(
            backend,
            current.location().root(),
            commit,
            WORKSPACE_MANIFEST,
        )?;
        let lock = committed_text(backend, current.location().root(), commit, LOCK_PATH)?;
        build_v1_acceptance(
            V1AcceptanceRecord::V1(record),
            V1AcceptanceMetadata::SelectedRootResult {
                commit,
                manifest_exact_yaml: &manifest,
                lock_exact_yaml: &lock,
            },
        )?
    } else {
        verify_unselected_root_baseline(backend, current)?;
        build_v1_acceptance(
            V1AcceptanceRecord::V1(record),
            V1AcceptanceMetadata::OperationBaseline,
        )?
    };
    let proof = PreparedAcceptedWorkspace::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "freeze_acceptance",
        "prepared",
        accepted.into_accepted_workspace(),
    )?;
    Ok(completed(CompletedObservation::Acceptance(Box::new(proof))))
}

fn verify_participants<B: GitBackend>(backend: &B, current: &StoredV1Record) -> ModelResult<()> {
    verify_participant_outcomes(backend, current, true)
}

pub(super) fn verify_non_root_participants<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<()> {
    verify_participant_outcomes(backend, current, false)
}

fn verify_participant_outcomes<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
    include_root: bool,
) -> ModelResult<()> {
    let record = current.record();
    for member_id in &record.selected_targets {
        let row = record.participants.get(member_id).ok_or_else(|| {
            acceptance_error(
                record,
                &format!("selected participant '{member_id}' is missing"),
            )
        })?;
        if !include_root && (member_id == "@root" || row.target_kind == MergeTargetKind::Root) {
            continue;
        }
        if !matches!(
            row.state,
            ParticipantState::UpToDate
                | ParticipantState::FastForwarded
                | ParticipantState::Merged
                | ParticipantState::Continued
        ) || row.pending_action.is_some()
            || row.error.is_some()
        {
            return Err(member_drift(
                member_id,
                &row.path,
                "durable participant outcome is not successful and settled",
            ));
        }
        let expected = row.resulting_commit.as_deref().ok_or_else(|| {
            member_drift(member_id, &row.path, "durable resulting commit is missing")
        })?;
        let path = participant_path(
            current.location().root(),
            member_id,
            row.target_kind,
            &row.path,
        );
        let state = backend
            .repository_state(&path)
            .map_err(|error| attach(error, member_id, &row.path))?;
        let head = backend
            .head(&path)
            .map_err(|error| attach(error, member_id, &row.path))?;
        let status = backend
            .status(&path)
            .map_err(|error| attach(error, member_id, &row.path))?;
        if state != GitRepositoryState::Clean
            || head.is_detached
            || head.branch.as_deref() != Some(row.target_branch.as_str())
            || head.commit.as_deref() != Some(expected)
            || status.is_dirty
            || status.unresolved != 0
        {
            return Err(member_drift(
                member_id,
                &row.path,
                "live repository no longer exactly matches the successful outcome",
            ));
        }
    }
    Ok(())
}

pub(super) fn verify_accepted_root<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<()> {
    verify_metadata_path_parent(current.location().root())?;
    let record = current.record();
    let accepted = record
        .accepted_workspace
        .as_ref()
        .ok_or_else(|| acceptance_error(record, "accepted workspace is missing"))?;
    let root = current.location().root();
    if backend.repository_state(root)? != GitRepositoryState::Clean {
        return Err(root_drift(
            "workspace root has a native operation in progress",
        ));
    }
    let head = backend.head(root)?;
    let head_matches = match &accepted.root.base {
        AcceptedRootBaseV1::BornAttached {
            commit,
            symbolic_branch,
        } => {
            !head.is_detached
                && head.commit.as_deref() == Some(commit)
                && head.branch.as_deref() == Some(symbolic_branch)
        }
        AcceptedRootBaseV1::BornDetached { commit } => {
            head.is_detached && head.commit.as_deref() == Some(commit)
        }
        AcceptedRootBaseV1::UnbornAttached { symbolic_branch } => {
            !head.is_detached
                && head.commit.is_none()
                && head.branch.as_deref() == Some(symbolic_branch)
        }
    };
    let metadata_matches = exact_files(
        backend,
        root,
        &[
            (
                WORKSPACE_MANIFEST,
                accepted.metadata_base.manifest_exact_yaml.as_str(),
            ),
            (LOCK_PATH, accepted.metadata_base.lock_exact_yaml.as_str()),
        ],
    )?;
    if head_matches && metadata_matches {
        Ok(())
    } else {
        Err(root_drift(
            "workspace root no longer exactly matches the frozen accepted input",
        ))
    }
}

pub(super) fn verify_frozen_manifest<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<()> {
    let record = current.record();
    let accepted = record
        .accepted_workspace
        .as_ref()
        .ok_or_else(|| acceptance_error(record, "accepted workspace is missing"))?;
    if exact_files(
        backend,
        current.location().root(),
        &[(
            WORKSPACE_MANIFEST,
            accepted.metadata_base.manifest_exact_yaml.as_str(),
        )],
    )? {
        Ok(())
    } else {
        Err(root_drift(
            "workspace manifest no longer matches the frozen accepted input",
        ))
    }
}

fn verify_metadata_path_parent(root: &Path) -> ModelResult<()> {
    verify_real_directory_chains(root, &["gwz.conf"])
}

pub(super) fn verify_publication_path_parents(root: &Path) -> ModelResult<()> {
    verify_real_directory_chains(root, &["gwz.conf/markers", ".git/info"])
}

fn verify_real_directory_chains(root: &Path, relatives: &[&str]) -> ModelResult<()> {
    for relative in relatives {
        let mut current = root.to_path_buf();
        for component in Path::new(relative).components() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_dir() => {}
                Ok(_) => {
                    return Err(root_drift(&format!(
                        "GWZ-owned path parent '{}' is not a real directory",
                        current.display()
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(ModelError::new(ErrorCode::IoError, error.to_string())),
            }
        }
    }
    Ok(())
}

fn exact_files<B: GitBackend>(
    backend: &B,
    root: &Path,
    expected: &[(&str, &str)],
) -> ModelResult<bool> {
    let files = expected
        .iter()
        .map(|(path, value)| GitCandidateFile {
            path: (*path).into(),
            bytes: value.as_bytes().to_vec(),
        })
        .collect::<Vec<_>>();
    if !backend.index_entries_match_candidate_files(root, &files, &[])? {
        return Ok(false);
    }
    expected
        .iter()
        .map(|(path, value)| regular_file_equals(&root.join(path), value))
        .try_fold(true, |exact, next| next.map(|next| exact && next))
}

fn verify_unselected_root_baseline<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<()> {
    let record = current.record();
    let root = current.location().root();
    if backend.repository_state(root)? != GitRepositoryState::Clean {
        return Err(root_drift(
            "workspace root has a native operation in progress",
        ));
    }
    let head = backend.head(root)?;
    let head_matches = match (&record.baseline.root_head, &record.baseline.root_branch) {
        (Some(commit), Some(branch)) => {
            !head.is_detached
                && head.commit.as_deref() == Some(commit)
                && head.branch.as_deref() == Some(branch)
        }
        (Some(commit), None) => head.is_detached && head.commit.as_deref() == Some(commit),
        (None, Some(branch)) => {
            !head.is_detached && head.commit.is_none() && head.branch.as_deref() == Some(branch)
        }
        (None, None) => false,
    };
    let manifest =
        record.baseline.manifest_yaml.as_deref().ok_or_else(|| {
            acceptance_error(record, "operation baseline manifest bytes are missing")
        })?;
    let lock = record
        .baseline
        .lock_yaml
        .as_deref()
        .ok_or_else(|| acceptance_error(record, "operation baseline lock bytes are missing"))?;
    let files_match = exact_files(
        backend,
        root,
        &[(WORKSPACE_MANIFEST, manifest), (LOCK_PATH, lock)],
    )?;
    if head_matches && files_match {
        Ok(())
    } else {
        Err(root_drift(
            "workspace root no longer exactly matches the accepted operation baseline",
        ))
    }
}

fn committed_text<B: GitBackend>(
    backend: &B,
    root: &Path,
    commit: &str,
    relative: &str,
) -> ModelResult<String> {
    let bytes = backend
        .read_file_at_commit(root, commit, relative)?
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::AcceptanceInputDrift,
                format!("selected-root result is missing '{relative}'"),
            )
            .with_member("@root", ".")
        })?;
    String::from_utf8(bytes).map_err(|error| {
        ModelError::new(
            ErrorCode::AcceptanceInputDrift,
            format!("selected-root result '{relative}' is not UTF-8: {error}"),
        )
        .with_member("@root", ".")
    })
}

fn participant_path(root: &Path, member_id: &str, kind: MergeTargetKind, path: &str) -> PathBuf {
    if member_id == "@root" || kind == MergeTargetKind::Root {
        root.to_path_buf()
    } else {
        root.join(path)
    }
}

fn regular_file_equals(path: &Path, expected: &str) -> ModelResult<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(ModelError::new(ErrorCode::IoError, error.to_string())),
    };
    if !metadata.file_type().is_file() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 != 0 {
            return Ok(false);
        }
    }
    let bytes =
        fs::read(path).map_err(|error| ModelError::new(ErrorCode::IoError, error.to_string()))?;
    Ok(bytes == expected.as_bytes())
}

fn completed(value: CompletedObservation) -> ExactObservationFact {
    ExactObservationFact::Completed(value)
}

pub(super) fn ambiguity(current: &StoredV1Record) -> ModelResult<ExactObservationFact> {
    let origin = match current.record().state {
        OperationState::Executing => RecoveryOriginStateV1::Executing,
        OperationState::AwaitingResolution => RecoveryOriginStateV1::AwaitingResolution,
        OperationState::Halted => RecoveryOriginStateV1::Halted,
        OperationState::Finalizing => RecoveryOriginStateV1::Finalizing,
        OperationState::Preserving => RecoveryOriginStateV1::Preserving,
        OperationState::RollingBack => RecoveryOriginStateV1::RollingBack,
        OperationState::Completed | OperationState::Aborted | OperationState::RecoveryRequired => {
            return Err(authority_error(
                "terminal/recovery state has no new ambiguity origin",
            ));
        }
    };
    let proof = BoundAmbiguityEvidence::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "enter_recovery",
        "ambiguous",
        origin,
    )?;
    Ok(ExactObservationFact::Ambiguous(proof))
}

fn acceptance_error(record: &MergeOperationRecordV1, detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::AcceptanceInputDrift,
        format!("merge '{}' acceptance rejected: {detail}", record.merge_id),
    )
}

fn member_drift(member_id: &str, path: &str, detail: &str) -> ModelError {
    ModelError::new(ErrorCode::MergeDrift, detail).with_member(member_id, path)
}

fn root_drift(detail: &str) -> ModelError {
    ModelError::new(ErrorCode::MergeDrift, detail).with_member("@root", ".")
}

fn attach(mut error: ModelError, member_id: &str, path: &str) -> ModelError {
    if error.member_id.is_none() {
        error = error.with_member(member_id, path);
    }
    error
}
