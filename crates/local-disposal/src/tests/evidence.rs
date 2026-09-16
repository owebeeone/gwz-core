//! Work and history evidence: unknown answers, uninterpretable trees, port
//! failures and evidence from outside the deletion tree.

use super::*;

/// Design §5.1: unknown work or unknown history refuses ordinary
/// deletion, and **no** force name waives it.
#[test]
fn unknown_work_or_history_refuses_and_no_force_waives_it() {
    let unreadable = UnknownReason::new(UnknownKind::Unreadable, "the index could not be read");
    let cases: Vec<(&str, TargetEvidence, HistoryAnswer)> = vec![
        (
            "unknown work inventory",
            TargetEvidence {
                repositories: vec![RepositoryEvidence {
                    work: Observation::Unknown(vec![unreadable.clone()]),
                    ..repository(RepoKey::Root, WS_A)
                }],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "a known inventory with an unestablished path",
            TargetEvidence {
                repositories: vec![RepositoryEvidence {
                    work: Observation::Known(WorkObservation {
                        unknown: vec![unreadable.clone()],
                        ..WorkObservation::default()
                    }),
                    ..repository(RepoKey::Root, WS_A)
                }],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "uninterpretable gwz evidence",
            TargetEvidence {
                repositories: vec![RepositoryEvidence {
                    gwz: GwzEvidence {
                        stash: EvidenceState::Unknown {
                            detail: "unsupported stash record".to_owned(),
                        },
                        ..GwzEvidence::default()
                    },
                    ..repository(RepoKey::Root, WS_A)
                }],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "unknown history inventory",
            TargetEvidence {
                repositories: vec![RepositoryEvidence {
                    history: Observation::Unknown(vec![unreadable.clone()]),
                    ..repository(RepoKey::Root, WS_A)
                }],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "the verifier hit a resource limit",
            clean_evidence(),
            HistoryAnswer::Unknown {
                reasons: vec![UnknownReason::new(
                    UnknownKind::LimitExceeded,
                    "100000 roots",
                )],
            },
        ),
    ];
    for (label, evidence, history) in cases {
        for waivers in [vec![], HazardWaiver::ALL.to_vec()] {
            let (_store, mut session) = ready();
            let mut ports = scripted(evidence.clone(), history.clone());
            let failure = dispose(&delete(&waivers), &mut session, &mut ports).unwrap_err();
            assert!(
                matches!(failure.error, DisposeError::Unknown(ref reasons) if !reasons.is_empty()),
                "{label} with {waivers:?} must refuse as unknown, got {:?}",
                failure.error
            );
            assert!(failure.effects.is_empty());
            assert_no_removal(&ports);
        }
    }
}

/// Design §5.1: an uninterpretable layout, and a tree in which no
/// repository was observed at all, refuse as unknown.
#[test]
fn uninterpretable_or_unrecognised_trees_refuse_as_unknown() {
    let (_store, mut session) = ready();
    let mut ports = scripted(
        TargetEvidence {
            unknown: vec![UnknownReason::new(
                UnknownKind::UnsupportedLayout,
                "nested repository with external alternates",
            )],
            ..clean_evidence()
        },
        HistoryAnswer::Preserved,
    );
    let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
    assert!(matches!(failure.error, DisposeError::Unknown(_)));
    assert_no_removal(&ports);

    let (_store, mut session) = ready();
    let mut ports = scripted(
        TargetEvidence {
            repositories: Vec::new(),
            ..clean_evidence()
        },
        HistoryAnswer::Preserved,
    );
    let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
    assert!(
        matches!(failure.error, DisposeError::Unknown(_)),
        "a tree with no observed repository is not recognised: {:?}",
        failure.error
    );
    assert_no_removal(&ports);
}

/// An evidence port that cannot answer refuses; it never reads as clean.
#[test]
fn an_evidence_port_failure_refuses() {
    let (_store, mut session) = ready();
    let mut ports = RecordingDisposalPorts::new();
    ports.fail_evidence(PortError::Evidence {
        detail: "the deletion tree could not be walked".to_owned(),
    });
    let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
    assert!(
        matches!(
            failure.error,
            DisposeError::Port(PortError::Evidence { .. })
        ),
        "{:?}",
        failure.error
    );
    assert!(failure.effects.is_empty());
    assert_no_removal(&ports);
}

/// Design §5.2 step 4: nothing outside the validated target is ever
/// considered, so an observation that reached out of the tree — the
/// visible evidence of a symlink entry into an external tree — refuses.
#[test]
fn evidence_from_outside_the_deletion_tree_refuses() {
    let outside = [
        ("/fam/ws-B", None),
        ("/fam/ws-A-sibling", None),
        (WS_A, Some(PathBuf::from("/fam/ws-B/.git"))),
    ];
    for (path, common_dir) in outside {
        let mut evidence = repository(
            RepoKey::Member {
                id: "m1".to_owned(),
            },
            path,
        );
        if let Some(common_dir) = common_dir {
            evidence.info.common_dir = common_dir;
        }
        let (_store, mut session) = ready();
        let mut ports = scripted(
            TargetEvidence {
                repositories: vec![repository(RepoKey::Root, WS_A), evidence],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        );
        let failure = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports).unwrap_err();
        assert!(
            matches!(failure.error, DisposeError::PathMismatch { .. }),
            "{path} must refuse, got {:?}",
            failure.error
        );
        assert_no_removal(&ports);
    }
}
