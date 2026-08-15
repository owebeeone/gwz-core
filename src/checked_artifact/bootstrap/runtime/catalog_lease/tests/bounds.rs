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
fn maximum_batch_orders_and_deduplicates_without_hidden_sort_allocation() {
    let repo = TempRepo::new("maximum-batch-ordering");
    let request = CatalogLeaseTargetRequestV1::workspace(repo.path());
    let batch = CatalogLeaseTargetBatchV1::try_new(std::iter::repeat_n(
        request,
        MAX_CATALOG_LEASE_TARGETS_V1,
    ))
    .unwrap();
    let leases = CatalogLeaseSetV1::try_acquire(batch)
        .unwrap()
        .expect("maximum exact-duplicate batch");
    assert_eq!(leases.len(), 1);
    assert_catalog_roles_absent(&repo.path().join(crate::workspace::RUNTIME_DIR));
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

#[test]
fn case_fold_alias_scan_rejects_non_ascii_names_after_charging_them() {
    let parent = TempRepo::new("alias-native-equivalence");
    let scan_parent = parent.path().join("scan-parent");
    fs::create_dir(&scan_parent).unwrap();
    fs::write(scan_parent.join("ordinary-\u{212a}"), b"").unwrap();
    let retained = retain_ambient_directory(&scan_parent, "alias test parent").unwrap();

    assert!(
        reject_equivalent_alias_with_mode_for_test(
            retained.handle(),
            OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
            PathComponentMode::AsciiCaseFold,
        )
        .is_err()
    );
    assert!(
        reject_equivalent_alias_with_mode_for_test(
            retained.handle(),
            OsStr::new(GIT_CATALOG_MUTATOR_LOCK_NAME),
            PathComponentMode::Sensitive,
        )
        .is_ok()
    );
}

#[cfg(unix)]
#[test]
fn unix_non_utf8_alias_names_are_charged_losslessly() {
    use std::os::unix::ffi::OsStringExt;

    let name = std::ffi::OsString::from_vec(vec![0xff, b'x']);
    assert_eq!(native_name_charge_for_test(&name).unwrap(), (2, 2));
}

// Enable NTFS per-directory case sensitivity, fail-not-skip (R0-L probe
// doctrine; R2-F map G-2). The documented operator surface `fsutil.exe file
// setCaseSensitiveInfo <dir> enable` is probed first (hosted runners execute
// elevated); the direct FileCaseSensitiveInfo write is the fallback. A
// refusal panics with both reports instead of skipping: per the evidence
// map, a runner image that refuses the flag is itself a reviewed decision
// (GwzM5-8R2F-EvidenceMap.md §5.3 item 4), never a silent skip.
#[cfg(windows)]
fn enable_case_sensitivity(directory: &Path) {
    let fsutil = std::process::Command::new("fsutil.exe")
        .args(["file", "setCaseSensitiveInfo"])
        .arg(directory)
        .arg("enable")
        .output();
    let fsutil_report = match &fsutil {
        Ok(output) if output.status.success() => return,
        Ok(output) => format!(
            "status {}: {} {}",
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => format!("spawn failed: {error}"),
    };
    match set_case_sensitive_by_handle(directory) {
        Ok(()) => (),
        Err(direct) => panic!(
            "cannot enable NTFS per-directory case sensitivity on this runner \
             (fsutil: {fsutil_report}; FileCaseSensitiveInfo: {direct}); \
             fail-not-skip per the R0-L probe doctrine — route this refusal \
             to the R2-F evidence map's reviewed-decision list (§5.3 item 4)"
        ),
    }
}

#[cfg(windows)]
fn set_case_sensitive_by_handle(directory: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_CASE_SENSITIVE_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES,
        FileCaseSensitiveInfo, SetFileInformationByHandle,
    };

    const FILE_CS_FLAG_CASE_SENSITIVE_DIR: u32 = 1;

    let handle = fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(directory)?;
    let info = FILE_CASE_SENSITIVE_INFO {
        Flags: FILE_CS_FLAG_CASE_SENSITIVE_DIR,
    };
    if unsafe {
        SetFileInformationByHandle(
            handle.as_raw_handle(),
            FileCaseSensitiveInfo,
            std::ptr::addr_of!(info).cast(),
            std::mem::size_of::<FILE_CASE_SENSITIVE_INFO>() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

// Native execution of the `PathComponentMode::Sensitive` arm
// (provider/platform/windows.rs:50-53), which no Windows run had ever
// driven (R2-F evidence map W-2/G-2): the per-directory mode read must
// report Sensitive, case-variant spellings must coexist as distinct
// component identities, and the case-fold alias/rejection branches must
// not fire on layouts the forced-fold scan provably rejects.
#[cfg(windows)]
#[test]
fn native_case_sensitive_parent_reports_sensitive_and_bypasses_fold_rejections() {
    use crate::checked_artifact::capability::{
        DurableIdentityProvider, HostPlatform, PathEquivalenceProvider,
    };

    let parent = TempRepo::new("native-case-sensitive");
    let sensitive_parent = parent.path().join("cs-parent");
    fs::create_dir(&sensitive_parent).unwrap();
    enable_case_sensitivity(&sensitive_parent);

    let retained =
        retain_ambient_directory(&sensitive_parent, "case-sensitive test parent").unwrap();
    assert_eq!(
        HostPlatform.parent_mode(retained.handle()).unwrap(),
        PathComponentMode::Sensitive
    );

    // The reading is the per-directory flag, not an image-global setting:
    // an unflagged sibling on the same volume stays case-fold.
    let folded_parent = parent.path().join("fold-parent");
    fs::create_dir(&folded_parent).unwrap();
    let folded = retain_ambient_directory(&folded_parent, "case-fold test parent").unwrap();
    assert_eq!(
        HostPlatform.parent_mode(folded.handle()).unwrap(),
        PathComponentMode::AsciiCaseFold
    );

    // Case-variant spellings coexist and carry distinct component
    // identities on the sensitive parent.
    fs::write(sensitive_parent.join("target"), b"lower\n").unwrap();
    fs::write(sensitive_parent.join("TARGET"), b"upper\n").unwrap();
    assert_eq!(
        fs::read(sensitive_parent.join("target")).unwrap(),
        b"lower\n"
    );
    assert_eq!(
        fs::read(sensitive_parent.join("TARGET")).unwrap(),
        b"upper\n"
    );
    let lower = retained.handle().open("target").unwrap();
    let upper = retained.handle().open("TARGET").unwrap();
    let lower_identity = HostPlatform.file_identity(&lower).unwrap();
    let upper_identity = HostPlatform.file_identity(&upper).unwrap();
    assert_ne!(lower_identity.durable(), upper_identity.durable());
    assert_ne!(lower_identity.invocation(), upper_identity.invocation());

    // Fold-only rejection branches must not fire: plant a noncanonical
    // case-variant alias of the expected slot and a non-ASCII sibling —
    // the exact layouts the fold-mode scan rejects — and require the
    // native-mode scan (which reads Sensitive from the parent) to accept.
    let canonical = GIT_CATALOG_MUTATOR_LOCK_NAME;
    let alias = canonical.to_ascii_uppercase();
    assert_ne!(alias, canonical);
    fs::write(sensitive_parent.join(&alias), b"").unwrap();
    fs::write(sensitive_parent.join("ordinary-\u{212a}"), b"").unwrap();
    assert!(
        reject_equivalent_alias(
            retained.handle(),
            OsStr::new(canonical),
            "native case-sensitive parent",
        )
        .is_ok()
    );
    assert!(
        reject_equivalent_alias_with_mode_for_test(
            retained.handle(),
            OsStr::new(canonical),
            PathComponentMode::AsciiCaseFold,
        )
        .is_err()
    );
}
