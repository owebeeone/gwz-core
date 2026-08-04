use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::artifact::LOCK_PATH;
use crate::git::{GitBackend, GitCandidateFile};
use crate::workspace::WORKSPACE_MANIFEST;
use crate::workspace_ops::merge::MergeOperationRecord;

pub(super) fn recover_exact_baseline<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> Option<MergeOperationRecord> {
    if persisted_bytes_are_invalid(record) {
        return None;
    }
    if baseline_is_complete(record) {
        return Some(record.clone());
    }
    recover_from_selected_root(backend, root, record)
        .or_else(|| recover_from_candidate_and_manifest(backend, root, record))
        .or_else(|| recover_from_exact_live_baseline(backend, root, record))
}

fn recover_from_selected_root<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> Option<MergeOperationRecord> {
    let participant = record.participants.get("@root")?;
    let lock = backend
        .read_file_at_commit(root, &participant.before_commit, LOCK_PATH)
        .ok()??;
    let manifest = backend
        .read_file_at_commit(root, &participant.before_commit, WORKSPACE_MANIFEST)
        .ok()??;
    if !matches_all_digests(
        &lock,
        &record.baseline.lock_sha256,
        record.baseline.lock_commit_sha256.as_deref(),
    ) || !matches_all_digests(
        &manifest,
        &record.baseline.manifest_sha256,
        record.baseline.manifest_commit_sha256.as_deref(),
    ) {
        return None;
    }
    fill_missing(record, lock, manifest)
}

fn recover_from_candidate_and_manifest<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> Option<MergeOperationRecord> {
    if record.participants.contains_key("@root") {
        return None;
    }
    let candidate = record.publication.as_ref()?.candidate.as_ref()?;
    let lock = candidate.baseline_lock_yaml.as_bytes().to_vec();
    let manifest = fs::read(root.join(WORKSPACE_MANIFEST)).ok()?;
    if digest(&lock) != record.baseline.lock_sha256
        || digest(&manifest) != record.baseline.manifest_sha256
        || !index_matches(backend, root, &[(WORKSPACE_MANIFEST, &manifest)])
    {
        return None;
    }
    fill_missing(record, lock, manifest)
}

fn recover_from_exact_live_baseline<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> Option<MergeOperationRecord> {
    if root_evidence_was_mutated(record) {
        return None;
    }
    let head = backend.head(root).ok()?;
    if head.is_detached
        || head.commit != record.baseline.root_head
        || head.branch != record.baseline.root_branch
    {
        return None;
    }
    let lock = fs::read(root.join(LOCK_PATH)).ok()?;
    let manifest = fs::read(root.join(WORKSPACE_MANIFEST)).ok()?;
    if digest(&lock) != record.baseline.lock_sha256
        || digest(&manifest) != record.baseline.manifest_sha256
        || !index_matches(
            backend,
            root,
            &[(LOCK_PATH, &lock), (WORKSPACE_MANIFEST, &manifest)],
        )
    {
        return None;
    }
    fill_missing(record, lock, manifest)
}

fn root_evidence_was_mutated(record: &MergeOperationRecord) -> bool {
    record.publication.as_ref().is_some_and(|publication| {
        publication.composition_commit.is_some()
            || publication.composition_tree.is_some()
            || !publication.candidate_hashes.is_empty()
            || publication.root_merge_commit.is_some()
            || publication.evidence_rolled_back
    })
}

fn index_matches<B: GitBackend>(backend: &B, root: &Path, files: &[(&str, &[u8])]) -> bool {
    let expected = files
        .iter()
        .map(|(path, bytes)| GitCandidateFile {
            path: (*path).to_owned(),
            bytes: (*bytes).to_vec(),
        })
        .collect::<Vec<_>>();
    backend
        .index_matches_candidate_files(root, &expected, &[])
        .unwrap_or(false)
}

fn fill_missing(
    record: &MergeOperationRecord,
    lock: Vec<u8>,
    manifest: Vec<u8>,
) -> Option<MergeOperationRecord> {
    let mut recovered = record.clone();
    if recovered.baseline.lock_yaml.is_none() {
        recovered.baseline.lock_yaml = Some(String::from_utf8(lock).ok()?);
    }
    if recovered.baseline.manifest_yaml.is_none() {
        recovered.baseline.manifest_yaml = Some(String::from_utf8(manifest).ok()?);
    }
    Some(recovered)
}

fn baseline_is_complete(record: &MergeOperationRecord) -> bool {
    record.baseline.lock_yaml.is_some() && record.baseline.manifest_yaml.is_some()
}

fn persisted_bytes_are_invalid(record: &MergeOperationRecord) -> bool {
    record
        .baseline
        .lock_yaml
        .as_deref()
        .is_some_and(|value| digest(value.as_bytes()) != record.baseline.lock_sha256)
        || record
            .baseline
            .manifest_yaml
            .as_deref()
            .is_some_and(|value| digest(value.as_bytes()) != record.baseline.manifest_sha256)
}

fn matches_all_digests(bytes: &[u8], worktree: &str, committed: Option<&str>) -> bool {
    let actual = digest(bytes);
    actual == worktree && committed.is_none_or(|expected| actual == expected)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
