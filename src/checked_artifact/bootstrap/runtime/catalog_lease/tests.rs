use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::super::fault::{RuntimeBootstrapFault, run_next_at};
use super::super::try_acquire_workspace_runtime;
use super::*;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn workspace_runtime_lease_borrows_only_its_exact_catalog_target() {
    let first = TempRepo::new("workspace-bound-first");
    let second = TempRepo::new("workspace-bound-second");
    let runtime = try_acquire_workspace_runtime(first.path())
        .unwrap()
        .expect("workspace runtime lease");

    let catalog = runtime.catalog_mutation_lease();
    assert_eq!(
        catalog.root_kind_for_test(),
        PreCatalogRootKindV1::Workspace
    );
    assert_eq!(
        catalog.canonical_target_path_for_test(),
        fs::canonicalize(first.path()).unwrap()
    );
    assert_ne!(catalog.canonical_target_path_for_test(), second.path());
}

#[test]
fn workspace_batch_target_contends_on_the_existing_compatibility_lock() {
    let repo = TempRepo::new("workspace-shared-final-slot");
    let runtime = try_acquire_workspace_runtime(repo.path())
        .unwrap()
        .expect("workspace runtime lease");

    assert!(
        CatalogLeaseSetV1::try_acquire([CatalogLeaseTargetRequestV1::workspace(repo.path())])
            .unwrap()
            .is_none()
    );
    assert_eq!(
        runtime
            .catalog_mutation_lease()
            .canonical_target_path_for_test(),
        fs::canonicalize(repo.path()).unwrap()
    );
    assert_catalog_roles_absent(&repo.path().join(crate::workspace::RUNTIME_DIR));
}

#[test]
fn linked_worktrees_resolving_to_one_git_target_share_one_final_lock() {
    let main = TempRepo::new("linked-main");
    let linked_parent = TempRepo::new("linked-parent");
    let linked_root = linked_parent.path().join("linked");
    let repository = git2::Repository::open(main.path()).unwrap();
    repository.worktree("linked", &linked_root, None).unwrap();
    let linked = git2::Repository::open(&linked_root).unwrap();
    assert_eq!(repository.commondir(), linked.commondir());

    let first = CatalogLeaseSetV1::try_acquire([CatalogLeaseTargetRequestV1::git_directory(
        repository.commondir(),
    )])
    .unwrap()
    .expect("first Git-target lease set");
    assert!(
        CatalogLeaseSetV1::try_acquire([CatalogLeaseTargetRequestV1::git_directory(
            linked.commondir(),
        )])
        .unwrap()
        .is_none()
    );
    assert_eq!(first.len(), 1);
}

#[test]
fn duplicate_targets_are_deduplicated_and_held_in_canonical_order() {
    let first = TempRepo::new("order-first");
    let second = TempRepo::new("order-second");
    let first_git = git2::Repository::open(first.path())
        .unwrap()
        .path()
        .to_path_buf();
    let second_git = git2::Repository::open(second.path())
        .unwrap()
        .path()
        .to_path_buf();

    let set = CatalogLeaseSetV1::try_acquire([
        CatalogLeaseTargetRequestV1::git_directory(&second_git),
        CatalogLeaseTargetRequestV1::git_directory(&first_git),
        CatalogLeaseTargetRequestV1::git_directory(&second_git),
    ])
    .unwrap()
    .expect("canonical lease set");

    assert_eq!(set.len(), 2);
    let keys = set
        .leases()
        .map(|lease| lease.canonical_order_key_for_test().to_vec())
        .collect::<Vec<_>>();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn wrong_kind_git_lock_rejects_before_any_catalog_namespace_mutation() {
    let repo = TempRepo::new("wrong-kind-git-lock");
    let git = git2::Repository::open(repo.path())
        .unwrap()
        .path()
        .to_path_buf();
    fs::create_dir(git.join(GIT_CATALOG_MUTATOR_LOCK_NAME)).unwrap();

    assert!(
        CatalogLeaseSetV1::try_acquire([CatalogLeaseTargetRequestV1::git_directory(&git)]).is_err()
    );
    assert_catalog_roles_absent(&git);
}

#[cfg(unix)]
#[test]
fn symlinked_git_lock_rejects_without_following_the_target() {
    use std::os::unix::fs::symlink;

    let repo = TempRepo::new("symlink-git-lock");
    let git = git2::Repository::open(repo.path())
        .unwrap()
        .path()
        .to_path_buf();
    let outside = repo.path().join("outside-lock");
    fs::write(&outside, b"outside\n").unwrap();
    symlink(&outside, git.join(GIT_CATALOG_MUTATOR_LOCK_NAME)).unwrap();

    assert!(
        CatalogLeaseSetV1::try_acquire([CatalogLeaseTargetRequestV1::git_directory(&git)]).is_err()
    );
    assert_eq!(fs::read(outside).unwrap(), b"outside\n");
    assert_catalog_roles_absent(&git);
}

#[cfg(unix)]
#[test]
fn replaced_git_lock_between_open_and_lock_is_rejected() {
    let repo = TempRepo::new("replaced-git-lock");
    let git = git2::Repository::open(repo.path())
        .unwrap()
        .path()
        .to_path_buf();
    run_next_at(RuntimeBootstrapFault::CatalogFinalLeaseOpen, {
        let lock = git.join(GIT_CATALOG_MUTATOR_LOCK_NAME);
        move || {
            fs::remove_file(&lock).unwrap();
            fs::write(lock, b"replacement\n").unwrap();
        }
    });

    assert!(
        CatalogLeaseSetV1::try_acquire([CatalogLeaseTargetRequestV1::git_directory(&git)]).is_err()
    );
    assert_eq!(
        fs::read(git.join(GIT_CATALOG_MUTATOR_LOCK_NAME)).unwrap(),
        b"replacement\n"
    );
    assert_catalog_roles_absent(&git);
}

#[cfg(unix)]
#[test]
fn substituted_git_target_after_final_lock_is_rejected() {
    let repo = TempRepo::new("substituted-git-target");
    let git = git2::Repository::open(repo.path())
        .unwrap()
        .path()
        .to_path_buf();
    run_next_at(RuntimeBootstrapFault::CatalogFinalLeaseLock, {
        let replacement = git.clone();
        let retired = repo.path().join("retired-git-directory");
        move || {
            fs::rename(&replacement, &retired).unwrap();
            fs::create_dir(&replacement).unwrap();
        }
    });

    assert!(
        CatalogLeaseSetV1::try_acquire([CatalogLeaseTargetRequestV1::git_directory(&git)]).is_err()
    );
    assert_catalog_roles_absent(&git);
}

#[cfg(unix)]
#[test]
fn target_reacquisition_mismatch_after_preparation_is_read_only() {
    let repo = TempRepo::new("reacquisition-mismatch");
    let git = git2::Repository::open(repo.path())
        .unwrap()
        .path()
        .to_path_buf();
    run_next_at(RuntimeBootstrapFault::CatalogPreparation, {
        let replacement = git.clone();
        let retired = repo.path().join("prepared-git-directory");
        move || {
            fs::rename(&replacement, &retired).unwrap();
            fs::create_dir(&replacement).unwrap();
        }
    });

    assert!(
        CatalogLeaseSetV1::try_acquire([CatalogLeaseTargetRequestV1::git_directory(&git)]).is_err()
    );
    assert_catalog_roles_absent(&git);
}

#[test]
fn later_target_contention_releases_every_earlier_final_lock() {
    let first = TempRepo::new("contention-first");
    let second = TempRepo::new("contention-second");
    let first_request = git_request(first.path());
    let second_request = git_request(second.path());
    let mut ordered = vec![first_request, second_request];
    ordered.sort_by_key(|request| request.canonical_order_key_for_test().unwrap());

    let _later = CatalogLeaseSetV1::try_acquire([ordered[1].clone()])
        .unwrap()
        .expect("later target blocker");
    assert!(
        CatalogLeaseSetV1::try_acquire(ordered.clone())
            .unwrap()
            .is_none()
    );
    assert!(
        CatalogLeaseSetV1::try_acquire([ordered[0].clone()])
            .unwrap()
            .is_some(),
        "failed batch must release the earlier final lock"
    );
}

#[test]
fn preparation_failure_occurs_while_no_final_target_lock_is_held() {
    let first = TempRepo::new("prepare-first");
    let second = TempRepo::new("prepare-second");
    let mut ordered = vec![git_request(first.path()), git_request(second.path())];
    ordered.sort_by_key(|request| request.canonical_order_key_for_test().unwrap());
    fs::create_dir(
        ordered[1]
            .canonical_target_path_for_test()
            .unwrap()
            .join(GIT_CATALOG_MUTATOR_LOCK_NAME),
    )
    .unwrap();

    assert!(CatalogLeaseSetV1::try_acquire(ordered.clone()).is_err());
    assert!(
        CatalogLeaseSetV1::try_acquire([ordered[0].clone()])
            .unwrap()
            .is_some(),
        "preparation may not retain an earlier final lock"
    );
}

fn git_request(worktree: &Path) -> CatalogLeaseTargetRequestV1 {
    let git = git2::Repository::open(worktree)
        .unwrap()
        .path()
        .to_path_buf();
    CatalogLeaseTargetRequestV1::git_directory(git)
}

fn assert_catalog_roles_absent(parent: &Path) {
    for name in [
        "checked-artifacts-catalog-bootstrap-v1.active",
        "checked-artifacts-catalog-bootstrap-v1.staging",
        "checked-artifacts",
    ] {
        assert!(
            !parent.join(name).exists(),
            "unexpected catalog role {name}"
        );
    }
}

struct TempRepo(PathBuf);

impl TempRepo {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gwz-catalog-lease-{name}-{}-{}",
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
