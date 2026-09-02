use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::git::{GitBackend, GitDirectRefObservation};
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

pub(super) struct PreparedArchivedCleanup {
    artifacts: Vec<ArchivedBackupArtifact>,
}

struct ArchivedBackupArtifact {
    target_id: String,
    relative_path: String,
    path: PathBuf,
    name: String,
    commit: String,
    delete: bool,
}

/// [2026-09-02, R2-E E4.7: RE-REASONED. This allowance carried a reason
/// byte-identical to `v1_lifecycle/archive.rs`'s and was named by no authority
/// at all — `GwzM5-8R2E-CapabilityFreeAmendment.md` §5's three-name extent of
/// the `gc_archived` family misses it. It covers the family's downstream half:
/// `preflight_archived_cleanup`, `delete_preflighted_backup_refs`,
/// `require_backup_refs_absent` and `PreparedArchivedCleanup`, whose ONLY
/// callers are `archive.rs`'s `gc_archived_with_hook`. MEASURED at E4.7: this
/// allow is today redundant — `archive.rs`'s own allowance seeds the family's
/// liveness, so removing this one alone leaves `clippy -D warnings` green — but
/// it is KEPT, because it is the record that this half of the family travels
/// with the other, and because deleting the family is DR-1's choice, not
/// E4.7's (the O13 shrinkage arm; see `archive.rs`'s note).]
#[allow(
    dead_code,
    reason = "PERMANENT PENDING DR-1: the checked archive route this family \
              was built for has no consumer to arrive — the archive is carved \
              out (dev-docs/GwzM5-8R2E-CapabilityFreeAmendment.md §3, ADOPTED \
              2026-09-02) and E4.4 does not start (§7). O8's gc_archived route \
              RE-OWNS to DR-1, conditional on (C) resurrecting the archive \
              conversion (§5). The live GC deletion path is store/gc.rs and \
              store/retention.rs, which this family is NOT."
)]
pub(super) fn preflight_archived_cleanup<B: GitBackend>(
    backend: &B,
    root: &Path,
    merge_id: &str,
    cleanup: &super::record_wire::ArchivedCleanupWorklist,
) -> ModelResult<PreparedArchivedCleanup> {
    let root = root
        .canonicalize()
        .map_err(|error| ModelError::new(ErrorCode::IoError, error.to_string()))?;
    let mut artifacts = Vec::with_capacity(cleanup.backup_refs().len());
    let mut seen = BTreeSet::new();
    for owner in cleanup.backup_refs() {
        let relative = Path::new(owner.path());
        let is_root = relative == Path::new(".");
        let path = if is_root {
            root.clone()
        } else {
            let member_path = crate::workspace::MemberPath::parse(owner.path()).map_err(|_| {
                cleanup_error(
                    owner.target_id(),
                    owner.path(),
                    ErrorCode::ArchivedRecordUnreadable,
                    "archive cleanup owner path is not canonical",
                )
            })?;
            root.join(member_path.as_str())
        };
        let canonical = path.canonicalize().map_err(|error| {
            attach_member(
                ModelError::new(ErrorCode::IoError, error.to_string()),
                owner.target_id(),
                owner.path(),
            )
        })?;
        if canonical != path || !canonical.starts_with(&root) {
            return Err(cleanup_error(
                owner.target_id(),
                owner.path(),
                ErrorCode::ArchivedRecordUnreadable,
                "archive cleanup owner repository is not a canonical workspace path",
            ));
        }
        let key = if is_root { "root" } else { owner.target_id() };
        let expected_name = format!("refs/gwz/merge/{merge_id}/{key}/head");
        if owner.name() != expected_name
            || !matches!(owner.target_commit().len(), 40 | 64)
            || !owner
                .target_commit()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !seen.insert((path.clone(), owner.name().to_owned()))
        {
            return Err(cleanup_error(
                owner.target_id(),
                owner.path(),
                ErrorCode::ArchivedRecordUnreadable,
                "archive cleanup ref identity is not canonical and unique",
            ));
        }
        let delete = preflight_direct_ref(
            backend,
            &path,
            owner.name(),
            owner.target_commit(),
            owner.target_id(),
            owner.path(),
        )?;
        artifacts.push(ArchivedBackupArtifact {
            target_id: owner.target_id().to_owned(),
            relative_path: owner.path().to_owned(),
            path,
            name: owner.name().to_owned(),
            commit: owner.target_commit().to_owned(),
            delete,
        });
    }
    Ok(PreparedArchivedCleanup { artifacts })
}

pub(super) fn delete_preflighted_backup_refs<B: GitBackend>(
    backend: &B,
    prepared: &PreparedArchivedCleanup,
) -> ModelResult<()> {
    for artifact in prepared.artifacts.iter().filter(|artifact| artifact.delete) {
        backend
            .delete_backup_ref_checked(&artifact.path, &artifact.name, &artifact.commit)
            .map_err(|error| attach_member(error, &artifact.target_id, &artifact.relative_path))?;
    }
    Ok(())
}

pub(super) fn require_backup_refs_absent<B: GitBackend>(
    backend: &B,
    prepared: &PreparedArchivedCleanup,
) -> ModelResult<()> {
    for artifact in &prepared.artifacts {
        match backend
            .observe_direct_ref(&artifact.path, &artifact.name)
            .map_err(|error| attach_member(error, &artifact.target_id, &artifact.relative_path))?
        {
            GitDirectRefObservation::Absent => {}
            GitDirectRefObservation::Direct { .. } | GitDirectRefObservation::NonDirect => {
                return Err(ModelError::new(
                    ErrorCode::MergeDrift,
                    format!(
                        "preservation ref '{}' reappeared during archive cleanup",
                        artifact.name
                    ),
                )
                .with_member(&artifact.target_id, &artifact.relative_path));
            }
        }
    }
    Ok(())
}

fn cleanup_error(
    target_id: &str,
    path: &str,
    code: ErrorCode,
    detail: impl Into<String>,
) -> ModelError {
    ModelError::new(code, detail).with_member(target_id, path)
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
    let locations = super::record_wire::acquire_canonical_merge_locations(root, merge_id)?;
    let (_, bytes, _) = locations.archived().exact().ok_or_else(|| {
        ModelError::new(
            ErrorCode::OperationNotFound,
            format!("archived merge record '{merge_id}' was not found"),
        )
    })?;
    let (_, _, record) = super::record_wire::decode_archived_common(bytes.as_slice())
        .map_err(|_| archived_record_unreadable(merge_id))?;
    let artifacts = preflight_backup_artifacts(backend, root, &record)?;
    let archived = super::record_wire::decode_archived(bytes.as_slice(), merge_id)?;
    let response = super::response::attach_archived_record_projection(
        post_gc_record(record).to_response(context)?,
        merge_id,
        archived.projection(),
    )?;
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
        preflight_direct_ref(backend, path, name, commit, target_id, relative_path)?;
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

fn preflight_direct_ref<B: GitBackend>(
    backend: &B,
    path: &Path,
    name: &str,
    expected_target: &str,
    target_id: &str,
    relative_path: &str,
) -> ModelResult<bool> {
    let observation = backend
        .observe_direct_ref(path, name)
        .map_err(|error| attach_member(error, target_id, relative_path))?;
    match observation {
        GitDirectRefObservation::Absent => Ok(false),
        GitDirectRefObservation::Direct { target } if target == expected_target => Ok(true),
        GitDirectRefObservation::Direct { .. } => Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!(
                "preservation ref '{name}' no longer points to recorded commit '{expected_target}'"
            ),
        )
        .with_member(target_id, relative_path)),
        GitDirectRefObservation::NonDirect => Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!("preservation ref '{name}' is not a direct ref"),
        )
        .with_member(target_id, relative_path)),
    }
}

/// Shapes the GC **response projection** only — there is no post-GC durable
/// record rewrite (the archive is deleted at the `store.gc` call above). The
/// durable-cursor amendment's "post-GC record rewrite" phrasing is an erratum;
/// this is the retention edge its §2.2 terminal-plane fate actually rides.
///
/// Visibility is module-internal so the §8.6 acceptance pin can drive marker
/// rows through it directly; no behavior changes.
pub(in crate::workspace_ops::merge) fn post_gc_record(
    mut record: MergeOperationRecord,
) -> MergeOperationRecord {
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

fn archived_record_unreadable(merge_id: &str) -> ModelError {
    ModelError::new(
        ErrorCode::ArchivedRecordUnreadable,
        format!("archived merge record '{merge_id}' is unreadable"),
    )
}
