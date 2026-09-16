//! The sweep proving no refusal path ever reaches the remover.

use super::*;

/// The whole refusal surface in one sweep: on every one of them the
/// removal port recorded zero calls and no row was removed.
#[test]
fn no_refusal_path_ever_reaches_the_remover() {
    let unreadable = UnknownReason::new(UnknownKind::Unreadable, "unreadable");
    let cases: Vec<(&str, DisposeRequest, TargetEvidence, HistoryAnswer)> = vec![
        (
            "unknown member",
            DisposeRequest {
                name: name("Z"),
                ..delete(&HazardWaiver::ALL)
            },
            clean_evidence(),
            HistoryAnswer::Preserved,
        ),
        (
            "repeated waiver",
            delete(&[HazardWaiver::Dirty, HazardWaiver::Dirty]),
            clean_evidence(),
            HistoryAnswer::Preserved,
        ),
        (
            "cwd inside the target",
            DisposeRequest {
                cwd: PathBuf::from("/fam/ws-A/src"),
                ..delete(&HazardWaiver::ALL)
            },
            clean_evidence(),
            HistoryAnswer::Preserved,
        ),
        (
            "a root the lock does not hold",
            DisposeRequest {
                root: PathBuf::from("/fam/elsewhere"),
                cwd: PathBuf::from("/fam/elsewhere"),
                ..delete(&HazardWaiver::ALL)
            },
            clean_evidence(),
            HistoryAnswer::Preserved,
        ),
        (
            "a replaced target",
            delete(&HazardWaiver::ALL),
            TargetEvidence {
                target: TargetObservation::Present {
                    pointer: PointerObservation::OtherFamily,
                    marker: MarkerObservation::Mismatch,
                },
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "an uninterpretable nested layout",
            delete(&HazardWaiver::ALL),
            TargetEvidence {
                unknown: vec![unreadable.clone()],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "no repository in the tree",
            delete(&HazardWaiver::ALL),
            TargetEvidence {
                repositories: Vec::new(),
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "a repository outside the tree",
            delete(&HazardWaiver::ALL),
            TargetEvidence {
                repositories: vec![repository(RepoKey::Root, "/fam/ws-B")],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "unknown work",
            delete(&HazardWaiver::ALL),
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
            "unknown history",
            delete(&HazardWaiver::ALL),
            clean_evidence(),
            HistoryAnswer::Unknown {
                reasons: vec![unreadable.clone()],
            },
        ),
        (
            "unwaived dirt",
            delete(&[HazardWaiver::OpenMerge, HazardWaiver::UnpreservedHistory]),
            TargetEvidence {
                repositories: vec![RepositoryEvidence {
                    work: dirty_work(),
                    ..repository(RepoKey::Root, WS_A)
                }],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "unwaived open merge",
            delete(&[HazardWaiver::Dirty, HazardWaiver::UnpreservedHistory]),
            TargetEvidence {
                repositories: vec![RepositoryEvidence {
                    gwz: open_merge(),
                    ..repository(RepoKey::Root, WS_A)
                }],
                ..clean_evidence()
            },
            HistoryAnswer::Preserved,
        ),
        (
            "unwaived unpreserved history",
            delete(&[HazardWaiver::Dirty, HazardWaiver::OpenMerge]),
            clean_evidence(),
            HistoryAnswer::Unpreserved {
                detail: "unique".to_owned(),
            },
        ),
    ];
    for (label, request, evidence, history) in cases {
        let (store, mut session) = ready();
        let mut ports = scripted(evidence, history);
        let failure = dispose(&request, &mut session, &mut ports)
            .err()
            .unwrap_or_else(|| panic!("{label} must refuse"));
        assert!(
            failure.effects.is_empty(),
            "{label} completed {:?} before refusing",
            failure.effects
        );
        assert_no_removal(&ports);
        assert_eq!(
            store.pointers().len(),
            1,
            "{label}: the clone pointer still stands"
        );
        assert_eq!(
            session.reread().unwrap().unwrap().members.len(),
            1,
            "{label}: the row still stands"
        );
    }
}
