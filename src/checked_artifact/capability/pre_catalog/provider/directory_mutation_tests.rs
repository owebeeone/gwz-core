use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::directory_mutation::{CatalogDirectoryMutationFaultV1, run_next_at};
use crate::checked_artifact::bootstrap::try_acquire_workspace_runtime;
use crate::checked_artifact::catalog::recover_or_create;
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::InfrastructureSlotV1;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2c2-directory-mutation-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        fs::create_dir(root.join(".gwz")).unwrap();
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn private(&self, name: CatalogPrivateNameV1) -> PathBuf {
        self.root
            .join(".gwz")
            .join(std::str::from_utf8(name.leaf_bytes()).expect("fixed catalog name is ASCII"))
    }

    fn run(&self) -> Result<(), crate::checked_artifact::capability::CheckedFsError> {
        let runtime = try_acquire_workspace_runtime(self.path())
            .unwrap()
            .expect("workspace lease");
        recover_or_create(runtime.catalog_mutation_lease()).map(|_| ())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn substituted_staging_after_open_is_not_mutated() {
    let fixture = Fixture::new("staging-after-open");
    let staging = fixture.private(CatalogPrivateNameV1::BootstrapStaging);
    let displaced = fixture.root.join(".gwz/displaced-staging");
    let sentinel = b"replacement staging\n".to_vec();
    run_next_at(CatalogDirectoryMutationFaultV1::StagingAfterOpen, {
        let staging = staging.clone();
        let displaced = displaced.clone();
        let sentinel = sentinel.clone();
        move || {
            fs::rename(&staging, displaced).unwrap();
            fs::create_dir(&staging).unwrap();
            fs::write(staging.join("foreign"), sentinel).unwrap();
        }
    });

    assert!(fixture.run().is_err());

    assert_eq!(fs::read(staging.join("foreign")).unwrap(), sentinel);
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn substituted_staging_before_final_publish_is_not_published() {
    let fixture = Fixture::new("final-before-rename");
    let staging = fixture.private(CatalogPrivateNameV1::BootstrapStaging);
    let final_directory = fixture.private(CatalogPrivateNameV1::Final);
    let displaced = fixture.root.join(".gwz/displaced-exact-staging");
    let sentinel = b"replacement final source\n".to_vec();
    run_next_at(CatalogDirectoryMutationFaultV1::FinalPublishBeforeRename, {
        let staging = staging.clone();
        let displaced = displaced.clone();
        let sentinel = sentinel.clone();
        move || {
            fs::rename(&staging, displaced).unwrap();
            fs::create_dir(&staging).unwrap();
            fs::write(staging.join("foreign"), sentinel).unwrap();
        }
    });

    assert!(fixture.run().is_err());

    assert_eq!(fs::read(staging.join("foreign")).unwrap(), sentinel);
    assert!(!final_directory.exists());
}

#[test]
fn changed_staging_contents_before_final_publish_are_not_published() {
    let fixture = Fixture::new("final-content-drift");
    let staging = fixture.private(CatalogPrivateNameV1::BootstrapStaging);
    let final_directory = fixture.private(CatalogPrivateNameV1::Final);
    run_next_at(CatalogDirectoryMutationFaultV1::FinalPublishBeforeRename, {
        let staging = staging.clone();
        move || fs::write(staging.join("foreign"), b"content drift\n").unwrap()
    });

    assert!(fixture.run().is_err());

    assert_eq!(
        fs::read(staging.join("foreign")).unwrap(),
        b"content drift\n"
    );
    assert!(!final_directory.exists());
}

#[test]
fn interior_drift_after_final_recheck_is_rejected_inside_the_primitive() {
    let fixture = Fixture::new("final-drift-after-recheck");
    let staging = fixture.private(CatalogPrivateNameV1::BootstrapStaging);
    let final_directory = fixture.private(CatalogPrivateNameV1::Final);
    run_next_at(
        CatalogDirectoryMutationFaultV1::FinalPublishAfterInteriorRecheck,
        {
            let staging = staging.clone();
            move || fs::write(staging.join("foreign"), b"sliver drift\n").unwrap()
        },
    );

    assert!(fixture.run().is_err());

    assert_eq!(
        fs::read(staging.join("foreign")).unwrap(),
        b"sliver drift\n"
    );
    assert!(!final_directory.exists());
}

#[test]
fn every_anchor_namespace_prefix_converges_after_restart() {
    for point in [
        CatalogDirectoryMutationFaultV1::AnchorAfterPublishA,
        CatalogDirectoryMutationFaultV1::AnchorAfterMoveToB,
        CatalogDirectoryMutationFaultV1::AnchorAfterReturnA,
    ] {
        let fixture = Fixture::new(&format!("anchor-restart-{point:?}"));
        run_next_at(point, || panic!("simulated process stop after anchor edge"));

        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fixture.run()));

        assert!(interrupted.is_err());
        fixture.run().unwrap();
    }
}

#[test]
fn substituted_active_before_retirement_is_not_moved_into_final() {
    let fixture = Fixture::new("active-before-retire");
    let active = fixture.private(CatalogPrivateNameV1::BootstrapActive);
    let displaced = fixture.root.join(".gwz/displaced-active");
    let sentinel = b"replacement active\n".to_vec();
    run_next_at(CatalogDirectoryMutationFaultV1::ActiveRetireBeforeRename, {
        let active = active.clone();
        let displaced = displaced.clone();
        let sentinel = sentinel.clone();
        move || {
            fs::rename(&active, displaced).unwrap();
            fs::write(&active, sentinel).unwrap();
        }
    });

    assert!(fixture.run().is_err());

    assert_eq!(fs::read(&active).unwrap(), sentinel);
    assert!(
        !fixture
            .private(CatalogPrivateNameV1::Final)
            .join(InfrastructureSlotV1::CatalogBootstrapRetired.name())
            .exists()
    );
}

#[test]
fn changed_final_contents_before_retirement_leave_active_in_place() {
    let fixture = Fixture::new("final-before-retire-drift");
    let active = fixture.private(CatalogPrivateNameV1::BootstrapActive);
    let final_directory = fixture.private(CatalogPrivateNameV1::Final);
    run_next_at(CatalogDirectoryMutationFaultV1::ActiveRetireBeforeRename, {
        let final_directory = final_directory.clone();
        move || fs::write(final_directory.join("foreign"), b"content drift\n").unwrap()
    });

    assert!(fixture.run().is_err());

    assert!(active.exists());
    assert_eq!(
        fs::read(final_directory.join("foreign")).unwrap(),
        b"content drift\n"
    );
    assert!(
        !final_directory
            .join(InfrastructureSlotV1::CatalogBootstrapRetired.name())
            .exists()
    );
}

#[test]
fn destination_drift_after_retire_recheck_is_rejected_inside_the_primitive() {
    let fixture = Fixture::new("retire-drift-after-recheck");
    let active = fixture.private(CatalogPrivateNameV1::BootstrapActive);
    let final_directory = fixture.private(CatalogPrivateNameV1::Final);
    run_next_at(
        CatalogDirectoryMutationFaultV1::ActiveRetireAfterInteriorRecheck,
        {
            let final_directory = final_directory.clone();
            move || fs::write(final_directory.join("foreign"), b"sliver drift\n").unwrap()
        },
    );

    assert!(fixture.run().is_err());

    assert!(active.exists());
    assert_eq!(
        fs::read(final_directory.join("foreign")).unwrap(),
        b"sliver drift\n"
    );
    assert!(
        !final_directory
            .join(InfrastructureSlotV1::CatalogBootstrapRetired.name())
            .exists()
    );
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn substituted_final_during_completion_is_not_returned() {
    let fixture = Fixture::new("final-during-complete");
    let final_directory = fixture.private(CatalogPrivateNameV1::Final);
    let displaced = fixture.root.join(".gwz/displaced-final");
    let sentinel = b"replacement completed catalog\n".to_vec();
    run_next_at(CatalogDirectoryMutationFaultV1::CompleteAfterFinalOpen, {
        let final_directory = final_directory.clone();
        let displaced = displaced.clone();
        let sentinel = sentinel.clone();
        move || {
            fs::rename(&final_directory, displaced).unwrap();
            fs::create_dir(&final_directory).unwrap();
            fs::write(final_directory.join("foreign"), sentinel).unwrap();
        }
    });

    assert!(fixture.run().is_err());

    assert_eq!(fs::read(final_directory.join("foreign")).unwrap(), sentinel);
}
