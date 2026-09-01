use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::checked_artifact::bootstrap::{
    CatalogLeaseSetV1, CatalogLeaseTargetBatchV1, CatalogLeaseTargetRequestV1,
    try_acquire_workspace_runtime,
};
use crate::checked_artifact::catalog::CatalogScratchNameV1;
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::fault_v1::{
    CheckedArtifactFaultKeyV1 as Fault, run_next_at as run_next_catalog_fault,
};
use crate::checked_artifact::protocol::{InfrastructureSlotV1, decode_catalog_bootstrap_record};

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

#[test]
fn fresh_workspace_converges_to_the_exact_completed_catalog() {
    let fixture = Fixture::new("complete-catalog");
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace runtime lease");

    let retained = recover_or_create(runtime.catalog_mutation_lease()).unwrap();

    retained.revalidate_for_test().unwrap();
    let catalog = fixture.path().join(".gwz/catalog-final");
    let mut present = fs::read_dir(&catalog)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    present.sort_unstable();
    let mut expected = [
        InfrastructureSlotV1::CatalogFormat,
        InfrastructureSlotV1::CatalogAnchorA,
        InfrastructureSlotV1::RoamingAnchorHome,
        InfrastructureSlotV1::RetiredActions,
        InfrastructureSlotV1::RetiredActionsDescriptor,
        InfrastructureSlotV1::CatalogBootstrapRetired,
    ]
    .map(|slot| slot.name().to_owned())
    .to_vec();
    expected.sort_unstable();
    assert_eq!(present, expected);
    assert!(
        !fixture
            .path()
            .join(".gwz/checked-artifacts-catalog-bootstrap-v1.active")
            .exists()
    );
    assert!(
        !fixture
            .path()
            .join(".gwz/checked-artifacts-catalog-bootstrap-v1.staging")
            .exists()
    );
}

#[test]
fn git_directory_target_creates_its_parent_then_converges_to_completion() {
    let fixture = Fixture::new("git-target-complete");
    let repository = git2::Repository::open(fixture.path()).unwrap();
    let private_parent = repository.commondir().join("gwz");
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(fixture.path());
    let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
    let leases = CatalogLeaseSetV1::try_acquire(batch)
        .unwrap()
        .expect("Git catalog lease");
    let lease = leases.leases().next().expect("one Git lease");

    let retained = recover_or_create(lease).unwrap();

    retained.revalidate_for_test().unwrap();
    assert!(private_parent.join("catalog-final").is_dir());
    assert!(
        !private_parent
            .join("checked-artifacts-catalog-bootstrap-v1.active")
            .exists()
    );
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn returned_catalog_rejects_named_final_substitution() {
    let fixture = Fixture::new("retained-final-substitution");
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace runtime lease");
    let retained = recover_or_create(runtime.catalog_mutation_lease()).unwrap();
    let final_directory = fixture.path().join(".gwz/catalog-final");
    fs::rename(
        &final_directory,
        fixture.path().join(".gwz/displaced-catalog-final"),
    )
    .unwrap();
    fs::create_dir(&final_directory).unwrap();
    fs::write(final_directory.join("foreign"), b"replacement\n").unwrap();

    assert!(retained.revalidate_for_test().is_err());
    assert_eq!(
        fs::read(final_directory.join("foreign")).unwrap(),
        b"replacement\n"
    );
}

#[test]
fn returned_catalog_rejects_byte_identical_interior_file_substitution() {
    let fixture = Fixture::new("retained-interior-substitution");
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace runtime lease");
    let retained = recover_or_create(runtime.catalog_mutation_lease()).unwrap();
    let final_directory = fixture.path().join(".gwz/catalog-final");
    let format = final_directory.join(InfrastructureSlotV1::CatalogFormat.name());
    let bytes = fs::read(&format).unwrap();
    fs::rename(
        &format,
        fixture.path().join(".gwz/displaced-catalog-format"),
    )
    .unwrap();
    fs::write(&format, bytes).unwrap();

    assert!(retained.revalidate_for_test().is_err());
}

#[test]
fn every_completed_edge_survives_full_lease_reacquisition() {
    let fixture = Fixture::new("restart-every-edge");
    let mut retries = 0;
    loop {
        let runtime = try_acquire_workspace_runtime(fixture.path())
            .unwrap()
            .expect("workspace runtime lease");
        let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
        match CatalogOwnerV1::execute_one(witness).unwrap() {
            CatalogOwnerStepV1::Retry(_) => retries += 1,
            CatalogOwnerStepV1::Complete(retained) => {
                retained.revalidate_for_test().unwrap();
                break;
            }
        }
        assert!(retries <= 10, "catalog bootstrap did not converge");
    }
    assert_eq!(retries, 10);
}

#[test]
fn zero_and_partial_next_files_recover_for_every_staging_file_role() {
    for (edge_count, slot) in [
        (5, InfrastructureSlotV1::RoamingAnchorHome),
        (6, InfrastructureSlotV1::CatalogAnchorB),
        (7, InfrastructureSlotV1::RetiredActionsDescriptor),
        (8, InfrastructureSlotV1::CatalogFormat),
    ] {
        for divisor in [usize::MAX, 2] {
            let fixture = Fixture::new(&format!("partial-{slot:?}-{divisor}"));
            run_retry_edges(&fixture, edge_count);
            let path = fixture
                .path()
                .join(".gwz/checked-artifacts-catalog-bootstrap-v1.staging")
                .join(slot.name());
            let exact = fs::read(&path).unwrap();
            let prefix_len = if divisor == usize::MAX {
                0
            } else {
                exact.len() / divisor
            };
            fs::write(&path, &exact[..prefix_len]).unwrap();
            let runtime = try_acquire_workspace_runtime(fixture.path())
                .unwrap()
                .expect("workspace runtime lease");

            let retained = recover_or_create(runtime.catalog_mutation_lease()).unwrap();

            retained.revalidate_for_test().unwrap();
        }
    }
}

#[test]
fn unowned_staging_and_final_directories_are_read_only_ambiguity() {
    for role in [
        CatalogPrivateNameV1::BootstrapStaging,
        CatalogPrivateNameV1::Final,
    ] {
        let fixture = Fixture::new(&format!("unowned-{role:?}"));
        run_retry_edges(&fixture, 2);
        let path = fixture
            .path()
            .join(".gwz")
            .join(std::str::from_utf8(role.leaf_bytes()).expect("fixed catalog name is ASCII"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("foreign"), b"do not touch\n").unwrap();
        let runtime = try_acquire_workspace_runtime(fixture.path())
            .unwrap()
            .expect("workspace runtime lease");

        assert!(recover_or_create(runtime.catalog_mutation_lease()).is_err());
        assert_eq!(fs::read(path.join("foreign")).unwrap(), b"do not touch\n");
    }
}

#[test]
fn restart_and_substitution_matrix_covers_every_catalog_bootstrap_fault_key() {
    let mapped = [
        Fault::CatalogBootstrapReadyEdgeRootFlush,
        Fault::CatalogBootstrapScratchCreate,
        Fault::CatalogBootstrapScratchWrite,
        Fault::CatalogBootstrapScratchFlush,
        Fault::CatalogBootstrapScratchRootFlush,
        Fault::CatalogBootstrapActivePublish,
        Fault::CatalogBootstrapActiveReobserve,
        Fault::CatalogBootstrapStagingCreate,
        Fault::CatalogBootstrapInfrastructurePopulate,
        Fault::CatalogBootstrapInfrastructureFlush,
        Fault::CatalogBootstrapAnchorScratchCreate,
        Fault::CatalogBootstrapAnchorScratchFlush,
        Fault::CatalogBootstrapAnchorPublish,
        Fault::CatalogBootstrapAnchorReobserve,
        Fault::CatalogBootstrapAnchorHomeAExercise,
        Fault::CatalogBootstrapAnchorHomeBExercise,
        Fault::CatalogBootstrapStagingFlush,
        Fault::CatalogBootstrapFinalPublish,
        Fault::CatalogBootstrapFinalReopen,
        Fault::CatalogBootstrapFinalReobserve,
        Fault::CatalogBootstrapActiveRetire,
        Fault::CatalogBootstrapRetiredReobserve,
        Fault::CatalogBootstrapCatalogEnumerate,
    ];
    let git_directory_only = [
        Fault::CatalogBootstrapGitParentCreate,
        Fault::CatalogBootstrapGitParentReobserve,
    ];
    let mut actual = mapped
        .iter()
        .chain(git_directory_only.iter())
        .map(Fault::stable_key)
        .collect::<Vec<_>>();
    let mut expected = Fault::all()
        .into_iter()
        .filter_map(|key| {
            let value = key.stable_key();
            value.starts_with("catalog_bootstrap.").then_some(value)
        })
        .collect::<Vec<_>>();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);

    for key in mapped {
        let fixture = Fixture::new(&format!("fault-{}", key.stable_key()));
        run_next_catalog_fault(key, || panic!("simulated catalog process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let runtime = try_acquire_workspace_runtime(fixture.path())
                .unwrap()
                .expect("workspace runtime lease");
            let _ = recover_or_create(runtime.catalog_mutation_lease());
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {}",
            key.stable_key()
        );

        let runtime = try_acquire_workspace_runtime(fixture.path())
            .unwrap()
            .expect("reacquired workspace runtime lease");
        let retained = recover_or_create(runtime.catalog_mutation_lease()).unwrap();
        retained.revalidate_for_test().unwrap();
    }
}

#[test]
fn restart_and_substitution_matrix_covers_git_directory_targets() {
    let mapped = [
        Fault::CatalogBootstrapGitParentCreate,
        Fault::CatalogBootstrapGitParentReobserve,
        Fault::CatalogBootstrapReadyEdgeRootFlush,
        Fault::CatalogBootstrapScratchCreate,
        Fault::CatalogBootstrapScratchWrite,
        Fault::CatalogBootstrapScratchFlush,
        Fault::CatalogBootstrapScratchRootFlush,
        Fault::CatalogBootstrapActivePublish,
        Fault::CatalogBootstrapActiveReobserve,
        Fault::CatalogBootstrapStagingCreate,
        Fault::CatalogBootstrapInfrastructurePopulate,
        Fault::CatalogBootstrapInfrastructureFlush,
        Fault::CatalogBootstrapAnchorScratchCreate,
        Fault::CatalogBootstrapAnchorScratchFlush,
        Fault::CatalogBootstrapAnchorPublish,
        Fault::CatalogBootstrapAnchorReobserve,
        Fault::CatalogBootstrapAnchorHomeAExercise,
        Fault::CatalogBootstrapAnchorHomeBExercise,
        Fault::CatalogBootstrapStagingFlush,
        Fault::CatalogBootstrapFinalPublish,
        Fault::CatalogBootstrapFinalReopen,
        Fault::CatalogBootstrapFinalReobserve,
        Fault::CatalogBootstrapActiveRetire,
        Fault::CatalogBootstrapRetiredReobserve,
        Fault::CatalogBootstrapCatalogEnumerate,
    ];
    let mut actual = mapped.iter().map(Fault::stable_key).collect::<Vec<_>>();
    let mut expected = Fault::all()
        .into_iter()
        .filter_map(|key| {
            let value = key.stable_key();
            value.starts_with("catalog_bootstrap.").then_some(value)
        })
        .collect::<Vec<_>>();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);

    for key in mapped {
        let fixture = Fixture::new(&format!("git-fault-{}", key.stable_key()));
        run_next_catalog_fault(key, || panic!("simulated catalog process stop"));
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = drive_git_directory_recovery(&fixture);
        }));
        assert!(
            interrupted.is_err(),
            "fault point was not reached: {}",
            key.stable_key()
        );

        drive_git_directory_recovery(&fixture).unwrap();
    }
}

#[test]
fn resume_after_scratch_interrupt_reissues_the_root_barrier_before_completion() {
    // DirentBarrier [P3-1] regression: a drive interrupted at
    // `catalog_bootstrap.scratch_write` leaves a VFS-complete exact scratch,
    // so the restart classifies past the scratch edge and never reaches the
    // scratch-tail root anchor. The Ready-edge prologue must issue the
    // containing-root barrier on that resume drive before `Complete`.
    let fixture = Fixture::new("resume-root-barrier");
    run_next_catalog_fault(Fault::CatalogBootstrapScratchWrite, || {
        panic!("simulated catalog process stop")
    });
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = try_acquire_workspace_runtime(fixture.path())
            .unwrap()
            .expect("workspace runtime lease");
        let _ = recover_or_create(runtime.catalog_mutation_lease());
    }));
    assert!(
        interrupted.is_err(),
        "scratch-write fault point was not reached"
    );

    let barrier_issued = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    run_next_catalog_fault(Fault::CatalogBootstrapReadyEdgeRootFlush, {
        let barrier_issued = std::sync::Arc::clone(&barrier_issued);
        move || barrier_issued.store(true, Ordering::SeqCst)
    });
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("reacquired workspace runtime lease");
    let retained = recover_or_create(runtime.catalog_mutation_lease()).unwrap();
    retained.revalidate_for_test().unwrap();
    assert!(
        barrier_issued.load(Ordering::SeqCst),
        "resume drive converged without issuing the containing-root barrier"
    );
}

#[test]
fn entrant_git_parent_inside_the_creation_window_converges() {
    let fixture = Fixture::new("entrant-git-parent");
    let repository = git2::Repository::open(fixture.path()).unwrap();
    let git_directory = repository.commondir().to_path_buf();
    run_next_catalog_fault(Fault::CatalogBootstrapGitParentCreate, {
        let parent = git_directory.join("gwz");
        move || fs::create_dir(&parent).unwrap()
    });

    drive_git_directory_recovery(&fixture).unwrap();
}

fn drive_git_directory_recovery(fixture: &Fixture) -> Result<(), CheckedFsError> {
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(fixture.path());
    let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
    let leases = CatalogLeaseSetV1::try_acquire(batch)
        .unwrap()
        .expect("Git catalog lease");
    let lease = leases.leases().next().unwrap();
    let retained = recover_or_create(lease)?;
    retained.revalidate_for_test()
}

fn run_retry_edges(fixture: &Fixture, count: usize) {
    for _ in 0..count {
        let runtime = try_acquire_workspace_runtime(fixture.path())
            .unwrap()
            .expect("workspace runtime lease");
        let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
        assert!(matches!(
            CatalogOwnerV1::execute_one(witness).unwrap(),
            CatalogOwnerStepV1::Retry(_)
        ));
    }
}

// R2-F R1.1, 2026-09-01 — the split's executable rows
// (`GwzM5-8R2F-RelocationPlan.md` §1/§3, ADOPTED).

fn interior_names(directory: &Path) -> Vec<String> {
    let mut names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names
}

/// Every production source under `src/checked_artifact/` that NAMES
/// `prepare_private` — definitions plus uses, by path relative to the scan
/// root. Spelling-blind per the R1.2 [P1-1] precedent ([R1.1-P1-1]): a `use`
/// import, an alias (`use … as pp; pp(dir, …)`) and a qualified call all put
/// the bare identifier in the file, while a call-site matcher sees only the
/// spellings its prefix rules admit — over-eager toward RED by design.
/// `prepare_private` is `pub(super)` on `platform`, so no namer can exist
/// outside this subtree and the scan is exhaustive. `//` comments are
/// stripped first, so prose naming the identifier (as `platform.rs:437-439`
/// does) cannot move the list; the `_tests.rs` stems are `#[cfg(test)]`
/// companions and excluded with the `tests*` files.
fn production_namers_of_prepare_private() -> Vec<String> {
    fn walk(root: &Path, directory: &Path, found: &mut Vec<String>) {
        for entry in fs::read_dir(directory).expect("the scan root is readable") {
            let path = entry.expect("a readable directory entry").path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "tests" && name != "interface_tests" {
                    walk(root, &path, found);
                }
                continue;
            }
            if !name.ends_with(".rs") || name.starts_with("tests") || name.ends_with("_tests.rs") {
                continue;
            }
            let code = fs::read_to_string(&path)
                .expect("a production source is readable")
                .lines()
                .map(|line| line.split_once("//").map_or(line, |(kept, _)| kept))
                .collect::<String>();
            if code.contains("prepare_private") {
                let relative = path
                    .strip_prefix(root)
                    .expect("a source under the scan root");
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/checked_artifact");
    let mut found = Vec::new();
    walk(&root, &root, &mut found);
    found.sort_unstable();
    found
}

/// THE decisive row ([R2-P2-1] — the executable form of Phase4Closure §2.7's
/// coexistence criterion, strengthened by [RC-P3-1]).
///
/// Bootstrap the catalog, run ONE real legacy checked-artifact drive through
/// the production entry point, then re-observe the catalog: it must still be
/// recoverable. This row fails under BOTH designs the reviews killed — round
/// 1's, where bootstrap itself refuses on the resident directory's mere
/// presence (`provider/directory_mutation.rs:179-185`), and round 2's, where
/// renaming the shared `Final` marches the legacy writer into the catalog's
/// new home. The killing teeth there differ by platform ([R1.1-P2-1]): on
/// Windows the drive plants a `.ca1-*` anchor that makes re-observation
/// refuse (`provider/interior.rs:437-440`); on darwin/linux `prepare_private`
/// is a no-op (`platform.rs:344-351`) and a clean drive removes its
/// write-ahead names, so what discriminates is the disjointness assertion
/// below — the legacy area appears as its OWN directory — not the
/// recoverability lines. It passes only under the split.
#[test]
fn a_legacy_drive_after_bootstrap_leaves_the_catalog_recoverable() {
    let fixture = Fixture::new("legacy-drive-after-bootstrap");
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace runtime lease");
    let retained = recover_or_create(runtime.catalog_mutation_lease()).unwrap();
    let catalog = fixture.path().join(".gwz/catalog-final");
    let published = interior_names(&catalog);

    fs::create_dir_all(fixture.path().join("a")).unwrap();
    fs::write(fixture.path().join("a/value"), b"before").unwrap();
    crate::checked_artifact::entry::replace_merge_root_artifact(
        fixture.path(),
        Path::new("a/value"),
        b"before",
        b"after",
    )
    .unwrap();
    assert_eq!(fs::read(fixture.path().join("a/value")).unwrap(), b"after");

    // Disjointness — the property, asserted directly: the drive built and used
    // the LEGACY area, and the catalog's interior is byte-for-byte what
    // bootstrap published.
    assert!(
        fixture.path().join(".gwz/checked-artifacts").is_dir(),
        "the legacy writer creates its own private area, not the catalog's"
    );
    assert_eq!(interior_names(&catalog), published);

    // The structural no-anchor guarantee ([RC-P3-1]), identical on both
    // platforms and carried into the Windows first-dispatch obligation:
    // `platform::prepare_private` is the only planter of
    // `.ca1-durability-anchor-<32hex>` (Windows arm, `platform.rs:687-695`),
    // and the only production files NAMING it are its definer and its one
    // caller — `residue.rs:102`, inside the legacy writer's `open_private`.
    // The catalog's directory therefore cannot acquire an anchor, on any
    // platform, by construction rather than by test.
    assert_eq!(
        production_namers_of_prepare_private(),
        ["platform.rs", "residue.rs"]
    );

    // Still recoverable — the criterion itself.
    retained.revalidate_for_test().unwrap();
    recover_or_create(runtime.catalog_mutation_lease())
        .unwrap()
        .revalidate_for_test()
        .unwrap();
}

/// A resident `checked-artifacts/` is the legacy writer's LIVE area, never
/// residue — and after the split it is recognized by NOTHING: `fixed_roles`
/// (`catalog/enumeration.rs:373-388`) is a fixed variant-keyed table built from
/// `Final.leaf_bytes()`, now `catalog-final`, so the resident directory
/// classifies as an unrecognized sibling (`Ok(None)`), refuses nothing, and is
/// left exactly as found. Bootstrap must succeed on BOTH roots over it.
#[test]
fn bootstrap_over_a_resident_legacy_directory_on_the_workspace_root() {
    let fixture = Fixture::new("resident-legacy-workspace");
    let legacy = fixture.path().join(".gwz/checked-artifacts");
    fs::create_dir_all(&legacy).unwrap();
    let anchor = ".ca1-durability-anchor-deadbeefdeadbeefdeadbeefdeadbeef";
    fs::write(legacy.join(anchor), b"anchor\n").unwrap();
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace runtime lease");

    let retained = recover_or_create(runtime.catalog_mutation_lease()).unwrap();

    retained.revalidate_for_test().unwrap();
    assert!(fixture.path().join(".gwz/catalog-final").is_dir());
    assert_eq!(interior_names(&legacy), [anchor]);
}

#[test]
fn bootstrap_over_a_resident_legacy_directory_on_the_git_directory_root() {
    let fixture = Fixture::new("resident-legacy-git");
    let private_parent = git2::Repository::open(fixture.path())
        .unwrap()
        .commondir()
        .join("gwz");
    let legacy = private_parent.join("checked-artifacts");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("residue"), b"legacy\n").unwrap();

    drive_git_directory_recovery(&fixture).unwrap();

    assert!(private_parent.join("catalog-final").is_dir());
    assert_eq!(interior_names(&legacy), ["residue"]);
}

/// R2-E E4.1 precondition 6, SECOND arm (E0.2b §5.3 item 6): the partially
/// mutated state an interrupted activation leaves behind is proved CONVERGENT
/// on restart, fault-driven rather than asserted.
///
/// Driven through the PRODUCTION DOOR — `entry::activate_workspace_catalog`,
/// the crate's only production catalog activation — rather than through
/// `recover_or_create` directly, so what is proved convergent is the shape a
/// user's `--no-ff` merge actually executes, its error rendering included. The
/// first arm (the refusal precedes the operation's own first durable mutation)
/// is driven where the operation lives, at `v1_lifecycle::tests::checked`.
#[test]
fn the_production_activation_door_converges_after_an_interrupted_durable_edge() {
    let fixture = Fixture::new("e41-door-restart");
    run_next_catalog_fault(Fault::CatalogBootstrapScratchWrite, || {
        panic!("simulated catalog process stop")
    });
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = try_acquire_workspace_runtime(fixture.path())
            .unwrap()
            .expect("workspace runtime lease");
        let _ = crate::checked_artifact::entry::activate_workspace_catalog(
            runtime.catalog_mutation_lease(),
        );
    }));
    assert!(
        interrupted.is_err(),
        "the durable edge the restart converges over was never reached"
    );

    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("reacquired workspace runtime lease");
    crate::checked_artifact::entry::activate_workspace_catalog(runtime.catalog_mutation_lease())
        .expect("the production door converges over its own partial state");
}
