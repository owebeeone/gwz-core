use crate::filesystem::FileSystem;
#[cfg(test)]
use crate::filesystem::native_filesystem;
use crate::git::{Git2Repository, GitRepository};
#[cfg(test)]
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct OperationContext {
    filesystem: Arc<dyn FileSystem>,
    repository: Arc<dyn GitRepository + Send + Sync>,
}

/// Borrow dependencies already supplied by a caller without replacing its Git authority.
#[derive(Clone, Copy)]
pub(crate) struct BorrowedOperationContext<'a> {
    pub(crate) filesystem: &'a dyn FileSystem,
    pub(crate) repository: &'a dyn GitRepository,
}

impl<'a> BorrowedOperationContext<'a> {
    pub(crate) fn new(repository: &'a dyn GitRepository, filesystem: &'a dyn FileSystem) -> Self {
        Self {
            filesystem,
            repository,
        }
    }
}

impl OperationContext {
    /// Compatibility construction at entry points not yet accepting a context.
    pub(crate) fn existing() -> Self {
        #[cfg(not(test))]
        {
            Self::native()
        }
        #[cfg(test)]
        {
            Self {
                filesystem: Arc::new(crate::filesystem::make_filesystem()),
                repository: Arc::new(crate::git::make_repository()),
            }
        }
    }
    #[cfg(not(test))]
    pub(crate) fn native() -> Self {
        let repository = Git2Repository::new();
        Self {
            filesystem: repository.filesystem.clone(),
            repository: Arc::new(repository),
        }
    }
    pub(crate) fn filesystem(&self) -> &dyn FileSystem {
        self.filesystem.as_ref()
    }
    pub(crate) fn repository(&self) -> &(dyn GitRepository + Send + Sync) {
        self.repository.as_ref()
    }
}

#[cfg(test)]
pub(crate) struct TestWorld {
    context: OperationContext,
}

#[cfg(test)]
impl TestWorld {
    pub(crate) fn memory() -> Self {
        Self::with_backends(true, true)
    }
    pub(crate) fn selected() -> Self {
        let modes = crate::test_backend::modes();
        Self::with_backends(modes.fake_git, modes.fake_filesystem)
    }
    fn with_backends(fake_git: bool, fake_filesystem: bool) -> Self {
        assert!(
            !fake_filesystem || fake_git,
            "native Git requires a native filesystem"
        );
        let filesystem = if fake_filesystem {
            crate::filesystem::memory_filesystem()
        } else {
            native_filesystem()
        };
        let repository: Arc<dyn GitRepository + Send + Sync> = if fake_git {
            Arc::new(crate::git::FakeGitRepository::with_filesystem(
                filesystem.clone(),
            ))
        } else {
            {
                let mut repository = Git2Repository::new();
                repository.filesystem = filesystem.clone();
                Arc::new(repository)
            }
        };
        Self {
            context: OperationContext {
                filesystem,
                repository,
            },
        }
    }
    pub(crate) fn context(&self) -> OperationContext {
        self.context.clone()
    }
    fn workspace_at(&self, path: &Path) -> std::io::Result<crate::filesystem::TestFsWorkspace> {
        self.context.filesystem().test_workspace_at(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::{FsOpenMode, RenameMode};
    use crate::git::TestRepoSpec;
    use std::io::{Read, Seek, SeekFrom, Write};

    #[test]
    fn separate_worlds_isolate_identical_paths_and_retained_handles() {
        let left = TestWorld::memory();
        let right = TestWorld::memory();
        let left_context = left.context();
        let right_context = right.context();
        let workspace = left_context.filesystem().test_workspace().unwrap();
        let other = right.workspace_at(workspace.path()).unwrap();
        for context in [&left_context, &right_context] {
            context
                .repository()
                .test_init_repo(workspace.path(), &TestRepoSpec::default())
                .unwrap();
        }
        let root = left_context
            .filesystem()
            .open_directory(workspace.path())
            .unwrap();
        let other_root = right_context
            .filesystem()
            .open_directory(other.path())
            .unwrap();
        let mut file = root
            .open_file("value", &FsOpenMode::Write { create_new: true })
            .unwrap();
        file.write_all(b"left").unwrap();
        left_context
            .repository()
            .stage_paths(workspace.path(), &["value"])
            .unwrap();
        left_context
            .repository()
            .commit(workspace.path(), "left only", false)
            .unwrap();
        assert!(
            right_context
                .repository()
                .head(other.path())
                .unwrap()
                .commit
                .is_none()
        );
        let left_lock =
            crate::operation::WorkspaceMutatorLock::acquire_in(&left_context, workspace.path())
                .unwrap();
        let right_lock =
            crate::operation::WorkspaceMutatorLock::acquire_in(&right_context, other.path())
                .unwrap();
        drop(left_lock);
        assert!(
            crate::operation::WorkspaceMutatorLock::try_acquire_in(&right_context, other.path())
                .unwrap()
                .is_none()
        );
        drop(right_lock);
        assert!(
            right_context
                .filesystem()
                .read(&other.path().join("value"))
                .is_err()
        );
        let mut foreign = other_root
            .open_file("value", &FsOpenMode::Write { create_new: true })
            .unwrap();
        foreign.write_all(b"right").unwrap();
        assert!(
            left_context
                .filesystem()
                .rename_at(
                    &root,
                    "value".as_ref(),
                    &other_root,
                    "moved".as_ref(),
                    RenameMode::Replace
                )
                .is_err()
        );
        assert_ne!(
            left_context
                .filesystem()
                .persistent_directory_identity(&root)
                .unwrap()
                .persistent,
            right_context
                .filesystem()
                .persistent_directory_identity(&other_root)
                .unwrap()
                .persistent
        );
        drop(left_context);
        drop(left);
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = String::new();
        file.read_to_string(&mut bytes).unwrap();
        assert_eq!(bytes, "left");
        assert_eq!(
            right_context
                .filesystem()
                .read(&other.path().join("value"))
                .unwrap(),
            b"right"
        );
    }

    #[test]
    fn reopened_context_shares_git_history_and_the_same_worktree() {
        let world = TestWorld::selected();
        let context = world.context();
        let workspace = context.filesystem().test_workspace().unwrap();
        context
            .repository()
            .test_init_repo(workspace.path(), &TestRepoSpec::default())
            .unwrap();
        let file = context
            .filesystem()
            .create_file(&workspace.path().join("value"))
            .unwrap();
        context.filesystem().write_all(&file, b"committed").unwrap();
        context
            .repository()
            .stage_paths(workspace.path(), &["value"])
            .unwrap();
        let commit = context
            .repository()
            .commit(workspace.path(), "context snapshot", false)
            .unwrap()
            .commit;
        drop(context);
        let reopened = world.context();
        assert_eq!(
            reopened.repository().head(workspace.path()).unwrap().commit,
            Some(commit.clone())
        );
        assert_eq!(
            reopened
                .repository()
                .read_file_at_commit(workspace.path(), &commit, "value")
                .unwrap(),
            Some(b"committed".to_vec())
        );
        assert_eq!(
            reopened
                .filesystem()
                .read(&workspace.path().join("value"))
                .unwrap(),
            b"committed"
        );
    }
}

#[cfg(test)]
mod preservation_tests;
