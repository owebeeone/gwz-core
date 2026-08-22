//! Supporting unit tests for the durable-transition kernel of the admission
//! driver.
//!
//! `GwzM5-8R2D-Plan.md` §4 Step 1.1's six production-shaped cases (`../tests.rs`)
//! are the acceptance spec and are not touched here. These cases cover the two
//! properties that spec does not reach and that the Phase 1 dual gate reviews
//! directly (`GwzM5-8R2DInterfaceFreeze.md` §9 row 2, "`Idle`<->`Preparing`
//! treated as a durable-transition kernel"):
//!
//! * a *second, distinct* action, which is the only path that drives
//!   `Idle -> Preparing` with an active record already resident; and
//! * each install window the no-replace publication opens, reconstructed on
//!   disk rather than by fault injection — `admission.*` fault activation is
//!   R2-D Step 1.3 and is deliberately not started here.
//!
//! Living in a `tests`-prefixed file keeps this out of `production_rust_files`
//! (`scripts/checks/check_checked_artifact_boundaries.py:663-670`), so naming
//! `catalog_mutation_lease` here does not widen the checker's catalog-lease
//! reference set.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::checked_artifact::admission::ActionAdmissionOwnerV1;
use crate::checked_artifact::bootstrap::try_acquire_workspace_runtime;
use crate::checked_artifact::capability::CheckedFsError;
use crate::checked_artifact::catalog::recover_or_create;
use crate::checked_artifact::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1, ActionScheduleV1,
    AdmittedActionV1, CleanupAliasSetV1, InfrastructureSlotV1, ManagedBootstrapInputV1,
    RequestOwnerBindingV1,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2d-kernel-{label}-{}-{}",
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

    fn catalog_root(&self) -> PathBuf {
        self.root
            .join(CatalogPrivateNameV1::Final.relative_path(CatalogPrivateRootV1::Workspace))
    }

    fn slot(&self, slot: InfrastructureSlotV1) -> PathBuf {
        self.catalog_root().join(slot.name())
    }

    fn children(&self) -> Vec<String> {
        let mut names = fs::read_dir(self.catalog_root())
            .expect("the recovered catalog root must exist")
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One fresh-process attempt: acquire the runtime lease, recover through the
/// sole sealed catalog owner, drive the seam once, release.
fn attempt(
    fixture: &Fixture,
    expected: &ActionCapacityReservationV1,
) -> Result<AdmittedActionV1, CheckedFsError> {
    let runtime = try_acquire_workspace_runtime(fixture.path())
        .unwrap()
        .expect("workspace runtime lease");
    let retained = recover_or_create(runtime.catalog_mutation_lease())
        .expect("the sealed catalog owner must recover a complete catalog");
    ActionAdmissionOwnerV1::from_retained_catalog(retained).resume_or_admit(expected)
}

fn reservation(action: u8, barriers: usize) -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([action; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        ActionScheduleV1::try_new(
            barriers,
            vec![ManagedBootstrapInputV1::new([3; 32], 2).unwrap()],
            CleanupAliasSetV1::all(),
        )
        .unwrap(),
    )
}

fn encoded(record: &ActionDirectoryAdmissionV1) -> Vec<u8> {
    record
        .encode_canonical()
        .expect("the admission record encodes canonically")
}

/// The second action is the only path that reaches `Idle -> Preparing` with an
/// active admission record already resident, and it is also the first state in
/// which the catalog root carries more than one `RootEntryNameV1::ActiveAction`
/// row — the C-3 widening under load (interface freeze §4.4 Class 2).
#[test]
fn a_second_distinct_action_admits_and_leaves_both_actions_resumable() {
    let fixture = Fixture::new("second-action");
    let first = reservation(0x77, 2);
    let second = reservation(0x88, 1);
    assert_ne!(first.action_digest(), second.action_digest());

    let first_handoff = attempt(&fixture, &first).expect("the first action admits");
    let second_handoff = attempt(&fixture, &second).expect("the second action admits");
    assert_ne!(first_handoff, second_handoff);
    assert_eq!(second_handoff.reservation(), &second);

    // Seven infrastructure rows plus exactly two action rows, all inside the
    // frozen root grammar, and the record settled back to idle.
    let settled = fixture.children();
    assert_eq!(
        settled
            .iter()
            .filter(|name| name.starts_with("action-") && !name.contains("admission"))
            .count(),
        2,
        "both actions must be resident: {settled:?}"
    );
    assert!(
        !fixture
            .slot(InfrastructureSlotV1::ActionAdmissionStaging)
            .exists()
    );

    // Both actions still resume exactly, and neither resume mutates anything.
    assert_eq!(
        attempt(&fixture, &first).expect("the first action resumes"),
        first_handoff
    );
    assert_eq!(
        attempt(&fixture, &second).expect("the second action resumes"),
        second_handoff
    );
    assert_eq!(fixture.children(), settled);
}

/// Each durable window the retire-then-publish install opens, reconstructed on
/// disk and re-entered as a fresh process. Every window converges to the same
/// settled state and hands back the same action, so the kernel is restart-closed
/// across the whole transition rather than only at its endpoints.
#[test]
fn every_admission_record_install_window_converges_to_the_settled_state() {
    let fixture = Fixture::new("install-windows");
    let expected = reservation(0x99, 2);
    let settled_handoff = attempt(&fixture, &expected).expect("the first admission settles");
    let settled = fixture.children();

    let active = fixture.slot(InfrastructureSlotV1::ActionAdmissionActive);
    let scratch = fixture.slot(InfrastructureSlotV1::ActionAdmissionScratch);
    let idle = encoded(&ActionDirectoryAdmissionV1::idle());
    let preparing = encoded(&ActionDirectoryAdmissionV1::preparing(&expected));

    // Each row is one reconstructed window plus whether the driver still owes
    // an install from it. The last row is the only one that owes nothing: with
    // both slots absent the action is already exactly published, and an absent
    // active slot *is* `ActionDirectoryAdmissionV1::idle()` — the record
    // carries no field beyond "not preparing". Re-materialising it would be a
    // durable write with no durable content, so the driver correctly performs
    // no mutation at all and the catalog stays one row short of `settled`.
    let mut settled_without_record = settled.clone();
    settled_without_record
        .retain(|name| name != InfrastructureSlotV1::ActionAdmissionActive.name());
    let windows = [
        // Interrupted between the retirement and the no-replace publish.
        InstallWindowV1 {
            label: "retired, scratch idle",
            active: None,
            scratch: Some(&idle),
            installs: true,
        },
        // Interrupted before the retirement.
        InstallWindowV1 {
            label: "preparing, scratch idle",
            active: Some(&preparing),
            scratch: Some(&idle),
            installs: true,
        },
        // The whole triad lost after the action was already published.
        InstallWindowV1 {
            label: "triad absent",
            active: None,
            scratch: None,
            installs: false,
        },
    ];
    for window in windows {
        let label = window.label;
        let _ = fs::remove_file(&active);
        let _ = fs::remove_file(&scratch);
        if let Some(bytes) = window.active {
            fs::write(&active, bytes).unwrap();
        }
        if let Some(bytes) = window.scratch {
            fs::write(&scratch, bytes).unwrap();
        }

        let resumed = attempt(&fixture, &expected)
            .unwrap_or_else(|error| panic!("install window `{label}` must converge: {error:?}"));
        assert_eq!(
            resumed, settled_handoff,
            "install window `{label}` admitted a different action"
        );
        let expected_children = if window.installs {
            &settled
        } else {
            &settled_without_record
        };
        assert_eq!(
            &fixture.children(),
            expected_children,
            "install window `{label}` did not converge to the expected catalog"
        );
        assert!(
            !scratch.exists(),
            "install window `{label}` left the admission scratch resident"
        );
    }
}

/// One reconstructed durable window: the bytes resident in each slot of the
/// `ActionAdmission*` pair, and whether the driver still owes an install from
/// it.
struct InstallWindowV1<'a> {
    label: &'a str,
    active: Option<&'a [u8]>,
    scratch: Option<&'a [u8]>,
    installs: bool,
}
