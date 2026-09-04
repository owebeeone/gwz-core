//! The v1 evidence-rollback step/shape table, frozen.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** These two cases came from
//! `merge/abort/tests/recovery.rs`, whose other three cases drove the v0
//! abort engine's retry loop and are deleted with it.

use super::evidence::{V1EvidenceRollbackObservation as E, classify_v1_evidence_shape_for_test};
use crate::workspace_ops::merge::model::v1::EvidenceRollbackStepV1;

const ALL_EVIDENCE_SHAPES: [&str; 16] = [
    "BBBB", "BBBC", "BBCB", "BBCC", "BCBB", "BCBC", "BCCB", "BCCC", "CBBB", "CBBC", "CBCB", "CBCC",
    "CCBB", "CCBC", "CCCB", "CCCC",
];

#[test]
fn evidence_commit_accepts_exactly_the_nine_frozen_shapes() {
    const ALLOWED: [&str; 9] = [
        "BBBB", "BBCB", "BCCB", "CBCB", "CCCB", "BBCC", "BCCC", "CBCC", "CCCC",
    ];
    for shape in ALL_EVIDENCE_SHAPES {
        for (head_before, head_after, expected) in
            [(true, false, E::Before), (false, true, E::After)]
        {
            assert_eq!(
                classify_v1_evidence_shape_for_test(
                    EvidenceRollbackStepV1::EvidenceCommit,
                    head_before,
                    head_after,
                    shape,
                ),
                if ALLOWED.contains(&shape) {
                    expected
                } else {
                    E::Ambiguous
                },
                "{expected:?} {shape}",
            );
        }
    }
}

#[test]
fn every_later_step_accepts_only_its_frozen_before_and_after_rows() {
    let rows = [
        (
            EvidenceRollbackStepV1::Boundary,
            &["CBCB", "CCCB", "CBCC", "CCCC"][..],
            &["BBBB", "BBCB", "BCCB", "BBCC", "BCCC"][..],
        ),
        (
            EvidenceRollbackStepV1::Lock,
            &["BCCB", "BCCC"][..],
            &["BBBB", "BBCB", "BBCC"][..],
        ),
        (
            EvidenceRollbackStepV1::Marker,
            &["BBCB", "BBCC"][..],
            &["BBBB", "BBBC"][..],
        ),
        (EvidenceRollbackStepV1::Index, &["BBBC"][..], &["BBBB"][..]),
        (EvidenceRollbackStepV1::Complete, &[][..], &["BBBB"][..]),
    ];
    for (step, before, after) in rows {
        for shape in ALL_EVIDENCE_SHAPES {
            let expected = if before.contains(&shape) {
                E::Before
            } else if after.contains(&shape) {
                E::After
            } else {
                E::Ambiguous
            };
            assert_eq!(
                classify_v1_evidence_shape_for_test(step, false, true, shape),
                expected,
                "{step:?} {shape}",
            );
        }
    }
}
