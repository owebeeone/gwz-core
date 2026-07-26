use super::*;

#[derive(Clone, Copy, Debug)]
enum RootMutation {
    Lock,
    Marker,
    Boundary,
    Index,
    #[cfg(unix)]
    Executable,
    #[cfg(unix)]
    Symlink,
    #[cfg(unix)]
    BoundarySymlink,
}

fn interrupted_normalization(
    mutation: RootMutation,
) -> (
    TempDir,
    crate::git::Git2Backend,
    FaultingMergeStore,
    MergeOperationRecord,
) {
    let temp = TempDir::new(&format!("merge-preserve-root-retry-{mutation:?}"));
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(
        temp.path(),
        &backend,
        &format!("merge-preserve-root-retry-{mutation:?}"),
    );
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let store = FaultingMergeStore::new(FinalizationFault::AfterEvidencePersistence);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_root_retry_safety_start",
    )
    .unwrap_err();
    let mut record = store.discover_open(temp.path()).unwrap().unwrap();
    fs::write(temp.path().join("root-untracked.txt"), "preserve me\n").unwrap();
    record.state = OperationState::Preserving;
    record.publication.as_mut().unwrap().preservation_prefix = Some("baseline".to_owned());
    let candidate = record
        .publication
        .as_ref()
        .unwrap()
        .candidate
        .as_ref()
        .unwrap()
        .clone();
    store.write_open(temp.path(), &record).unwrap();

    let marker_relative = format!(
        "{}/{}.yaml",
        crate::artifact::MARKER_DIR,
        candidate.marker_id
    );
    crate::artifact::write_atomic(
        &temp.path().join(crate::artifact::LOCK_PATH),
        &candidate.lock_yaml,
    )
    .unwrap();
    crate::artifact::write_atomic(&temp.path().join(&marker_relative), &candidate.marker_yaml)
        .unwrap();
    backend
        .stage_paths(
            temp.path(),
            &[crate::artifact::LOCK_PATH, marker_relative.as_str()],
        )
        .unwrap();

    match mutation {
        RootMutation::Lock => {
            fs::write(
                temp.path().join(crate::artifact::LOCK_PATH),
                "user lock edit\n",
            )
            .unwrap();
        }
        RootMutation::Marker => {
            fs::write(temp.path().join(marker_relative), "user marker edit\n").unwrap();
        }
        RootMutation::Boundary => {
            fs::write(
                temp.path().join(".git/info/exclude"),
                "# user boundary edit\n/scratch/\n",
            )
            .unwrap();
        }
        RootMutation::Index => {
            fs::write(
                temp.path().join(crate::artifact::LOCK_PATH),
                "user staged lock\n",
            )
            .unwrap();
            backend
                .stage_paths(temp.path(), &[crate::artifact::LOCK_PATH])
                .unwrap();
            crate::artifact::write_atomic(
                &temp.path().join(crate::artifact::LOCK_PATH),
                &candidate.lock_yaml,
            )
            .unwrap();
        }
        #[cfg(unix)]
        RootMutation::Executable => {
            use std::os::unix::fs::PermissionsExt;

            let lock = temp.path().join(crate::artifact::LOCK_PATH);
            let mut permissions = fs::metadata(&lock).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&lock, permissions).unwrap();
            backend
                .stage_paths(temp.path(), &[crate::artifact::LOCK_PATH])
                .unwrap();
        }
        #[cfg(unix)]
        RootMutation::Symlink => {
            use std::os::unix::fs::symlink;

            let lock = temp.path().join(crate::artifact::LOCK_PATH);
            let target = temp.path().join("root-lock-target");
            fs::write(&target, &candidate.lock_yaml).unwrap();
            fs::remove_file(&lock).unwrap();
            symlink("../root-lock-target", &lock).unwrap();
            backend
                .stage_paths(temp.path(), &[crate::artifact::LOCK_PATH])
                .unwrap();
        }
        #[cfg(unix)]
        RootMutation::BoundarySymlink => {
            use std::os::unix::fs::symlink;

            let boundary = temp.path().join(".git/info/exclude");
            let target = temp.path().join(".git/info/exclude-target");
            fs::write(&target, fs::read(&boundary).unwrap()).unwrap();
            fs::remove_file(&boundary).unwrap();
            symlink("exclude-target", &boundary).unwrap();
        }
    }
    (temp, backend, store, record)
}

#[test]
fn preserve_retry_rejects_user_changes_to_root_normalization_state() {
    let mut mutations = vec![
        RootMutation::Lock,
        RootMutation::Marker,
        RootMutation::Boundary,
        RootMutation::Index,
    ];
    #[cfg(unix)]
    mutations.extend([
        RootMutation::Executable,
        RootMutation::Symlink,
        RootMutation::BoundarySymlink,
    ]);
    for mutation in mutations {
        let (temp, backend, store, record) = interrupted_normalization(mutation);
        let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
        let marker_path = crate::artifact::marker_path(
            temp.path(),
            &record
                .publication
                .as_ref()
                .unwrap()
                .candidate
                .as_ref()
                .unwrap()
                .marker_id,
        );
        let marker_before = fs::read(&marker_path).unwrap();
        let boundary_before = fs::read(temp.path().join(".git/info/exclude")).unwrap();
        let mut abort = recovery_request(crate::MergeOp::Abort, Some(record.merge_id));
        abort.preserve = Some(true);

        let error = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            abort,
            "op_root_retry_safety_abort",
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired, "{mutation:?}");
        assert_eq!(error.member_id.as_deref(), Some("@root"), "{mutation:?}");
        assert_eq!(
            fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
            lock_before,
            "{mutation:?}"
        );
        assert_eq!(
            fs::read(marker_path).unwrap(),
            marker_before,
            "{mutation:?}"
        );
        assert_eq!(
            fs::read(temp.path().join(".git/info/exclude")).unwrap(),
            boundary_before,
            "{mutation:?}"
        );
        #[cfg(unix)]
        match mutation {
            RootMutation::Executable => {
                use std::os::unix::fs::PermissionsExt;
                assert_ne!(
                    fs::metadata(temp.path().join(crate::artifact::LOCK_PATH))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o111,
                    0
                );
            }
            RootMutation::Symlink => assert!(
                fs::symlink_metadata(temp.path().join(crate::artifact::LOCK_PATH))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            ),
            RootMutation::BoundarySymlink => assert!(
                fs::symlink_metadata(temp.path().join(".git/info/exclude"))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            ),
            _ => {}
        }
    }
}
