use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::fault::{RuntimeBootstrapFault, run_next_at};
use super::*;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
#[test]
fn changed_final_identity_between_open_and_lock_is_rejected() {
    let temp = TempRepo::new("changed-final");
    let locks = temp
        .path()
        .join(crate::workspace::RUNTIME_DIR)
        .join("locks");
    fs::create_dir_all(&locks).unwrap();
    let lease = locks.join(WORKSPACE_MUTATOR_LOCK_NAME);
    fs::write(&lease, b"first").unwrap();
    run_next_at(RuntimeBootstrapFault::FinalLeaseOpen, {
        let lease = lease.clone();
        move || {
            fs::remove_file(&lease).unwrap();
            fs::write(lease, b"replacement").unwrap();
        }
    });

    assert!(try_acquire_workspace_runtime(temp.path()).is_err());
    assert_eq!(fs::read(lease).unwrap(), b"replacement");
}

#[cfg(unix)]
#[test]
fn substituted_runtime_parent_after_final_lock_is_rejected() {
    let temp = TempRepo::new("substituted-parent");
    run_next_at(RuntimeBootstrapFault::FinalLeaseLock, {
        let root = temp.path().to_path_buf();
        move || {
            let runtime = root.join(crate::workspace::RUNTIME_DIR);
            fs::rename(&runtime, root.join(".gwz-replaced")).unwrap();
            let replacement_locks = runtime.join("locks");
            fs::create_dir_all(&replacement_locks).unwrap();
            fs::write(
                replacement_locks.join(WORKSPACE_MUTATOR_LOCK_NAME),
                b"replacement",
            )
            .unwrap();
        }
    });

    assert!(try_acquire_workspace_runtime(temp.path()).is_err());
    assert_eq!(
        fs::read(
            temp.path()
                .join(crate::workspace::RUNTIME_DIR)
                .join("locks")
                .join(WORKSPACE_MUTATOR_LOCK_NAME)
        )
        .unwrap(),
        b"replacement"
    );
}

#[cfg(unix)]
#[test]
fn changed_linked_worktree_git_indirection_after_lock_is_rejected() {
    let main = TempRepo::new("linked-main");
    let linked_parent = TempRepo::new("linked-parent");
    let linked_root = linked_parent.path().join("linked");
    let main_repo = git2::Repository::open(main.path()).unwrap();
    main_repo.worktree("linked", &linked_root, None).unwrap();
    run_next_at(RuntimeBootstrapFault::FinalLeaseLock, {
        let git_file = linked_root.join(".git");
        let replacement_git_dir = main_repo.path().to_path_buf();
        move || {
            fs::write(
                git_file,
                format!("gitdir: {}\n", replacement_git_dir.display()),
            )
            .unwrap();
        }
    });

    assert!(try_acquire_workspace_runtime(&linked_root).is_err());
}

struct TempRepo(PathBuf);

impl TempRepo {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gwz-runtime-bootstrap-{name}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        let repository = git2::Repository::init(&path).unwrap();
        let mut index = repository.index().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repository.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid").unwrap();
        repository
            .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
