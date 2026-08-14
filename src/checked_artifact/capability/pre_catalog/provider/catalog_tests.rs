use std::fs;
use std::path::{Path, PathBuf};

use super::filesystem::platform_pre_catalog_provider;
use super::*;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::capability::{CatalogPreflightV1, preflight_catalog_target};
use crate::checked_artifact::catalog::{CatalogScratchNameV1, MAX_CATALOG_PARENT_ENTRIES_V1};
use crate::checked_artifact::protocol::{
    CatalogBootstrapOwnershipTokenV1, CatalogBootstrapRecordV1, CatalogBootstrapRecoveryDecisionV1,
    InfrastructureSlotV1,
};

mod grammar;
mod preflight;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "gwz-r2c1-catalog-provider-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git2::Repository::init(&root).unwrap();
        fs::create_dir(root.join(".gwz")).unwrap();
        Self { root }
    }

    fn private_parent(&self) -> PathBuf {
        self.root.join(".gwz")
    }

    fn repo(&self) -> git2::Repository {
        git2::Repository::open(&self.root).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn observe(
    root: &Path,
) -> Result<RawPreCatalogObservationV1<RetainedPlatformRoot>, CheckedFsError> {
    platform_pre_catalog_provider().observe_and_revalidate_workspace_for_test(root)
}

fn fresh_record(fixture: &Fixture, token: u8) -> CatalogBootstrapRecordV1 {
    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("workspace parent must produce a ready permit");
    };
    permit.record_for_test(
        CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([token; 32]).unwrap(),
    )
}
