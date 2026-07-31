use super::*;

#[derive(Clone, Copy, Debug)]
enum ArchiveShape {
    CompletedCandidate,
    CompletedNoPublication,
    CompletedEvidenceGap,
    AbortedPreAcceptance,
    AbortedCompleteCandidate,
    AbortedPartialCandidate,
}

fn archived_shape(name: &str, shape: ArchiveShape) -> (TempDir, String) {
    let live = TempDir::new(&format!("archive-v0-live-{name}"));
    let backend = crate::git::Git2Backend::new();
    let _fixture =
        init_one_member_workspace(live.path(), &backend, &format!("archive-v0-source-{name}"));

    let response = match shape {
        ArchiveShape::CompletedNoPublication | ArchiveShape::CompletedEvidenceGap => {
            backend
                .branch_create(&live.path().join("remote"), "feature/source", "HEAD")
                .unwrap();
            handle_merge(
                &backend,
                live.path(),
                request(false),
                format!("op_archive_v0_{name}"),
            )
            .unwrap()
        }
        ArchiveShape::CompletedCandidate => {
            feature_commit(
                &backend,
                &live.path().join("remote"),
                "README.md",
                "source\n",
            );
            handle_merge(
                &backend,
                live.path(),
                request(false),
                format!("op_archive_v0_{name}"),
            )
            .unwrap()
        }
        ArchiveShape::AbortedPreAcceptance
        | ArchiveShape::AbortedCompleteCandidate
        | ArchiveShape::AbortedPartialCandidate => {
            feature_commit(
                &backend,
                &live.path().join("remote"),
                "README.md",
                "source\n",
            );
            let fault = match shape {
                ArchiveShape::AbortedPreAcceptance => FinalizationFault::AfterEnteringFinalizing,
                ArchiveShape::AbortedCompleteCandidate => {
                    FinalizationFault::AfterEvidencePersistence
                }
                ArchiveShape::AbortedPartialCandidate => {
                    FinalizationFault::AfterCandidatePersistence
                }
                _ => unreachable!(),
            };
            let store = FaultingMergeStore::new(fault);
            invoke_with_store(
                &backend,
                &store,
                live.path(),
                request(false),
                &format!("op_archive_v0_fault_{name}"),
            )
            .unwrap_err();
            let merge_id = store.discover_open(live.path()).unwrap().unwrap().merge_id;
            invoke_with_store(
                &backend,
                &store,
                live.path(),
                recovery_request(crate::MergeOp::Abort, Some(merge_id)),
                &format!("op_archive_v0_abort_{name}"),
            )
            .unwrap()
        }
    };
    let merge_id = response.merge_id.unwrap();
    let source = archive_path(live.path(), &merge_id);
    let archive_only = TempDir::new(&format!("archive-v0-only-{name}"));
    let destination = archive_path(archive_only.path(), &merge_id);
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::copy(source, destination).unwrap();
    if matches!(shape, ArchiveShape::CompletedEvidenceGap) {
        mutate_archive(archive_only.path(), &merge_id, |raw| {
            remove_key(&mut raw["baseline"], "lock_yaml")
        });
    }
    (archive_only, merge_id)
}

fn archive_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(format!(".gwz/merge/done/{merge_id}.yaml"))
}

fn archived_status(root: &Path, merge_id: &str) -> ModelResult<crate::MergeResponse> {
    let mut request = recovery_request(crate::MergeOp::Status, Some(merge_id.to_owned()));
    request.meta.workspace = Some(crate::WorkspaceRef {
        root: Some(root.to_string_lossy().into_owned()),
        workspace_id: None,
    });
    handle_merge(
        &crate::git::Git2Backend::new(),
        root,
        request,
        "op_archive_v0_status",
    )
}

fn mutate_archive(root: &Path, merge_id: &str, mutate: impl FnOnce(&mut serde_yaml::Value)) {
    let path = archive_path(root, merge_id);
    let mut raw: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
    mutate(&mut raw);
    fs::write(path, serde_yaml::to_string(&raw).unwrap()).unwrap();
}

fn remove_key(value: &mut serde_yaml::Value, key: &str) {
    value
        .as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::String(key.to_owned()));
}

fn assert_same_historical_status(
    before: &crate::MergeResponse,
    after: &crate::MergeResponse,
    name: &str,
) {
    assert_eq!(after.state, before.state, "{name}");
    assert_eq!(after.open, before.open, "{name}");
    assert_eq!(
        after.participant_counts, before.participant_counts,
        "{name}"
    );
    assert_eq!(after.repos, before.repos, "{name}");
    assert_eq!(after.preservation, before.preservation, "{name}");
    assert_eq!(after.publication_step, before.publication_step, "{name}");
    assert!(
        after.repos.iter().all(|repo| repo.live_commit.is_none()),
        "{name}"
    );
}

fn assert_archive_shape(root: &Path, merge_id: &str, shape: ArchiveShape) {
    let record = FileMergeStore.load_archived(root, merge_id).unwrap();
    match shape {
        ArchiveShape::CompletedCandidate => {
            assert_eq!(record.state, OperationState::Completed);
            let publication = record.publication.unwrap();
            assert!(publication.candidate.is_some());
            assert!(publication.composition_commit.is_some());
        }
        ArchiveShape::CompletedNoPublication => {
            assert_eq!(record.state, OperationState::Completed);
            assert!(record.publication.unwrap().candidate.is_none());
        }
        ArchiveShape::CompletedEvidenceGap => {
            assert_eq!(record.state, OperationState::Completed);
            assert!(record.baseline.lock_yaml.is_none());
        }
        ArchiveShape::AbortedPreAcceptance => {
            assert_eq!(record.state, OperationState::Aborted);
            assert!(
                record
                    .publication
                    .is_none_or(|publication| publication.candidate.is_none())
            );
        }
        ArchiveShape::AbortedCompleteCandidate => {
            assert_eq!(record.state, OperationState::Aborted);
            let publication = record.publication.unwrap();
            assert!(publication.candidate.is_some());
            assert!(publication.composition_commit.is_some());
            assert!(publication.evidence_rolled_back);
        }
        ArchiveShape::AbortedPartialCandidate => {
            assert_eq!(record.state, OperationState::Aborted);
            let publication = record.publication.unwrap();
            assert!(publication.candidate.is_some());
            assert!(publication.composition_commit.is_none());
        }
    }
}

#[test]
fn archived_v0_b_through_g_status_uses_only_archive_bytes() {
    let cases = [
        ("av0_b", ArchiveShape::CompletedCandidate),
        ("av0_c", ArchiveShape::CompletedNoPublication),
        ("av0_d", ArchiveShape::CompletedEvidenceGap),
        ("av0_e", ArchiveShape::AbortedPreAcceptance),
        ("av0_f", ArchiveShape::AbortedCompleteCandidate),
        ("av0_g", ArchiveShape::AbortedPartialCandidate),
    ];
    for (name, shape) in cases {
        let (archive_only, merge_id) = archived_shape(name, shape);
        assert_archive_shape(archive_only.path(), &merge_id, shape);
        assert!(!archive_only.path().join(".git").exists(), "{name}");
        assert!(!archive_only.path().join("gwz.conf").exists(), "{name}");
        assert!(!archive_only.path().join("remote").exists(), "{name}");

        let absent = archived_status(archive_only.path(), &merge_id).unwrap();
        assert!(!absent.open, "{name}");

        fs::create_dir_all(archive_only.path().join(".git")).unwrap();
        fs::write(archive_only.path().join(".git/HEAD"), "changed\n").unwrap();
        fs::create_dir_all(archive_only.path().join("gwz.conf")).unwrap();
        fs::write(
            archive_only
                .path()
                .join(crate::workspace::WORKSPACE_MANIFEST),
            "changed: true\n",
        )
        .unwrap();
        fs::write(
            archive_only.path().join(crate::artifact::LOCK_PATH),
            "changed: true\n",
        )
        .unwrap();
        fs::create_dir_all(archive_only.path().join("remote/.git")).unwrap();
        fs::write(archive_only.path().join("remote/README.md"), "changed\n").unwrap();

        let changed = archived_status(archive_only.path(), &merge_id).unwrap();
        assert_same_historical_status(&absent, &changed, name);
    }
}

#[test]
fn archived_v0_optional_evidence_gaps_remain_readable_and_untouched() {
    let cases = [
        ("exact_lock_bytes", ArchiveShape::CompletedNoPublication),
        (
            "complete_member_audit",
            ArchiveShape::CompletedNoPublication,
        ),
        ("accepted_root_input", ArchiveShape::CompletedNoPublication),
        ("publication_evidence", ArchiveShape::CompletedCandidate),
    ];
    for (gap, shape) in cases {
        let (archive_only, merge_id) = archived_shape(gap, shape);
        mutate_archive(archive_only.path(), &merge_id, |raw| match gap {
            "exact_lock_bytes" => remove_key(&mut raw["baseline"], "lock_yaml"),
            "complete_member_audit" => {
                remove_key(&mut raw["participants"]["mem_remote"], "resulting_commit")
            }
            "accepted_root_input" => {
                remove_key(&mut raw["baseline"], "root_head");
                remove_key(&mut raw["baseline"], "root_branch");
            }
            "publication_evidence" => {
                remove_key(&mut raw["publication"], "composition_commit");
                remove_key(&mut raw["publication"], "composition_tree");
                remove_key(&mut raw["publication"], "candidate_hashes");
            }
            _ => unreachable!(),
        });
        let path = archive_path(archive_only.path(), &merge_id);
        let before = fs::read(&path).unwrap();

        let record = FileMergeStore
            .load_archived(archive_only.path(), &merge_id)
            .unwrap();
        match gap {
            "exact_lock_bytes" => assert!(record.baseline.lock_yaml.is_none()),
            "complete_member_audit" => {
                assert!(record.participants["mem_remote"].resulting_commit.is_none())
            }
            "accepted_root_input" => {
                assert!(record.baseline.root_head.is_none());
                assert!(record.baseline.root_branch.is_none());
            }
            "publication_evidence" => {
                let publication = record.publication.unwrap();
                assert!(publication.composition_commit.is_none());
                assert!(publication.composition_tree.is_none());
                assert!(publication.candidate_hashes.is_empty());
            }
            _ => unreachable!(),
        }

        let status = archived_status(archive_only.path(), &merge_id).unwrap();

        assert!(!status.open, "{gap}");
        assert_eq!(fs::read(path).unwrap(), before, "{gap}");
    }
}

#[test]
fn archived_v0_unknown_fields_and_raw_bytes_survive_status_and_retention() {
    let (archive_only, merge_id) =
        archived_shape("unknown_retention", ArchiveShape::CompletedCandidate);
    mutate_archive(archive_only.path(), &merge_id, |raw| {
        raw["future_record"] = serde_yaml::Value::String("record-value".to_owned());
        raw["baseline"]["future_baseline"] = serde_yaml::Value::String("baseline-value".to_owned());
        raw["participants"]["mem_remote"]["future_participant"] =
            serde_yaml::Value::String("participant-value".to_owned());
        raw["publication"]["future_publication"] =
            serde_yaml::Value::String("publication-value".to_owned());
        raw["publication"]["candidate"]["future_candidate"] =
            serde_yaml::Value::String("candidate-value".to_owned());
    });
    let path = archive_path(archive_only.path(), &merge_id);
    let before = fs::read(&path).unwrap();

    archived_status(archive_only.path(), &merge_id).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    FileMergeStore.gc(archive_only.path(), None).unwrap();
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn archived_v0_missing_optional_evidence_is_not_an_unreadable_contradiction() {
    let (missing, missing_id) =
        archived_shape("missing_not_corrupt", ArchiveShape::CompletedNoPublication);
    mutate_archive(missing.path(), &missing_id, |raw| {
        remove_key(&mut raw["baseline"], "lock_yaml");
        remove_key(&mut raw["baseline"], "root_branch");
    });
    assert!(archived_status(missing.path(), &missing_id).is_ok());

    for contradiction in ["identity", "schema", "field_type"] {
        let (archive_only, merge_id) =
            archived_shape(contradiction, ArchiveShape::CompletedNoPublication);
        mutate_archive(archive_only.path(), &merge_id, |raw| match contradiction {
            "identity" => raw["merge_id"] = serde_yaml::Value::String("merge_other".to_owned()),
            "schema" => {
                raw["schema"] = serde_yaml::Value::String("gwz.merge-operation/bad".to_owned())
            }
            "field_type" => {
                raw["baseline"]["lock_sha256"] =
                    serde_yaml::Value::Sequence(vec![serde_yaml::Value::Bool(true)])
            }
            _ => unreachable!(),
        });

        let error = archived_status(archive_only.path(), &merge_id).unwrap_err();
        assert_eq!(
            error.code,
            ErrorCode::MergeRecordUnreadable,
            "{contradiction}"
        );
        assert!(archive_path(archive_only.path(), &merge_id).is_file());
    }
}
