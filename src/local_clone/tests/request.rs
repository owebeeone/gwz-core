//! `local_clone::tests::request`: the dispatch slots refuse in order and
//! without effect on a real workspace.

use super::fixture::{TempDir, family_files_absent, meta, workspace};
use crate::git::Git2Backend;
use crate::model::ErrorCode;
use crate::operation::NullSink;
use crate::workspace_ops::{
    handle_clone_local_workspace, handle_local_family, handle_merge_with_local_family,
};

fn clone_request(name: &str, dry_run: Option<bool>) -> crate::CloneLocalWorkspaceRequest {
    let mut request = crate::CloneLocalWorkspaceRequest {
        meta: meta("req-clone-local"),
        name: name.to_owned(),
        dest: None,
        mode: crate::LocalCloneMode::Verbatim,
        branch: None,
        copy_source: None,
    };
    request.meta.dry_run = dry_run;
    request
}

fn family_request(op: crate::LocalFamilyOp, name: Option<&str>) -> crate::LocalFamilyRequest {
    crate::LocalFamilyRequest {
        meta: meta("req-local-family"),
        op,
        name: name.map(ToOwned::to_owned),
        keep: None,
        force_hazards: Vec::new(),
    }
}

fn merge_request(op: crate::MergeOp, selector: Option<&str>) -> crate::MergeRequest {
    crate::MergeRequest {
        meta: meta("req-merge-family"),
        op,
        source_ref: None,
        merge_id: None,
        mode: None,
        message: None,
        preserve: None,
        filesystem_strict: None,
        local_source_name: selector.map(ToOwned::to_owned),
    }
}

#[test]
fn clone_local_refuses_unsupported_after_shape_and_before_any_family_file() {
    let temp = TempDir::new("clone-local");
    let root = workspace(&temp);
    let backend = Git2Backend::without_credential_helpers();

    let error =
        handle_clone_local_workspace(&backend, &root, clone_request("A", None), "op-1", &NullSink)
            .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    assert!(error.message.contains("verbatim"), "{}", error.message);
    assert!(
        family_files_absent(&root),
        "no lock, index, pointer or marker"
    );

    let dry = handle_clone_local_workspace(
        &backend,
        &root,
        clone_request("A", Some(true)),
        "op-2",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(dry.code, ErrorCode::UnsupportedOperation);
    assert!(dry.message.contains("dry_run"), "{}", dry.message);
    assert!(family_files_absent(&root));

    let reserved = handle_clone_local_workspace(
        &backend,
        &root,
        clone_request("origin", None),
        "op-3",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(reserved.code, ErrorCode::InvalidRequest);
    assert!(family_files_absent(&root));

    // Tag 6 (`copy_source`, `--from`; operator ruling 2026-09-05) is
    // decoded and refused as unsupported until LCM3.2, before any family
    // file and before the copy it would otherwise redirect.
    let mut from = clone_request("A", None);
    from.copy_source = Some("B".to_owned());
    let from = handle_clone_local_workspace(&backend, &root, from, "op-3b", &NullSink).unwrap_err();
    assert_eq!(from.code, ErrorCode::UnsupportedOperation);
    assert!(from.message.contains("--from"), "{}", from.message);
    assert!(family_files_absent(&root));

    let outside = handle_clone_local_workspace(
        &backend,
        temp.path(),
        clone_request("A", None),
        "op-4",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(
        outside.code,
        ErrorCode::WorkspaceNotFound,
        "shape passes, then workspace discovery refuses outside a workspace"
    );
}

#[test]
fn local_family_ops_refuse_unsupported_without_writing() {
    let temp = TempDir::new("local-family");
    let root = workspace(&temp);
    let backend = Git2Backend::without_credential_helpers();

    for (op, name, expected) in [
        (crate::LocalFamilyOp::List, None, "local family list"),
        (crate::LocalFamilyOp::Dispose, Some("C"), "local dispose"),
        (crate::LocalFamilyOp::Disband, None, "local disband"),
    ] {
        let error = handle_local_family(&backend, &root, family_request(op, name), "op", &NullSink)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedOperation, "{op:?}");
        assert!(
            error.message.contains(expected),
            "{op:?}: {}",
            error.message
        );
        assert!(family_files_absent(&root), "{op:?}");
    }

    let mut keep_force = family_request(crate::LocalFamilyOp::Dispose, Some("C"));
    keep_force.keep = Some(true);
    keep_force.force_hazards = vec!["dirty".to_owned()];
    let error = handle_local_family(&backend, &root, keep_force, "op", &NullSink).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);

    let mut unknown = family_request(crate::LocalFamilyOp::Dispose, Some("C"));
    unknown.force_hazards = vec!["everything".to_owned()];
    let error = handle_local_family(&backend, &root, unknown, "op", &NullSink).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("unknown hazard"));

    let root_dispose = family_request(crate::LocalFamilyOp::Dispose, Some("root"));
    let error = handle_local_family(&backend, &root, root_dispose, "op", &NullSink).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(family_files_absent(&root));
}

#[test]
fn family_merge_refuses_in_order_and_plain_merges_reach_the_engine() {
    let temp = TempDir::new("family-merge");
    let root = workspace(&temp);
    let backend = Git2Backend::without_credential_helpers();

    // A family selector on a non-start op is malformed before anything else.
    let error = handle_merge_with_local_family(
        &backend,
        &root,
        merge_request(crate::MergeOp::Status, Some("A")),
        "op-1",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(family_files_absent(&root));

    // Family dry-run refuses before any lock file.
    let mut dry = merge_request(crate::MergeOp::Start, Some("A"));
    dry.meta.dry_run = Some(true);
    let error =
        handle_merge_with_local_family(&backend, &root, dry, "op-2", &NullSink).unwrap_err();
    assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    assert!(error.message.contains("dry_run"));
    assert!(family_files_absent(&root));

    // A well-formed family start stops at the family observation.
    let error = handle_merge_with_local_family(
        &backend,
        &root,
        merge_request(crate::MergeOp::Start, Some("A")),
        "op-3",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    assert!(
        error.message.contains("local family merge from `A`"),
        "{}",
        error.message
    );
    assert!(family_files_absent(&root));
    let repo = git2::Repository::open(&root).unwrap();
    assert!(
        repo.references_glob("refs/gwz/local-imports/*")
            .unwrap()
            .next()
            .is_none(),
        "no import ref was created"
    );

    // No selector: the existing engine answers exactly as before.
    let via_wrapper = handle_merge_with_local_family(
        &backend,
        &root,
        merge_request(crate::MergeOp::Status, None),
        "op-4",
        &NullSink,
    );
    let direct = crate::workspace_ops::handle_merge_with_events(
        &backend,
        &root,
        merge_request(crate::MergeOp::Status, None),
        "op-4",
        &NullSink,
    );
    assert_eq!(
        via_wrapper.map(|response| response.state),
        direct.map(|response| response.state)
    );

    // A selector that reaches the engine directly is refused by the engine.
    let bypass = crate::workspace_ops::handle_merge_with_events(
        &backend,
        &root,
        merge_request(crate::MergeOp::Start, Some("A")),
        "op-5",
        &NullSink,
    )
    .unwrap_err();
    assert_eq!(bypass.code, ErrorCode::MergeValidationFailed);
}

/// LCM1.0c-rem1 (Code P3-1; design §6.2 "an invalid family merge start
/// request must not fetch first"): a family start the engine would refuse is
/// refused by the wrapper with the engine's code BEFORE the family
/// observation, so no import ref can exist. A guard today (the wrapper stops
/// at the observation anyway); load-bearing once lane X wires the import.
#[test]
fn a_malformed_family_start_is_refused_with_the_engine_code_before_any_import() {
    let temp = TempDir::new("family-merge-shape");
    let root = workspace(&temp);
    let backend = Git2Backend::without_credential_helpers();

    let mut whitespace = merge_request(crate::MergeOp::Start, Some("A"));
    whitespace.message = Some(" \t\n".to_owned());
    let error =
        handle_merge_with_local_family(&backend, &root, whitespace, "op-1", &NullSink).unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::MergeValidationFailed,
        "{}",
        error.message
    );
    assert!(
        error.message.contains("must not be empty"),
        "{}",
        error.message
    );
    assert!(family_files_absent(&root));
    let repo = git2::Repository::open(&root).unwrap();
    assert!(
        repo.references_glob("refs/gwz/local-imports/*")
            .unwrap()
            .next()
            .is_none(),
        "no import ref was created"
    );
}
