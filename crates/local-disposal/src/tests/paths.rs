//! Step 1 path validation: protected roots, overlaps, moved roots and
//! incomplete or interrupted targets.

use super::*;

/// Design §5.2 step 1 and §8.4: the root is never deleted, and the
/// target may never contain the working directory.
#[test]
fn the_root_and_the_working_directory_are_protected() {
    // Host paths the pure rules cannot see: `../root` and `../../fam`
    // are legal member-path spellings that resolve back onto the root
    // and onto a directory containing it. Only a decoded index can carry
    // them, so they arrive through a session, not through an allocation.
    for path in ["../root", "../../fam"] {
        let mut session = ScriptedSession::ready_at(path);
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert_eq!(failure.error, DisposeError::RootImmutable, "path {path}");
        assert!(failure.effects.is_empty());
        assert_no_removal(&ports);
    }
    // The same for the spellings the pure rules do refuse.
    for path in [".", "..", "ws/.."] {
        let mut session = ScriptedSession::ready_at(path);
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert_eq!(failure.error, DisposeError::RootImmutable, "path {path}");
    }

    // The invoking process stands inside the target.
    for cwd in [WS_A, "/fam/ws-A/sub/dir", "/fam/root/../ws-A"] {
        let (_store, mut session) = ready();
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let request = DisposeRequest {
            cwd: PathBuf::from(cwd),
            ..delete(&HazardWaiver::ALL)
        };
        let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
        assert_eq!(
            failure.error,
            DisposeError::TargetContainsCwd {
                target: PathBuf::from(WS_A)
            },
            "cwd {cwd}"
        );
        assert_no_removal(&ports);
        // `--keep` removes no file, but step 1 still runs before step 2.
        let (_store, mut session) = ready();
        let request = DisposeRequest {
            cwd: PathBuf::from(cwd),
            ..keep()
        };
        let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
        assert!(matches!(
            failure.error,
            DisposeError::TargetContainsCwd { .. }
        ));
        assert_eq!(
            session.reread().unwrap().unwrap().members.len(),
            1,
            "the row survives the refusal"
        );
    }
}

/// A recorded path that overlaps the root or another member would delete
/// a directory the row does not own. `validate_transition` refuses these
/// at allocation; a decoded index can still carry them.
#[test]
fn an_overlapping_or_unusable_recorded_path_refuses() {
    // Nested inside the root.
    let mut session = ScriptedSession::ready_at("../root/sub");
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
    assert_eq!(
        failure.error,
        DisposeError::Refused(Refusal::NestedPath {
            path: "../root/sub".to_owned(),
            other: ".".to_owned(),
        })
    );
    assert_no_removal(&ports);

    // Containing another member's tree.
    let mut session = ScriptedSession::with_rows(&[
        (
            "A",
            MemberRow {
                state: MemberState::Ready,
                ..row("../ws-A")
            },
        ),
        (
            "B",
            MemberRow {
                state: MemberState::Ready,
                allocation_id: AllocationId::new("alloc-B").unwrap(),
                ..row("../ws-A/nested")
            },
        ),
    ]);
    let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
    assert_eq!(
        failure.error,
        DisposeError::Refused(Refusal::NestedPath {
            path: "../ws-A".to_owned(),
            other: "../ws-A/nested".to_owned(),
        })
    );
    assert_no_removal(&ports);

    // Spellings the index must never record, and paths that are not
    // member paths at all.
    for (path, expected) in [
        (
            "../ws-A/../ws-A",
            DisposeError::Refused(Refusal::PathNotNormalised {
                name: name("A"),
                path: "../ws-A/../ws-A".to_owned(),
                normalised: "../ws-A".to_owned(),
            }),
        ),
        (
            "/fam/ws-A",
            DisposeError::Refused(Refusal::InvalidRow {
                name: name("A"),
                detail: gwz_family_model::PathError::Absolute {
                    path: "/fam/ws-A".to_owned(),
                }
                .to_string(),
            }),
        ),
        (
            "",
            DisposeError::Refused(Refusal::InvalidRow {
                name: name("A"),
                detail: gwz_family_model::PathError::Empty.to_string(),
            }),
        ),
    ] {
        let mut session = ScriptedSession::ready_at(path);
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert_eq!(failure.error, expected, "path `{path}`");
        assert!(failure.effects.is_empty());
        assert_no_removal(&ports);
    }
}

/// Checkpoint §11 lane-D note: a moved root, a replaced target or an
/// interrupted detach is a `PathMismatch`, and force never excuses it.
#[test]
fn a_moved_root_or_replaced_target_refuses_as_a_path_mismatch() {
    // The request's root is not the root whose lock this session holds.
    let (_store, mut session) = ready();
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let request = DisposeRequest {
        root: PathBuf::from("/fam/moved-root"),
        cwd: PathBuf::from("/fam/moved-root"),
        ..delete(&HazardWaiver::ALL)
    };
    let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
    assert!(
        matches!(failure.error, DisposeError::PathMismatch { .. }),
        "a moved root must be a PathMismatch, got {:?}",
        failure.error
    );
    assert_no_removal(&ports);

    // What stands at the recorded path is not this member.
    let replaced = [
        TargetObservation::Present {
            pointer: PointerObservation::OtherFamily,
            marker: MarkerObservation::Matches,
        },
        TargetObservation::Present {
            pointer: PointerObservation::Matches,
            marker: MarkerObservation::Mismatch,
        },
        TargetObservation::Present {
            pointer: PointerObservation::IsIndex,
            marker: MarkerObservation::Absent,
        },
        TargetObservation::Present {
            pointer: PointerObservation::Absent,
            marker: MarkerObservation::Matches,
        },
        TargetObservation::Malformed {
            detail: "the family pointer is not YAML".to_owned(),
        },
    ];
    for target in replaced {
        let (_store, mut session) = ready();
        let mut ports = scripted(
            TargetEvidence {
                target: target.clone(),
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        );
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert!(
            matches!(failure.error, DisposeError::PathMismatch { .. }),
            "{target:?} must refuse as a PathMismatch, got {:?}",
            failure.error
        );
        assert!(failure.effects.is_empty());
        assert_no_removal(&ports);
    }
}

/// Design §5.2/§5.3: an incomplete create and an interrupted deletion
/// are retained and are not forceable, but `--keep` detaches them.
#[test]
fn incomplete_and_interrupted_targets_refuse_deletion_but_accept_keep() {
    for state in [MemberState::Creating, MemberState::Disposing] {
        let (_store, mut session) = family("../ws-A", state);
        let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert_eq!(
            failure.error,
            DisposeError::Refused(Refusal::WrongState {
                name: name("A"),
                expected: MemberState::Ready,
                actual: state,
            }),
            "{state:?} is not forceable through this path"
        );
        assert!(failure.effects.is_empty());
        assert_no_removal(&ports);

        let report = dispose(&keep(), &mut session, &mut ports).expect("keep detaches");
        assert_eq!(
            report.effects,
            vec![DisposeEffect::PointerRemoved, DisposeEffect::RowDetached]
        );
        assert!(
            session.reread().unwrap().unwrap().members.is_empty(),
            "the row is detached"
        );
        assert_no_removal(&ports);
    }
}
