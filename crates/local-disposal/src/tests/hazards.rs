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

/// D9: a forced deletion reports the hazards the inspection raised and the
/// named waivers covered, not the names the operator gave. A named waiver
/// over a hazard that never arose carries nothing, so the caller can say it
/// went unused; an unforced clean deletion carries nothing at all.
#[test]
fn a_forced_deletion_carries_the_hazards_it_actually_waived() {
    let (_store, mut session) = ready();
    let mut ports = scripted(
        TargetEvidence {
            repositories: vec![RepositoryEvidence {
                work: dirty_work(),
                ..repository(RepoKey::Root, WS_A)
            }],
            ..clean_evidence()
        },
        HistoryAnswer::Preserved,
    );
    let report = dispose(&delete(&HazardWaiver::ALL), &mut session, &mut ports)
        .expect("every name was given");
    assert!(report.effects.contains(&DisposeEffect::DirectoryRemoved));
    assert!(!report.waived.is_empty());
    assert!(
        report
            .waived
            .iter()
            .all(|finding| finding.waiver == HazardWaiver::Dirty
                && finding.repository == RepoKey::Root),
        "{:?}",
        report.waived
    );
    assert_eq!(required_waivers(&report.waived), vec![HazardWaiver::Dirty]);

    let (_store, mut session) = ready();
    let mut ports = scripted(clean_evidence(), HistoryAnswer::Preserved);
    let report = dispose(&delete(&[HazardWaiver::Dirty]), &mut session, &mut ports)
        .expect("a clean lane deletes whatever was named");
    assert!(report.waived.is_empty(), "{:?}", report.waived);
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

/// R2, R0: a copy witness that covers every hazard, with the family still
/// holding it, makes the disposal need no waiver at all -- and the witness
/// is per repository, so it never speaks for a repository it does not name.
#[test]
fn a_copy_witness_over_every_hazard_disposes_with_no_waiver() {
    let mut baseline = gwz_work_detector::CopyBaseline::default();
    baseline.set(
        b"notes.txt".to_vec(),
        gwz_work_detector::Provenance::UnchangedCopy,
    );
    let evidence = TargetEvidence {
        repositories: vec![RepositoryEvidence {
            work: dirty_work(),
            ..repository(RepoKey::Root, WS_A)
        }],
        copy: Some(CopyWitness {
            repositories: vec![CopiedRepository {
                key: RepoKey::Root,
                baseline,
            }],
        }),
        ..clean_evidence()
    };

    let (_store, mut session) = ready();
    let mut ports = scripted(evidence.clone(), HistoryAnswer::Preserved);
    let report = dispose(&delete(&[]), &mut session, &mut ports)
        .expect("the family still holds every hazard, so nothing is a loss");
    assert!(report.effects.contains(&DisposeEffect::DirectoryRemoved));

    // The same witness under another key says nothing about this one.
    let elsewhere = TargetEvidence {
        copy: Some(CopyWitness {
            repositories: vec![CopiedRepository {
                key: RepoKey::Member {
                    id: "mem_app".to_owned(),
                },
                baseline: gwz_work_detector::CopyBaseline::default(),
            }],
        }),
        ..evidence
    };
    let (_store, mut session) = ready();
    let mut ports = scripted(elsewhere, HistoryAnswer::Preserved);
    let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
    assert!(
        matches!(&failure.error, DisposeError::Hazards(findings) if findings.len() == 1),
        "{:?}",
        failure.error
    );
    assert_no_removal(&ports);
}

/// R9: a refusal carries the categories it did *not* refuse over, so the
/// report can name every one of them. The unchanged copy is listed beside
/// the unpreserved history that refused.
#[test]
fn a_refusal_carries_the_unchanged_copies_it_found_beside_the_loss() {
    let mut baseline = gwz_work_detector::CopyBaseline::default();
    baseline.set(
        b"notes.txt".to_vec(),
        gwz_work_detector::Provenance::UnchangedCopy,
    );
    let (_store, mut session) = ready();
    let mut ports = scripted(
        TargetEvidence {
            repositories: vec![RepositoryEvidence {
                work: dirty_work(),
                ..repository(RepoKey::Root, WS_A)
            }],
            copy: Some(CopyWitness {
                repositories: vec![CopiedRepository {
                    key: RepoKey::Root,
                    baseline,
                }],
            }),
            ..clean_evidence()
        },
        HistoryAnswer::Unpreserved {
            detail: "lane/agent-17 is unique".to_owned(),
        },
    );
    let failure = dispose(&delete(&[]), &mut session, &mut ports).unwrap_err();
    let DisposeError::Hazards(findings) = &failure.error else {
        panic!("{:?}", failure.error);
    };
    assert_eq!(findings.len(), 2, "{findings:?}");
    let dirt = findings
        .iter()
        .find(|finding| finding.waiver == HazardWaiver::Dirty)
        .expect("the unchanged copy is reported");
    assert!(!dirt.refuses(), "but it is not what refused: {dirt:?}");
    assert!(
        findings
            .iter()
            .any(|finding| finding.waiver == HazardWaiver::UnpreservedHistory && finding.refuses())
    );
    assert_no_removal(&ports);
}

/// R9, R10: the categories are a partition of the findings, empty ones are
/// still reported, and the waiver list is exactly what refused -- once
/// each, in this vocabulary's order.
#[test]
fn findings_sort_into_every_category_and_name_only_the_waivers_that_refused() {
    let hazard = |provenance, path: &str| gwz_work_detector::Hazard {
        kind: gwz_work_detector::HazardKind::Work(WorkKind::Ignored),
        path: Some(path.as_bytes().to_vec()),
        detail: "ignored user data".to_owned(),
        provenance,
    };
    let findings = vec![
        HazardFinding {
            waiver: HazardWaiver::Dirty,
            repository: RepoKey::Root,
            hazards: vec![
                hazard(gwz_work_detector::Provenance::UnchangedCopy, "target/"),
                hazard(gwz_work_detector::Provenance::ChangedCopy, ".venv/"),
                hazard(gwz_work_detector::Provenance::Unique, "notes.txt"),
            ],
            detail: None,
        },
        HazardFinding {
            waiver: HazardWaiver::UnpreservedHistory,
            repository: RepoKey::Root,
            hazards: Vec::new(),
            detail: Some("1 protected root: Head 0123abcd".to_owned()),
        },
    ];

    let categories = categorise(&findings);
    assert_eq!(
        categories
            .iter()
            .map(|report| (report.category, report.count()))
            .collect::<Vec<_>>(),
        vec![
            (HazardCategory::Regenerable, 0),
            (HazardCategory::UnchangedCopy, 1),
            (HazardCategory::ChangedCopy, 1),
            (HazardCategory::Unique, 2),
        ],
        "every category is reported, Phase 2's included and empty"
    );
    assert_eq!(
        categories.iter().map(CategoryReport::count).sum::<usize>(),
        4,
        "a partition loses nothing and counts nothing twice"
    );
    // An object id has no path; a worktree entry has one.
    let unique = &categories[3];
    assert_eq!(
        unique.items[0].path.as_deref(),
        Some(b"notes.txt".as_slice())
    );
    assert_eq!(unique.items[1].path, None);
    assert!(unique.items[1].detail.contains("0123abcd"));

    assert_eq!(
        required_waivers(&findings),
        vec![HazardWaiver::Dirty, HazardWaiver::UnpreservedHistory]
    );
    // A finding that refuses nothing needs no waiver and names none.
    let copied_only = vec![HazardFinding {
        hazards: vec![hazard(
            gwz_work_detector::Provenance::UnchangedCopy,
            "target/",
        )],
        ..findings[0].clone()
    }];
    assert!(required_waivers(&copied_only).is_empty());
    assert_eq!(
        categorise(&copied_only)
            .iter()
            .map(CategoryReport::count)
            .sum::<usize>(),
        1,
        "and is still reported"
    );
}
