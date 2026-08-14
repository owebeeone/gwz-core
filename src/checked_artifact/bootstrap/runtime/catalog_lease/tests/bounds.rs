use super::*;
use crate::checked_artifact::bootstrap::runtime::retain_ambient_directory;
use crate::checked_artifact::capability::PathComponentMode;

#[test]
fn target_batch_is_nonempty_bounded_and_stops_an_infinite_iterator() {
    let repo = TempRepo::new("batch-bounds");
    let request = CatalogLeaseTargetRequestV1::workspace(repo.path());
    assert!(CatalogLeaseTargetBatchV1::try_new(std::iter::empty()).is_err());
    assert!(
        CatalogLeaseTargetBatchV1::try_new(std::iter::repeat_n(
            request.clone(),
            MAX_CATALOG_LEASE_TARGETS_V1
        ))
        .is_ok()
    );
    assert!(
        CatalogLeaseTargetBatchV1::try_new(std::iter::repeat_n(
            request.clone(),
            MAX_CATALOG_LEASE_TARGETS_V1 + 1,
        ))
        .is_err()
    );
    assert!(CatalogLeaseTargetBatchV1::try_new(std::iter::repeat(request)).is_err());
}

#[test]
fn batch_allocation_failure_rejects_before_runtime_or_catalog_mutation() {
    let repo = TempRepo::new("batch-allocation-failure");
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(repo.path());
    let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
    fail_next_catalog_batch_allocation_for_test();
    assert!(CatalogLeaseSetV1::try_acquire(batch).is_err());
    let git = git2::Repository::open(repo.path())
        .unwrap()
        .commondir()
        .to_path_buf();
    assert!(!git.join(BOOTSTRAP_GUARD_NAME).exists());
    assert!(!git.join(GIT_CATALOG_MUTATOR_LOCK_NAME).exists());
    assert_catalog_roles_absent(&git);
}

#[test]
fn case_fold_alias_scan_has_literal_lossless_parent_budgets() {
    let parent = TempRepo::new("alias-bounds");
    let scan_parent = parent.path().join("scan-parent");
    fs::create_dir(&scan_parent).unwrap();
    for ordinal in 0..MAX_CATALOG_ALIAS_PARENT_ENTRIES_V1 {
        fs::write(scan_parent.join(format!("ordinary-{ordinal:04}")), b"").unwrap();
    }
    let retained = retain_ambient_directory(&scan_parent, "alias test parent").unwrap();
    assert!(
        reject_equivalent_alias_with_mode_for_test(
            retained.handle(),
            OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
            PathComponentMode::AsciiCaseFold,
        )
        .is_ok()
    );
    fs::write(scan_parent.join("one-too-many"), b"").unwrap();
    assert!(
        reject_equivalent_alias_with_mode_for_test(
            retained.handle(),
            OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
            PathComponentMode::AsciiCaseFold,
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn unix_non_utf8_alias_names_are_charged_losslessly() {
    use std::os::unix::ffi::OsStringExt;

    let name = std::ffi::OsString::from_vec(vec![0xff, b'x']);
    assert_eq!(native_name_charge_for_test(&name).unwrap(), (2, 2));
}
