//! Native implementation of the fixture-only GitRepository methods.
use super::*;

pub(super) fn test_init_repo(_: &Git2Backend, path: &Path, spec: &TestRepoSpec) -> ModelResult<()> {
    let mut options = git2::RepositoryInitOptions::new();
    options
        .no_reinit(true)
        .mkpath(true)
        .external_template(false)
        .bare(spec.bare)
        .initial_head(&spec.branch)
        .object_format(if spec.sha256 {
            git2::ObjectFormat::Sha256
        } else {
            git2::ObjectFormat::Sha1
        });
    let repo = git2::Repository::init_opts(path, &options).map_err(git_error)?;
    let mut config = repo.config().map_err(git_error)?;
    for (key, value) in [
        ("user.name", "GWZ Test"),
        ("user.email", "gwz@example.invalid"),
        ("core.autocrlf", "false"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("gc.auto", "0"),
    ] {
        config.set_str(key, value).map_err(git_error)?;
    }
    let absent = repo.path().join("gwz-fixture-absent");
    for key in ["core.hooksPath", "core.excludesFile", "core.attributesFile"] {
        config
            .set_str(key, &absent.to_string_lossy())
            .map_err(git_error)?;
    }
    std::fs::create_dir_all(repo.path().join("info")).map_err(io_error)?;
    std::fs::write(repo.path().join("info/exclude"), "").map_err(io_error)?;
    Ok(())
}
fn signature(value: &GitPreparedSignature) -> ModelResult<git2::Signature<'static>> {
    git2::Signature::new(
        &value.name,
        &value.email,
        &git2::Time::new(value.time_seconds, value.timezone_offset_minutes),
    )
    .map_err(git_error)
}
pub(super) fn test_create_commit(
    _: &Git2Backend,
    path: &Path,
    spec: &TestCommitSpec,
) -> ModelResult<String> {
    let repo = open_repo(path)?;
    let mut index = match &spec.tree {
        TestCommitTree::Index => repo.index().map_err(git_error)?,
        TestCommitTree::Files(files) => {
            let mut index = git2::Index::new().map_err(git_error)?;
            for file in files {
                let id = repo.blob(&file.bytes).map_err(git_error)?;
                index
                    .add(&native_entry(&TestIndexEntry {
                        path: file.path.as_bytes().to_vec(),
                        object_id: id.to_string(),
                        mode: 0o100644,
                        stage: 0,
                        assume_valid: false,
                        skip_worktree: false,
                        intent_to_add: false,
                    })?)
                    .map_err(git_error)?;
            }
            index
        }
    };
    if index.has_conflicts() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "cannot commit unresolved index",
        ));
    }
    let tree = repo
        .find_tree(index.write_tree_to(&repo).map_err(git_error)?)
        .map_err(git_error)?;
    let parents = spec
        .parents
        .iter()
        .map(|oid| {
            repo.find_commit(git2::Oid::from_str(oid).map_err(git_error)?)
                .map_err(git_error)
        })
        .collect::<ModelResult<Vec<_>>>()?;
    let oid = repo
        .commit(
            None,
            &signature(&spec.author)?,
            &signature(&spec.committer)?,
            &spec.message,
            &tree,
            &parents.iter().collect::<Vec<_>>(),
        )
        .map_err(git_error)?;
    Ok(oid.to_string())
}
pub(super) fn test_read_commit(_: &Git2Backend, path: &Path, oid: &str) -> ModelResult<TestCommit> {
    let repo = open_repo(path)?;
    let commit = repo
        .find_commit(git2::Oid::from_str(oid).map_err(git_error)?)
        .map_err(git_error)?;
    let signature = |s: git2::Signature<'_>| GitPreparedSignature {
        name: s.name().unwrap_or_default().into(),
        email: s.email().unwrap_or_default().into(),
        time_seconds: s.when().seconds(),
        timezone_offset_minutes: s.when().offset_minutes(),
    };
    Ok(TestCommit {
        tree: commit.tree_id().to_string(),
        parents: commit.parent_ids().map(|oid| oid.to_string()).collect(),
        message: String::from_utf8(commit.message_bytes().to_vec())
            .map_err(|e| ModelError::new(ErrorCode::InvalidRequest, e.to_string()))?,
        author: signature(commit.author()),
        committer: signature(commit.committer()),
    })
}
pub(super) fn test_set_ref(
    _: &Git2Backend,
    path: &Path,
    name: &str,
    target: Option<&TestRefTarget>,
) -> ModelResult<()> {
    let repo = open_repo(path)?;
    match target {
        Some(TestRefTarget::Direct(oid)) => {
            repo.reference(
                name,
                git2::Oid::from_str(oid).map_err(git_error)?,
                true,
                "test fixture",
            )
            .map_err(git_error)?;
        }
        Some(TestRefTarget::Symbolic(target)) => {
            repo.reference_symbolic(name, target, true, "test fixture")
                .map_err(git_error)?;
        }
        None => match repo.find_reference(name) {
            Ok(mut value) => value.delete().map_err(git_error)?,
            Err(e) if e.code() == git2::ErrorCode::NotFound => (),
            Err(e) => return Err(git_error(e)),
        },
    }
    Ok(())
}
pub(super) fn test_set_head(_: &Git2Backend, path: &Path, state: &TestHead) -> ModelResult<()> {
    let repo = open_repo(path)?;
    match state {
        TestHead::Attached(name) => repo.set_head(name),
        TestHead::Detached(oid) => {
            repo.set_head_detached(git2::Oid::from_str(oid).map_err(git_error)?)
        }
    }
    .map_err(git_error)
}
fn native_entry(entry: &TestIndexEntry) -> ModelResult<git2::IndexEntry> {
    Ok(git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: entry.mode,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: git2::Oid::from_str(&entry.object_id).map_err(git_error)?,
        flags: (u16::from(entry.stage) << 12) | if entry.assume_valid { 0x8000 } else { 0 },
        flags_extended: (if entry.skip_worktree { 0x4000 } else { 0 })
            | if entry.intent_to_add { 0x2000 } else { 0 },
        path: entry.path.clone(),
    })
}
pub(super) fn test_replace_index(
    _: &Git2Backend,
    path: &Path,
    entries: &[TestIndexEntry],
) -> ModelResult<()> {
    let repo = open_repo(path)?;
    let mut index = repo.index().map_err(git_error)?;
    let native = entries
        .iter()
        .map(native_entry)
        .collect::<ModelResult<Vec<_>>>()?;
    index.clear().map_err(git_error)?;
    for entry in native {
        index.add(&entry).map_err(git_error)?;
    }
    index.write().map_err(git_error)
}
pub(super) fn test_read_index(_: &Git2Backend, path: &Path) -> ModelResult<Vec<TestIndexEntry>> {
    let repo = open_repo(path)?;
    Ok(repo
        .index()
        .map_err(git_error)?
        .iter()
        .map(|entry| TestIndexEntry {
            path: entry.path,
            object_id: entry.id.to_string(),
            mode: entry.mode,
            stage: ((entry.flags >> 12) & 3) as u8,
            assume_valid: entry.flags & 0x8000 != 0,
            skip_worktree: entry.flags_extended & 0x4000 != 0,
            intent_to_add: entry.flags_extended & 0x2000 != 0,
        })
        .collect())
}
pub(super) fn test_set_config(
    _: &Git2Backend,
    path: &Path,
    key: &str,
    values: &[String],
) -> ModelResult<()> {
    let repo = open_repo(path)?;
    let mut config = repo
        .config()
        .map_err(git_error)?
        .open_level(git2::ConfigLevel::Local)
        .map_err(git_error)?;
    match config.remove_multivar(key, ".*") {
        Ok(()) => (),
        Err(e) if e.code() == git2::ErrorCode::NotFound => (),
        Err(e) => return Err(git_error(e)),
    }
    for value in values {
        config.set_multivar(key, "a^", value).map_err(git_error)?;
    }
    Ok(())
}
pub(super) fn test_read_config(
    _: &Git2Backend,
    path: &Path,
    key: &str,
) -> ModelResult<Vec<String>> {
    let repo = open_repo(path)?;
    let config = repo
        .config()
        .map_err(git_error)?
        .open_level(git2::ConfigLevel::Local)
        .map_err(git_error)?;
    let mut values = Vec::new();
    let mut entries = match config.multivar(key, None) {
        Ok(entries) => entries,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(values),
        Err(e) => return Err(git_error(e)),
    };
    while let Some(entry) = entries.next() {
        values.push(
            entry
                .map_err(git_error)?
                .value()
                .map_err(git_error)?
                .to_owned(),
        );
    }
    Ok(values)
}

pub(super) fn test_force_checkout(
    _backend: &Git2Backend,
    path: &Path,
    commit: &str,
) -> ModelResult<()> {
    let repo = open_repo(path)?;
    let object = repo
        .find_object(commit.parse().map_err(git_error)?, None)
        .map_err(git_error)?;
    repo.reset(&object, git2::ResetType::Hard, None)
        .map_err(git_error)
}

pub(super) fn test_reset_mixed(
    _backend: &Git2Backend,
    path: &Path,
    commit: &str,
) -> ModelResult<()> {
    let repo = open_repo(path)?;
    let object = repo
        .find_object(commit.parse().map_err(git_error)?, None)
        .map_err(git_error)?;
    repo.reset(&object, git2::ResetType::Mixed, None)
        .map_err(git_error)
}

pub(super) fn test_set_repository_state(
    _backend: &Git2Backend,
    path: &Path,
    state: GitRepositoryState,
    merge_head: Option<&str>,
) -> ModelResult<()> {
    if state != GitRepositoryState::Merge {
        return Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "native test repository state fixture supports only merge state",
        ));
    }
    let repo = open_repo(path)?;
    let head = merge_head.ok_or_else(|| {
        ModelError::new(ErrorCode::InvalidRequest, "merge state requires MERGE_HEAD")
    })?;
    std::fs::write(repo.path().join("MERGE_HEAD"), format!("{head}\n")).map_err(io_error)?;
    std::fs::write(repo.path().join("MERGE_MSG"), "GWZ test merge\n").map_err(io_error)
}

pub(super) fn test_seed_merge_conflict(
    backend: &Git2Backend,
    path: &Path,
    before: &str,
    source: &str,
) -> ModelResult<GitMergeConflictSnapshot> {
    let result = backend.merge_upstream_checked(path, "main", before, source, "merge", None)?;
    if result.commit.is_some() || result.conflicts.is_empty() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "test merge did not produce a conflict",
        ));
    }
    backend.merge_conflict_snapshot(path, before, source)
}

pub(super) fn test_create_commit_from_parent(
    _backend: &Git2Backend,
    path: &Path,
    parent: &str,
    message: &str,
    edits: &[TestCommitFileEdit],
) -> ModelResult<String> {
    let repo = open_repo(path)?;
    let parent = repo
        .find_commit(parent.parse().map_err(git_error)?)
        .map_err(git_error)?;
    let tree = parent.tree().map_err(git_error)?;
    let mut index = git2::Index::new().map_err(git_error)?;
    index.read_tree(&tree).map_err(git_error)?;
    for edit in edits {
        match &edit.bytes {
            Some(bytes) => {
                let object_id = repo.blob(bytes).map_err(git_error)?;
                index
                    .add(&native_entry(&TestIndexEntry {
                        path: edit.path.as_bytes().to_vec(),
                        object_id: object_id.to_string(),
                        mode: 0o100644,
                        stage: 0,
                        assume_valid: false,
                        skip_worktree: false,
                        intent_to_add: false,
                    })?)
                    .map_err(git_error)?;
            }
            None => index
                .remove_path(Path::new(&edit.path))
                .map_err(git_error)?,
        }
    }
    let tree_id = index.write_tree_to(&repo).map_err(git_error)?;
    let tree = repo.find_tree(tree_id).map_err(git_error)?;
    let spec = TestCommitSpec::from_index(message, vec![parent.id().to_string()]);
    let author = signature(&spec.author)?;
    let committer = signature(&spec.committer)?;
    repo.commit(None, &author, &committer, message, &tree, &[&parent])
        .map(|oid| oid.to_string())
        .map_err(git_error)
}
