//! Track-P spike for the R2-D interface freeze.
//!
//! Executes the exact physical edge shape R2-D Phase 1 admission will use --
//! deterministic indexed staging name, no-replace publication into the catalog
//! interior, reobservation through the retained parent, then cross-parent
//! retirement -- against the already-sealed source-associated primitive
//! `publish_verified_no_replace` proven by R2-C2. Its purpose is to answer one
//! Track-P question before the freeze is reviewed: does any admission edge
//! require a *new* platform primitive? (`GwzProcessOptimization.md` §3.1,
//! `GwzFasterProposal.md` §3 Step C.)
//!
//! File naming: the boundary checker's `production_rust_files` excludes only
//! paths with a `tests` component or a `tests`-prefixed file name. A spike that
//! names the sealed primitive from any other file under `provider/` would join
//! `publication_callers` and fail the six-move publication-seam rule
//! (`scripts/checks/check_checked_artifact_boundaries.py:816-856`). The
//! `tests_`-prefix keeps the production caller inventory at exactly
//! `mutation.rs` + `directory_mutation.rs`, so this spike proves the primitive
//! without widening the production seam.

use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

use super::platform::HostPlatform;
use super::publication::{DestinationRecheckV1, PublicationSourceV1, publish_verified_no_replace};
use super::retained::encode_identity;
use crate::checked_artifact::capability::DurableIdentityProvider;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct SpikeFixture {
    root: PathBuf,
}

impl SpikeFixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "gwz-r2d-track-p-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }
}

impl Drop for SpikeFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_and_flush(parent: &Dir, name: &OsStr, bytes: &[u8]) {
    let mut file = parent.create(name).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

fn identity_of(parent: &Dir, name: &OsStr) -> Vec<u8> {
    let file = parent.open(name).unwrap();
    encode_identity(&HostPlatform.file_identity(&file).unwrap())
}

/// Track-P: admission publish and retirement are served by the sealed
/// primitive with no new platform capability.
#[test]
fn admission_publish_and_retire_edges_use_the_sealed_publication_primitive() {
    let fixture = SpikeFixture::new("admission-edge");
    // Catalog-interior shape: action rows are published inside the Final
    // directory; terminal retirement lands under the retired root.
    fs::create_dir(fixture.root.join("final")).unwrap();
    fs::create_dir(fixture.root.join("retired")).unwrap();
    let final_dir = Dir::open_ambient_dir(fixture.root.join("final"), ambient_authority()).unwrap();
    let retired_dir =
        Dir::open_ambient_dir(fixture.root.join("retired"), ambient_authority()).unwrap();

    // Deterministic indexed names; no nonce is allocated on any edge or retry
    // (RemPlan-4 §4 R2 stop clause :1089-1092).
    let staging = OsStr::new("staging-0000");
    let published = OsStr::new("action-0000");
    let retired = OsStr::new("retired-action-0000");
    let reservation = b"gwz-r2d-track-p-resident-reservation".to_vec();

    write_and_flush(&final_dir, staging, &reservation);
    let staged_identity = identity_of(&final_dir, staging);

    // Edge 1 -- staging to final action name, no-replace, catalog interior.
    publish_verified_no_replace(
        &final_dir,
        staging,
        &final_dir,
        published,
        PublicationSourceV1::regular_file(&staged_identity, &reservation),
        DestinationRecheckV1::None,
        "r2d admission publication spike",
    )
    .unwrap();

    // Reobserve through the same retained parent: the staging name is
    // consumed, and the published object is the identical object and bytes.
    assert!(
        final_dir.open(staging).is_err(),
        "publication must consume the staging name"
    );
    assert_eq!(final_dir.read(published).unwrap(), reservation);
    assert_eq!(identity_of(&final_dir, published), staged_identity);

    // No-replace proof: a second publication into the occupied destination
    // rejects read-only and leaves the published object untouched.
    write_and_flush(&final_dir, staging, &reservation);
    let republished_identity = identity_of(&final_dir, staging);
    let collision = publish_verified_no_replace(
        &final_dir,
        staging,
        &final_dir,
        published,
        PublicationSourceV1::regular_file(&republished_identity, &reservation),
        DestinationRecheckV1::None,
        "r2d admission publication spike",
    );
    assert!(
        collision.is_err(),
        "publication must never replace an occupied action destination"
    );
    assert_eq!(identity_of(&final_dir, published), staged_identity);
    final_dir.remove_file(staging).unwrap();

    // Edge 2 -- final action to the retired root, cross-parent retirement.
    publish_verified_no_replace(
        &final_dir,
        published,
        &retired_dir,
        retired,
        PublicationSourceV1::regular_file(&staged_identity, &reservation),
        DestinationRecheckV1::None,
        "r2d admission retirement spike",
    )
    .unwrap();

    assert!(
        final_dir.open(published).is_err(),
        "retirement must consume the final action name"
    );
    assert_eq!(retired_dir.read(retired).unwrap(), reservation);
    assert_eq!(identity_of(&retired_dir, retired), staged_identity);
}

/// Track-P: a source substituted after the owner observed it rejects before
/// the namespace edge, so the admission publish edge inherits the primitive's
/// identity-compare guarantee unchanged (amendment §4.1).
#[test]
fn admission_publish_rejects_a_substituted_source_before_the_namespace_edge() {
    let fixture = SpikeFixture::new("substituted-source");
    fs::create_dir(fixture.root.join("final")).unwrap();
    let final_dir = Dir::open_ambient_dir(fixture.root.join("final"), ambient_authority()).unwrap();

    let staging = OsStr::new("staging-0001");
    let published = OsStr::new("action-0001");
    let reservation = b"gwz-r2d-track-p-resident-reservation".to_vec();

    write_and_flush(&final_dir, staging, &reservation);
    let observed_identity = identity_of(&final_dir, staging);

    // Same name, different object: the owner's observed identity is stale.
    final_dir.remove_file(staging).unwrap();
    write_and_flush(&final_dir, staging, &reservation);

    let substituted = publish_verified_no_replace(
        &final_dir,
        staging,
        &final_dir,
        published,
        PublicationSourceV1::regular_file(&observed_identity, &reservation),
        DestinationRecheckV1::None,
        "r2d admission publication spike",
    );
    assert!(
        substituted.is_err(),
        "a substituted publication source must reject before the edge"
    );
    assert!(
        final_dir.open(published).is_err(),
        "a rejected publication performs no namespace mutation"
    );
}
