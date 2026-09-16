//! The history port: unpreserved answers, their one waiver and how often
//! the port is asked.

use super::*;

/// The operator's standing default (design §5): unpreserved history
/// refuses, and nothing is removed.
#[test]
fn unpreserved_history_refuses_before_any_removal() {
    let (_store, mut session) = ready();
    let mut ports = scripted(
        clean_evidence(),
        HistoryAnswer::Unpreserved {
            detail: "refs/heads/lane/agent-17 is in no survivor".to_owned(),
        },
    );
    let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
    assert_eq!(
        failure.error,
        DisposeError::Hazards(vec![HazardFinding {
            waiver: HazardWaiver::UnpreservedHistory,
            repository: RepoKey::Root,
            hazards: Vec::new(),
            detail: Some("refs/heads/lane/agent-17 is in no survivor".to_owned()),
        }])
    );
    assert!(failure.effects.is_empty());
    assert_no_removal(&ports);
    assert_eq!(
        session.reread().unwrap().unwrap().members[&name("A")].state,
        MemberState::Ready,
        "the row is untouched"
    );
}

/// A clean tree with unique history still refuses without the name
/// (design §8.4), and proceeds past the check with it.
#[test]
fn unpreserved_history_is_waivable_only_by_its_own_name() {
    for waivers in [
        vec![],
        vec![HazardWaiver::Dirty],
        vec![HazardWaiver::OpenMerge, HazardWaiver::Dirty],
    ] {
        let (_store, mut session) = ready();
        let mut ports = scripted(
            clean_evidence(),
            HistoryAnswer::Unpreserved {
                detail: "unique".to_owned(),
            },
        );
        let failure = dispose(&delete(&waivers), &mut session, &mut ports).unwrap_err();
        assert!(
            matches!(failure.error, DisposeError::Hazards(ref findings)
                if findings.iter().all(|f| f.waiver == HazardWaiver::UnpreservedHistory)),
            "{waivers:?} must not waive unpreserved history: {:?}",
            failure.error
        );
        assert_no_removal(&ports);
    }
}

/// Design §5.1: every repository in the deletion tree is inspected, so
/// the history port is asked once per repository, keyed by identity.
#[test]
fn the_history_port_is_asked_once_per_repository_in_the_tree() {
    let keys = [
        RepoKey::Root,
        RepoKey::Member {
            id: "taut".to_owned(),
        },
        RepoKey::Member {
            id: "nested".to_owned(),
        },
    ];
    let (_store, mut session) = ready();
    let mut ports = scripted(
        TargetEvidence {
            repositories: vec![
                repository(keys[0].clone(), WS_A),
                repository(keys[1].clone(), "/fam/ws-A/taut"),
                repository(keys[2].clone(), "/fam/ws-A/taut/nested"),
            ],
            ..clean_evidence()
        },
        HistoryAnswer::Preserved,
    );
    // The middle repository is the only one whose history is unique.
    ports.history_sequence([
        HistoryAnswer::Preserved,
        HistoryAnswer::Unpreserved {
            detail: "taut has unique commits".to_owned(),
        },
        HistoryAnswer::Preserved,
    ]);
    let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
    assert_eq!(
        failure.error,
        DisposeError::Hazards(vec![HazardFinding {
            waiver: HazardWaiver::UnpreservedHistory,
            repository: keys[1].clone(),
            hazards: Vec::new(),
            detail: Some("taut has unique commits".to_owned()),
        }]),
        "the refusal names the repository that is not preserved"
    );
    let queried: Vec<RepoKey> = ports
        .calls()
        .iter()
        .filter_map(|call| match call {
            DisposalCall::CheckHistory { query } => Some(query.target.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(queried, keys, "one query per repository, in tree order");
    assert_no_removal(&ports);
}
