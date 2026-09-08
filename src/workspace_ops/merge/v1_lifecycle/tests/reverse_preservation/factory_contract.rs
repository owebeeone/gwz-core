use super::*;
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    V1LifecycleRequest, V1ResponseDisposition,
};
use crate::workspace_ops::merge::v1_lifecycle::reverse::ReverseRuntime;
use crate::workspace_ops::merge::v1_lifecycle::service::run_test;
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;

#[test]
fn activated_lease_creates_and_reopens_the_catalog_through_the_factory() {
    use crate::filesystem::{FileSystem, FsKind, make_filesystem};
    use crate::workspace_ops::merge::v1_lifecycle::checked::V1MutationLease;
    let fixture = dirty_root_handoff_fixture_using(
        "factory-catalog-activation",
        false,
        false,
        false,
        make_repository(),
    );
    let root = &fixture.base.root.path;
    let catalog = root.join(".gwz/catalog-final");
    assert!(make_filesystem().kind(&catalog).is_err());
    let lease = V1MutationLease::acquire_activated_for_test(root).unwrap();
    assert_eq!(make_filesystem().kind(&catalog).unwrap(), FsKind::Directory);
    let directory = make_filesystem().open_directory(&catalog).unwrap();
    let identity = make_filesystem()
        .persistent_directory_identity(&directory)
        .unwrap();
    drop(lease);
    let _reopened = V1MutationLease::acquire_activated_for_test(root).unwrap();
    let directory = make_filesystem().open_directory(&catalog).unwrap();
    assert_eq!(
        identity,
        make_filesystem()
            .persistent_directory_identity(&directory)
            .unwrap()
    );
    drop(_reopened);
    let _creation =
        V1MutationLease::acquire_for_merge_start(root, &fixture.base.model.workspace_id).unwrap();
    assert_eq!(
        make_filesystem().kind(&root.join(".gwz/merge")).unwrap(),
        FsKind::Directory
    );
    assert_eq!(
        make_filesystem()
            .kind(&root.join(crate::stash::STASH_BUNDLE_DIR))
            .unwrap(),
        FsKind::Directory
    );
}

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
