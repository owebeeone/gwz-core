//! Git facts used by the shared root-preservation protocol.
use super::*;

pub(super) fn worktree(
    backend: &FakeGitRepository,
    repo: &RepositoryState,
    path: &Path,
) -> ModelResult<FileTree> {
    let mut ignored = Vec::new();
    let boundary = backend
        .filesystem
        .as_ref()
        .read(&path.join(".git/info/exclude"))
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default();
    for rule in boundary
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        if rule.contains(['*', '?', '[', '!']) {
            return unsupported("wildcard exclude rule");
        }
        let prefix = rule.trim_start_matches('/');
        ignored.push(prefix.to_owned());
    }
    read_worktree(backend.filesystem.as_ref(), path, &ignored, &repo.index)
}
fn image(index: &FileTree, worktree: &FileTree, committed: &FileTree) -> GitPreservationImage {
    let paths: BTreeSet<_> = index
        .keys()
        .chain(worktree.keys())
        .chain(committed.keys())
        .collect();
    let mut dirty = GitPreservationDirtySummary::default();
    for name in paths {
        dirty.staged |= index.get(name) != committed.get(name);
        if index.contains_key(name) || committed.contains_key(name) {
            dirty.unstaged |= index.get(name) != worktree.get(name);
        } else {
            dirty.untracked |= worktree.contains_key(name);
        }
    }
    GitPreservationImage {
        preimage_sha256: format!("{:x}", Sha256::digest(format!("{index:?}{worktree:?}"))),
        dirty,
    }
}
pub(super) fn capture(
    backend: &FakeGitRepository,
    path: &Path,
    clean: Option<&GitRootManagedForm>,
    excluded: &[String],
) -> ModelResult<GitPreservationImage> {
    let all = backend.repositories.lock().unwrap();
    let repo = all
        .get(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    let mut index = repo.index.clone();
    let mut disk = worktree(backend, repo, path)?;
    let committed = repo
        .head
        .as_ref()
        .and_then(|oid| repo.commits.get(oid))
        .cloned()
        .unwrap_or_default();
    if let Some(clean) = clean {
        for name in [fact_path(&clean.index.marker), fact_path(&clean.index.lock)] {
            let name = std::str::from_utf8(name).map_err(|e| failed(e.to_string()))?;
            index.remove(name);
            disk.remove(name);
        }
        for file in clean.marker.iter().chain(std::iter::once(&clean.lock)) {
            index.insert(file.path.clone(), file.bytes.clone());
            disk.insert(file.path.clone(), file.bytes.clone());
        }
    }
    disk.retain(|name, _| {
        !excluded.iter().any(|prefix| {
            name == prefix || name.starts_with(&format!("{}/", prefix.trim_end_matches('/')))
        })
    });
    Ok(image(&index, &disk, &committed))
}

fn fact_path(fact: &GitRootManagedIndexFact) -> &[u8] {
    match fact {
        GitRootManagedIndexFact::Absent { path } => path,
        GitRootManagedIndexFact::Present(entry) => &entry.path,
    }
}
pub(super) fn validate(
    backend: &FakeGitRepository,
    path: &Path,
    spec: &GitRootPreservationSpec,
) -> ModelResult<()> {
    for (commit, form) in [
        (&spec.attached_commit, &spec.attached_clean_form),
        (&spec.restore_commit, &spec.restore_clean_form),
    ] {
        if backend.read_file_at_commit(path, commit, &spec.managed_marker_path)?
            != form.marker.as_ref().map(|file| file.bytes.clone())
            || backend.read_file_at_commit(path, commit, &form.lock.path)?
                != Some(form.lock.bytes.clone())
        {
            return Err(ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                "managed clean form differs from commit",
            ));
        }
    }
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    for form in [
        &spec.attached_clean_form,
        &spec.restore_clean_form,
        &spec.handoff_form,
    ] {
        for file in form.marker.iter().chain(std::iter::once(&form.lock)) {
            let oid = fixture::object_id(repo.sha256, git2::ObjectType::Blob, &file.bytes)?;
            repo.blobs.insert(oid, file.bytes.clone());
        }
    }
    for form in [
        &spec.attached_clean_form,
        &spec.restore_clean_form,
        &spec.handoff_form,
    ] {
        for (fact, name) in [
            (&form.index.marker, spec.managed_marker_path.as_str()),
            (&form.index.lock, crate::artifact::LOCK_PATH),
        ] {
            if fact_path(fact) != name.as_bytes() {
                return Err(failed("managed path differs"));
            }
            if let GitRootManagedIndexFact::Present(entry) = fact
                && (!repo.blobs.contains_key(&entry.object_id)
                    || entry.mode != 0o100644
                    || entry.stage != 0
                    || entry.assume_valid
                    || entry.skip_worktree
                    || entry.intent_to_add)
            {
                return Err(failed("managed index fact invalid"));
            }
        }
    }
    Ok(())
}
pub(super) fn index_matches(
    backend: &FakeGitRepository,
    path: &Path,
    form: &GitRootManagedIndexForm,
) -> ModelResult<bool> {
    let entries = backend.test_read_index(path)?;
    let marker_prefix = format!("{}/", crate::artifact::MARKER_DIR);
    if entries.iter().any(|entry| {
        entry.path.starts_with(marker_prefix.as_bytes()) && entry.path != fact_path(&form.marker)
    }) {
        return Ok(false);
    }
    for fact in [&form.marker, &form.lock] {
        let actual: Vec<_> = entries
            .iter()
            .filter(|entry| entry.path == fact_path(fact))
            .collect();
        match fact {
            GitRootManagedIndexFact::Absent { .. } if actual.is_empty() => (),
            GitRootManagedIndexFact::Present(entry) if actual == [entry] => (),
            _ => return Ok(false),
        }
    }
    Ok(true)
}
pub(super) fn rewrite(
    backend: &FakeGitRepository,
    path: &Path,
    form: &GitRootManagedIndexForm,
) -> ModelResult<()> {
    let mut entries = backend.test_read_index(path)?;
    entries.retain(|entry| {
        entry.path != fact_path(&form.marker) && entry.path != fact_path(&form.lock)
    });
    for fact in [&form.marker, &form.lock] {
        if let GitRootManagedIndexFact::Present(entry) = fact {
            entries.push(entry.clone());
        }
    }
    preservation::fault(preservation::FaultBoundary::BeforeIndexCommit)?;
    backend.test_replace_index(path, &entries)?;
    preservation::fault(preservation::FaultBoundary::AfterIndexCommit)
}
pub(super) fn checkout_matches(
    backend: &FakeGitRepository,
    path: &Path,
    commit: &str,
    overlay: &GitCheckoutOverlay,
) -> ModelResult<bool> {
    let all = backend.repositories.lock().unwrap();
    let repo = all
        .get(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    if repo.index_override.as_ref().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry.stage != 0
                || entry.assume_valid
                || entry.skip_worktree
                || entry.intent_to_add
                || entry.mode != 0o100644
        })
    }) {
        return Ok(false);
    }
    let expected = repo
        .commits
        .get(commit)
        .ok_or_else(|| failed("commit missing"))?;
    let disk = worktree(backend, repo, path)?;
    let matches = |actual: &FileTree, ignored: &[String]| {
        let filter = |tree: &FileTree| {
            tree.iter()
                .filter(|(name, _)| !ignored.contains(name))
                .map(|(name, bytes)| (name.clone(), bytes.clone()))
                .collect::<FileTree>()
        };
        filter(actual) == filter(expected)
    };
    Ok(matches(&repo.index, &overlay.index_paths) && matches(&disk, &overlay.worktree_paths))
}
pub(super) fn candidate_matches(
    backend: &FakeGitRepository,
    path: &Path,
    files: &[GitCandidateFile],
    absent: &[String],
) -> ModelResult<bool> {
    let entries = backend.test_read_index(path)?;
    let sha256 = backend
        .repositories
        .lock()
        .unwrap()
        .get(&repository_key(backend.filesystem.as_ref(), path))
        .unwrap()
        .sha256;
    for file in files {
        let oid = fixture::object_id(sha256, git2::ObjectType::Blob, &file.bytes)?;
        let expected = TestIndexEntry {
            path: file.path.as_bytes().to_vec(),
            object_id: oid,
            mode: 0o100644,
            stage: 0,
            assume_valid: false,
            skip_worktree: false,
            intent_to_add: false,
        };
        if entries
            .iter()
            .filter(|entry| entry.path == expected.path)
            .collect::<Vec<_>>()
            != [&expected]
        {
            return Ok(false);
        }
    }
    Ok(!entries
        .iter()
        .any(|entry| absent.iter().any(|name| entry.path == name.as_bytes())))
}
pub(super) fn scoped_commit(
    backend: &FakeGitRepository,
    path: &Path,
    expected: Option<&str>,
    files: &[GitCandidateFile],
    message: &str,
) -> ModelResult<GitScopedCommitResult> {
    let mut all = backend.repositories.lock().unwrap();
    let repo = all
        .get_mut(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    if repo.detached || repo.head.as_deref() != expected {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "scoped commit HEAD differs",
        ));
    }
    let mut tree = expected
        .and_then(|oid| repo.commits.get(oid))
        .cloned()
        .unwrap_or_default();
    let mut hashes = BTreeMap::new();
    for file in files {
        if !file.path.starts_with("gwz.conf/")
            || Path::new(&file.path)
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || hashes.contains_key(&file.path)
        {
            return Err(failed("invalid candidate path"));
        }
        tree.insert(file.path.clone(), file.bytes.clone());
        hashes.insert(
            file.path.clone(),
            format!("{:x}", Sha256::digest(&file.bytes)),
        );
    }
    let spec =
        TestCommitSpec::from_index(message, expected.map(str::to_owned).into_iter().collect());
    let commit = fixture::store_commit(repo, tree, &spec)?;
    repo.head = Some(commit.clone());
    repo.refs.insert(
        repo.attached_ref
            .clone()
            .unwrap_or_else(|| "refs/heads/main".into()),
        commit.clone(),
    );
    Ok(GitScopedCommitResult {
        tree: repo.metadata[&commit].tree.clone(),
        commit,
        candidate_hashes: hashes
            .into_iter()
            .map(|(path, sha256)| GitCandidateHash { path, sha256 })
            .collect(),
    })
}
pub(super) fn verify_scoped(
    backend: &FakeGitRepository,
    path: &Path,
    commit: &str,
    parent: Option<&str>,
    files: &[GitCandidateFile],
    message: &str,
) -> ModelResult<GitScopedCommitResult> {
    let all = backend.repositories.lock().unwrap();
    let repo = all
        .get(&repository_key(backend.filesystem.as_ref(), path))
        .ok_or_else(|| failed("repository missing"))?;
    let metadata = repo
        .metadata
        .get(commit)
        .ok_or_else(|| failed("commit missing"))?;
    let mut tree = parent
        .and_then(|oid| repo.commits.get(oid))
        .cloned()
        .unwrap_or_default();
    let mut hashes = BTreeMap::new();
    for file in files {
        tree.insert(file.path.clone(), file.bytes.clone());
        hashes.insert(
            file.path.clone(),
            format!("{:x}", Sha256::digest(&file.bytes)),
        );
    }
    if metadata.parents != parent.map(str::to_owned).into_iter().collect::<Vec<_>>()
        || metadata.message != message
        || repo.commits.get(commit) != Some(&tree)
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            "scoped commit evidence differs",
        ));
    }
    Ok(GitScopedCommitResult {
        commit: commit.into(),
        tree: metadata.tree.clone(),
        candidate_hashes: hashes
            .into_iter()
            .map(|(path, sha256)| GitCandidateHash { path, sha256 })
            .collect(),
    })
}
