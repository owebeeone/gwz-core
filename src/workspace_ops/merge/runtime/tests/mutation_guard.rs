use super::super::*;
use crate::workspace_ops::tests::TempDir;
use std::path::Path;

#[test]
fn authoritative_guard_retains_mutator_lock_until_drop() {
    let root = TempDir::new("merge-retained-guard");
    git2::Repository::init(root.path()).unwrap();
    let workspace = crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    };
    let guard = acquire_workspace_mutation_guard(
        root.path(),
        Some(&workspace),
        crate::operation::OpenMergeCommand::Push,
        false,
    )
    .unwrap();
    assert!(guard.writes().is_some());
    assert!(
        crate::operation::WorkspaceMutatorLock::try_acquire(root.path())
            .unwrap()
            .is_none()
    );
    drop(guard);
    assert!(
        crate::operation::WorkspaceMutatorLock::try_acquire(root.path())
            .unwrap()
            .is_some()
    );
}

#[test]
fn dry_run_guard_checks_the_effective_root_without_taking_the_mutator_lock() {
    let root = TempDir::new("merge-dry-run-no-lock");
    git2::Repository::init(root.path()).unwrap();
    let workspace = crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    };

    let (guard, resolved) = guarded_workspace_root(
        Path::new("/unrelated/cwd"),
        Some(&workspace),
        crate::operation::OpenMergeCommand::MergeStart,
        true,
    )
    .unwrap();

    assert!(guard.is_none());
    assert_eq!(resolved, root.path());
    assert!(!root.path().join(crate::workspace::RUNTIME_DIR).exists());
    assert!(
        crate::operation::WorkspaceMutatorLock::try_acquire(root.path())
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_dry_run_acquisition_yields_no_write_authority() {
    // The seam's whole point: a caller that states `dry_run` cannot reach the guard
    // that authorizes a write, so a new handler cannot forget the flag by omission.
    let root = TempDir::new("merge-plan-only-guard");
    git2::Repository::init(root.path()).unwrap();
    let workspace = crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    };
    let access = acquire_workspace_mutation_guard(
        root.path(),
        Some(&workspace),
        crate::operation::OpenMergeCommand::Push,
        true,
    )
    .unwrap();
    assert!(access.is_dry_run());
    assert!(access.writes().is_none());
    assert_eq!(access.root(), root.path());
}
