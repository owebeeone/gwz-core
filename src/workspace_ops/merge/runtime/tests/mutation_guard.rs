use super::super::*;
use super::write_open_v1_record;
use crate::model::ErrorCode;
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

/// **The guard's v1 twin, and the second half of the shipped defect.** The
/// authoritative mutation guard discovered the open record through the same
/// v0-only store, so `gwz commit`, `capture`, `push` and `pull` on a
/// conflicted `--no-ff` merge answered `UnsupportedRecordVersion` instead of
/// the open-merge remedy — while the record sat there, open and recoverable.
///
/// `Block` rows refuse and take no lock past the refusal; `Allow` rows still
/// hand back a real guard on the very same workspace.
#[test]
fn the_authoritative_guard_blocks_a_mutation_against_an_open_v1_record() {
    let root = TempDir::new("merge-guard-v1");
    git2::Repository::init(root.path()).unwrap();
    let merge_id = write_open_v1_record(root.path());
    let workspace = crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    };

    for blocked in [
        crate::operation::OpenMergeCommand::Commit,
        crate::operation::OpenMergeCommand::Capture,
        crate::operation::OpenMergeCommand::Push,
        crate::operation::OpenMergeCommand::Pull,
        crate::operation::OpenMergeCommand::Snapshot,
        crate::operation::OpenMergeCommand::TagMutate,
        crate::operation::OpenMergeCommand::BranchMutate,
    ] {
        for dry_run in [false, true] {
            let error =
                acquire_workspace_mutation_guard(root.path(), Some(&workspace), blocked, dry_run)
                    .err()
                    .expect("a Block row must refuse against an open v1 record");
            assert_eq!(
                error.code,
                ErrorCode::OpenOperation,
                "{blocked:?} {dry_run}"
            );
            for named in [
                merge_id.as_str(),
                "merge status",
                "merge continue",
                "merge abort",
            ] {
                assert!(
                    error.message.contains(named),
                    "{blocked:?} {dry_run}: {}",
                    error.message
                );
            }
            // The refusal released the mutator lock it took to check.
            assert!(
                crate::operation::WorkspaceMutatorLock::try_acquire(root.path())
                    .unwrap()
                    .is_some()
            );
        }
    }

    let allowed = acquire_workspace_mutation_guard(
        root.path(),
        Some(&workspace),
        crate::operation::OpenMergeCommand::Status,
        false,
    )
    .unwrap();
    assert!(allowed.writes().is_some());
    assert_eq!(allowed.root(), root.path());
}

/// `add` is the `Conditional` row: the guard admits it against an open v1
/// record — the narrower participant check in `enforce_open_merge_stage_targets`
/// owns the refusal — and resolves the workspace root from a nested start path
/// with no explicit workspace, through the envelope-aware ancestor walk.
#[test]
fn the_guard_admits_stage_and_resolves_the_root_from_an_open_v1_record() {
    let root = TempDir::new("merge-guard-v1-stage");
    git2::Repository::init(root.path()).unwrap();
    write_open_v1_record(root.path());
    let nested = root.path().join("members/a/src");
    std::fs::create_dir_all(&nested).unwrap();

    let access = acquire_workspace_mutation_guard(
        &nested,
        None,
        crate::operation::OpenMergeCommand::StageConflictResolution,
        false,
    )
    .unwrap();

    assert!(access.writes().is_some());
    assert_eq!(access.root(), root.path());
}
