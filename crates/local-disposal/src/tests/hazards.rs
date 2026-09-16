//! The hazard vocabulary and the waiver rules over it.

use super::*;

#[test]
fn hazard_vocabulary_is_exact() {
    assert_eq!(
        HazardWaiver::parse("open-merge"),
        Some(HazardWaiver::OpenMerge)
    );
    assert_eq!(HazardWaiver::parse("dirty"), Some(HazardWaiver::Dirty));
    assert_eq!(
        HazardWaiver::parse("unpreserved-history"),
        Some(HazardWaiver::UnpreservedHistory)
    );
    assert_eq!(HazardWaiver::parse("force"), None);
    assert_eq!(HazardWaiver::parse("Dirty"), None);
    let parsed = HazardWaiver::parse_all(&["dirty".to_owned(), "open-merge".to_owned()]).unwrap();
    assert_eq!(parsed, vec![HazardWaiver::Dirty, HazardWaiver::OpenMerge]);
    assert_eq!(HazardWaiver::parse_all(&[]).unwrap(), Vec::new());
    let unknown = HazardWaiver::parse_all(&["true".to_owned()]).unwrap_err();
    assert_eq!(unknown.name, "true");
    assert!(unknown.to_string().contains("unpreserved-history"));
    assert!(HazardWaiver::parse_all(&["dirty".to_owned(), "dirty".to_owned()]).is_err());
}

/// Design §5.2 step 3: empty and unknown force names and keep+force all
/// refuse, and every hazard the classifier can name has a waiver.
#[test]
fn empty_unknown_and_keep_plus_force_names_refuse() {
    assert_eq!(
        DisposePolicy::parse(true, &[]).unwrap(),
        DisposePolicy::Keep
    );
    assert_eq!(
        DisposePolicy::parse(false, &[]).unwrap(),
        DisposePolicy::Delete {
            waivers: Vec::new()
        },
        "an absent force list is no force, not a refusal"
    );
    for name in ["", " ", "all", "true", "unpreserved history"] {
        let error = DisposePolicy::parse(false, &[name.to_owned()]).unwrap_err();
        assert_eq!(
            error,
            PolicyError::UnknownHazard(UnknownHazard {
                name: name.to_owned()
            }),
            "`{name}` is not a hazard name"
        );
    }
    let keep_force = DisposePolicy::parse(true, &["dirty".to_owned()]).unwrap_err();
    assert_eq!(
        keep_force,
        PolicyError::KeepWithForce {
            names: vec!["dirty".to_owned()]
        }
    );
    assert!(keep_force.to_string().contains("mutually exclusive"));
    // The classifier's force names and this vocabulary are one map.
    for kind in [
        gwz_work_detector::HazardKind::Work(WorkKind::Untracked),
        gwz_work_detector::HazardKind::Suppressed,
        gwz_work_detector::HazardKind::NativeStash,
        gwz_work_detector::HazardKind::OpenNativeOperation,
        gwz_work_detector::HazardKind::OpenGwzMerge,
        gwz_work_detector::HazardKind::OpenGwzStash,
        gwz_work_detector::HazardKind::OpenGwzRecord,
    ] {
        let force = kind.force_name().expect("a known hazard has a force name");
        assert!(
            HazardWaiver::parse(force).is_some(),
            "{kind:?} names `{force}`, which this crate cannot waive"
        );
    }
    assert_eq!(
        gwz_work_detector::HazardKind::UninterpretableEvidence.force_name(),
        None,
        "uninterpretable evidence is never waivable"
    );
}

/// Each known hazard refuses without its own name (design §8.4's
/// `--force open-merge` still refusing dirt and history).
#[test]
fn each_known_hazard_refuses_until_its_own_name_is_given() {
    let evidence = TargetEvidence {
        repositories: vec![RepositoryEvidence {
            work: dirty_work(),
            gwz: open_merge(),
            ..repository(RepoKey::Root, WS_A)
        }],
        ..clean_evidence()
    };
    for (waivers, still) in [
        (
            vec![],
            vec![
                HazardWaiver::OpenMerge,
                HazardWaiver::Dirty,
                HazardWaiver::UnpreservedHistory,
            ],
        ),
        (
            vec![HazardWaiver::OpenMerge],
            vec![HazardWaiver::Dirty, HazardWaiver::UnpreservedHistory],
        ),
        (
            vec![HazardWaiver::OpenMerge, HazardWaiver::Dirty],
            vec![HazardWaiver::UnpreservedHistory],
        ),
        (
            vec![HazardWaiver::Dirty, HazardWaiver::UnpreservedHistory],
            vec![HazardWaiver::OpenMerge],
        ),
    ] {
        let (_store, mut session) = ready();
        let mut ports = scripted(
            evidence.clone(),
            HistoryAnswer::Unpreserved {
                detail: "unique".to_owned(),
            },
        );
        let failure = dispose(&delete(&waivers), &mut session, &mut ports).unwrap_err();
        let DisposeError::Hazards(findings) = &failure.error else {
            panic!(
                "{waivers:?} must refuse with hazards, got {:?}",
                failure.error
            );
        };
        let mut named: Vec<HazardWaiver> = findings.iter().map(|f| f.waiver).collect();
        named.sort();
        named.dedup();
        let mut expected = still.clone();
        expected.sort();
        assert_eq!(named, expected, "waived {waivers:?}");
        assert_no_removal(&ports);
    }
}

/// Design §8.4: the explicit destructive alternative. Every named
/// hazard, and only the named ones, is waived.
#[test]
fn every_named_hazard_is_waived_over_an_intact_ready_tree() {
    let (_store, mut session) = ready();
    let mut ports = scripted(
        TargetEvidence {
            repositories: vec![RepositoryEvidence {
                work: dirty_work(),
                gwz: open_merge(),
                ..repository(RepoKey::Root, WS_A)
            }],
            ..clean_evidence()
        },
        HistoryAnswer::Unpreserved {
            detail: "lane/agent-17 is unique".to_owned(),
        },
    );
    let report = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports)
        .expect("all three names waive all three hazards");
    assert!(report.effects.contains(&DisposeEffect::DirectoryRemoved));
    assert!(session.reread().unwrap().unwrap().members.is_empty());
}

/// A repeated waiver is a malformed request, refused before any effect.
#[test]
fn a_repeated_waiver_refuses_before_any_effect() {
    let (_store, mut session) = ready();
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let request = delete(&[HazardWaiver::Dirty, HazardWaiver::Dirty]);
    let failure = dispose(&request, &mut session, &mut ports).unwrap_err();
    assert!(matches!(
        failure.error,
        DisposeError::Refused(Refusal::InvalidRow { .. })
    ));
    assert!(ports.calls().is_empty());
}
