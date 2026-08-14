use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::catalog::CatalogScratchNameV1;
use crate::checked_artifact::protocol::decode_catalog_bootstrap_record;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2c2-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn missing_git_private_parent_edge_creates_only_the_parent_and_retries() {
    let fixture = Fixture::new("missing-git-parent");
    let repository = git2::Repository::open(fixture.path()).unwrap();
    let git_directory = repository.commondir().to_path_buf();
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(fixture.path());
    let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
    let leases = CatalogLeaseSetV1::try_acquire(batch)
        .unwrap()
        .expect("Git catalog lease");
    let witness = leases.leases().next().unwrap().begin_preflight().unwrap();

    assert!(matches!(
        CatalogOwnerV1::execute_one(witness).unwrap(),
        CatalogOwnerStepV1::Retry(_)
    ));

    let parent = git_directory.join("gwz");
    assert!(parent.is_dir());
    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
}

#[test]
fn fresh_workspace_first_edge_writes_one_exact_canonical_scratch() {
    let fixture = Fixture::new("fresh-scratch");
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();

    assert!(matches!(
        CatalogOwnerV1::execute_one(witness).unwrap(),
        CatalogOwnerStepV1::Retry(_)
    ));

    let mut scratch = fs::read_dir(fixture.path().join(".gwz"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("checked-artifacts-catalog-bootstrap-v1.scratch.")
        })
        .collect::<Vec<_>>();
    assert_eq!(scratch.len(), 1);
    let entry = scratch.pop().unwrap();
    let name = entry.file_name();
    let parsed = CatalogScratchNameV1::parse(name.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(parsed.as_bytes().len(), 241);
    let bytes = fs::read(entry.path()).unwrap();
    let record = decode_catalog_bootstrap_record(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(
        record.durable_target_digest(),
        parsed.durable_target_digest()
    );
    assert_eq!(
        record.historical_collision_digest(),
        parsed.historical_collision_digest()
    );
    assert_eq!(record.bootstrap_ownership_token(), parsed.ownership_token());
}

#[test]
fn exact_scratch_is_published_no_replace_as_active_on_the_next_edge() {
    let fixture = Fixture::new("publish-active");
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogOwnerStepV1::Retry(witness) = CatalogOwnerV1::execute_one(witness).unwrap() else {
        panic!("scratch edge must retry");
    };
    let CatalogOwnerStepV1::Retry(_) = CatalogOwnerV1::execute_one(witness).unwrap() else {
        panic!("active publication must retry");
    };

    let parent = fixture.path().join(".gwz");
    let active = parent.join("checked-artifacts-catalog-bootstrap-v1.active");
    let bytes = fs::read(active).unwrap();
    decode_catalog_bootstrap_record(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(
        fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("checked-artifacts-catalog-bootstrap-v1.scratch."))
            .count(),
        0
    );
}
