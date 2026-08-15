use super::*;
use crate::checked_artifact::bootstrap::runtime::fault::{RuntimeBootstrapFault, run_next_at};
use crate::checked_artifact::bootstrap::runtime::{
    LOCKS_DIRECTORY_NAME, WORKSPACE_MUTATOR_LOCK_NAME,
};

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn post_return_git_target_replacement_invalidates_preflight_authority() {
    let repo = TempRepo::new("post-return-target-replacement");
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(repo.path());
    let set = try_acquire([request.clone()])
        .unwrap()
        .expect("original target lease");
    let git = fs::canonicalize(git2::Repository::open(repo.path()).unwrap().commondir()).unwrap();
    let retired = repo.path().join("retired-git-target");
    fs::rename(&git, &retired).unwrap();
    git2::Repository::init(repo.path()).unwrap();

    let replacement = try_acquire([request])
        .unwrap()
        .expect("replacement target has a distinct lock");
    assert!(set.leases().next().unwrap().begin_preflight().is_err());
    drop(replacement);
    assert_catalog_roles_absent(&git);
    assert_catalog_roles_absent(&retired);
}

#[test]
fn post_return_git_lock_replacement_invalidates_every_later_edge() {
    let repo = TempRepo::new("post-return-lock-replacement");
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(repo.path());
    let set = try_acquire([request.clone()])
        .unwrap()
        .expect("original target lease");
    let witness = set
        .leases()
        .next()
        .unwrap()
        .begin_preflight()
        .expect("initial lease-derived witness");
    let git = fs::canonicalize(git2::Repository::open(repo.path()).unwrap().commondir()).unwrap();
    let lock = git.join(GIT_CATALOG_MUTATOR_LOCK_NAME);
    let retired = git.join("retired-catalog-mutator.lock");
    fs::rename(&lock, &retired).unwrap();
    fs::write(&lock, b"replacement\n").unwrap();

    let replacement = try_acquire([request])
        .unwrap()
        .expect("replacement slot is independently lockable");
    assert!(witness.revalidate_for_test().is_err());
    drop(replacement);
    assert_catalog_roles_absent(&git);
}

#[test]
fn workspace_compatibility_borrow_revalidates_its_named_slot() {
    let repo = TempRepo::new("workspace-borrow-lock-replacement");
    let runtime = try_acquire_workspace_runtime(repo.path())
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime
        .catalog_mutation_lease()
        .begin_preflight()
        .expect("initial workspace witness");
    let lock = repo
        .path()
        .join(crate::workspace::RUNTIME_DIR)
        .join(LOCKS_DIRECTORY_NAME)
        .join(WORKSPACE_MUTATOR_LOCK_NAME);
    let retired = lock.with_file_name("retired-workspace-mutator.lock");
    fs::rename(&lock, &retired).unwrap();
    fs::write(&lock, b"replacement\n").unwrap();

    let replacement = try_acquire([CatalogLeaseTargetRequestV1::workspace(repo.path())])
        .unwrap()
        .expect("replacement workspace slot is independently lockable");
    assert!(witness.revalidate_for_test().is_err());
    drop(replacement);
    assert_catalog_roles_absent(&repo.path().join(crate::workspace::RUNTIME_DIR));
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn workspace_compatibility_borrow_rejects_post_return_root_replacement() {
    let repo = TempRepo::new("workspace-borrow-root-replacement");
    let root = repo.path().to_path_buf();
    let retired = root.with_extension("retired");
    let runtime = try_acquire_workspace_runtime(&root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime
        .catalog_mutation_lease()
        .begin_preflight()
        .expect("initial workspace witness");
    fs::rename(&root, &retired).unwrap();
    fs::create_dir(&root).unwrap();
    git2::Repository::init(&root).unwrap();

    let replacement = try_acquire([CatalogLeaseTargetRequestV1::workspace(&root)])
        .unwrap()
        .expect("replacement workspace has a distinct final slot");
    assert!(witness.revalidate_for_test().is_err());
    drop(replacement);
    assert_catalog_roles_absent(&root.join(crate::workspace::RUNTIME_DIR));
    assert_catalog_roles_absent(&retired.join(crate::workspace::RUNTIME_DIR));
    fs::remove_dir_all(&retired).unwrap();
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn duplicate_location_with_changed_identity_rejects_before_preparation_in_both_orders() {
    let repo = TempRepo::new("duplicate-location-identity-race");
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(repo.path());
    let first = RetainedCatalogTargetV1::retain(&request).unwrap();
    let git = first.binding.canonical_path.clone();
    let retired = repo.path().join("retired-before-dedupe");
    fs::rename(&git, &retired).unwrap();
    git2::Repository::init(repo.path()).unwrap();
    let second = RetainedCatalogTargetV1::retain(&request).unwrap();
    assert_ne!(
        first.binding.durable_identity,
        second.binding.durable_identity
    );

    for reverse in [false, true] {
        let prepared = if reverse {
            vec![
                prepared_target(&request, &second, second.binding.clone()),
                prepared_target(&request, &first, first.binding.clone()),
            ]
        } else {
            vec![
                prepared_target(&request, &first, first.binding.clone()),
                prepared_target(&request, &second, second.binding.clone()),
            ]
        };
        assert!(deduplicate_exact_locations(prepared).is_err());
    }
    assert!(!git.join(GIT_CATALOG_MUTATOR_LOCK_NAME).exists());
    assert!(!git.join(BOOTSTRAP_GUARD_NAME).exists());
    assert!(!retired.join(GIT_CATALOG_MUTATOR_LOCK_NAME).exists());
    assert!(!retired.join(BOOTSTRAP_GUARD_NAME).exists());
}

#[test]
fn duplicate_location_requires_exact_live_target_and_repository_bindings() {
    let repo = TempRepo::new("duplicate-location-live-binding");
    let request = CatalogLeaseTargetRequestV1::workspace(repo.path());
    let retained = RetainedCatalogTargetV1::retain(&request).unwrap();

    for changed in [
        {
            let mut changed = retained.binding.clone();
            changed.target_invocation_identity.push(0);
            changed
        },
        {
            let mut changed = retained.binding.clone();
            changed.related_git_invocation_identity.push(0);
            changed
        },
    ] {
        let prepared = vec![
            prepared_target(&request, &retained, retained.binding.clone()),
            prepared_target(&request, &retained, changed),
        ];
        assert!(deduplicate_exact_locations(prepared).is_err());
    }
    assert!(!repo.path().join(BOOTSTRAP_GUARD_NAME).exists());
    assert!(!repo.path().join(crate::workspace::RUNTIME_DIR).exists());
}

#[test]
fn linked_membership_drift_after_initial_retention_rejects_before_preparation_in_both_orders() {
    for linked_first in [false, true] {
        let main = TempRepo::new("membership-initial-main");
        let linked_parent = TempRepo::new("membership-initial-parent");
        let linked_root = linked_parent.path().join("linked");
        git2::Repository::open(main.path())
            .unwrap()
            .worktree("linked", &linked_root, None)
            .unwrap();
        let common =
            fs::canonicalize(git2::Repository::open(main.path()).unwrap().commondir()).unwrap();
        let main_request =
            CatalogLeaseTargetRequestV1::repository_common_git_directory(main.path());
        let linked_request =
            CatalogLeaseTargetRequestV1::repository_common_git_directory(&linked_root);
        let requests = if linked_first {
            [linked_request, main_request]
        } else {
            [main_request, linked_request]
        };
        run_next_at(RuntimeBootstrapFault::CatalogInitialRetentionComplete, {
            let main = main.path().to_path_buf();
            let linked = linked_root.clone();
            move || repoint_worktree_membership(&main, &linked)
        });

        assert!(try_acquire(requests).is_err());
        assert!(!common.join(BOOTSTRAP_GUARD_NAME).exists());
        assert!(!common.join(GIT_CATALOG_MUTATOR_LOCK_NAME).exists());
        assert_catalog_roles_absent(&common);
    }
}

#[test]
fn every_deduplicated_linked_membership_is_revalidated_after_return() {
    for linked_first in [false, true] {
        let main = TempRepo::new("membership-return-main");
        let linked_parent = TempRepo::new("membership-return-parent");
        let linked_root = linked_parent.path().join("linked");
        git2::Repository::open(main.path())
            .unwrap()
            .worktree("linked", &linked_root, None)
            .unwrap();
        let main_request =
            CatalogLeaseTargetRequestV1::repository_common_git_directory(main.path());
        let linked_request =
            CatalogLeaseTargetRequestV1::repository_common_git_directory(&linked_root);
        let requests = if linked_first {
            [linked_request, main_request]
        } else {
            [main_request, linked_request]
        };
        let set = try_acquire(requests)
            .unwrap()
            .expect("shared common-Git target lease");
        repoint_worktree_membership(main.path(), &linked_root);

        assert!(set.leases().next().unwrap().begin_preflight().is_err());
        let common =
            fs::canonicalize(git2::Repository::open(main.path()).unwrap().commondir()).unwrap();
        assert_catalog_roles_absent(&common);
    }
}

#[test]
fn single_git_request_membership_is_revalidated_after_return() {
    let main = TempRepo::new("membership-single-main");
    let linked_parent = TempRepo::new("membership-single-parent");
    let linked_root = linked_parent.path().join("linked");
    git2::Repository::open(main.path())
        .unwrap()
        .worktree("linked", &linked_root, None)
        .unwrap();
    let set =
        try_acquire([CatalogLeaseTargetRequestV1::repository_common_git_directory(&linked_root)])
            .unwrap()
            .expect("single linked-worktree lease");
    repoint_worktree_membership(main.path(), &linked_root);

    assert!(set.leases().next().unwrap().begin_preflight().is_err());
    let common =
        fs::canonicalize(git2::Repository::open(main.path()).unwrap().commondir()).unwrap();
    assert_catalog_roles_absent(&common);
}

#[test]
fn linked_membership_drift_after_preparation_rejects_before_final_acquisition() {
    let main = TempRepo::new("membership-prepared-main");
    let linked_parent = TempRepo::new("membership-prepared-parent");
    let linked_root = linked_parent.path().join("linked");
    git2::Repository::open(main.path())
        .unwrap()
        .worktree("linked", &linked_root, None)
        .unwrap();
    run_next_at(RuntimeBootstrapFault::CatalogPreparation, {
        let main = main.path().to_path_buf();
        let linked = linked_root.clone();
        move || repoint_worktree_membership(&main, &linked)
    });

    assert!(
        try_acquire([
            CatalogLeaseTargetRequestV1::repository_common_git_directory(main.path()),
            CatalogLeaseTargetRequestV1::repository_common_git_directory(&linked_root),
        ])
        .is_err()
    );
    let common =
        fs::canonicalize(git2::Repository::open(main.path()).unwrap().commondir()).unwrap();
    assert_catalog_roles_absent(&common);
}

#[cfg(unix)]
#[test]
fn symlinked_repository_request_rejects_before_runtime_mutation() {
    use std::os::unix::fs::symlink;

    let repo = TempRepo::new("membership-symlink-target");
    let link_parent = TempRepo::new("membership-symlink-parent");
    let link = link_parent.path().join("repository-link");
    symlink(repo.path(), &link).unwrap();
    let common =
        fs::canonicalize(git2::Repository::open(repo.path()).unwrap().commondir()).unwrap();

    assert!(
        try_acquire([CatalogLeaseTargetRequestV1::repository_common_git_directory(&link),])
            .is_err()
    );
    assert!(!common.join(BOOTSTRAP_GUARD_NAME).exists());
    assert!(!common.join(GIT_CATALOG_MUTATOR_LOCK_NAME).exists());
    assert_catalog_roles_absent(&common);
}

fn repoint_worktree_membership(main: &Path, linked: &Path) {
    let main_git = fs::canonicalize(git2::Repository::open(main).unwrap().path()).unwrap();
    fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", main_git.display()),
    )
    .unwrap();
}

fn prepared_target(
    request: &CatalogLeaseTargetRequestV1,
    retained: &RetainedCatalogTargetV1,
    binding: CatalogTargetBindingV1,
) -> PreparedCatalogTargetV1 {
    PreparedCatalogTargetV1 {
        requests: vec![PreparedCatalogRequestV1 {
            request: request.clone(),
            git_association: retained.git_association_binding().cloned(),
        }],
        binding,
    }
}
