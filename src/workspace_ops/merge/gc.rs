use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OperationContext, WorkspaceMutatorLock};

use super::{MergeOperationRecord, MergeStore};

struct BackupArtifact {
    target_id: String,
    relative_path: String,
    path: PathBuf,
    name: String,
    commit: String,
}

pub(super) fn handle_gc<B: GitBackend, S: MergeStore>(
    backend: &B,
    store: &S,
    root: &Path,
    merge_id: Option<&str>,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let _guard = WorkspaceMutatorLock::acquire(root)?;
    if let Some(open) = store.discover_open(root)? {
        return Err(ModelError::new(
            ErrorCode::OpenOperation,
            format!(
                "cannot collect archived merge records while merge '{}' is open",
                open.merge_id
            ),
        ));
    }
    let Some(merge_id) = merge_id else {
        store.gc(root, None)?;
        return super::response::idle_status_response(context);
    };
    let record = store.load_archived(root, merge_id)?;
    let artifacts = preflight_backup_artifacts(backend, root, &record)?;
    let response = post_gc_record(record).to_response(context)?;
    for artifact in artifacts {
        backend
            .delete_backup_ref_checked(&artifact.path, &artifact.name, &artifact.commit)
            .map_err(|error| attach_member(error, &artifact.target_id, &artifact.relative_path))?;
    }
    store.gc(root, Some(merge_id))?;
    Ok(response)
}

fn preflight_backup_artifacts<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Vec<BackupArtifact>> {
    if record.state.is_open() {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!(
                "cannot collect open merge '{}' in state {:?}",
                record.merge_id, record.state
            ),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut artifacts = Vec::new();
    for (target_id, participant) in &record.participants {
        let path = super::status::validated_participant_path(root, target_id, participant)?;
        let key = if participant.target_kind == super::MergeTargetKind::Root {
            "root"
        } else {
            target_id
        };
        preflight_owner_evidence(
            backend,
            record,
            target_id,
            &participant.path,
            key,
            &path,
            &participant.preservation,
            &mut seen,
            &mut artifacts,
        )?;
    }
    if let Some(publication) = record.publication.as_ref() {
        preflight_owner_evidence(
            backend,
            record,
            "@root",
            ".",
            "root",
            root,
            &publication.root_preservation,
            &mut seen,
            &mut artifacts,
        )?;
    }
    Ok(artifacts)
}

#[allow(clippy::too_many_arguments)]
fn preflight_owner_evidence<B: GitBackend>(
    backend: &B,
    record: &MergeOperationRecord,
    target_id: &str,
    relative_path: &str,
    target_key: &str,
    path: &Path,
    evidence_rows: &[super::PreservationEvidence],
    seen: &mut BTreeSet<(String, String)>,
    artifacts: &mut Vec<BackupArtifact>,
) -> ModelResult<()> {
    if evidence_rows.len() > 1 {
        return Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "preservation owner has multiple evidence rows",
        )
        .with_member(target_id, relative_path));
    }
    for evidence in evidence_rows {
        if evidence.backup_ref.is_some() != evidence.backup_commit.is_some()
            || evidence.stash_id.is_some() != evidence.stash_object_id.is_some()
        {
            return Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "preservation evidence is incomplete",
            )
            .with_member(target_id, relative_path));
        }
        let (Some(name), Some(commit)) = (
            evidence.backup_ref.as_ref(),
            evidence.backup_commit.as_ref(),
        ) else {
            continue;
        };
        if commit.len() != 40
            || !commit
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "preservation backup commit is not a canonical Git object id",
            )
            .with_member(target_id, relative_path));
        }
        let expected = format!("refs/gwz/merge/{}/{target_key}/head", record.merge_id);
        if name != &expected {
            return Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!(
                    "preservation ref '{name}' is not owned by merge '{}' target '{target_id}'",
                    record.merge_id
                ),
            )
            .with_member(target_id, relative_path));
        }
        if !seen.insert((relative_path.to_owned(), name.clone())) {
            return Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "duplicate preservation backup ref evidence",
            )
            .with_member(target_id, relative_path));
        }
        let observed = backend
            .read_ref(path, name)
            .map_err(|error| attach_member(error, target_id, relative_path))?;
        if observed.as_deref().is_some_and(|actual| actual != commit) {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                format!("preservation ref '{name}' no longer points to recorded commit '{commit}'"),
            )
            .with_member(target_id, relative_path));
        }
        artifacts.push(BackupArtifact {
            target_id: target_id.to_owned(),
            relative_path: relative_path.to_owned(),
            path: path.to_path_buf(),
            name: name.clone(),
            commit: commit.clone(),
        });
    }
    Ok(())
}

fn post_gc_record(mut record: MergeOperationRecord) -> MergeOperationRecord {
    for participant in record.participants.values_mut() {
        retain_remaining_stashes(&mut participant.preservation);
    }
    if let Some(publication) = record.publication.as_mut() {
        retain_remaining_stashes(&mut publication.root_preservation);
    }
    record
}

fn retain_remaining_stashes(evidence: &mut Vec<super::PreservationEvidence>) {
    for row in evidence.iter_mut() {
        row.backup_ref = None;
        row.backup_commit = None;
    }
    evidence.retain(|row| row.stash_id.is_some() && row.stash_object_id.is_some());
}

fn attach_member(mut error: ModelError, target_id: &str, path: &str) -> ModelError {
    if error.member_id.is_none() {
        error.member_id = Some(target_id.to_owned());
        error.member_path = Some(path.to_owned());
    }
    error
}
