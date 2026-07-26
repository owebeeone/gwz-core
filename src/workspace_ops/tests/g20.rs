use std::fs;
use std::path::Path;

use crate::git::{GitBackend, GitStashTarget};
use crate::model::ErrorCode;
use crate::stash::{self, StashPushLifecycle, StashRestoreState};

use super::*;

mod m3_review;

fn stash_request(op: crate::StashOp, stash_id: &str) -> crate::StashRequest {
    crate::StashRequest {
        meta: request_meta(),
        op,
        stash_id: Some(stash_id.to_owned()),
        message: Some("test stash".to_owned()),
        include_untracked: None,
        include_ignored: None,
        expanded: None,
        preserve_index: None,
    }
}

fn selected_stash_request(
    op: crate::StashOp,
    stash_id: &str,
    member_ids: &[&str],
) -> crate::StashRequest {
    let mut request = stash_request(op, stash_id);
    request.meta.selection = Some(crate::Selection {
        all: Some(false),
        member_ids: member_ids.iter().map(|value| (*value).to_owned()).collect(),
        paths: Vec::new(),
        targets: Vec::new(),
        exclude_targets: Vec::new(),
    });
    request
}

fn assert_text_eq(path: impl AsRef<Path>, expected: &str) {
    let actual = fs::read_to_string(path)
        .unwrap()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    assert_eq!(actual, expected);
}

fn init_two_member_workspace(temp: &Path, backend: &crate::git::Git2Backend) {
    let app = RemoteFixture::new("stash-app-source");
    let lib = RemoteFixture::new("stash-lib-source");
    app.commit_and_push("tracked.txt", "base\n", "initial", backend);
    lib.commit_and_push("tracked.txt", "base\n", "initial", backend);
    handle_init_from_sources(
        backend,
        temp,
        crate::InitFromSourcesRequest {
            meta: request_meta(),
            workspace_root: temp.to_string_lossy().into_owned(),
            sources: vec![
                crate::SourceUrl {
                    url: app.remote_url().to_owned(),
                    path: Some("app".to_owned()),
                    remote_name: None,
                    branch: None,
                },
                crate::SourceUrl {
                    url: lib.remote_url().to_owned(),
                    path: Some("lib".to_owned()),
                    remote_name: None,
                    branch: None,
                },
            ],
            target: None,
            workspace_id: Some("ws_ops".to_owned()),
        },
        "op_init",
        &CollectingSink::default(),
    )
    .unwrap();
}

#[test]
fn stash_push_mixed_clean_dirty_records_noop_tuple() {
    let temp = TempDir::new("stash-mixed");
    let backend = crate::git::Git2Backend::new();
    init_two_member_workspace(temp.path(), &backend);
    fs::write(temp.path().join("app/tracked.txt"), "changed\n").unwrap();

    let response = handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_mixed"),
        "op_stash",
    )
    .unwrap();

    let bundle = response.bundles.unwrap().single().clone();
    assert_eq!(bundle.selected_members, vec!["mem_app", "mem_lib"]);
    let app = bundle
        .members
        .iter()
        .find(|member| member.member_id == "mem_app")
        .unwrap();
    assert_eq!(app.push_lifecycle, crate::StashPushLifecycle::Saved);
    assert_eq!(app.restore_state, crate::StashRestoreState::Pending);
    let lib = bundle
        .members
        .iter()
        .find(|member| member.member_id == "mem_lib")
        .unwrap();
    assert_eq!(lib.participation, crate::StashParticipation::Empty);
    assert_eq!(lib.push_lifecycle, crate::StashPushLifecycle::Empty);
    assert_eq!(lib.restore_state, crate::StashRestoreState::Noop);
}

#[test]
fn stash_push_untracked_and_ignored_options_control_native_save() {
    let temp = TempDir::new("stash-options");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stash-options-source");
    let member = temp.path().join("remote");

    fs::write(member.join("untracked.txt"), "new\n").unwrap();
    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_default"),
        "op_default",
    )
    .unwrap();
    assert!(member.join("untracked.txt").exists());

    let mut include_untracked = stash_request(crate::StashOp::Push, "stash_untracked");
    include_untracked.include_untracked = Some(true);
    handle_stash(&backend, temp.path(), include_untracked, "op_untracked").unwrap();
    assert!(!member.join("untracked.txt").exists());

    fs::write(member.join(".gitignore"), "ignored.txt\n").unwrap();
    let parent = git2::Oid::from_str(&backend.head(&member).unwrap().commit.unwrap()).unwrap();
    commit_file(&member, ".gitignore", "ignored.txt\n", "ignore", &[parent]).unwrap();
    fs::write(member.join("ignored.txt"), "ignored\n").unwrap();
    let mut include_ignored = stash_request(crate::StashOp::Push, "stash_ignored");
    include_ignored.include_ignored = Some(true);
    handle_stash(&backend, temp.path(), include_ignored, "op_ignored").unwrap();
    assert!(!member.join("ignored.txt").exists());
}

#[test]
fn stash_list_sorts_newest_first_and_returns_details() {
    let temp = TempDir::new("stash-list-op");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stash-list-op-source");
    let member = temp.path().join("remote");

    fs::write(member.join("README.md"), "one\n").unwrap();
    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_first"),
        "op_first",
    )
    .unwrap();
    fs::write(member.join("README.md"), "two\n").unwrap();
    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_second"),
        "op_second",
    )
    .unwrap();

    let listed = handle_stash(
        &backend,
        temp.path(),
        crate::StashRequest {
            meta: request_meta(),
            op: crate::StashOp::List,
            stash_id: None,
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: Some(true),
            preserve_index: None,
        },
        "op_list",
    )
    .unwrap();
    let bundles = listed.bundles.unwrap();
    assert_eq!(bundles[0].stash_id, "stash_second");
    assert_eq!(bundles[1].stash_id, "stash_first");
    assert_eq!(bundles[0].members.single().member_id, "mem_remote");
    assert_eq!(
        bundles[0].members.single().push_lifecycle,
        crate::StashPushLifecycle::Saved
    );
}

#[test]
fn stash_list_surfaces_orphan_native_stash_without_local_bundle() {
    let temp = TempDir::new("stash-orphan-native");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stash-orphan-source");
    let member = temp.path().join("remote");

    fs::write(member.join("README.md"), "orphaned\n").unwrap();
    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_orphan_native"),
        "op_push",
    )
    .unwrap();
    fs::remove_file(stash::bundle_path(temp.path(), "stash_orphan_native")).unwrap();

    let listed = handle_stash(
        &backend,
        temp.path(),
        crate::StashRequest {
            meta: request_meta(),
            op: crate::StashOp::List,
            stash_id: None,
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: Some(true),
            preserve_index: None,
        },
        "op_list",
    )
    .unwrap();
    let bundle = listed.bundles.unwrap().single().clone();

    assert_eq!(bundle.stash_id, "stash_orphan_native");
    assert!(bundle.members.is_empty());
    assert_eq!(bundle.warnings.single().code, "orphan_native_stash");
    assert_eq!(
        bundle.warnings.single().member_id.as_deref(),
        Some("mem_remote")
    );
}

#[test]
fn stash_apply_keeps_native_stash_and_marks_applied() {
    let temp = TempDir::new("stash-apply-op");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stash-apply-op-source");
    let member = temp.path().join("remote");
    fs::write(member.join("README.md"), "changed\n").unwrap();
    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_apply_op"),
        "op_push",
    )
    .unwrap();

    let applied = handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Apply, "stash_apply_op"),
        "op_apply",
    )
    .unwrap();

    assert_text_eq(member.join("README.md"), "changed\n");
    assert_eq!(
        applied
            .bundles
            .unwrap()
            .single()
            .members
            .single()
            .restore_state,
        crate::StashRestoreState::Applied
    );
    assert_eq!(backend.stash_list(&member).unwrap().len(), 1);
    assert!(stash::bundle_path(temp.path(), "stash_apply_op").exists());

    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Drop, "stash_apply_op"),
        "op_drop_after_apply",
    )
    .unwrap();
    assert!(backend.stash_list(&member).unwrap().is_empty());
    assert!(!stash::bundle_path(temp.path(), "stash_apply_op").exists());
}

#[test]
fn stash_pop_restores_and_deletes_complete_bundle() {
    let temp = TempDir::new("stash-pop-op");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stash-pop-op-source");
    let member = temp.path().join("remote");
    fs::write(member.join("README.md"), "changed\n").unwrap();
    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_pop_op"),
        "op_push",
    )
    .unwrap();

    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Pop, "stash_pop_op"),
        "op_pop",
    )
    .unwrap();

    assert_text_eq(member.join("README.md"), "changed\n");
    assert!(backend.stash_list(&member).unwrap().is_empty());
    assert!(!stash::bundle_path(temp.path(), "stash_pop_op").exists());
}

#[test]
fn stash_explicit_member_pop_leaves_remaining_pending() {
    let temp = TempDir::new("stash-partial-pop");
    let backend = crate::git::Git2Backend::new();
    init_two_member_workspace(temp.path(), &backend);
    fs::write(temp.path().join("app/tracked.txt"), "app\n").unwrap();
    fs::write(temp.path().join("lib/tracked.txt"), "lib\n").unwrap();
    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_partial"),
        "op_push",
    )
    .unwrap();

    handle_stash(
        &backend,
        temp.path(),
        selected_stash_request(crate::StashOp::Pop, "stash_partial", &["mem_app"]),
        "op_pop",
    )
    .unwrap();

    let bundle = stash::read_bundle(temp.path(), "stash_partial").unwrap();
    assert_eq!(
        bundle
            .members
            .iter()
            .find(|member| member.member_id == "mem_app")
            .unwrap()
            .restore_state,
        StashRestoreState::Popped
    );
    assert_eq!(
        bundle
            .members
            .iter()
            .find(|member| member.member_id == "mem_lib")
            .unwrap()
            .restore_state,
        StashRestoreState::Pending
    );
}

#[test]
fn stash_missing_native_payload_returns_incomplete() {
    let temp = TempDir::new("stash-missing-native");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stash-missing-source");
    let member = temp.path().join("remote");
    fs::write(member.join("README.md"), "changed\n").unwrap();
    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_missing_native"),
        "op_push",
    )
    .unwrap();
    let object_id = stash::read_bundle(temp.path(), "stash_missing_native")
        .unwrap()
        .members[0]
        .native_stash_object_id
        .clone()
        .unwrap();
    backend
        .stash_drop(&member, &GitStashTarget::object_id(object_id))
        .unwrap();

    let error = handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Pop, "stash_missing_native"),
        "op_pop",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::StashIncomplete);
}

#[test]
fn stash_dirty_destination_rejects_before_restore_mutation() {
    let temp = TempDir::new("stash-dirty-destination");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "stash-dirty-source");
    let member = temp.path().join("remote");
    fs::write(member.join("README.md"), "stashed\n").unwrap();
    handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Push, "stash_dirty_target"),
        "op_push",
    )
    .unwrap();
    fs::write(member.join("README.md"), "destination dirty\n").unwrap();

    let error = handle_stash(
        &backend,
        temp.path(),
        stash_request(crate::StashOp::Pop, "stash_dirty_target"),
        "op_pop",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::DirtyMember);
    assert_eq!(backend.stash_list(&member).unwrap().len(), 1);
    let bundle = stash::read_bundle(temp.path(), "stash_dirty_target").unwrap();
    assert_eq!(bundle.members[0].push_lifecycle, StashPushLifecycle::Saved);
    assert_eq!(bundle.members[0].restore_state, StashRestoreState::Pending);
}
