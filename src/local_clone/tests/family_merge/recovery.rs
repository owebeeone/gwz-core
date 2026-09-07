use super::*;
use crate::local_clone::family_merge::selected_keys;
use gwz_repo_contract::RepoKey;

#[test]
fn merge_default_and_all_include_root_but_explicit_member_stays_partial() {
    let fixture = super::super::fixture::clean_family_workspace("merge-selection-recovery");
    let manifest = crate::artifact::read_manifest(&fixture.root).unwrap();
    for selection in [
        None,
        Some(crate::Selection {
            targets: vec!["@all".into()],
            ..Default::default()
        }),
    ] {
        let keys = selected_keys(&manifest, selection.as_ref()).unwrap();
        assert!(keys.contains(&RepoKey::Root), "{keys:?}");
        assert!(keys.contains(&RepoKey::Member {
            id: "mem_app".into()
        }));
    }
    let keys = selected_keys(
        &manifest,
        Some(&crate::Selection {
            targets: vec!["mem_app".into()],
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(
        keys,
        vec![RepoKey::Member {
            id: "mem_app".into()
        }]
    );
}

#[test]
fn default_family_merge_preserves_root_and_member_history_for_disposal() {
    let fixture = super::super::fixture::clean_family_workspace("merge-dispose-recovery");
    let backend = Git2Backend::without_credential_helpers();
    let root_repo = git2::Repository::open(&fixture.root).unwrap();
    let mut config = root_repo.config().unwrap();
    config.set_str("user.name", "GWZ Fixture").unwrap();
    config
        .set_str("user.email", "fixture@example.invalid")
        .unwrap();
    handle_clone_local_workspace(
        &backend,
        &fixture.root,
        crate::CloneLocalWorkspaceRequest {
            meta: meta("clone"),
            name: "A".into(),
            mode: crate::LocalCloneMode::Verbatim,
            dest: None,
            branch: None,
            copy_source: None,
        },
        "clone",
        &NullSink,
    )
    .unwrap();
    let lane = fixture.sibling("A");
    let root_commit = commit(&lane, "root-work.txt", "root work\n", "root work", "HEAD");
    let member_commit = commit(
        &lane.join("app"),
        "member-work.txt",
        "member work\n",
        "member work",
        "HEAD",
    );
    let mut request = family_merge_request("A", None);
    request.meta.selection = None;
    let response = handle_merge_with_local_family(
        &backend,
        &fixture.root,
        request,
        "op_merge_recovery",
        &NullSink,
    )
    .unwrap();
    assert_eq!(
        response.state,
        crate::MergeOperationState::Completed,
        "{response:?}"
    );
    assert_eq!(repo(&response, "@root").source_commit, root_commit);
    assert_eq!(repo(&response, "mem_app").source_commit, member_commit);
    crate::workspace_ops::handle_local_family(
        &backend,
        &fixture.root,
        crate::LocalFamilyRequest {
            meta: meta("dispose"),
            op: crate::LocalFamilyOp::Dispose,
            name: Some("A".into()),
            keep: None,
            force_hazards: vec![],
        },
        "dispose",
        &NullSink,
    )
    .unwrap();
    assert!(!lane.exists());
}
