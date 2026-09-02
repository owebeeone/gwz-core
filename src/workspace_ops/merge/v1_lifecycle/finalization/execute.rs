//! The v1 forward publication's physical arms.
//!
//! **THE THREE DATED RESIDUALS, [R2-P3-1] (E4.7, 2026-09-02).** All three raw
//! arms below — the marker, the lock and the boundary — STAY RAW. They are NOT
//! convertible forward arms and no E4 step converts them; each carries its own
//! ground at its own site.
//!
//! Operator ruling (a), 2026-09-02, verbatim: *"E4.5-B does not open. The
//! marker write at execute.rs:45 joins lock and boundary in the dated residual,
//! on the directional-residue ground (interrupted checked publication strands
//! gwz merge --abort). Do not convert it. Do not lift 'no (C) inside E4' for an
//! observer cure. E4.7 carries the three residual sentences and the amendment
//! corrections. DR-1's agenda gains: directional-residue class, classifier
//! widening, preservation-bundle audit (same hazard). Phase E4 conversions are
//! E4.1 and E4.2; the rest is carve-out, pins, GC, and close-out."*
//!
//! The ruling's `execute.rs:45`, and the amendment's `:48`/`:51`, are line numbers at
//! the base sha `f563446`; unmoved in content, the three arms are `:79`, `:88` and `:98`
//! at this tree (re-pointed by DR-1 S1, 2026-09-03), each below its own sentence.
//!
//! Evidence for the `WriteMarker` ground is `dev-docs/GwzM5-8R2E-E45B-Report.md`
//! (driven and ablated at main `f563446`); for `WriteLock`/`WriteBoundary` it is
//! `GwzM5-8R2E-CapabilityFreeAmendment.md` §7's E4.5/6-B disposition. The cures
//! are observer-side and belong to DR-1, which the ruling forbids opening
//! inside E4.

use super::*;
use crate::artifact::{self, LOCK_PATH};
use crate::workspace_ops::merge::acceptance::{
    v1_candidate_files, v1_composition_message, v1_publication_base,
};
use crate::workspace_ops::publish_workspace_exclude_candidate;

pub(super) fn publication<B: MergeAuthorityBackend>(
    backend: &B,
    current: &StoredV1Record,
    action: PublicationPhysicalAction,
) -> ModelResult<()> {
    verify_finalization_action(backend, current, action)?;
    let record = current.record();
    let progress = record.publication.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "publication progress is missing",
        )
    })?;
    let candidate = progress.candidate.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "publication candidate is missing",
        )
    })?;
    let root = current.location().root();
    match action {
        PublicationPhysicalAction::EvidenceCommit => {
            let (parent, _) = v1_publication_base(record)?;
            backend.commit_gwz_paths_checked(
                root,
                parent,
                &v1_candidate_files(record)?,
                &v1_composition_message(record),
            )?;
        }
        PublicationPhysicalAction::WriteMarker => {
            let path = progress.candidate_marker_path.as_ref().ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    "candidate marker path is missing",
                )
            })?;
            // [R2-P3-1] DATED RESIDUAL, E4.7 2026-09-02 — the marker STAYS RAW: a crash inside a
            // checked publication leaves a forward-pair residue that the abort's reverse-pair
            // `classify_remove` reads as `Ambiguous` — `inspect_family` sets `foreign` BY NAME
            // (`residue.rs:179-181`, `:205-206`) and `classification.rs:141-143` returns it, one
            // frame BEFORE the `:175-177` check this sentence used to name (LAYER CORRECTED by
            // DR-1 S1, 2026-09-03; the footnote at the foot of this file) → `abort/evidence.rs`
            // → `Other` → `RecoveryRequired`, stranding `gwz merge --abort` — a directional-residue
            // window, distinct from and stronger than the detach window below. Ruling (a) 2026-09-02.
            artifact::write_atomic(&root.join(path), &candidate.marker_yaml)?;
        }
        PublicationPhysicalAction::WriteLock => {
            // [R2-P3-1] DATED RESIDUAL, E4.7 2026-09-02 — the lock STAYS RAW: a
            // `Bytes → Bytes` replacement, so the checked boundary's `replace_exact` detaches
            // before publishing and the shipped forward (`authority/observe/finalization/publication/live.rs`) and abort
            // (`merge/abort/evidence.rs::classify_file`) observers refuse to classify the
            // absence — an observation-dead window. The raw rename is atomic and opens
            // none. DR-1 (`dev-docs/GwzM5-8R2E-CapabilityFreeAmendment.md` §5).
            artifact::write_atomic(&root.join(LOCK_PATH), &candidate.lock_yaml)?;
        }
        PublicationPhysicalAction::WriteBoundary => {
            // [R2-P3-1] DATED RESIDUAL, E4.7 2026-09-02 — the boundary STAYS RAW, on
            // the same ground as the lock above: a `Bytes → Bytes` replacement whose
            // `replace_exact` detaches before publishing, into an absence the shipped
            // forward (`authority/observe/finalization/publication/live.rs`) and abort (`merge/abort/evidence.rs::classify_file`)
            // observers both refuse to classify — an observation-dead window. DR-1
            // (`dev-docs/GwzM5-8R2E-CapabilityFreeAmendment.md` §5); row `:279`'s
            // frozen cell-2 wording travels there with it.
            publish_workspace_exclude_candidate(root, &candidate.boundary_text)?;
        }
        PublicationPhysicalAction::StageIndex => {
            let marker = progress.candidate_marker_path.as_deref().ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecordUnreadable,
                    "candidate marker path is missing",
                )
            })?;
            backend.stage_paths(root, &[LOCK_PATH, marker])?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// [DR-1 S1] THE MARKER RESIDUAL'S MECHANISM, DRIVEN AND LAYER-CORRECTED
// (2026-09-03). `dev-docs/GwzM5-8DR1-Reconciliation-Design.md` §1.4; the driven
// wall is `dev-docs/GwzM5-8R2E-E45B-Report.md`, whose own sentence — repeated
// into the residual above until this step — named the wrong frame.
//
// The refusal is NOT the authority-current check at `classification.rs:175-177`.
// An interrupted forward publication of the marker mints its `.authority` and
// `.goal` under the FORWARD `action_key`, which hashes `expected` AND `goal`
// (`authority.rs:204-223`). The abort asks the counterpart REVERSE question
// `(Bytes(marker_yaml) -> Missing, Remove)`; its `inspect_family` enumerates the
// SAME family, because `family_key` is direction-free (`authority.rs:196-202`),
// and then rejects both members by name — `residue.rs:179-181` on the
// `.authority`, `residue.rs:205-206` on the `.goal` — returning
// `FamilyResidue { authority: None, .., foreign: true }`. `classify_exact`
// short-circuits at `classification.rs:141-143`, one frame EARLIER; `:175-177`
// is unreachable for this residue precisely because `residue.authority` is
// `None`. `merge/abort/evidence.rs:307-311` maps `Ambiguous` to
// `FileState::Other`, the evidence shape stops being exact, and the user gets
// `Ok(RecoveryRequired)`.
//
// Pinned, cause and all, by `checked_artifact::tests::removal_recovery::
// a_counterpart_forward_family_refuses_as_foreign_before_the_authority_check`.
// DR-1's cure must flip that pin deliberately: S2's direction-free survey, S2b's
// reconciler (retire-only for this leaf — nothing was detached), S3's pre-pass.
