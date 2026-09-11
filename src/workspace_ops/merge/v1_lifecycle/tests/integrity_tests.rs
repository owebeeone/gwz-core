use super::*;
use crate::artifact::{CONF_INTEGRITY_MARKER_PATH, ConfIntegrityVerdict, inspect_conf_integrity};
use crate::workspace_ops::merge::v1_lifecycle::{
    authority::V1LifecycleRequest, reverse::ReverseRuntime, service, store::CheckedV1Store,
};

#[test]
fn marker_publication_before_index_staging_can_resume_or_abort() {
    for existing in [false, true] {
        for abort in [false, true] {
            let (root, backend, mut model) = fixture("integrity-partial-stage", true);
            let baseline = if existing {
                let baseline =
                    b"# stale committed marker\nschema: gwz.conf-integrity/v0\nfiles: {}\n"
                        .to_vec();
                fs::create_dir_all(root.path.join("gwz.conf/markers")).unwrap();
                fs::write(root.path.join(CONF_INTEGRITY_MARKER_PATH), &baseline).unwrap();
                backend
                    .stage_paths(&root.path, &[CONF_INTEGRITY_MARKER_PATH])
                    .unwrap();
                backend
                    .commit(&root.path, "old integrity marker", false)
                    .unwrap();
                model.baseline.root_head = backend.head(&root.path).unwrap().commit;
                Some(baseline)
            } else {
                None
            };
            seed_open(&root, &model);
            let context = context();
            let mut runtime =
                CrashBeforeRuntime::new(&backend, &context, PublicationPhysicalAction::StageIndex);
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service::run_test(
                    &CheckedV1Store::default(),
                    &root.path,
                    &model.merge_id,
                    V1LifecycleRequest::Continue,
                    &mut runtime
                )))
                .is_err()
            );
            let current = CheckedV1Store::default()
                .load_open(&root.path, &model.merge_id)
                .unwrap();
            let marker = &current
                .record()
                .publication
                .as_ref()
                .unwrap()
                .candidate
                .as_ref()
                .unwrap()
                .conf_integrity
                .as_ref()
                .unwrap()
                .yaml;
            // The marker write is durable, but staging has not happened yet.
            fs::write(root.path.join(CONF_INTEGRITY_MARKER_PATH), marker).unwrap();
            if abort {
                let mut runtime = ReverseRuntime::new(&backend, &context);
                let response = service::run_test(
                    &CheckedV1Store::default(),
                    &root.path,
                    &model.merge_id,
                    V1LifecycleRequest::Abort,
                    &mut runtime,
                )
                .unwrap();
                assert_eq!(response.current().record().state, OperationState::Aborted);
                assert_eq!(
                    fs::read(root.path.join(CONF_INTEGRITY_MARKER_PATH)).ok(),
                    baseline
                );
                assert_eq!(
                    backend.head(&root.path).unwrap().commit,
                    model.baseline.root_head
                );
            } else {
                let mut runtime = FinalizationRuntime::new(&backend, &context);
                let response = service::run_test(
                    &CheckedV1Store::default(),
                    &root.path,
                    &model.merge_id,
                    V1LifecycleRequest::Continue,
                    &mut runtime,
                )
                .unwrap();
                assert_eq!(response.current().record().state, OperationState::Completed);
                assert_eq!(
                    inspect_conf_integrity(&root.path),
                    ConfIntegrityVerdict::Verified
                );
            }
            assert!(
                !backend
                    .status(&root.path)
                    .unwrap()
                    .files
                    .iter()
                    .any(|file| file.path == CONF_INTEGRITY_MARKER_PATH)
            );
        }
    }
}

#[test]
fn unexpected_integrity_bytes_are_preserved_and_refuse_resumption() {
    let (root, backend, current) = interrupted_before(PublicationPhysicalAction::StageIndex);
    fs::write(root.path.join(CONF_INTEGRITY_MARKER_PATH), "user work\n").unwrap();
    let context = context();
    let mut runtime = FinalizationRuntime::new(&backend, &context);
    let response = service::run_test(
        &CheckedV1Store::default(),
        &root.path,
        &current.record().merge_id,
        V1LifecycleRequest::Continue,
        &mut runtime,
    )
    .unwrap();
    assert_eq!(
        response.current().record().state,
        OperationState::RecoveryRequired
    );
    assert_eq!(
        fs::read_to_string(root.path.join(CONF_INTEGRITY_MARKER_PATH)).unwrap(),
        "user work\n"
    );
}
