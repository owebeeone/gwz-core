use super::repository_support::open_repo;
use super::*;

use std::collections::{BTreeMap, BTreeSet};

const PREIMAGE_FRAME: &[u8] = b"gwz.merge-preservation-preimage/v1\0";
const CHECKED_ARTIFACT_PRIVATE_PATH: &str = ".gwz/checked-artifacts";

#[derive(Default, Eq, PartialEq)]
struct ImageEntry {
    index: Vec<IndexImage>,
    worktree: Option<WorktreeImage>,
}

#[derive(Clone, Eq, PartialEq)]
struct IndexImage {
    stage: u8,
    mode: u32,
    semantic: u32,
    oid: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WorktreeImage {
    Regular { executable: bool, bytes: Vec<u8> },
    Symlink(Vec<u8>),
    Gitlink(Vec<u8>),
}

pub(super) fn capture(root: &Path, include_untracked: bool) -> ModelResult<GitPreservationImage> {
    capture_inner(root, include_untracked, None)
}

pub(super) fn capture_normalized(
    root: &Path,
    form: &GitRootManagedForm,
    excluded_paths: &[String],
) -> ModelResult<GitPreservationImage> {
    let excluded_paths = raw_excluded_paths(excluded_paths)?;
    let (entries, dirty) = live_entries(root, true, Some(form), &excluded_paths)?;
    encode(entries, dirty)
}

fn capture_inner(
    root: &Path,
    include_untracked: bool,
    managed: Option<&GitRootManagedForm>,
) -> ModelResult<GitPreservationImage> {
    let (entries, dirty) = live_entries(root, include_untracked, managed, &[])?;
    encode(entries, dirty)
}

fn live_entries(
    root: &Path,
    include_untracked: bool,
    managed: Option<&GitRootManagedForm>,
    excluded_paths: &[Vec<u8>],
) -> ModelResult<(BTreeMap<Vec<u8>, ImageEntry>, GitPreservationDirtySummary)> {
    let repo = open_repo(root)?;
    let index = repo.index().map_err(git_error)?;
    let mut entries = BTreeMap::<Vec<u8>, ImageEntry>::new();
    for item in index.iter() {
        if excluded_paths
            .iter()
            .any(|excluded| path_is_at_or_below(&item.path, excluded))
        {
            continue;
        }
        let semantic = semantic_flags(&item)?;
        let stage = ((item.flags >> 12) & 3) as u8;
        let row = entries.entry(item.path.clone()).or_default();
        if row.index.iter().any(|prior| prior.stage == stage) {
            return Err(preimage_error("duplicate index path/stage"));
        }
        row.index.push(IndexImage {
            stage,
            mode: item.mode,
            semantic,
            oid: item.id.as_bytes().to_vec(),
        });
    }

    let managed_paths = managed.map_or_else(BTreeSet::new, |form| {
        [
            preservation_root::index::fact_path(&form.index.marker).to_vec(),
            preservation_root::index::fact_path(&form.index.lock).to_vec(),
        ]
        .into_iter()
        .collect()
    });
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(include_untracked)
        .recurse_untracked_dirs(include_untracked)
        .include_ignored(false)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false);
    let statuses = repo.statuses(Some(&mut options)).map_err(git_error)?;
    let mut dirty = GitPreservationDirtySummary::default();
    for status in statuses.iter() {
        let path = status.path_bytes();
        if excluded_paths
            .iter()
            .any(|excluded| path_is_at_or_below(path, excluded))
        {
            continue;
        }
        let flags = status.status();
        if managed_paths.contains(path) {
            continue;
        }
        dirty.staged |= flags.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        );
        dirty.unstaged |= flags.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE
                | git2::Status::WT_UNREADABLE,
        );
        dirty.untracked |= flags.contains(git2::Status::WT_NEW);
        if flags.contains(git2::Status::CONFLICTED) {
            return Err(preimage_error(
                "unresolved index entries are not preservable",
            ));
        }
        if include_untracked && flags.contains(git2::Status::WT_NEW) {
            entries.entry(path.to_vec()).or_default();
        }
    }
    for (raw_path, row) in &mut entries {
        row.index.sort_by_key(|item| item.stage);
        row.worktree = read_worktree(root, raw_path, row.index.first())?;
    }
    if let Some(form) = managed {
        substitute_managed(&mut entries, form)?;
    }
    Ok((entries, dirty))
}

pub(super) fn checkout_matches_commit_except(
    root: &Path,
    commit: &str,
    allowed_paths: &[String],
) -> ModelResult<bool> {
    checkout_matches_commit_with_overlay(
        root,
        commit,
        &GitCheckoutOverlay {
            worktree_paths: allowed_paths.to_vec(),
            index_paths: allowed_paths.to_vec(),
        },
    )
}

pub(super) fn checkout_matches_commit_with_overlay(
    root: &Path,
    commit: &str,
    overlay: &GitCheckoutOverlay,
) -> ModelResult<bool> {
    let repo = open_repo(root)?;
    let commit = preservation_root::index::parse_exact_oid(&repo, commit, "checkout commit")?;
    let commit = repo.find_commit(commit).map_err(git_error)?;
    let expected_tree = flatten_tree(&repo, &commit.tree().map_err(git_error)?)?;
    let mut expected = expected_tree
        .into_iter()
        .map(|(path, item)| {
            let worktree = item.worktree();
            (
                path,
                ImageEntry {
                    index: vec![IndexImage {
                        stage: 0,
                        mode: item.mode,
                        semantic: 0,
                        oid: item.oid,
                    }],
                    worktree: Some(worktree),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let (mut live, _) = live_entries(root, true, None, &[])?;
    let worktree_paths = raw_excluded_paths(&overlay.worktree_paths)?;
    let index_paths = raw_excluded_paths(&overlay.index_paths)?;
    apply_overlay(&mut live, &worktree_paths, &index_paths);
    apply_overlay(&mut expected, &worktree_paths, &index_paths);
    Ok(live == expected)
}

fn apply_overlay(
    entries: &mut BTreeMap<Vec<u8>, ImageEntry>,
    worktree_paths: &[Vec<u8>],
    index_paths: &[Vec<u8>],
) {
    for (path, entry) in entries.iter_mut() {
        if worktree_paths
            .iter()
            .any(|prefix| path_is_at_or_below(path, prefix))
        {
            entry.worktree = None;
        }
        if index_paths
            .iter()
            .any(|prefix| path_is_at_or_below(path, prefix))
        {
            entry.index.clear();
        }
    }
    entries.retain(|_, entry| !entry.index.is_empty() || entry.worktree.is_some());
}

fn raw_excluded_paths(paths: &[String]) -> ModelResult<Vec<Vec<u8>>> {
    std::iter::once(CHECKED_ARTIFACT_PRIVATE_PATH)
        .chain(paths.iter().map(String::as_str))
        .map(|path| {
            let mut raw = preservation_root::files::path_to_raw(Path::new(path))?;
            while raw.last() == Some(&b'/') {
                raw.pop();
            }
            Ok(raw)
        })
        .collect()
}

fn path_is_at_or_below(candidate: &[u8], prefix: &[u8]) -> bool {
    candidate == prefix
        || candidate
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.first() == Some(&b'/'))
}

fn substitute_managed(
    entries: &mut BTreeMap<Vec<u8>, ImageEntry>,
    form: &GitRootManagedForm,
) -> ModelResult<()> {
    substitute(
        entries,
        preservation_root::index::fact_path(&form.index.marker),
        form.marker.as_ref(),
        &form.index.marker,
    )?;
    substitute(
        entries,
        preservation_root::index::fact_path(&form.index.lock),
        Some(&form.lock),
        &form.index.lock,
    )
}

fn substitute(
    entries: &mut BTreeMap<Vec<u8>, ImageEntry>,
    path: &[u8],
    worktree: Option<&GitCandidateFile>,
    index: &GitRootManagedIndexFact,
) -> ModelResult<()> {
    entries.remove(path);
    let index = match index {
        GitRootManagedIndexFact::Absent { .. } => Vec::new(),
        GitRootManagedIndexFact::Present(entry) => vec![IndexImage {
            stage: entry.stage,
            mode: entry.mode,
            semantic: semantic_bits(entry),
            oid: decode_oid(&entry.object_id)?,
        }],
    };
    let worktree = worktree.map(|file| WorktreeImage::Regular {
        executable: false,
        bytes: file.bytes.clone(),
    });
    if !index.is_empty() || worktree.is_some() {
        entries.insert(path.to_vec(), ImageEntry { index, worktree });
    }
    Ok(())
}

pub(super) fn decode_stashes(
    backend: &Git2Backend,
    root: &Path,
    merge_id: &str,
) -> ModelResult<Vec<GitPreservationStashEvidence>> {
    let repo = open_repo(root)?;
    let expected = format!("gwz:stash_{merge_id}: merge preservation");
    backend
        .stash_list(root)?
        .into_iter()
        .filter(|entry| canonical_stash_message(&entry.message, &expected))
        .map(|entry| decode_stash(&repo, entry.object_id, expected.clone()))
        .collect()
}

pub(super) fn canonical_stash_message(native: &str, expected: &str) -> bool {
    native == expected
        || native
            .strip_prefix("On ")
            .and_then(|value| value.split_once(": "))
            .is_some_and(|(branch, message)| !branch.is_empty() && message == expected)
}

fn decode_stash(
    repo: &git2::Repository,
    object_id: String,
    message: String,
) -> ModelResult<GitPreservationStashEvidence> {
    let oid = preservation_root::index::parse_exact_oid(repo, &object_id, "stash")?;
    let stash = repo.find_commit(oid).map_err(git_error)?;

    if !(2..=3).contains(&stash.parent_count()) {
        return Err(preimage_error(
            "preservation stash has an unknown parent layout",
        ));
    }
    let head = stash.parent(0).map_err(git_error)?;
    let index = stash.parent(1).map_err(git_error)?;
    let head_tree = flatten_tree(repo, &head.tree().map_err(git_error)?)?;
    let index_tree = flatten_tree(repo, &index.tree().map_err(git_error)?)?;
    let worktree = flatten_tree(repo, &stash.tree().map_err(git_error)?)?;
    let untracked = if stash.parent_count() == 3 {
        flatten_tree(
            repo,
            &stash
                .parent(2)
                .map_err(git_error)?
                .tree()
                .map_err(git_error)?,
        )?
    } else {
        BTreeMap::new()
    };
    let mut entries = BTreeMap::new();
    for (path, item) in &index_tree {
        entries.insert(
            path.clone(),
            ImageEntry {
                index: vec![IndexImage {
                    stage: 0,
                    mode: item.mode,
                    semantic: 0,
                    oid: item.oid.clone(),
                }],
                worktree: worktree.get(path).map(TreeImage::worktree),
            },
        );
    }
    for (path, item) in worktree.iter().chain(untracked.iter()) {
        entries.entry(path.clone()).or_insert_with(|| ImageEntry {
            index: Vec::new(),
            worktree: Some(item.worktree()),
        });
    }
    let dirty = GitPreservationDirtySummary {
        staged: head_tree != index_tree,
        unstaged: index_tree != worktree,
        untracked: !untracked.is_empty(),
    };
    Ok(GitPreservationStashEvidence {
        object_id,
        message,
        head_commit: head.id().to_string(),
        image: encode(entries, dirty)?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TreeImage {
    mode: u32,
    oid: Vec<u8>,
    body: Option<Vec<u8>>,
}

impl TreeImage {
    fn worktree(&self) -> WorktreeImage {
        match self.mode {
            0o120000 => WorktreeImage::Symlink(self.body.clone().unwrap_or_default()),
            0o160000 => WorktreeImage::Gitlink(self.oid.clone()),
            mode => WorktreeImage::Regular {
                executable: mode & 0o111 != 0,
                bytes: self.body.clone().unwrap_or_default(),
            },
        }
    }
}

fn flatten_tree(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
) -> ModelResult<BTreeMap<Vec<u8>, TreeImage>> {
    fn walk(
        repo: &git2::Repository,
        tree: &git2::Tree<'_>,
        prefix: &[u8],
        out: &mut BTreeMap<Vec<u8>, TreeImage>,
    ) -> ModelResult<()> {
        for entry in tree.iter() {
            let mut path = prefix.to_vec();
            if !path.is_empty() {
                path.push(b'/');
            }
            path.extend(entry.name_bytes());
            if entry.kind() == Some(git2::ObjectType::Tree) {
                walk(
                    repo,
                    &repo.find_tree(entry.id()).map_err(git_error)?,
                    &path,
                    out,
                )?;
            } else {
                let mode = entry.filemode_raw() as u32;
                let body = (mode != 0o160000)
                    .then(|| {
                        repo.find_blob(entry.id())
                            .map(|blob| blob.content().to_vec())
                    })
                    .transpose()
                    .map_err(git_error)?;
                if out
                    .insert(
                        path,
                        TreeImage {
                            mode,
                            oid: entry.id().as_bytes().to_vec(),
                            body,
                        },
                    )
                    .is_some()
                {
                    return Err(preimage_error("duplicate tree path"));
                }
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(repo, tree, &[], &mut out)?;
    Ok(out)
}

fn read_worktree(
    root: &Path,
    raw_path: &[u8],
    index: Option<&IndexImage>,
) -> ModelResult<Option<WorktreeImage>> {
    let path = root.join(preservation_root::files::raw_path_to_path(raw_path)?);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(crate::git::io_error(error)),
    };
    if metadata.file_type().is_symlink() {
        return Ok(Some(WorktreeImage::Symlink(
            preservation_root::files::path_to_raw(
                &std::fs::read_link(path).map_err(crate::git::io_error)?,
            )?,
        )));
    }
    if metadata.is_dir() && index.is_some_and(|item| item.mode == 0o160000) {
        return Ok(Some(WorktreeImage::Gitlink(index.unwrap().oid.clone())));
    }
    if !metadata.is_file() {
        return Err(preimage_error(format!(
            "unsupported worktree file kind at '{}'",
            path.display()
        )));
    }
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = index.is_some_and(|item| item.mode & 0o111 != 0);
    Ok(Some(WorktreeImage::Regular {
        executable,
        bytes: std::fs::read(path).map_err(crate::git::io_error)?,
    }))
}

fn encode(
    entries: BTreeMap<Vec<u8>, ImageEntry>,
    dirty: GitPreservationDirtySummary,
) -> ModelResult<GitPreservationImage> {
    let mut bytes = PREIMAGE_FRAME.to_vec();
    push_u64(&mut bytes, entries.len())?;
    for (path, entry) in entries {
        if entry.index.is_empty() && entry.worktree.is_none() {
            return Err(preimage_error("empty preservation image entry"));
        }
        push_bytes(&mut bytes, &path)?;
        bytes.push(u8::from(entry.index.is_empty()));
        push_u64(&mut bytes, entry.index.len())?;
        for index in entry.index {
            bytes.push(index.stage);
            bytes.extend(index.mode.to_be_bytes());
            bytes.extend(index.semantic.to_be_bytes());
            push_oid(&mut bytes, &index.oid)?;
        }
        match entry.worktree {
            None => bytes.push(0),
            Some(worktree) => {
                bytes.push(1);
                let (kind, executable, body) = match worktree {
                    WorktreeImage::Regular { executable, bytes } => (0, executable, bytes),
                    WorktreeImage::Symlink(bytes) => (1, false, bytes),
                    WorktreeImage::Gitlink(oid) => {
                        bytes.extend([2, 0]);
                        push_oid(&mut bytes, &oid)?;
                        continue;
                    }
                };
                bytes.extend([kind, u8::from(executable)]);
                push_u64(&mut bytes, body.len())?;
                bytes.extend(Sha256::digest(body));
            }
        }
    }
    Ok(GitPreservationImage {
        preimage_sha256: format!("{:x}", Sha256::digest(bytes)),
        dirty,
    })
}

fn semantic_flags(entry: &git2::IndexEntry) -> ModelResult<u32> {
    if entry.flags & 0xc000 != 0 || entry.flags_extended != 0 {
        return Err(preimage_error(
            "assume-valid, skip-worktree, intent-to-add, or unknown index flags are not preservable",
        ));
    }
    Ok(0)
}

fn semantic_bits(entry: &GitRootManagedIndexEntry) -> u32 {
    u32::from(entry.assume_valid)
        | u32::from(entry.skip_worktree) << 1
        | u32::from(entry.intent_to_add) << 2
}

fn decode_oid(value: &str) -> ModelResult<Vec<u8>> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(preimage_error("invalid complete Git object id"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex is ASCII");
            u8::from_str_radix(text, 16).map_err(|_| preimage_error("invalid Git object id"))
        })
        .collect()
}

fn push_u64(bytes: &mut Vec<u8>, value: usize) -> ModelResult<()> {
    bytes.extend(
        u64::try_from(value)
            .map_err(|_| preimage_error("preimage length overflow"))?
            .to_be_bytes(),
    );
    Ok(())
}

fn push_bytes(bytes: &mut Vec<u8>, value: &[u8]) -> ModelResult<()> {
    push_u64(bytes, value.len())?;
    bytes.extend(value);
    Ok(())
}

fn push_oid(bytes: &mut Vec<u8>, oid: &[u8]) -> ModelResult<()> {
    let algorithm = match oid.len() {
        20 => 1,
        32 => 2,
        _ => return Err(preimage_error("unsupported Git object-id length")),
    };
    bytes.extend([algorithm, oid.len() as u8]);
    bytes.extend(oid);
    Ok(())
}

pub(super) fn preimage_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
}

#[cfg(test)]
pub(crate) fn raw_path_preimage_for_test(
    entries: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>,
) -> ModelResult<GitPreservationImage> {
    let entries = entries
        .into_iter()
        .map(|(path, bytes)| {
            (
                path,
                ImageEntry {
                    index: Vec::new(),
                    worktree: Some(WorktreeImage::Regular {
                        executable: false,
                        bytes,
                    }),
                },
            )
        })
        .collect();
    encode(
        entries,
        GitPreservationDirtySummary {
            untracked: true,
            ..GitPreservationDirtySummary::default()
        },
    )
}
