use super::*;
use crate::stash::StashParticipation;

fn root_list_request() -> crate::StashRequest {
    crate::StashRequest {
        meta: crate::RequestMeta {
            selection: Some(crate::Selection {
                targets: vec!["@root".to_owned()],
                ..Default::default()
            }),
            ..request_meta()
        },
        op: crate::StashOp::List,
        stash_id: None,
        message: None,
        include_untracked: None,
        include_ignored: None,
        expanded: Some(true),
        preserve_index: None,
    }
}

fn save_root_native_stash(
    backend: &crate::git::Git2Backend,
    root: &Path,
    stash_id: &str,
) -> crate::git::GitStashPushResult {
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    fs::write(root.join("root.txt"), "root stash work\n").unwrap();
    backend
        .stash_push(
            root,
            &format!("gwz:{stash_id}: root preservation"),
            crate::git::GitStashPushOptions::default(),
        )
        .unwrap()
}

fn root_bundle(
    workspace_id: &str,
    stash_id: &str,
    native: Option<&crate::git::GitStashPushResult>,
) -> stash::StashBundle {
    let members = native
        .map(|native| {
            vec![stash::StashBundleMember {
                member_id: "@root".to_owned(),
                path: ".".to_owned(),
                participation: StashParticipation::Stashed,
                push_lifecycle: StashPushLifecycle::Saved,
                restore_state: StashRestoreState::Pending,
                branch_before: Some("main".to_owned()),
                head_before: None,
                full_stash_message: native.message.clone(),
                dirty_summary: stash::StashDirtySummary {
                    staged: false,
                    unstaged: true,
                    untracked: false,
                    ignored: false,
                },
                native_stash_object_id: Some(native.object_id.clone()),
                native_stash_display_ref: None,
                error: None,
            }]
        })
        .unwrap_or_else(|| {
            vec![stash::StashBundleMember {
                member_id: "mem_remote".to_owned(),
                path: "remote".to_owned(),
                participation: StashParticipation::Empty,
                push_lifecycle: StashPushLifecycle::Empty,
                restore_state: StashRestoreState::Noop,
                branch_before: Some("main".to_owned()),
                head_before: None,
                full_stash_message: format!("gwz:{stash_id}: member"),
                dirty_summary: stash::StashDirtySummary {
                    staged: false,
                    unstaged: false,
                    untracked: false,
                    ignored: false,
                },
                native_stash_object_id: None,
                native_stash_display_ref: None,
                error: None,
            }]
        });
    stash::StashBundle {
        schema: stash::STASH_BUNDLE_SCHEMA.to_owned(),
        workspace_id: workspace_id.to_owned(),
        stash_id: stash_id.to_owned(),
        created_at: "now".to_owned(),
        message_suffix: "merge preservation".to_owned(),
        include_untracked: true,
        include_ignored: false,
        selected_members: members
            .iter()
            .map(|member| member.member_id.clone())
            .collect(),
        members,
        warnings: Vec::new(),
        drift: Vec::new(),
    }
}

#[test]
fn explicit_root_only_list_reconciles_registered_root_bundle() {
    let temp = TempDir::new("stash-explicit-root-list");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stash-root-list-source");
    let native = save_root_native_stash(&backend, temp.path(), "stash_root_list");
    let manifest = crate::artifact::read_manifest(temp.path()).unwrap();
    stash::write_bundle(
        temp.path(),
        &root_bundle(&manifest.workspace.id, "stash_root_list", Some(&native)),
    )
    .unwrap();

    let response =
        handle_stash(&backend, temp.path(), root_list_request(), "op_root_list").unwrap();

    let bundle = response
        .bundles
        .unwrap()
        .into_iter()
        .find(|bundle| bundle.stash_id == "stash_root_list")
        .unwrap();
    assert_eq!(bundle.members.single().member_id, "@root");
    assert!(bundle.drift.is_empty());
}

#[test]
fn root_orphan_with_known_bundle_id_is_attached_to_existing_bundle() {
    let temp = TempDir::new("stash-root-partial-orphan");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stash-root-orphan-source");
    save_root_native_stash(&backend, temp.path(), "stash_partial");
    let manifest = crate::artifact::read_manifest(temp.path()).unwrap();
    stash::write_bundle(
        temp.path(),
        &root_bundle(&manifest.workspace.id, "stash_partial", None),
    )
    .unwrap();

    let response =
        handle_stash(&backend, temp.path(), root_list_request(), "op_root_orphan").unwrap();

    let bundles = response.bundles.unwrap();
    assert_eq!(
        bundles
            .iter()
            .filter(|bundle| bundle.stash_id == "stash_partial")
            .count(),
        1
    );
    let bundle = bundles
        .iter()
        .find(|bundle| bundle.stash_id == "stash_partial")
        .unwrap();
    assert!(bundle.warnings.iter().any(|warning| {
        warning.code == "orphan_native_stash" && warning.member_id.as_deref() == Some("@root")
    }));
}
