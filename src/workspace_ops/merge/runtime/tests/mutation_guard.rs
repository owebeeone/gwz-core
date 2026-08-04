use super::super::*;
use crate::workspace_ops::tests::TempDir;
use std::path::Path;

#[test]
fn authoritative_guard_retains_mutator_lock_until_drop() {
    let root = TempDir::new("merge-retained-guard");
    let workspace = crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    };
    let guard = acquire_workspace_mutation_guard(
        root.path(),
        Some(&workspace),
        crate::operation::OpenMergeCommand::Push,
    )
    .unwrap();
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
    assert!(
        crate::operation::WorkspaceMutatorLock::try_acquire(root.path())
            .unwrap()
            .is_some()
    );
}
