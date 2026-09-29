use crate::filesystem::FileSystem;
#[cfg(test)]
use crate::filesystem::native_filesystem;
#[cfg(test)]
use crate::git::Git2Repository;
use crate::git::GitRepository;
use gwz_ids::IdSource;
#[cfg(test)]
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct OperationServices {
    filesystem: Arc<dyn FileSystem>,
    repository: Arc<dyn GitRepository + Send + Sync>,
    /// The context's unique numbers, shared by its clones (GwzCoreSessionCrateMap §2).
    ids: Arc<IdSource>,
}

/// A new context's [`IdSource`]. Its 64-bit prefix is drawn from the operating
/// system's random source, so the names it mints are unique across contexts and
/// processes unless two draws are equal (GwzCoreSessionCrateMap §2).
///
/// `getrandom` is core's existing source and cannot fail on a supported host short
/// of a broken OS. If it does, std's hasher keys, which are also seeded from the OS,
/// supply the prefix. That is safe because every temporary-name site creates its
/// file exclusively, so a weaker prefix can only cost a refused create, never a
/// clobbered file.
pub(crate) fn new_id_source() -> IdSource {
    use std::hash::BuildHasher;
    let prefix = getrandom::u64()
        .unwrap_or_else(|_| std::hash::RandomState::new().hash_one(std::process::id()));
    IdSource::new(prefix)
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

impl OperationServices {
    pub(crate) fn from_services(
        filesystem: Arc<dyn FileSystem>,
        repository: Arc<dyn GitRepository + Send + Sync>,
    ) -> Self {
        Self {
            filesystem,
            repository,
            ids: Arc::new(new_id_source()),
        }
    }

    /// Preserve the admitted backend's configured transport and storage world.
    pub(crate) fn for_merge(backend: &impl crate::git::MergeAuthorityBackend) -> Self {
        backend.operation_services()
    }
    /// Compatibility construction at entry points not yet accepting services.
    ///
    /// This is always native. Tests choose `TestWorld` explicitly; a public
    /// operation must never silently switch its dependencies because a test
    /// executable set a process-wide mode.
    #[cfg(test)]
    pub(crate) fn existing() -> Self {
        Self::native()
    }

    #[cfg(test)]
    pub(crate) fn native() -> Self {
        let repository = Git2Repository::new();
        Self {
            filesystem: repository.filesystem.clone(),
            repository: Arc::new(repository),
            ids: Arc::new(new_id_source()),
        }
    }
    pub(crate) fn filesystem(&self) -> &dyn FileSystem {
        self.filesystem.as_ref()
    }
    pub(crate) fn repository(&self) -> &(dyn GitRepository + Send + Sync) {
        self.repository.as_ref()
    }
    /// The source for this operation's unique numbers, such as temporary names.
    pub(crate) fn ids(&self) -> &IdSource {
        &self.ids
    }
}

#[cfg(test)]
pub(crate) struct TestWorld {
    context: OperationServices,
    repository: crate::git::GitTestRepository,
}

#[cfg(test)]
impl TestWorld {
    /// A real Git/filesystem world for tests that deliberately exercise OS or libgit2 behavior.
    pub(crate) fn physical() -> Self {
        Self::with_backends(false, false)
    }

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
        let repository = if fake_git {
            crate::git::GitTestRepository::Fake(Box::new(
                crate::git::FakeGitRepository::with_filesystem(filesystem.clone()),
            ))
        } else {
            {
                let mut repository = Git2Repository::new();
                repository.filesystem = filesystem.clone();
                crate::git::GitTestRepository::Real(Box::new(repository))
            }
        };
        Self {
            context: OperationServices {
                filesystem,
                repository: Arc::new(repository.clone()),
                ids: Arc::new(new_id_source()),
            },
            repository,
        }
    }
    pub(crate) fn context(&self) -> OperationServices {
        self.context.clone()
    }

    pub(crate) fn repository(&self) -> crate::git::GitTestRepository {
        self.repository.clone()
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
    fn each_context_owns_one_source_that_its_clones_share() {
        // GwzCoreSessionCrateMap §2: unique numbers come from the context's
        // own IdSource, whose prefix core draws from the OS random source.
        let context = TestWorld::memory().context();
        let clone = context.clone();
        assert_eq!(context.ids().next(), 0);
        assert_eq!(clone.ids().next(), 1, "a clone draws from the same source");
        let other = TestWorld::memory().context();
        assert_eq!(other.ids().next(), 0, "another context counts on its own");
        assert_ne!(other.ids().prefix(), context.ids().prefix());
        assert_ne!(new_id_source().prefix(), new_id_source().prefix());
    }

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
