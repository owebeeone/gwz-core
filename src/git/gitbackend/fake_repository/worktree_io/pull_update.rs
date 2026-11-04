//! The fake's model of pull's worktree contract (`GitPullUpdate`): the
//! collision rule over its committed, index and worktree file maps. No case
//! folding (the fake filesystem is case-sensitive) and no ignored entries (the
//! fake worktree leaves excluded paths out). The merge pipeline keeps the trait
//! defaults.

use super::*;

/// One incoming path the update would write and the untracked worktree entry in its
/// way: a file at the path, at a parent of it, or below it (a directory there, or a
/// tracked directory the update turns into a file).
struct FakeBlocked {
    incoming: String,
    blocker: String,
}

impl FakeGitRepository {
    fn blocked(&self, path: &Path, target: &str) -> ModelResult<Vec<FakeBlocked>> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        let empty = BTreeMap::new();
        let head = repo
            .head
            .as_ref()
            .and_then(|oid| repo.commits.get(oid))
            .unwrap_or(&empty);
        let target = repo
            .commits
            .get(target)
            .ok_or_else(|| failed("target commit missing"))?;
        let worktree = super::super::root::worktree(self, repo, path)?;
        let untracked = worktree
            .keys()
            .filter(|name| !head.contains_key(*name) && !repo.index.contains_key(*name))
            .collect::<Vec<_>>();
        let filesystem = self.filesystem.as_ref();
        let mut blocked = Vec::new();
        for incoming in target.keys().filter(|name| !head.contains_key(*name)) {
            let as_directory = format!("{incoming}/");
            let in_the_way = untracked.iter().any(|entry| {
                *entry == incoming
                    || incoming.starts_with(&format!("{entry}/"))
                    || entry.starts_with(&as_directory)
            });
            if !in_the_way {
                continue;
            }
            let Some(blocker) = blocker(filesystem, path, incoming) else {
                continue;
            };
            blocked.push(FakeBlocked {
                incoming: incoming.clone(),
                blocker,
            });
        }
        Ok(blocked)
    }
}

impl crate::git::GitPullUpdateSealed for FakeGitRepository {}

impl GitPullUpdate for FakeGitRepository {
    fn update_collisions(&self, path: &Path, target: &str) -> ModelResult<Vec<GitUpdateCollision>> {
        let filesystem = self.filesystem.as_ref();
        self.blocked(path, target)?
            .into_iter()
            .map(|blocked| {
                let nested = filesystem
                    .kind(&path.join(&blocked.blocker).join(".git"))
                    .is_ok();
                Ok(GitUpdateCollision {
                    path: blocked.incoming,
                    reason: nested.then(|| {
                        format!(
                            "'{}' holds a repository; move it out of the way, then run pull again",
                            blocked.blocker
                        )
                    }),
                })
            })
            .collect()
    }
}

fn blocker(filesystem: &dyn FileSystem, root: &Path, incoming: &str) -> Option<String> {
    let parts = incoming.split('/').collect::<Vec<_>>();
    for depth in 1..parts.len() {
        let parent = parts[..depth].join("/");
        if filesystem
            .kind(&root.join(&parent))
            .is_ok_and(|kind| kind != FsKind::Directory)
        {
            return Some(parent);
        }
    }
    filesystem
        .kind(&root.join(incoming))
        .is_ok()
        .then(|| incoming.to_owned())
}
