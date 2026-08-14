use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::mutation::{CatalogMutationFaultV1, run_next_at};
use crate::checked_artifact::bootstrap::try_acquire_workspace_runtime;
use crate::checked_artifact::capability::{CatalogPreflightV1, preflight_catalog_target};
use crate::checked_artifact::catalog::{CatalogScratchNameV1, recover_or_create};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{
    CatalogBootstrapOwnershipTokenV1, CatalogBootstrapRecordV1,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2c2-mutation-{label}-{}-{}",
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

    fn private_parent(&self) -> PathBuf {
        self.root.join(".gwz")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn expected_record(fixture: &Fixture, token: u8) -> CatalogBootstrapRecordV1 {
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("workspace parent must be ready");
    };
    permit.record_for_test(
        CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([token; 32]).unwrap(),
    )
}

fn scratch_path(fixture: &Fixture, record: &CatalogBootstrapRecordV1) -> PathBuf {
    let scratch = CatalogScratchNameV1::new(
        record.durable_target_digest(),
        record.historical_collision_digest(),
        record.bootstrap_ownership_token(),
    );
    fixture
        .private_parent()
        .join(std::str::from_utf8(scratch.as_bytes()).expect("canonical scratch name is ASCII"))
}

fn run_recovery(fixture: &Fixture) {
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace lease");
    assert!(recover_or_create(runtime.catalog_mutation_lease()).is_err());
}

#[test]
fn replacement_before_rewrite_open_is_not_truncated_or_overwritten() {
    let fixture = Fixture::new("replace-before-open");
    let record = expected_record(&fixture, 31);
    let path = scratch_path(&fixture, &record);
    fs::write(&path, &record.encode_canonical()[..1]).unwrap();
    let replacement = b"replacement-before-open\n".to_vec();
    run_next_at(CatalogMutationFaultV1::ScratchBeforeOpen, {
        let path = path.clone();
        let replacement = replacement.clone();
        move || {
            fs::remove_file(&path).unwrap();
            fs::write(path, replacement).unwrap();
        }
    });

    run_recovery(&fixture);

    assert_eq!(fs::read(path).unwrap(), replacement);
}

#[test]
fn replacement_after_rewrite_open_is_not_truncated_or_overwritten() {
    let fixture = Fixture::new("replace-after-open");
    let record = expected_record(&fixture, 32);
    let path = scratch_path(&fixture, &record);
    fs::write(&path, &record.encode_canonical()[..1]).unwrap();
    let replacement = b"replacement-after-open\n".to_vec();
    run_next_at(CatalogMutationFaultV1::ScratchAfterOpen, {
        let path = path.clone();
        let replacement = replacement.clone();
        move || {
            fs::remove_file(&path).unwrap();
            fs::write(path, replacement).unwrap();
        }
    });

    run_recovery(&fixture);

    assert_eq!(fs::read(path).unwrap(), replacement);
}

#[test]
fn replacement_before_active_rename_is_not_published() {
    let fixture = Fixture::new("replace-before-rename");
    let record = expected_record(&fixture, 33);
    let path = scratch_path(&fixture, &record);
    fs::write(&path, record.encode_canonical()).unwrap();
    let replacement = b"replacement-before-rename\n".to_vec();
    run_next_at(CatalogMutationFaultV1::PublishBeforeRename, {
        let path = path.clone();
        let replacement = replacement.clone();
        move || {
            fs::remove_file(&path).unwrap();
            fs::write(path, replacement).unwrap();
        }
    });

    run_recovery(&fixture);

    assert_eq!(fs::read(path).unwrap(), replacement);
    assert!(
        !fixture
            .private_parent()
            .join(std::str::from_utf8(CatalogPrivateNameV1::BootstrapActive.leaf_bytes()).unwrap())
            .exists()
    );
}
