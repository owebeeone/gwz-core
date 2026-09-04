//! The v1 reverse path's preservation artifacts.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** The v0 preserve-then-abort engine's
//! artifact half — `verify_root_publication`, `restore_root_publication`,
//! `persist_stash_bundle` and their neighbours — left with the engine. What
//! remains is the v1 preservation observation the reverse service drives.

use super::*;

pub(in crate::workspace_ops::merge) fn v1_root_preservation_spec<
    B: crate::git::MergeAuthorityBackend,
>(
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

#[forbid(clippy::disallowed_methods)]
fn clean_form<B: crate::git::MergeAuthorityBackend>(
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

#[forbid(clippy::disallowed_methods)]
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

#[forbid(clippy::disallowed_methods)]
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

// Counts live preservation-image captures for the §8.2(a) acceptance suite of
// `GwzM5-8DurableCursorAmendment.md`, which requires the durable-marker path
// proven image-capture-free for earlier owners. `MergeAuthorityBackend` is a
// sealed trait, so a counting backend cannot be written outside `git`; this
// counter sits at the real capture seam instead.
//
// Thread-local because the test harness runs suites in parallel and each test
// drives its whole operation on one thread — a process-global counter would be
// raced by concurrent suites. Test-only: the function it guards is itself
// `#[cfg(test)]`.
thread_local! {
    pub(in crate::workspace_ops::merge) static V1_PRESERVATION_IMAGE_CAPTURES:
        std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[forbid(clippy::disallowed_methods)]
pub(in crate::workspace_ops::merge) fn v1_preservation_image<
    B: crate::git::MergeAuthorityBackend,
>(
    backend: &B,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    plan: &super::plan::V1PreservationOwnerPlan,
    attached_commit: &str,
) -> ModelResult<crate::git::GitPreservationImage> {
    V1_PRESERVATION_IMAGE_CAPTURES.with(|count| count.set(count.get() + 1));
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

#[forbid(clippy::disallowed_methods)]
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

#[forbid(clippy::disallowed_methods)]
pub(super) fn v1_error(
    plan: &super::plan::V1PreservationOwnerPlan,
    detail: impl Into<String>,
) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
        .with_member(&plan.target_id, &plan.relative_path)
}
