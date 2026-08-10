use super::super::*;
use super::{FaultBoundary, fault};

use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalEntry {
    path: Vec<u8>,
    object_id: String,
    mode: u32,
    stage: u8,
    assume_valid: bool,
    extended_flags: u16,
}

pub(super) fn validate_spec(root: &Path, spec: &GitRootPreservationSpec) -> ModelResult<()> {
    let repo = open_repo(root)?;
    let marker = spec.managed_marker_path.as_str();
    let prefix = format!("{}/", crate::artifact::MARKER_DIR);
    let leaf = marker
        .strip_prefix(&prefix)
        .filter(|leaf| !leaf.is_empty() && !leaf.contains('/') && leaf.ends_with(".yaml"))
        .ok_or_else(|| invalid("managed marker path is not canonical"))?;
    if matches!(leaf, "." | "..")
        || !git2::Reference::is_valid_name(&format!("refs/heads/{}", spec.attached_branch))
    {
        return Err(invalid("managed marker path or attached branch is invalid"));
    }
    for form in [
        &spec.attached_clean_form,
        &spec.restore_clean_form,
        &spec.handoff_form,
    ] {
        if form.marker.as_ref().is_some_and(|file| file.path != marker)
            || form.lock.path != crate::artifact::LOCK_PATH
        {
            return Err(invalid("managed form contains a second managed path"));
        }
        validate_fact(&repo, &form.index.marker, marker.as_bytes())?;
        validate_fact(
            &repo,
            &form.index.lock,
            crate::artifact::LOCK_PATH.as_bytes(),
        )?;
    }
    let attached = derive_clean_form(root, &spec.attached_commit, marker)?;
    let restore = derive_clean_form(root, &spec.restore_commit, marker)?;
    if attached != spec.attached_clean_form || restore != spec.restore_clean_form {
        return Err(evidence_error(
            "supplied clean form does not match its exact commit tree",
        ));
    }
    Ok(())
}

pub(super) fn derive_clean_form(
    root: &Path,
    commit: &str,
    marker_path: &str,
) -> ModelResult<GitRootManagedForm> {
    let repo = open_repo(root)?;
    let oid = exact_commit(&repo, commit)?;
    let tree = repo
        .find_commit(oid)
        .and_then(|commit| commit.tree())
        .map_err(git_error)?;
    validate_marker_tree(&repo, &tree, marker_path)?;
    let marker = tree_file(&repo, &tree, marker_path, false)?;
    let lock = tree_file(&repo, &tree, crate::artifact::LOCK_PATH, true)?
        .expect("required tree file was checked");
    let marker_index = marker.as_ref().map_or_else(
        || {
            Ok(GitRootManagedIndexFact::Absent {
                path: marker_path.as_bytes().to_vec(),
            })
        },
        |file| present_fact(&repo, file),
    )?;
    let lock_index = present_fact(&repo, &lock)?;
    Ok(GitRootManagedForm {
        marker,
        lock,
        index: GitRootManagedIndexForm {
            marker: marker_index,
            lock: lock_index,
        },
    })
}

fn validate_marker_tree(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
    marker_path: &str,
) -> ModelResult<()> {
    let expected = marker_path
        .strip_prefix(&format!("{}/", crate::artifact::MARKER_DIR))
        .ok_or_else(|| evidence_error("managed marker path is outside its canonical directory"))?;
    let entry = match tree.get_path(Path::new(crate::artifact::MARKER_DIR)) {
        Ok(entry) => entry,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(()),
        Err(error) => return Err(git_error(error)),
    };
    if entry.kind() != Some(git2::ObjectType::Tree) {
        return Err(evidence_error("commit marker directory is not a tree"));
    }
    let markers = repo.find_tree(entry.id()).map_err(git_error)?;
    if markers
        .iter()
        .any(|entry| entry.name_bytes() != expected.as_bytes())
    {
        return Err(evidence_error(
            "commit marker directory contains an unexpected entry",
        ));
    }
    Ok(())
}

fn exact_commit(repo: &git2::Repository, value: &str) -> ModelResult<git2::Oid> {
    let oid = parse_exact_oid(repo, value, "commit")?;
    repo.find_commit(oid).map_err(git_error)?;
    Ok(oid)
}

fn tree_file(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
    path: &str,
    required: bool,
) -> ModelResult<Option<GitCandidateFile>> {
    let entry = match tree.get_path(Path::new(path)) {
        Ok(entry) => entry,
        Err(error) if error.code() == git2::ErrorCode::NotFound && !required => return Ok(None),
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            return Err(evidence_error(format!(
                "commit tree is missing managed path '{path}'"
            )));
        }
        Err(error) => return Err(git_error(error)),
    };
    if entry.kind() != Some(git2::ObjectType::Blob) || entry.filemode_raw() != 0o100644 {
        return Err(evidence_error(format!(
            "commit tree managed path '{path}' is not a regular non-executable file"
        )));
    }
    let bytes = repo
        .find_blob(entry.id())
        .map_err(git_error)?
        .content()
        .to_vec();
    Ok(Some(GitCandidateFile {
        path: path.to_owned(),
        bytes,
    }))
}

fn present_fact(
    repo: &git2::Repository,
    file: &GitCandidateFile,
) -> ModelResult<GitRootManagedIndexFact> {
    let oid = git2::Oid::hash_object_ext(git2::ObjectType::Blob, &file.bytes, repo.object_format())
        .map_err(git_error)?;
    Ok(GitRootManagedIndexFact::Present(GitRootManagedIndexEntry {
        path: file.path.as_bytes().to_vec(),
        object_id: oid.to_string(),
        mode: 0o100644,
        stage: 0,
        assume_valid: false,
        skip_worktree: false,
        intent_to_add: false,
    }))
}

pub(super) fn observe(root: &Path, expected: &GitRootManagedIndexForm) -> ModelResult<bool> {
    let repo = open_repo(root)?;
    let index = super::index_format::read(&repo)?;
    validate_marker_namespace(&index, fact_path(&expected.marker))?;
    Ok(observe_fact(&index, &expected.marker)? && observe_fact(&index, &expected.lock)?)
}

fn validate_marker_namespace(
    index: &super::index_format::RawIndex,
    selected: &[u8],
) -> ModelResult<()> {
    let directory = crate::artifact::MARKER_DIR.as_bytes();
    if index.entries.iter().any(|entry| {
        entry.path != selected
            && (entry.path == directory
                || entry
                    .path
                    .strip_prefix(directory)
                    .is_some_and(|suffix| suffix.starts_with(b"/")))
    }) {
        return Err(evidence_error(
            "Git index contains an unowned marker-directory entry",
        ));
    }
    Ok(())
}

fn observe_fact(
    index: &super::index_format::RawIndex,
    expected: &GitRootManagedIndexFact,
) -> ModelResult<bool> {
    let path = fact_path(expected);
    let entries = index
        .entries
        .iter()
        .filter(|entry| entry.path == path)
        .collect::<Vec<_>>();
    match expected {
        GitRootManagedIndexFact::Absent { .. } => Ok(entries.is_empty()),
        GitRootManagedIndexFact::Present(expected) => {
            let Some(entry) = entries.first() else {
                return Ok(false);
            };
            Ok(entries.len() == 1
                && entry.object_id == expected.object_id
                && entry.mode == expected.mode
                && entry.stage == expected.stage
                && entry.flags & 0x8000 == u16::from(expected.assume_valid) << 15
                && entry.extended_flags == 0)
        }
    }
}

pub(super) fn rewrite(root: &Path, goal: &GitRootManagedIndexForm) -> ModelResult<()> {
    let repo = open_repo(root)?;
    let raw_before = super::index_format::read(&repo)?;
    validate_marker_namespace(&raw_before, fact_path(&goal.marker))?;
    let mut index = repo.index().map_err(git_error)?;
    let managed = [fact_path(&goal.marker), fact_path(&goal.lock)];
    let before = unrelated_entries(&raw_before, &managed);
    for fact in [&goal.marker, &goal.lock] {
        let path = std::str::from_utf8(fact_path(fact))
            .map_err(|_| evidence_error("managed index path is not UTF-8"))?;
        index.remove_path(Path::new(path)).map_err(git_error)?;
        if let GitRootManagedIndexFact::Present(entry) = fact {
            add_entry(&repo, &mut index, entry)?;
        }
    }
    fault(FaultBoundary::BeforeIndexCommit)?;
    index.write().map_err(git_error)?;
    fault(FaultBoundary::AfterIndexCommit)?;
    let verified = super::index_format::read(&repo)?;
    validate_marker_namespace(&verified, fact_path(&goal.marker))?;
    super::index_format::require_managed_tree_invalidation(&verified, &managed)?;
    if unrelated_entries(&verified, &managed) != before
        || !observe_fact(&verified, &goal.marker)?
        || !observe_fact(&verified, &goal.lock)?
    {
        return Err(evidence_error(
            "managed index rewrite failed semantic post-verification",
        ));
    }
    Ok(())
}

fn add_entry(
    repo: &git2::Repository,
    index: &mut git2::Index,
    entry: &GitRootManagedIndexEntry,
) -> ModelResult<()> {
    let oid = parse_exact_oid(repo, &entry.object_id, "managed index object")?;
    let blob = repo.find_blob(oid).map_err(git_error)?;
    let file_size = u32::try_from(blob.size())
        .map_err(|_| evidence_error("managed blob is too large for the index"))?;
    index
        .add(&git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: entry.mode,
            uid: 0,
            gid: 0,
            file_size,
            id: oid,
            flags: 0,
            flags_extended: 0,
            path: entry.path.clone(),
        })
        .map_err(git_error)
}

fn unrelated_entries(
    index: &super::index_format::RawIndex,
    managed: &[&[u8]],
) -> BTreeMap<(Vec<u8>, u8), CanonicalEntry> {
    index
        .entries
        .iter()
        .filter(|entry| !managed.contains(&entry.path.as_slice()))
        .map(|entry| {
            let stage = entry.stage;
            (
                (entry.path.clone(), stage),
                CanonicalEntry {
                    path: entry.path.clone(),
                    object_id: entry.object_id.clone(),
                    mode: entry.mode,
                    stage,
                    assume_valid: entry.flags & 0x8000 != 0,
                    extended_flags: entry.extended_flags,
                },
            )
        })
        .collect()
}

pub(super) fn validate_fact(
    repo: &git2::Repository,
    fact: &GitRootManagedIndexFact,
    expected_path: &[u8],
) -> ModelResult<()> {
    if fact_path(fact) != expected_path {
        return Err(invalid("managed index fact uses the wrong path"));
    }
    if let GitRootManagedIndexFact::Present(entry) = fact
        && (entry.stage != 0
            || entry.mode != 0o100644
            || entry.assume_valid
            || entry.skip_worktree
            || entry.intent_to_add
            || parse_exact_oid(repo, &entry.object_id, "managed index object").is_err())
    {
        return Err(invalid("managed index fact is not canonical stage zero"));
    }
    if let GitRootManagedIndexFact::Present(entry) = fact {
        let oid = parse_exact_oid(repo, &entry.object_id, "managed index object")?;
        repo.find_blob(oid).map_err(git_error)?;
    }
    Ok(())
}

pub(in crate::git::gitbackend) fn parse_exact_oid(
    repo: &git2::Repository,
    value: &str,
    kind: &str,
) -> ModelResult<git2::Oid> {
    let width = match repo.object_format() {
        git2::ObjectFormat::Sha1 => 40,
        git2::ObjectFormat::Sha256 => 64,
    };
    if value.len() != width
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(evidence_error(format!(
            "{kind} id is not complete lowercase {}",
            repo.object_format()
        )));
    }
    let oid = git2::Oid::from_str_ext(value, repo.object_format())
        .map_err(|_| evidence_error(format!("invalid {kind} id")))?;
    if oid.to_string() != value {
        return Err(evidence_error(format!("{kind} id is not canonical")));
    }
    Ok(oid)
}

pub(in crate::git::gitbackend) fn fact_path(fact: &GitRootManagedIndexFact) -> &[u8] {
    match fact {
        GitRootManagedIndexFact::Absent { path } => path,
        GitRootManagedIndexFact::Present(entry) => &entry.path,
    }
}

fn evidence_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
}

fn invalid(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, detail.into())
}
