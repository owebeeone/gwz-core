use super::*;

pub(super) fn test_init_repo(
    backend: &FakeGitRepository,
    path: &Path,
    spec: &TestRepoSpec,
) -> ModelResult<()> {
    if !git2::Reference::is_valid_name(&format!("refs/heads/{}", spec.branch)) {
        return Err(failed("invalid initial branch"));
    }
    if spec.bare {
        return unsupported("bare fixture repository");
    }
    backend.create_repo(path)?;
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .unwrap();
    repo.attached_ref = Some(format!("refs/heads/{}", spec.branch));
    repo.sha256 = spec.sha256;
    backend
        .filesystem
        .as_ref()
        .create_directories(&path.join(".git/info"))
        .map_err(|e| failed(e.to_string()))?;
    write_worktree_file(
        backend.filesystem.as_ref(),
        &path.join(".git/info/exclude"),
        b"",
    )?;
    Ok(())
}
pub(super) fn object_id(sha256: bool, kind: git2::ObjectType, bytes: &[u8]) -> ModelResult<String> {
    git2::Oid::hash_object_ext(
        kind,
        bytes,
        if sha256 {
            git2::ObjectFormat::Sha256
        } else {
            git2::ObjectFormat::Sha1
        },
    )
    .map(|oid| oid.to_string())
    .map_err(git_error)
}
pub(super) fn tree_id(
    sha256: bool,
    tree: &FileTree,
    blobs: &mut BTreeMap<String, Vec<u8>>,
) -> ModelResult<String> {
    fn build(
        sha256: bool,
        files: &FileTree,
        prefix: &str,
        blobs: &mut BTreeMap<String, Vec<u8>>,
    ) -> ModelResult<String> {
        let mut entries = BTreeMap::<String, (bool, String)>::new();
        for (name, bytes) in files {
            let Some(relative) = name.strip_prefix(prefix) else {
                continue;
            };
            if let Some((directory, _)) = relative.split_once('/') {
                let key = format!("{directory}/");
                if let std::collections::btree_map::Entry::Vacant(entry) = entries.entry(key) {
                    entry.insert((
                        true,
                        build(sha256, files, &format!("{prefix}{directory}/"), blobs)?,
                    ));
                }
            } else {
                let oid = object_id(sha256, git2::ObjectType::Blob, bytes)?;
                blobs.insert(oid.clone(), bytes.clone());
                entries.insert(relative.into(), (false, oid));
            }
        }
        let mut raw = Vec::new();
        for (name, (directory, oid)) in entries {
            raw.extend_from_slice(if directory { b"40000 " } else { b"100644 " });
            raw.extend_from_slice(name.trim_end_matches('/').as_bytes());
            raw.push(0);
            raw.extend_from_slice(git2::Oid::from_str(&oid).map_err(git_error)?.as_bytes());
        }
        object_id(sha256, git2::ObjectType::Tree, &raw)
    }
    build(sha256, tree, "", blobs)
}
fn signature(s: &GitPreparedSignature) -> String {
    let offset = s.timezone_offset_minutes;
    format!(
        "{} <{}> {} {}{:02}{:02}",
        s.name,
        s.email,
        s.time_seconds,
        if offset < 0 { '-' } else { '+' },
        offset.abs() / 60,
        offset.abs() % 60
    )
}
pub(super) fn store_commit(
    repo: &mut RepositoryState,
    tree: FileTree,
    spec: &TestCommitSpec,
) -> ModelResult<String> {
    for parent in &spec.parents {
        if !repo.commits.contains_key(parent) {
            return Err(failed("parent commit missing"));
        }
    }
    let tree_oid = tree_id(repo.sha256, &tree, &mut repo.blobs)?;
    let mut raw = format!("tree {tree_oid}\n");
    for parent in &spec.parents {
        raw.push_str(&format!("parent {parent}\n"));
    }
    raw.push_str(&format!(
        "author {}\ncommitter {}\n\n{}",
        signature(&spec.author),
        signature(&spec.committer),
        spec.message
    ));
    let oid = object_id(repo.sha256, git2::ObjectType::Commit, raw.as_bytes())?;
    repo.commits.insert(oid.clone(), tree);
    repo.parents
        .insert(oid.clone(), spec.parents.first().cloned());
    repo.metadata.insert(
        oid.clone(),
        TestCommit {
            tree: tree_oid,
            parents: spec.parents.clone(),
            message: spec.message.clone(),
            author: spec.author.clone(),
            committer: spec.committer.clone(),
        },
    );
    Ok(oid)
}
pub(super) fn test_create_commit(
    backend: &FakeGitRepository,
    path: &Path,
    spec: &TestCommitSpec,
) -> ModelResult<String> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    if matches!(spec.tree, TestCommitTree::Index)
        && repo
            .index_override
            .as_ref()
            .is_some_and(|entries| entries.iter().any(|entry| entry.stage != 0))
    {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "cannot commit unresolved index",
        ));
    }
    if matches!(spec.tree, TestCommitTree::Index)
        && repo.index_override.as_ref().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.mode != 0o100644 || entry.intent_to_add)
        })
    {
        return unsupported("commit non-regular fixture index");
    }
    let tree = match &spec.tree {
        TestCommitTree::Index => repo.index.clone(),
        TestCommitTree::Files(files) => files
            .iter()
            .map(|file| (file.path.clone(), file.bytes.clone()))
            .collect(),
    };
    store_commit(repo, tree, spec)
}
pub(super) fn test_read_commit(
    backend: &FakeGitRepository,
    path: &Path,
    oid: &str,
) -> ModelResult<TestCommit> {
    backend
        .repositories
        .lock()
        .unwrap()
        .get(&repository_key(backend.filesystem.as_ref(), path))
        .and_then(|repo| repo.metadata.get(oid))
        .cloned()
        .ok_or_else(|| failed("commit missing"))
}
pub(super) fn test_set_ref(
    backend: &FakeGitRepository,
    path: &Path,
    name: &str,
    target: Option<&TestRefTarget>,
) -> ModelResult<()> {
    if !git2::Reference::is_valid_name(name) {
        return Err(ModelError::new(ErrorCode::InvalidRequest, "invalid ref"));
    }
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    match target {
        Some(TestRefTarget::Direct(oid)) => {
            if !repo.commits.contains_key(oid) && !repo.blobs.contains_key(oid) {
                return Err(failed("ref target missing"));
            }
            repo.symbolic_refs.remove(name);
            repo.refs.insert(name.into(), oid.clone());
        }
        Some(TestRefTarget::Symbolic(target)) => {
            if !git2::Reference::is_valid_name(target) {
                return Err(failed("invalid symbolic target"));
            }
            repo.refs.remove(name);
            repo.symbolic_refs.insert(name.into(), target.clone());
        }
        None => {
            repo.refs.remove(name);
            repo.symbolic_refs.remove(name);
        }
    }
    if !repo.detached {
        repo.head = resolve_ref(
            repo,
            repo.attached_ref.as_deref().unwrap_or("refs/heads/main"),
        )?;
    }
    Ok(())
}
pub(super) fn resolve_ref(repo: &RepositoryState, name: &str) -> ModelResult<Option<String>> {
    let mut seen = BTreeSet::new();
    let mut current = name;
    while let Some(target) = repo.symbolic_refs.get(current) {
        if !seen.insert(current) {
            return Err(failed("symbolic ref cycle"));
        }
        current = target;
    }
    Ok(repo.refs.get(current).cloned())
}
pub(super) fn test_set_head(
    backend: &FakeGitRepository,
    path: &Path,
    state: &TestHead,
) -> ModelResult<()> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    match state {
        TestHead::Attached(name) => {
            if !name.starts_with("refs/heads/") || !git2::Reference::is_valid_name(name) {
                return Err(failed("invalid attached branch"));
            }
            repo.head = resolve_ref(repo, name)?;
            repo.attached_ref = Some(name.clone());
            repo.detached = false;
        }
        TestHead::Detached(oid) => {
            if !repo.commits.contains_key(oid) {
                return Err(failed("commit missing"));
            }
            repo.head = Some(oid.clone());
            repo.detached = true;
        }
    }
    Ok(())
}

pub(super) fn test_read_index(
    backend: &FakeGitRepository,
    path: &Path,
) -> ModelResult<Vec<TestIndexEntry>> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    if let Some(entries) = &repo.index_override {
        return Ok(entries.clone());
    }
    repo.index
        .iter()
        .map(|(name, bytes)| {
            let oid = object_id(repo.sha256, git2::ObjectType::Blob, bytes)?;
            repo.blobs.insert(oid.clone(), bytes.clone());
            Ok(TestIndexEntry {
                path: name.as_bytes().to_vec(),
                object_id: oid,
                mode: 0o100644,
                stage: 0,
                assume_valid: false,
                skip_worktree: false,
                intent_to_add: false,
            })
        })
        .collect()
}
pub(super) fn test_replace_index(
    backend: &FakeGitRepository,
    path: &Path,
    entries: &[TestIndexEntry],
) -> ModelResult<()> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    let mut tree = BTreeMap::new();
    for entry in entries {
        if entry.stage == 0 {
            tree.insert(
                String::from_utf8(entry.path.clone()).map_err(|e| failed(e.to_string()))?,
                repo.blobs
                    .get(&entry.object_id)
                    .ok_or_else(|| failed("index object missing"))?
                    .clone(),
            );
        }
    }
    repo.index = tree;
    let mut entries = entries.to_vec();
    entries.sort_by(|a, b| (&a.path, a.stage).cmp(&(&b.path, b.stage)));
    repo.index_override = Some(entries);
    Ok(())
}
pub(super) fn test_set_config(
    backend: &FakeGitRepository,
    path: &Path,
    key: &str,
    values: &[String],
) -> ModelResult<()> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    if values.is_empty() {
        repo.config.remove(key);
    } else {
        repo.config.insert(key.into(), values.to_vec());
    }
    Ok(())
}
pub(super) fn test_read_config(
    backend: &FakeGitRepository,
    path: &Path,
    key: &str,
) -> ModelResult<Vec<String>> {
    let all = backend.repositories.lock().unwrap();
    let repo = all
        .get(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    Ok(repo.config.get(key).cloned().unwrap_or_default())
}

pub(super) fn test_force_checkout(
    backend: &FakeGitRepository,
    path: &Path,
    commit: &str,
) -> ModelResult<()> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    let tree = repo
        .commits
        .get(commit)
        .cloned()
        .ok_or_else(|| failed("commit missing"))?;
    let old_paths = repo.index.keys().cloned().collect::<Vec<_>>();
    for name in old_paths {
        if !tree.contains_key(&name) {
            remove_worktree_file(backend.filesystem.as_ref(), &path.join(name))?;
        }
    }
    for (name, bytes) in &tree {
        write_worktree_file(backend.filesystem.as_ref(), &path.join(name), bytes)?;
    }
    repo.index = tree;
    repo.index_override = None;
    repo.head = Some(commit.into());
    if let Some(name) = &repo.attached_ref {
        repo.refs.insert(name.clone(), commit.into());
    }
    Ok(())
}

pub(super) fn test_reset_mixed(
    backend: &FakeGitRepository,
    path: &Path,
    commit: &str,
) -> ModelResult<()> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    let tree = repo
        .commits
        .get(commit)
        .cloned()
        .ok_or_else(|| failed("commit missing"))?;
    repo.index = tree;
    repo.index_override = None;
    repo.head = Some(commit.into());
    if let Some(name) = &repo.attached_ref {
        repo.refs.insert(name.clone(), commit.into());
    }
    Ok(())
}

pub(super) fn test_set_repository_state(
    backend: &FakeGitRepository,
    path: &Path,
    state: GitRepositoryState,
    merge_head: Option<&str>,
) -> ModelResult<()> {
    if state == GitRepositoryState::Merge && merge_head.is_none() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "merge state requires MERGE_HEAD",
        ));
    }
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    repo.repository_state = Some(state);
    repo.merge_head = merge_head.map(str::to_owned);
    Ok(())
}

pub(super) fn test_seed_merge_conflict(
    backend: &FakeGitRepository,
    path: &Path,
    before: &str,
    source: &str,
) -> ModelResult<GitMergeConflictSnapshot> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    if repo.head.as_deref() != Some(before) {
        return Err(failed("conflict fixture head differs"));
    }
    let ours = repo
        .commits
        .get(before)
        .cloned()
        .ok_or_else(|| failed("before commit missing"))?;
    let theirs = repo
        .commits
        .get(source)
        .cloned()
        .ok_or_else(|| failed("source commit missing"))?;
    let base_oid = repo
        .metadata
        .get(before)
        .and_then(|commit| commit.parents.first())
        .cloned()
        .ok_or_else(|| failed("conflict fixture base missing"))?;
    let base = repo
        .commits
        .get(&base_oid)
        .cloned()
        .ok_or_else(|| failed("base commit missing"))?;
    let paths: Vec<String> = ours
        .keys()
        .filter(|name| theirs.get(*name) != ours.get(*name) && base.get(*name) != ours.get(*name))
        .cloned()
        .collect();
    if paths.is_empty() {
        return Err(failed("test merge did not produce a conflict"));
    }
    let mut entries = Vec::new();
    let mut snapshots = Vec::new();
    for name in paths {
        for (stage, tree) in [(1, &base), (2, &ours), (3, &theirs)] {
            if let Some(bytes) = tree.get(&name) {
                let object_id = object_id(repo.sha256, git2::ObjectType::Blob, bytes)?;
                repo.blobs.insert(object_id.clone(), bytes.clone());
                entries.push(TestIndexEntry {
                    path: name.as_bytes().to_vec(),
                    object_id,
                    mode: 0o100644,
                    stage,
                    assume_valid: false,
                    skip_worktree: false,
                    intent_to_add: false,
                });
            }
        }
        let bytes = format!(
            "<<<<<<< HEAD\n{}=======\n{}>>>>>>> source\n",
            String::from_utf8_lossy(ours.get(&name).unwrap()),
            String::from_utf8_lossy(theirs.get(&name).unwrap())
        )
        .into_bytes();
        write_worktree_file(backend.filesystem.as_ref(), &path.join(&name), &bytes)?;
        snapshots.push(GitConflictFileSnapshot {
            path: name,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        });
    }
    entries.sort_by(|a, b| (&a.path, a.stage).cmp(&(&b.path, b.stage)));
    repo.index.clear();
    repo.index_override = Some(entries.clone());
    repo.merge_index = Some(entries);
    repo.repository_state = Some(GitRepositoryState::Merge);
    repo.merge_head = Some(source.into());
    let snapshot = GitMergeConflictSnapshot { files: snapshots };
    repo.merge_conflict_snapshot = Some(snapshot.clone());
    Ok(snapshot)
}

pub(super) fn test_create_commit_from_parent(
    backend: &FakeGitRepository,
    path: &Path,
    parent: &str,
    message: &str,
    edits: &[TestCommitFileEdit],
) -> ModelResult<String> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    let mut tree = repo
        .commits
        .get(parent)
        .cloned()
        .ok_or_else(|| failed("parent commit missing"))?;
    for edit in edits {
        match &edit.bytes {
            Some(bytes) => {
                tree.insert(edit.path.clone(), bytes.clone());
            }
            None => {
                tree.remove(&edit.path);
            }
        }
    }
    store_commit(
        repo,
        tree,
        &TestCommitSpec::from_index(message, vec![parent.into()]),
    )
}
