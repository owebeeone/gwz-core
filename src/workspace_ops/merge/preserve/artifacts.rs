use super::*;

pub(super) fn verify_root_publication(
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<()> {
    let Some(publication) = record.publication.as_ref() else {
        return Ok(());
    };
    if publication.candidate.is_none() {
        return Ok(());
    }
    let prefix = super::super::publication::classify_candidate_publication(root, record)?
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeDrift,
                "workspace root candidate artifacts changed before preservation",
            )
            .with_member("@root", ".")
        })?;
    if !super::super::publication::publication_prefix_allowed(record, prefix)? {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "workspace root candidate artifacts do not match the recorded publication step",
        )
        .with_member("@root", "."));
    }
    Ok(())
}

pub(super) fn classify_index_aligned_root_publication<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> ModelResult<Option<super::super::publication::CandidatePublicationPrefix>> {
    let Some(prefix) = super::super::publication::classify_candidate_publication(root, record)?
    else {
        return Ok(None);
    };
    let mut expected_files = super::super::publication::candidate_files(record)?;
    let marker_path = expected_files
        .get(1)
        .ok_or_else(|| unreadable_root("root publication marker candidate is missing"))?
        .path
        .clone();
    let candidate = super::super::publication::candidate(record)?;
    let mut absent_paths = Vec::new();
    if matches!(
        prefix,
        super::super::publication::CandidatePublicationPrefix::Baseline
            | super::super::publication::CandidatePublicationPrefix::Marker
    ) {
        expected_files[0].bytes = candidate.baseline_lock_yaml.as_bytes().to_vec();
    }
    if prefix == super::super::publication::CandidatePublicationPrefix::Baseline {
        expected_files.pop();
        absent_paths.push(marker_path);
    }
    let candidate_paths = expected_files
        .iter()
        .map(|file| file.path.clone())
        .chain(absent_paths.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    let exact_index = backend
        .index_matches_candidate_files(root, &expected_files, &absent_paths)
        .map_err(|error| attach_member(error, "@root", "."))?;
    let expected_boundary =
        if prefix == super::super::publication::CandidatePublicationPrefix::Boundary {
            &candidate.boundary_text
        } else {
            &candidate.baseline_boundary_text
        };
    let exact_boundary = boundary_file_matches(root, expected_boundary)?;
    let status = backend
        .status(root)
        .map_err(|error| attach_member(error, "@root", "."))?;
    let index_matches_worktree = status.files.iter().all(|file| {
        let candidate_path = candidate_paths.contains(&file.path)
            || file
                .original_path
                .as_ref()
                .is_some_and(|path| candidate_paths.contains(path));
        !candidate_path || file.worktree_status == " "
    });
    Ok((exact_index && exact_boundary && index_matches_worktree).then_some(prefix))
}

fn unreadable_root(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message).with_member("@root", ".")
}

fn boundary_file_matches(root: &Path, expected: &str) -> ModelResult<bool> {
    let path = super::super::super::workspace_exclude_path(root);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(expected.is_empty());
        }
        Err(error) => {
            return Err(ModelError::new(
                ErrorCode::IoError,
                format!("failed to inspect workspace boundary file: {error}"),
            )
            .with_member("@root", "."));
        }
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
    std::fs::read(&path)
        .map(|bytes| bytes == expected.as_bytes())
        .map_err(|error| {
            ModelError::new(
                ErrorCode::IoError,
                format!("failed to read workspace boundary file: {error}"),
            )
            .with_member("@root", ".")
        })
}

pub(super) fn verify_artifacts<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    plans: &[PreservationPlan],
) -> ModelResult<()> {
    verify_root_publication(root, record)?;
    let prefix = format!("gwz:stash_{}:", record.merge_id);
    for plan in plans {
        let Some(evidence) = existing_evidence(record, plan)? else {
            continue;
        };
        if let (Some(name), Some(expected)) = (
            evidence.backup_ref.as_deref(),
            evidence.backup_commit.as_deref(),
        ) {
            let observed = member_result(backend.read_ref(&plan.path, name), plan)?;
            if observed.as_deref() != Some(expected) {
                return Err(drift(plan, "preservation ref failed verification"));
            }
        }
        if let Some(expected) = evidence.stash_object_id.as_deref()
            && !member_result(backend.stash_list(&plan.path), plan)?
                .iter()
                .any(|entry| entry.object_id == expected && entry.message.contains(&prefix))
        {
            return Err(drift(plan, "preservation stash failed verification"));
        }
    }
    Ok(())
}

pub(super) fn prepare_root_for_stash<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    plan: &PreservationPlan,
) -> ModelResult<()> {
    let files = super::super::publication::candidate_files(record)?;
    for file in &files {
        let contents = String::from_utf8(file.bytes.clone()).map_err(|error| {
            unreadable(
                plan,
                format!("root publication candidate is not UTF-8: {error}"),
            )
        })?;
        crate::artifact::write_atomic(&root.join(&file.path), contents)?;
    }
    let paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();
    member_result(backend.stage_paths(root, &paths), plan).map(|_| ())
}

pub(super) fn restore_root_publication<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    plan: &PreservationPlan,
    expected_prefix: super::super::publication::CandidatePublicationPrefix,
) -> ModelResult<()> {
    use super::super::publication::CandidatePublicationPrefix;

    let publication = record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .ok_or_else(|| unreadable(plan, "root publication candidate disappeared"))?;
    let marker_path = crate::artifact::marker_path(root, &publication.marker_id);
    let baseline_lock = matches!(
        expected_prefix,
        CandidatePublicationPrefix::Baseline | CandidatePublicationPrefix::Marker
    );
    crate::artifact::write_atomic(
        &root.join(crate::artifact::LOCK_PATH),
        if baseline_lock {
            &publication.baseline_lock_yaml
        } else {
            &publication.lock_yaml
        },
    )?;
    if expected_prefix == CandidatePublicationPrefix::Baseline {
        match std::fs::remove_file(&marker_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(attach_member(
                    ModelError::new(
                        ErrorCode::IoError,
                        format!("failed to restore root publication marker: {error}"),
                    ),
                    &plan.target_id,
                    &plan.relative_path,
                ));
            }
        }
    } else {
        crate::artifact::write_atomic(&marker_path, &publication.marker_yaml)?;
    }
    let boundary = if expected_prefix == CandidatePublicationPrefix::Boundary {
        &publication.boundary_text
    } else {
        &publication.baseline_boundary_text
    };
    super::super::super::publish_workspace_exclude_candidate(root, boundary)?;
    let marker_relative = format!(
        "{}/{}.yaml",
        crate::artifact::MARKER_DIR,
        publication.marker_id
    );
    member_result(
        backend.stage_paths(root, &[crate::artifact::LOCK_PATH, &marker_relative]),
        plan,
    )?;
    if classify_index_aligned_root_publication(backend, root, record)? != Some(expected_prefix) {
        return Err(drift(
            plan,
            "root publication artifacts were not restored after preservation",
        ));
    }
    Ok(())
}

pub(super) fn restore_root_publication_from_record<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    expected_prefix: super::super::publication::CandidatePublicationPrefix,
) -> ModelResult<()> {
    let candidate = record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "root preservation candidate disappeared",
            )
            .with_member("@root", ".")
        })?;
    let anchor = record
        .publication
        .as_ref()
        .and_then(|publication| publication.composition_commit.clone())
        .or_else(|| record.baseline.root_head.clone())
        .unwrap_or_default();
    let plan = PreservationPlan {
        target_id: "@root".to_owned(),
        path: root.to_path_buf(),
        relative_path: ".".to_owned(),
        target_branch: candidate.root_branch.clone(),
        anchor: anchor.clone(),
        live_commit: anchor,
        backup_ref: format!("refs/gwz/merge/{}/root/head", record.merge_id),
        status: GitStatus::clean(),
        preserve_commit: false,
        preserve_worktree: false,
        root_publication_prefix: Some(expected_prefix),
        evidence_owner: EvidenceOwner::PublicationRoot,
    };
    restore_root_publication(backend, root, record, &plan, expected_prefix)
}

pub(super) fn persist_stash_bundle(
    root: &Path,
    record: &MergeOperationRecord,
    plan: &PreservationPlan,
    message: &str,
    object_id: &str,
) -> ModelResult<()> {
    let stash_id = format!("stash_{}", record.merge_id);
    let mut bundle = if crate::stash::bundle_path(root, &stash_id).is_file() {
        crate::stash::read_bundle(root, &stash_id)?
    } else {
        StashBundle {
            schema: STASH_BUNDLE_SCHEMA.to_owned(),
            workspace_id: record.workspace_id.clone(),
            stash_id: stash_id.clone(),
            created_at: record.created_at.clone(),
            message_suffix: "merge preservation".to_owned(),
            include_untracked: true,
            include_ignored: false,
            selected_members: Vec::new(),
            members: Vec::new(),
            warnings: Vec::new(),
            drift: Vec::new(),
        }
    };
    if bundle.workspace_id != record.workspace_id
        || bundle.message_suffix != "merge preservation"
        || !bundle.include_untracked
        || bundle.include_ignored
    {
        return Err(drift(plan, "preservation stash bundle identity changed"));
    }
    let row = StashBundleMember {
        member_id: plan.target_id.clone(),
        path: plan.relative_path.clone(),
        participation: StashParticipation::Stashed,
        push_lifecycle: StashPushLifecycle::Saved,
        restore_state: StashRestoreState::Pending,
        branch_before: Some(plan.target_branch.clone()),
        head_before: Some(plan.live_commit.clone()),
        full_stash_message: message.to_owned(),
        dirty_summary: StashDirtySummary {
            staged: plan.status.staged > 0,
            unstaged: plan.status.unstaged > 0,
            untracked: plan.status.untracked > 0,
            ignored: false,
        },
        native_stash_object_id: Some(object_id.to_owned()),
        native_stash_display_ref: None,
        error: None,
    };
    if let Some(existing) = bundle
        .members
        .iter_mut()
        .find(|member| member.member_id == plan.target_id)
    {
        *existing = row;
    } else {
        bundle.selected_members.push(plan.target_id.clone());
        bundle.members.push(row);
    }
    crate::stash::write_bundle(root, &bundle)
}

pub(super) fn update_evidence(
    record: &mut MergeOperationRecord,
    plan: &PreservationPlan,
    update: impl FnOnce(&mut PreservationEvidence),
) -> ModelResult<()> {
    let evidence = match plan.evidence_owner {
        EvidenceOwner::Participant => {
            &mut record
                .participants
                .get_mut(&plan.target_id)
                .ok_or_else(|| unreadable(plan, "participant disappeared from durable record"))?
                .preservation
        }
        EvidenceOwner::PublicationRoot => {
            &mut record
                .publication
                .as_mut()
                .ok_or_else(|| {
                    unreadable(plan, "root publication disappeared from durable record")
                })?
                .root_preservation
        }
    };
    if evidence.is_empty() {
        evidence.push(PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: None,
            stash_object_id: None,
        });
    }
    if evidence.len() != 1 {
        return Err(unreadable(
            plan,
            "preservation owner has multiple evidence rows",
        ));
    }
    update(&mut evidence[0]);
    Ok(())
}

pub(super) fn existing_evidence<'a>(
    record: &'a MergeOperationRecord,
    plan: &PreservationPlan,
) -> ModelResult<Option<&'a PreservationEvidence>> {
    let evidence = match plan.evidence_owner {
        EvidenceOwner::Participant => {
            &record
                .participants
                .get(&plan.target_id)
                .ok_or_else(|| {
                    ModelError::new(
                        ErrorCode::MergeRecordUnreadable,
                        format!("merge record is missing participant '{}'", plan.target_id),
                    )
                })?
                .preservation
        }
        EvidenceOwner::PublicationRoot => {
            &record
                .publication
                .as_ref()
                .ok_or_else(|| {
                    unreadable(plan, "root publication disappeared from durable record")
                })?
                .root_preservation
        }
    };
    match evidence.as_slice() {
        [] => Ok(None),
        [evidence] => Ok(Some(evidence)),
        _ => Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "preservation owner has multiple evidence rows",
        )
        .with_member(&plan.target_id, &plan.relative_path)),
    }
}

pub(super) fn validate_evidence_shape(
    plan: &PreservationPlan,
    evidence: &PreservationEvidence,
    stash_id: &str,
) -> ModelResult<()> {
    if evidence.backup_ref.is_some() != evidence.backup_commit.is_some()
        || evidence.stash_id.is_some() != evidence.stash_object_id.is_some()
    {
        return Err(unreadable(plan, "preservation evidence is incomplete"));
    }
    if evidence
        .backup_ref
        .as_deref()
        .is_some_and(|name| name != plan.backup_ref)
        || evidence
            .stash_id
            .as_deref()
            .is_some_and(|id| id != stash_id)
    {
        return Err(unreadable(plan, "preservation evidence identity changed"));
    }
    Ok(())
}

pub(super) fn member_result<T>(result: ModelResult<T>, plan: &PreservationPlan) -> ModelResult<T> {
    result.map_err(|error| attach_member(error, &plan.target_id, &plan.relative_path))
}

pub(super) fn attach_member(mut error: ModelError, target_id: &str, path: &str) -> ModelError {
    if error.member_id.is_none() {
        error.member_id = Some(target_id.to_owned());
        error.member_path = Some(path.to_owned());
    }
    error
}

pub(super) fn drift(plan: &PreservationPlan, message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeDrift, message)
        .with_member(&plan.target_id, &plan.relative_path)
}

pub(super) fn unreadable(plan: &PreservationPlan, message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, message)
        .with_member(&plan.target_id, &plan.relative_path)
}

#[cfg(test)]
pub(in crate::workspace_ops::merge) fn v1_root_preservation_spec<B: GitBackend>(
    backend: &B,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    plan: &super::plan::V1PreservationOwnerPlan,
    attached_commit: &str,
) -> ModelResult<Option<crate::git::GitRootPreservationSpec>> {
    use crate::git::{
        GitCandidateFile, GitRootManagedForm, GitRootManagedIndexForm, GitRootPreservationSpec,
    };
    use crate::workspace_ops::merge::model::v1::{
        PublicationIndexFormV1 as I, PublicationPrefixV1 as P,
    };

    let Some(handoff) = plan.root_handoff else {
        return Ok(None);
    };
    let publication = record
        .publication
        .as_ref()
        .ok_or_else(|| v1_error(plan, "root handoff has no publication progress"))?;
    let candidate = publication
        .candidate
        .as_ref()
        .ok_or_else(|| v1_error(plan, "root handoff has no publication candidate"))?;
    let marker_path = publication
        .candidate_marker_path
        .as_ref()
        .ok_or_else(|| v1_error(plan, "root handoff has no managed marker path"))?;
    let attached = clean_form(backend, plan, attached_commit, marker_path)?;
    let restore = clean_form(backend, plan, &plan.anchor, marker_path)?;
    let marker = (!matches!(handoff.prefix, P::Baseline)).then(|| GitCandidateFile {
        path: marker_path.clone(),
        bytes: candidate.marker_yaml.as_bytes().to_vec(),
    });
    let lock_bytes = if matches!(handoff.prefix, P::Lock | P::Boundary) {
        candidate.lock_yaml.as_bytes()
    } else {
        candidate.baseline_lock_yaml.as_bytes()
    };
    let lock = GitCandidateFile {
        path: crate::artifact::LOCK_PATH.into(),
        bytes: lock_bytes.to_vec(),
    };
    let index = match handoff.index {
        I::Pre => {
            let baseline_lock = GitCandidateFile {
                path: crate::artifact::LOCK_PATH.into(),
                bytes: candidate.baseline_lock_yaml.as_bytes().to_vec(),
            };
            GitRootManagedIndexForm {
                marker: managed_fact(attached_commit, marker_path, None)?,
                lock: managed_fact(
                    attached_commit,
                    crate::artifact::LOCK_PATH,
                    Some(&baseline_lock),
                )?,
            }
        }
        I::Staged => GitRootManagedIndexForm {
            marker: managed_fact(attached_commit, marker_path, marker.as_ref())?,
            lock: managed_fact(attached_commit, crate::artifact::LOCK_PATH, Some(&lock))?,
        },
    };
    let boundary = if handoff.prefix == P::Boundary {
        candidate.boundary_text.as_bytes()
    } else {
        candidate.baseline_boundary_text.as_bytes()
    };
    let handoff_form = GitRootManagedForm {
        marker,
        lock,
        index,
    };
    let mut excluded_worktree_paths =
        crate::artifact::ManifestArtifact::from_yaml(
            record.baseline.manifest_yaml.as_deref().ok_or_else(|| {
                v1_error(plan, "root preservation baseline has no manifest bytes")
            })?,
        )?
        .members
        .into_iter()
        .map(|member| member.path)
        .collect::<Vec<_>>();
    // The operation journal and preservation artifacts mutate while this
    // preimage is being consumed. They are control-plane state, never user
    // work, even when an older publication boundary did not ignore `.gwz`.
    excluded_worktree_paths.push(".gwz".into());
    let spec = GitRootPreservationSpec {
        attached_branch: plan.branch.clone(),
        attached_commit: attached_commit.into(),
        restore_commit: plan.anchor.clone(),
        managed_marker_path: marker_path.clone(),
        attached_clean_form: attached,
        restore_clean_form: restore,
        handoff_form,
        handoff_boundary: boundary.to_vec(),
        excluded_worktree_paths,
    };
    Ok(Some(spec))
}

#[cfg(test)]
fn clean_form<B: GitBackend>(
    backend: &B,
    plan: &super::plan::V1PreservationOwnerPlan,
    commit: &str,
    marker_path: &str,
) -> ModelResult<crate::git::GitRootManagedForm> {
    use crate::git::{GitCandidateFile, GitRootManagedForm, GitRootManagedIndexForm};
    let marker = backend
        .read_file_at_commit(&plan.path, commit, marker_path)
        .map_err(|error| attach_v1(error, plan))?
        .map(|bytes| GitCandidateFile {
            path: marker_path.into(),
            bytes,
        });
    let lock = backend
        .read_file_at_commit(&plan.path, commit, crate::artifact::LOCK_PATH)
        .map_err(|error| attach_v1(error, plan))?
        .map(|bytes| GitCandidateFile {
            path: crate::artifact::LOCK_PATH.into(),
            bytes,
        })
        .ok_or_else(|| v1_error(plan, "root clean commit has no managed lock file"))?;
    Ok(GitRootManagedForm {
        index: GitRootManagedIndexForm {
            marker: managed_fact(commit, marker_path, marker.as_ref())?,
            lock: managed_fact(commit, crate::artifact::LOCK_PATH, Some(&lock))?,
        },
        marker,
        lock,
    })
}

#[cfg(test)]
fn managed_fact(
    commit: &str,
    path: &str,
    file: Option<&crate::git::GitCandidateFile>,
) -> ModelResult<crate::git::GitRootManagedIndexFact> {
    use crate::git::{GitRootManagedIndexEntry, GitRootManagedIndexFact};
    Ok(match file {
        None => GitRootManagedIndexFact::Absent {
            path: path.as_bytes().to_vec(),
        },
        Some(file) => GitRootManagedIndexFact::Present(GitRootManagedIndexEntry {
            path: path.as_bytes().to_vec(),
            object_id: blob_oid(commit, &file.bytes)?,
            mode: 0o100644,
            stage: 0,
            assume_valid: false,
            skip_worktree: false,
            intent_to_add: false,
        }),
    })
}

#[cfg(test)]
fn blob_oid(commit: &str, bytes: &[u8]) -> ModelResult<String> {
    use sha1::Sha1;
    use sha2::{Digest, Sha256};
    let mut input = format!("blob {}\0", bytes.len()).into_bytes();
    input.extend_from_slice(bytes);
    match commit.len() {
        40 => Ok(format!("{:x}", Sha1::digest(input))),
        64 => Ok(format!("{:x}", Sha256::digest(input))),
        _ => Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "root preservation commit has an unsupported object-id width",
        )),
    }
}

#[cfg(test)]
pub(in crate::workspace_ops::merge) fn v1_preservation_image<B: GitBackend>(
    backend: &B,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    plan: &super::plan::V1PreservationOwnerPlan,
    attached_commit: &str,
) -> ModelResult<crate::git::GitPreservationImage> {
    match v1_root_preservation_spec(backend, record, plan, attached_commit)? {
        Some(spec) => backend
            .prepare_root_preservation_stash(&plan.path, &spec)
            .map(|prepared| prepared.normalized_image)
            .map_err(|error| attach_v1(error, plan)),
        None => backend
            .preservation_image(&plan.path, true)
            .map_err(|error| attach_v1(error, plan)),
    }
}

#[cfg(test)]
pub(super) fn expected_bundle<B: GitBackend>(
    backend: &B,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    plans: &[super::plan::V1PreservationOwnerPlan],
) -> ModelResult<StashBundle> {
    let stash_id = format!("stash_{}", record.merge_id);
    let mut selected_members = Vec::new();
    let mut members = Vec::new();
    for plan in plans {
        let Some(evidence) = super::plan::v1_owner_evidence(record, &plan.owner)? else {
            continue;
        };
        let Some(expected_oid) = evidence.stash_object_id.as_deref() else {
            continue;
        };
        let stashes = backend
            .preservation_stashes(&plan.path, &record.merge_id)
            .map_err(|error| attach_v1(error, plan))?;
        let [stash] = stashes.as_slice() else {
            return Err(v1_error(
                plan,
                "bundle source stash is missing or duplicated",
            ));
        };
        if stash.object_id != expected_oid
            || evidence.stash_id.as_deref() != Some(stash_id.as_str())
            || stash.head_commit != plan.protected_commit
            || stash.message != format!("gwz:{stash_id}: merge preservation")
            || stash.image.dirty == crate::git::GitPreservationDirtySummary::default()
        {
            return Err(v1_error(
                plan,
                "bundle source stash does not match durable preservation evidence",
            ));
        }
        selected_members.push(plan.target_id.clone());
        members.push(StashBundleMember {
            member_id: plan.target_id.clone(),
            path: plan.relative_path.clone(),
            participation: StashParticipation::Stashed,
            push_lifecycle: StashPushLifecycle::Saved,
            restore_state: StashRestoreState::Pending,
            branch_before: Some(plan.branch.clone()),
            head_before: Some(stash.head_commit.clone()),
            full_stash_message: stash.message.clone(),
            dirty_summary: StashDirtySummary {
                staged: stash.image.dirty.staged,
                unstaged: stash.image.dirty.unstaged,
                untracked: stash.image.dirty.untracked,
                ignored: false,
            },
            native_stash_object_id: Some(stash.object_id.clone()),
            native_stash_display_ref: None,
            error: None,
        });
    }
    members.sort_by(|left, right| left.member_id.cmp(&right.member_id));
    selected_members.sort();
    Ok(StashBundle {
        schema: STASH_BUNDLE_SCHEMA.into(),
        workspace_id: record.workspace_id.clone(),
        stash_id,
        created_at: record.created_at.clone(),
        message_suffix: "merge preservation".into(),
        include_untracked: true,
        include_ignored: false,
        selected_members,
        members,
        warnings: Vec::new(),
        drift: Vec::new(),
    })
}

#[cfg(test)]
pub(super) fn attach_v1(
    mut error: ModelError,
    plan: &super::plan::V1PreservationOwnerPlan,
) -> ModelError {
    if error.member_id.is_none() {
        error.member_id = Some(plan.target_id.clone());
        error.member_path = Some(plan.relative_path.clone());
    }
    error
}

#[cfg(test)]
pub(super) fn v1_error(
    plan: &super::plan::V1PreservationOwnerPlan,
    detail: impl Into<String>,
) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
        .with_member(&plan.target_id, &plan.relative_path)
}
