//! A store or remover that fails mid-sequence: what stops, and what the
//! report leaves for manual cleanup.

use super::*;

/// Design §5.2 step 4: `disposing` is written first and its result is
/// checked, so a failed write stops before anything is removed.
#[test]
fn a_failed_disposing_write_stops_before_the_remover_is_called() {
    let mut session = ScriptedSession::ready_at("../ws-A");
    session.fail_apply = Some(StoreError::Io {
        operation: StoreOperation::WriteIndex,
        path: PathBuf::from("/fam/root/.gwz/local-family.yml"),
        detail: "no space left on device".to_owned(),
    });
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
    assert!(
        matches!(failure.error, DisposeError::Store(StoreError::Io { .. })),
        "{:?}",
        failure.error
    );
    assert!(failure.effects.is_empty(), "nothing completed");
    assert_no_removal(&ports);
    assert_eq!(
        session.applied(),
        vec![FamilyChange::MarkDisposing {
            name: name("A"),
            expected_allocation: allocation(),
        }],
        "the only write attempted was `disposing`, and it came first"
    );
    assert_eq!(
        session.reread().unwrap().unwrap().members[&name("A")].state,
        MemberState::Ready,
        "the failed write left the row alone"
    );
}

/// Checkpoint §11 (lane D): a pointer the store cannot physically remove
/// blocks row removal, and the report names the pointer, not the row.
#[test]
fn a_pointer_the_store_cannot_remove_blocks_the_row() {
    let pointer_failure = || StoreError::Io {
        operation: StoreOperation::RemovePointer,
        path: PathBuf::from("/fam/ws-A/.gwz/family-root"),
        detail: "permission denied".to_owned(),
    };
    // Through `--keep`.
    let mut session = ScriptedSession::ready_at("../ws-A");
    session.fail_remove_pointer = Some(pointer_failure());
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let failure = dispose(&keep(), &mut session, &mut ports).unwrap_err();
    assert_eq!(failure.error, DisposeError::Store(pointer_failure()));
    assert!(failure.effects.is_empty());
    assert!(
        session.applied().is_empty(),
        "the row removal was never attempted"
    );
    assert!(
        session
            .reread()
            .unwrap()
            .unwrap()
            .members
            .contains_key(&name("A")),
        "the row stands while its pointer does"
    );
    assert!(ports.calls().is_empty(), "keep consults no port");

    // And after a successful deletion: the row stays `disposing` for a
    // later command to report, and nothing is rolled back.
    let mut session = ScriptedSession::ready_at("../ws-A");
    session.fail_remove_pointer = Some(pointer_failure());
    let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
    assert_eq!(failure.error, DisposeError::Store(pointer_failure()));
    assert_eq!(
        failure.effects,
        vec![DisposeEffect::RowDisposing, DisposeEffect::DirectoryRemoved]
    );
    assert_eq!(
        session.applied(),
        vec![FamilyChange::MarkDisposing {
            name: name("A"),
            expected_allocation: allocation(),
        }],
        "RemoveRow was never attempted, so the row is not reported instead"
    );
}

/// Design §5.2/§5.3: a removal error stops, reports the remainder, and
/// rolls nothing back. A later explicit dispose reports the interrupted
/// state; once the contents are gone it may remove the stale row.
#[test]
fn a_removal_error_stops_and_leaves_the_remainder_for_manual_cleanup() {
    let (_store, mut session) = ready();
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let remaining = vec![
        PathBuf::from("/fam/ws-A/locked"),
        PathBuf::from("/fam/ws-A/locked/db"),
    ];
    ports.fail_removal(RemovalFailure {
        error: PortError::Removal {
            path: PathBuf::from("/fam/ws-A/locked/db"),
            detail: "resource busy".to_owned(),
        },
        remaining: remaining.clone(),
    });
    let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
    let DisposeError::RemovalStopped {
        remaining: reported,
        detail,
    } = &failure.error
    else {
        panic!("expected RemovalStopped, got {:?}", failure.error);
    };
    assert_eq!(reported, &remaining);
    assert!(detail.contains("resource busy"), "{detail}");
    assert_eq!(
        failure.effects,
        vec![DisposeEffect::RowDisposing],
        "the row was marked, nothing else completed"
    );
    assert_eq!(
        session.reread().unwrap().unwrap().members[&name("A")].state,
        MemberState::Disposing,
        "no rollback: the interrupted state stands for a later command to report"
    );

    // Repeating it is not a replay: the interrupted row is not forceable.
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
    assert_eq!(
        failure.error,
        DisposeError::Refused(Refusal::WrongState {
            name: name("A"),
            expected: MemberState::Ready,
            actual: MemberState::Disposing,
        })
    );
    assert_no_removal(&ports);

    // After manual cleanup the contents are gone, and an explicit
    // dispose may remove the stale row (design §5.2 step 5).
    let mut ports = scripted(
        TargetEvidence {
            target: TargetObservation::Missing,
            repositories: Vec::new(),
            copy: None,
            unknown: Vec::new(),
        },
        HistoryAnswer::Preserved,
    );
    let report = dispose(&delete(&[]), &mut session, &mut ports).expect("the stale row goes");
    assert_eq!(
        report.effects,
        vec![DisposeEffect::PointerRemoved, DisposeEffect::RowRemoved]
    );
    assert!(session.reread().unwrap().unwrap().members.is_empty());
    assert_no_removal(&ports);
}
