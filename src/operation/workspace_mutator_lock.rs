use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::RUNTIME_DIR;

pub const WORKSPACE_MUTATOR_LOCK_NAME: &str = "workspace-mutator.lock";

pub struct WorkspaceMutatorLock {
    lease: crate::checked_artifact::WorkspaceRuntimeLease,
}

impl WorkspaceMutatorLock {
    /// Acquire the workspace mutation lock or return the standard busy error.
    pub fn acquire(root: &Path) -> ModelResult<Self> {
        Self::try_acquire(root)?.ok_or_else(|| {
            ModelError::new(
                ErrorCode::UnsupportedOperation,
                "workspace mutator lock is already held",
            )
        })
    }

    /// Try to acquire the workspace-wide mutation lock.
    ///
    /// The lock is an OS advisory exclusive lock on `.gwz/locks/workspace-mutator.lock`.
    /// The file itself is stable runtime state and may remain after a process exits. A
    /// remaining unlocked file is not stale. If a process dies while holding the lock,
    /// the OS releases the file lock with that process' file descriptor. This lock is
    /// intentionally workspace-wide, so stash and branch mutators in separate processes
    /// serialize before changing native Git state or `.gwz/` registry files.
    ///
    /// Advisory file locking must be reliable on the workspace filesystem. Network
    /// filesystems with broken advisory-lock semantics are unsupported for concurrent
    /// GWZ mutators; run mutating operations serially there.
    pub fn try_acquire(root: &Path) -> ModelResult<Option<Self>> {
        crate::checked_artifact::try_acquire_workspace_runtime(root)
            .map(|lease| lease.map(|lease| Self { lease }))
    }

    pub fn path(&self) -> &Path {
        self.lease.path()
    }

    #[allow(
        dead_code,
        reason = "R2-C0 freezes the checked catalog borrow before the C1 owner consumes it"
    )]
    pub(crate) fn catalog_mutation_lease(
        &self,
    ) -> crate::checked_artifact::CatalogMutationLeaseV1<'_> {
        self.lease.catalog_mutation_lease()
    }
}

pub fn lock_path(root: &Path) -> PathBuf {
    root.join(RUNTIME_DIR)
        .join("locks")
        .join(WORKSPACE_MUTATOR_LOCK_NAME)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const CHILD_ENV: &str = "GWZ_WORKSPACE_MUTATOR_LOCK_CHILD_ROOT";
    const BOOTSTRAP_GUARD_NAME: &str = "gwz-runtime-bootstrap-v1.lock";

    #[test]
    fn non_git_root_is_rejected_without_creating_runtime_state() {
        let temp = TempDir::new_plain("mutator-lock-non-git");

        assert!(WorkspaceMutatorLock::try_acquire(temp.path()).is_err());
        assert!(!temp.path().join(crate::workspace::RUNTIME_DIR).exists());
        assert!(!temp.path().join(BOOTSTRAP_GUARD_NAME).exists());
    }

    #[test]
    fn bootstrap_creates_only_the_fixed_runtime_grammar() {
        let temp = TempDir::new("mutator-lock-grammar");
        let git_dir = git2::Repository::open(temp.path())
            .unwrap()
            .path()
            .to_path_buf();

        let lease = WorkspaceMutatorLock::try_acquire(temp.path())
            .unwrap()
            .expect("runtime lease acquired");

        assert!(git_dir.join(BOOTSTRAP_GUARD_NAME).is_file());
        assert!(temp.path().join(crate::workspace::RUNTIME_DIR).is_dir());
        assert!(
            temp.path()
                .join(crate::workspace::RUNTIME_DIR)
                .join("locks")
                .is_dir()
        );
        assert_eq!(lease.path(), lock_path(temp.path()));
        assert!(lease.path().is_file());
    }

    #[test]
    fn wrong_kind_runtime_root_is_rejected() {
        let temp = TempDir::new("mutator-lock-wrong-kind-runtime");
        fs::write(temp.path().join(crate::workspace::RUNTIME_DIR), b"foreign").unwrap();

        assert!(WorkspaceMutatorLock::try_acquire(temp.path()).is_err());
        assert_eq!(
            fs::read(temp.path().join(crate::workspace::RUNTIME_DIR)).unwrap(),
            b"foreign"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_runtime_root_is_rejected_without_mutating_its_target() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("mutator-lock-symlink-runtime");
        let outside = TempDir::new_plain("mutator-lock-symlink-runtime-target");
        symlink(
            outside.path(),
            temp.path().join(crate::workspace::RUNTIME_DIR),
        )
        .unwrap();

        assert!(WorkspaceMutatorLock::try_acquire(temp.path()).is_err());
        assert!(!outside.path().join("locks").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_bootstrap_guard_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("mutator-lock-symlink-guard");
        let outside = temp.path().join("foreign-guard");
        fs::write(&outside, b"foreign").unwrap();
        let git_dir = git2::Repository::open(temp.path())
            .unwrap()
            .path()
            .to_path_buf();
        symlink(&outside, git_dir.join(BOOTSTRAP_GUARD_NAME)).unwrap();

        assert!(WorkspaceMutatorLock::try_acquire(temp.path()).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"foreign");
        assert!(!temp.path().join(crate::workspace::RUNTIME_DIR).exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_locks_directory_is_rejected_without_mutating_its_target() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("mutator-lock-symlink-locks");
        let runtime = temp.path().join(crate::workspace::RUNTIME_DIR);
        fs::create_dir(&runtime).unwrap();
        let outside = TempDir::new_plain("mutator-lock-symlink-locks-target");
        symlink(outside.path(), runtime.join("locks")).unwrap();

        assert!(WorkspaceMutatorLock::try_acquire(temp.path()).is_err());
        assert!(!outside.path().join(WORKSPACE_MUTATOR_LOCK_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_final_lease_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("mutator-lock-symlink-lease");
        let lock_dir = temp
            .path()
            .join(crate::workspace::RUNTIME_DIR)
            .join("locks");
        fs::create_dir_all(&lock_dir).unwrap();
        let outside = temp.path().join("foreign-lease");
        fs::write(&outside, b"foreign").unwrap();
        symlink(&outside, lock_dir.join(WORKSPACE_MUTATOR_LOCK_NAME)).unwrap();

        assert!(WorkspaceMutatorLock::try_acquire(temp.path()).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"foreign");
    }

    #[test]
    fn linked_worktree_uses_its_actual_git_directory_for_the_guard() {
        let main = TempDir::new("mutator-lock-main-worktree");
        let linked_parent = TempDir::new_plain("mutator-lock-linked-parent");
        let linked_root = linked_parent.path().join("linked");
        let main_repo = git2::Repository::open(main.path()).unwrap();
        main_repo.worktree("linked", &linked_root, None).unwrap();
        let linked_repo = git2::Repository::open(&linked_root).unwrap();
        let linked_git_dir = linked_repo.path().to_path_buf();

        let lease = WorkspaceMutatorLock::try_acquire(&linked_root)
            .unwrap()
            .expect("linked-worktree lease acquired");

        assert!(linked_git_dir.join(BOOTSTRAP_GUARD_NAME).is_file());
        assert_eq!(lease.path(), lock_path(&linked_root));
        assert!(!main_repo.path().join(BOOTSTRAP_GUARD_NAME).exists());
    }

    #[test]
    fn concurrent_first_acquirers_converge_on_one_final_lease() {
        use std::sync::{Arc, Barrier};

        const CONTENDERS: usize = 8;

        let temp = TempDir::new("mutator-lock-first-race");
        let root = Arc::new(temp.path().to_path_buf());
        let start = Arc::new(Barrier::new(CONTENDERS));
        let acquired = Arc::new(Barrier::new(CONTENDERS));
        let threads = (0..CONTENDERS)
            .map(|_| {
                let root = Arc::clone(&root);
                let start = Arc::clone(&start);
                let acquired = Arc::clone(&acquired);
                std::thread::spawn(move || {
                    start.wait();
                    let lease = WorkspaceMutatorLock::try_acquire(&root);
                    acquired.wait();
                    lease
                })
            })
            .collect::<Vec<_>>();

        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let winners = results.iter().filter(|lease| lease.is_some()).count();
        assert_eq!(winners, 1);
        assert!(lock_path(&root).is_file());
        let git_dir = git2::Repository::open(&*root).unwrap().path().to_path_buf();
        assert!(git_dir.join(BOOTSTRAP_GUARD_NAME).is_file());
    }

    #[test]
    fn lock_file_may_remain_and_be_reacquired_after_release() {
        let temp = TempDir::new("mutator-lock-reacquire");
        let first = WorkspaceMutatorLock::try_acquire(temp.path())
            .unwrap()
            .expect("first lock acquired");
        let path = first.path().to_path_buf();
        assert_eq!(path, lock_path(temp.path()));
        drop(first);

        assert!(path.is_file(), "lock file remains as runtime state");
        let second = WorkspaceMutatorLock::try_acquire(temp.path())
            .unwrap()
            .expect("released lock can be reacquired");
        drop(second);
    }

    #[test]
    fn separate_process_cannot_acquire_held_workspace_mutator_lock() {
        let temp = TempDir::new("mutator-lock-process");
        let _held = WorkspaceMutatorLock::try_acquire(temp.path())
            .unwrap()
            .expect("parent lock acquired");

        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--ignored")
            .arg("--exact")
            .arg("operation::workspace_mutator_lock::tests::child_process_observes_lock_contention")
            .env(CHILD_ENV, temp.path())
            .status()
            .unwrap();

        assert!(status.success(), "child test process failed: {status}");
    }

    #[test]
    #[ignore]
    fn child_process_observes_lock_contention() {
        let Some(root) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        assert!(
            WorkspaceMutatorLock::try_acquire(Path::new(&root))
                .unwrap()
                .is_none()
        );
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let temp = Self::new_plain(name);
            init_repo(temp.path());
            temp
        }

        fn new_plain(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("gwz-core-{name}-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    fn init_repo(path: &Path) {
        let repo = git2::Repository::init(path).unwrap();
        let mut index = repo.index().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
