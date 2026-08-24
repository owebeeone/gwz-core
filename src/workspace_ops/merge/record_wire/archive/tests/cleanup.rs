use super::super::decode_archived;
use super::fixtures::{MERGE_ID, Shape, bytes, oid, v0_record};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::PreservationEvidence;

#[test]
fn cleanup_worklist_is_archive_derived_sorted_and_non_authorizing() {
    let mut record = v0_record(Shape::CompletedCandidate);
    record.participants.get_mut("mem_a").unwrap().preservation = vec![PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/mem_a/head")),
        backup_commit: Some(oid('d')),
        stash_id: Some(format!("stash_{MERGE_ID}")),
        stash_object_id: Some(oid('e')),
        noop_commit: None,
        reset_commit: None,
    }];
    let decoded = decode_archived(&bytes(&record), MERGE_ID).unwrap();
    assert_eq!(decoded.cleanup.backup_refs.len(), 1);
    assert_eq!(decoded.cleanup.backup_refs[0].target_id, "mem_a");
    assert_eq!(decoded.cleanup.backup_refs[0].path, "members/a");
    assert!(decoded.cleanup.has_stash_evidence);
}

#[test]
fn cleanup_rejects_noncanonical_incomplete_and_duplicate_owners() {
    let mut wrong = v0_record(Shape::CompletedCandidate);
    wrong.participants.get_mut("mem_a").unwrap().preservation = vec![PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/other/mem_a/head".to_owned()),
        backup_commit: Some(oid('d')),
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    }];
    let error = decode_archived(&bytes(&wrong), MERGE_ID).unwrap_err();
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
    assert!(
        error
            .message
            .ends_with("archive preservation ref is outside the canonical merge-owned namespace")
    );

    let mut incomplete = v0_record(Shape::CompletedCandidate);
    incomplete
        .participants
        .get_mut("mem_a")
        .unwrap()
        .preservation = vec![PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/mem_a/head")),
        backup_commit: None,
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    }];
    assert!(decode_archived(&bytes(&incomplete), MERGE_ID).is_err());

    let mut duplicate = v0_record(Shape::CompletedCandidate);
    let row = PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/mem_a/head")),
        backup_commit: Some(oid('d')),
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    };
    duplicate
        .participants
        .get_mut("mem_a")
        .unwrap()
        .preservation = vec![row.clone(), row];
    let error = decode_archived(&bytes(&duplicate), MERGE_ID).unwrap_err();
    assert!(
        error
            .message
            .ends_with("archive contains duplicate or colliding preservation owners")
    );

    let mut collision = v0_record(Shape::CompletedCandidate);
    let root_row = PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/root/head")),
        backup_commit: Some(oid('d')),
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    };
    let mut root = collision.participants["mem_a"].clone();
    root.path = ".".to_owned();
    root.preservation = vec![root_row.clone()];
    collision.participants.insert("@root".to_owned(), root);
    collision.publication.as_mut().unwrap().root_preservation = vec![root_row];
    assert!(super::super::cleanup::from_v0(&collision).is_err());
}

#[test]
fn cleanup_rejects_empty_or_noncanonical_stash_evidence() {
    for evidence in [
        PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: None,
            stash_object_id: None,
            noop_commit: None,
            reset_commit: None,
        },
        PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: Some("stash_wrong".to_owned()),
            stash_object_id: Some(oid('e')),
            noop_commit: None,
            reset_commit: None,
        },
        PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: Some(format!("stash_{MERGE_ID}")),
            stash_object_id: Some("not-an-oid".to_owned()),
            noop_commit: None,
            reset_commit: None,
        },
    ] {
        let mut record = v0_record(Shape::CompletedCandidate);
        record.participants.get_mut("mem_a").unwrap().preservation = vec![evidence];
        let error = decode_archived(&bytes(&record), MERGE_ID).unwrap_err();
        assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
        assert!(
            error
                .message
                .ends_with("archive envelope or terminal state is contradictory")
        );
    }
}

#[test]
fn selected_root_participant_uses_the_single_root_owner_key() {
    let mut record = v0_record(Shape::CompletedCandidate);
    let mut root = record.participants.remove("mem_a").unwrap();
    root.path = ".".to_owned();
    root.target_kind = crate::workspace_ops::merge::MergeTargetKind::Root;
    root.preservation = vec![PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/root/head")),
        backup_commit: Some(oid('d')),
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    }];
    record.participants.insert("mem_root".to_owned(), root);

    let cleanup = super::super::cleanup::from_v0(&record).unwrap();

    assert_eq!(cleanup.backup_refs.len(), 1);
    assert_eq!(cleanup.backup_refs[0].target_id, "mem_root");
    assert_eq!(
        cleanup.backup_refs[0].name,
        format!("refs/gwz/merge/{MERGE_ID}/root/head")
    );
}

// ---------------------------------------------------------------------------
// Durable preservation-cursor markers on the terminal plane.
// `GwzM5-8DurableCursorAmendment.md` §5 / §8.6.
// ---------------------------------------------------------------------------

/// §5: an `N`-only or `N+R` row is a new archived shape and the terminal plane
/// must accept it. Without the marker-aware arm one fully-noop owner would make
/// EVERY archived merge's worklist derivation fail, blocking all targeted
/// cleanup (a permanent-retention U8-class growth).
#[test]
fn marker_only_rows_derive_a_worklist_without_error_and_contribute_nothing() {
    for (label, noop, reset) in [
        ("N only", Some(oid('c')), None),
        ("N+R", Some(oid('c')), Some(oid('c'))),
    ] {
        let mut record = super::fixtures::v1_record(Shape::CompletedCandidate);
        record.participants.get_mut("mem_a").unwrap().preservation = vec![PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: None,
            stash_object_id: None,
            noop_commit: noop.clone(),
            reset_commit: reset.clone(),
        }];
        let cleanup = super::super::cleanup::from_v1(&record)
            .unwrap_or_else(|error| panic!("{label} row must derive: {error:?}"));
        assert!(
            cleanup.backup_refs.is_empty(),
            "{label} row contributed a backup ref"
        );
        assert!(
            !cleanup.has_stash_evidence,
            "{label} row claimed stash evidence"
        );
    }
}

/// §5: other owners' backup refs must still enumerate beside a marker-only row.
#[test]
fn a_marker_only_owner_does_not_hide_another_owners_backup_ref() {
    let mut record = super::fixtures::v1_record(Shape::CompletedCandidate);
    record.participants.get_mut("mem_a").unwrap().preservation = vec![PreservationEvidence {
        backup_ref: None,
        backup_commit: None,
        stash_id: None,
        stash_object_id: None,
        noop_commit: Some(oid('c')),
        reset_commit: Some(oid('c')),
    }];
    record.publication.as_mut().unwrap().root_preservation = vec![PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/root/head")),
        backup_commit: Some(oid('d')),
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    }];
    let cleanup = super::super::cleanup::from_v1(&record).unwrap();
    assert_eq!(cleanup.backup_refs.len(), 1);
    assert_eq!(
        cleanup.backup_refs[0].name,
        format!("refs/gwz/merge/{MERGE_ID}/root/head")
    );
}

/// §2.2 terminal-plane fate: a marker riding a surviving stash-bearing row
/// (`B+S+R`) still enumerates its backup ref and still reports stash evidence —
/// markers never block backup-ref deletion or worklist derivation.
#[test]
fn markers_beside_a_stash_pair_neither_block_nor_alter_the_worklist() {
    let mut record = super::fixtures::v1_record(Shape::CompletedCandidate);
    record.participants.get_mut("mem_a").unwrap().preservation = vec![PreservationEvidence {
        backup_ref: Some(format!("refs/gwz/merge/{MERGE_ID}/mem_a/head")),
        backup_commit: Some(oid('d')),
        stash_id: Some(format!("stash_{MERGE_ID}")),
        stash_object_id: Some(oid('e')),
        noop_commit: None,
        reset_commit: Some(oid('c')),
    }];
    let cleanup = super::super::cleanup::from_v1(&record).unwrap();
    assert_eq!(cleanup.backup_refs.len(), 1);
    assert_eq!(cleanup.backup_refs[0].target_id, "mem_a");
    assert!(cleanup.has_stash_evidence);
}

/// §5: the delta is v0-inert. The `from_v0` leg keeps rejecting the empty row
/// exactly as before, because no v0 record carries markers.
#[test]
fn the_empty_row_rejection_is_unchanged_on_both_legs() {
    let mut record = v0_record(Shape::CompletedCandidate);
    record.participants.get_mut("mem_a").unwrap().preservation = vec![PreservationEvidence {
        backup_ref: None,
        backup_commit: None,
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    }];
    assert!(super::super::cleanup::from_v0(&record).is_err());

    let mut v1 = super::fixtures::v1_record(Shape::CompletedCandidate);
    v1.participants.get_mut("mem_a").unwrap().preservation = vec![PreservationEvidence {
        backup_ref: None,
        backup_commit: None,
        stash_id: None,
        stash_object_id: None,
        noop_commit: None,
        reset_commit: None,
    }];
    assert!(super::super::cleanup::from_v1(&v1).is_err());
}

/// [Code review P2-1 / P3-3] The one shape that distinguishes a forked from an
/// unforked `collect_owner`: a row whose only content is a marker.
///
/// `GwzM5-8DurableCursorAmendment.md` §5 declares the marker-aware arm
/// "v0-inert: no v0 record carries markers, so the `from_v0` leg of the shared
/// derivation never sees the new arm". The shared row struct parses the two
/// names for v0 records too, so that inertness is enforced by the version fork
/// rather than assumed: on the v0 leg a fabricated marker never legitimizes an
/// otherwise-empty row, and retention keeps refusing fail-closed exactly as it
/// did before this amendment (§2.3 — "the value is never adopted").
#[test]
fn a_marker_only_row_is_row_content_on_the_v1_leg_and_an_empty_row_on_the_v0_leg() {
    for (label, noop, reset) in [
        ("N only", Some(oid('c')), None),
        ("R only", None, Some(oid('c'))),
        ("N+R", Some(oid('c')), Some(oid('c'))),
        // Not even a well-formed value: the §2.2 validator lives in the v1
        // model, so the v0 leg must not treat garbage as row-legitimizing.
        ("garbage marker", Some("not-a-commit".to_owned()), None),
    ] {
        let marker_row = PreservationEvidence {
            backup_ref: None,
            backup_commit: None,
            stash_id: None,
            stash_object_id: None,
            noop_commit: noop.clone(),
            reset_commit: reset.clone(),
        };

        let mut v0 = v0_record(Shape::CompletedCandidate);
        v0.participants.get_mut("mem_a").unwrap().preservation = vec![marker_row.clone()];
        assert_eq!(
            super::super::cleanup::from_v0(&v0),
            Err(super::super::cleanup::CleanupError::ContradictoryEvidence),
            "{label}: a fabricated marker must not legitimize a v0 row"
        );

        let mut v1 = super::fixtures::v1_record(Shape::CompletedCandidate);
        v1.participants.get_mut("mem_a").unwrap().preservation = vec![marker_row];
        let cleanup = super::super::cleanup::from_v1(&v1)
            .unwrap_or_else(|error| panic!("{label}: v1 leg must derive: {error:?}"));
        assert!(cleanup.backup_refs.is_empty(), "{label}");
        assert!(!cleanup.has_stash_evidence, "{label}");
    }
}
