//! PROBE BRANCH ONLY (`probe/g15-gate-dump`) — the diagnosis memo's prescribed
//! per-gate probe (GwzArmPreservationHandoffDiagnosis.md, "Prescribed probe").
//!
//! Asserts nothing. It builds the standard fixtures the runner-only class dies
//! on and prints every `prepare_root_preservation_stash` gate, so the false gate
//! and its divergent field are named in one instrumented run. Delete with the
//! branch.

use super::support::*;
use super::*;

#[test]
fn probe_prepare_gates_on_the_standard_fixtures() {
    // `sha256` first: it is the exact reproducing row named by the memo,
    // `observation::real_sha256_repository_prepares_exact_handoff`.
    for (label, fixture) in [
        ("sha256-fixture", fixture_with_format("sha256")),
        ("sha1-fixture", fixture_with_format("sha1")),
    ] {
        crate::git::dump_prepare_gates(&fixture.backend, &fixture.root, &fixture.spec, label);
        let prepared = fixture
            .backend
            .prepare_root_preservation_stash(&fixture.root, &fixture.spec);
        println!(
            "PROBE-G15 [{label}] prepare_root_preservation_stash = {}",
            match &prepared {
                Ok(stash) => format!("Ok(dirty {:?})", stash.normalized_image.dirty),
                Err(error) => format!("Err({:?}: {})", error.code, error.message),
            }
        );
    }
}
