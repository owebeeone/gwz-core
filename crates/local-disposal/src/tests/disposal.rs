//! The ordinary outcomes: keep, the single clean deletion, the stale-row
//! exit and an unknown member.

use super::*;

/// An unrecognised name refuses before any observation.
#[test]
fn an_unknown_member_refuses_before_any_port_call() {
    let (_store, mut session) = ready();
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let request = DisposeRequest {
        name: name("Z"),
        ..delete(&[])
    };
    let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
    assert_eq!(
        failure.error,
        DisposeError::Refused(Refusal::NotFound { name: name("Z") })
    );
    assert!(ports.calls().is_empty(), "no port was consulted");
}

/// Design §8.4: `--keep` detaches C's metadata and leaves its entire
/// tree, open merge and history on disk. It consults no port at all.
#[test]
fn keep_detaches_a_ready_member_without_consulting_any_port() {
    let (store, mut session) = ready();
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let report = dispose(&keep(), &mut session, &mut ports).expect("keep detaches");
    assert_eq!(
        report.effects,
        vec![DisposeEffect::PointerRemoved, DisposeEffect::RowDetached],
        "the pointer goes strictly before the row"
    );
    assert!(
        ports.calls().is_empty(),
        "no evidence, history or removal call: every file stays"
    );
    assert!(session.reread().unwrap().unwrap().members.is_empty());
    assert!(store.pointers().is_empty());
    // A detached tree is no longer a member, so a repeat has no row.
    let failure = dispose(&keep(), &mut session, &mut ports).unwrap_err();
    assert_eq!(
        failure.error,
        DisposeError::Refused(Refusal::NotFound { name: name("A") })
    );
}

/// Design §12: a clean intact lane whose protected history lives in a
/// survivor is deleted once, with no archive and no second removal.
#[test]
fn a_clean_preserved_intact_lane_is_deleted_once() {
    let (store, mut session) = ready();
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let report = dispose(&delete(&[]), &mut session, &mut ports).expect("deletion proceeds");
    assert_eq!(
        report.effects,
        vec![
            DisposeEffect::RowDisposing,
            DisposeEffect::DirectoryRemoved,
            DisposeEffect::PointerRemoved,
            DisposeEffect::RowRemoved,
        ],
        "disposing is written first; the pointer goes before the row"
    );
    assert_eq!(
        ports.calls(),
        [
            DisposalCall::ObserveTarget {
                target: PathBuf::from(WS_A)
            },
            DisposalCall::CheckHistory {
                query: HistoryQuery {
                    target: RepoKey::Root,
                    protected: ProtectedRoots::default(),
                }
            },
            DisposalCall::RemoveDirectory {
                target: PathBuf::from(WS_A)
            },
        ],
        "one observation, one history query, exactly one removal of the validated target"
    );
    assert!(session.reread().unwrap().unwrap().members.is_empty());
    assert!(store.pointers().is_empty(), "no pointer is stranded");
}

/// A stale row is removed only after validation, and its removal asks
/// neither the work detector nor the history verifier: no file is
/// touched, so there is nothing to lose.
#[test]
fn a_stale_row_for_an_absent_target_needs_no_checks_and_no_force() {
    let (store, mut session) = ready();
    let mut ports = RecordingDisposalPorts::new();
    ports.evidence(TargetEvidence {
        target: TargetObservation::Missing,
        repositories: Vec::new(),
        copy: None,
        unknown: Vec::new(),
    });
    // No history answer is scripted: an unscripted call would be
    // `Unknown` and would refuse, so a green run proves none was made.
    let report = dispose(&delete(&[]), &mut session, &mut ports).expect("the stale row goes");
    assert_eq!(
        report.effects,
        vec![DisposeEffect::PointerRemoved, DisposeEffect::RowRemoved]
    );
    assert_eq!(
        ports.calls(),
        [DisposalCall::ObserveTarget {
            target: PathBuf::from(WS_A)
        }],
        "the target was observed; nothing else was asked and nothing was removed"
    );
    assert!(store.pointers().is_empty());
    assert!(session.reread().unwrap().unwrap().members.is_empty());
}

/// The stale exit is the only one that removes a row with no work or
/// history check, so a self-contradicting observation refuses there.
#[test]
fn an_absent_target_holding_repositories_refuses() {
    let (store, mut session) = ready();
    let mut ports = scripted(
        TargetEvidence {
            target: TargetObservation::Missing,
            ..clean_evidence()
        },
        HistoryAnswer::Preserved,
    );
    let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
    assert!(
        matches!(failure.error, DisposeError::PathMismatch { .. }),
        "{:?}",
        failure.error
    );
    assert!(failure.effects.is_empty());
    assert_no_removal(&ports);
    assert_eq!(store.pointers().len(), 1, "the row and pointer stand");
}
