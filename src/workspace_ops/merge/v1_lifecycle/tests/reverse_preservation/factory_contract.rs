use super::*;
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    V1LifecycleRequest, V1ResponseDisposition,
};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::run_test;
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[test]
fn root_preservation_survives_factory_reopening() {
    let fixture = dirty_root_handoff_fixture_using(
        "factory-root-preservation",
        false,
        false,
        false,
        make_repository(),
    );
    fixture.base.seed_open();
    let backend = make_repository();
    let context = fixture.base.context();
    let mut runtime = ReverseRuntime::new(&backend, &context);
    let response = run_test(
        &CheckedV1Store::default(),
        &fixture.base.root.path,
        &fixture.base.model.merge_id,
        V1LifecycleRequest::Preserve,
        &mut runtime,
    )
    .unwrap();
    assert_eq!(
        response.disposition(),
        V1ResponseDisposition::Terminal(OperationState::Aborted)
    );
    assert!(response.current().record().pending_preservation.is_none());
    let stashes = make_repository()
        .preservation_stashes(&fixture.base.root.path, &fixture.base.model.merge_id)
        .unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].head_commit, fixture.protected);
    assert!(stashes[0].image.dirty.staged);
    assert!(stashes[0].image.dirty.unstaged);
    assert!(stashes[0].image.dirty.untracked);
}
