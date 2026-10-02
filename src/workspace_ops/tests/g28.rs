//! A `Partial`, `Failed` or `Rejected` result lists each failed or refused
//! member's error in its top-level `errors` (TR2.3, OD7 of
//! dev-docs/GwzTransportReleasePlan.md, for `Partial`; TR2.19 widened it to
//! `Failed` and `Rejected`. The contract is gwz-cli docs/MachineOutput.md,
//! "Failed, rejected and partial results"). Before, these results kept their
//! failures on the member entries alone, so a caller that read `errors` to
//! decide success saw none.

use std::path::Path;

use crate::MemberStatus::{Failed, Noop, Rejected, Skipped};
use crate::git::{Git2Backend, GitBackend};

use super::*;

fn statuses(envelope: &crate::ResponseEnvelope) -> Vec<(&str, crate::MemberStatus)> {
    envelope
        .members
        .iter()
        .map(|row| (row.member_id.as_str(), row.status))
        .collect()
}

/// Unchanged copies of the errors on these members' entries, in this order.
fn row_errors(envelope: &crate::ResponseEnvelope, member_ids: &[&str]) -> Vec<crate::GwzError> {
    let error = |id: &&str| {
        let row = envelope.members.iter().find(|row| row.member_id == *id);
        row.and_then(|row| row.error.clone())
            .unwrap_or_else(|| panic!("no error on {id}"))
    };
    member_ids.iter().map(error).collect()
}

/// A clone at `root/<path>` of a fresh one-commit remote; returns that commit.
fn clone_member(backend: &Git2Backend, root: &Path, fixture: &RemoteFixture, path: &str) -> String {
    let head = fixture.commit_and_push("README.md", "one", "initial", backend);
    backend
        .clone_repo(fixture.remote_url(), &root.join(path))
        .unwrap();
    head
}

/// A workspace whose members `app` and `lib` are clones of remotes of their
/// own; the root has no remote.
fn two_member_workspace(backend: &Git2Backend, temp: &TempDir) -> [RemoteFixture; 2] {
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let [app, lib] = ["app", "lib"].map(|name| RemoteFixture::new(&format!("copies-{name}")));
    let app_head = clone_member(backend, temp.path(), &app, "repos/app");
    let lib_head = clone_member(backend, temp.path(), &lib, "repos/lib");
    write_pull_fixture(
        temp.path(),
        vec![
            ("mem_app", "repos/app", app.remote_url(), &app_head),
            ("mem_lib", "repos/lib", lib.remote_url(), &lib_head),
        ],
    );
    [app, lib]
}

/// `good` answers; `broken`'s remote is gone, so its fetch fails; `gone` was
/// never materialized, so it is refused before the network.
fn fetch_workspace(backend: &Git2Backend, temp: &TempDir) -> [RemoteFixture; 3] {
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let [good, broken, gone] =
        ["good", "broken", "gone"].map(|name| RemoteFixture::new(&format!("partial-fetch-{name}")));
    let good_head = clone_member(backend, temp.path(), &good, "repos/good");
    let broken_head = clone_member(backend, temp.path(), &broken, "repos/broken");
    let gone_head = gone.commit_and_push("README.md", "one", "initial", backend);
    let absent = temp.path().join("absent.git");
    git2::Repository::open(temp.path().join("repos/broken"))
        .unwrap()
        .remote_set_url("origin", absent.to_str().unwrap())
        .unwrap();
    write_pull_fixture(
        temp.path(),
        vec![
            ("mem_good", "repos/good", good.remote_url(), &good_head),
            (
                "mem_broken",
                "repos/broken",
                broken.remote_url(),
                &broken_head,
            ),
            ("mem_gone", "repos/gone", gone.remote_url(), &gone_head),
        ],
    );
    [good, broken, gone]
}

fn fetch(
    backend: &Git2Backend,
    temp: &TempDir,
    meta: crate::RequestMeta,
) -> crate::ResponseEnvelope {
    let request = crate::FetchRequest { meta };
    handle_fetch(backend, temp.path(), request, "op_fetch")
        .unwrap()
        .response
}

#[test]
fn a_partial_fetch_lists_each_failed_and_refused_members_error_in_errors() {
    let temp = TempDir::new("partial-fetch-errors");
    let backend = Git2Backend::without_credential_helpers();
    let _remotes = fetch_workspace(&backend, &temp);

    let live = fetch(&backend, &temp, request_meta_with_workspace());

    assert_eq!(live.meta.aggregate_status, crate::AggregateStatus::Partial);
    let rows = [
        ("@root", Noop),
        ("mem_good", Noop),
        ("mem_broken", Failed),
        ("mem_gone", Rejected),
    ];
    assert_eq!(statuses(&live), rows);
    assert_eq!(live.errors, row_errors(&live, &["mem_broken", "mem_gone"]));

    // A dry run contacts nothing, so only the refusal fails; like the live
    // run it is `Partial`, and it lists that refusal.
    let dry_run = crate::RequestMeta {
        dry_run: Some(true),
        ..request_meta_with_workspace()
    };
    let dry = fetch(&backend, &temp, dry_run);
    assert_eq!(dry.meta.aggregate_status, crate::AggregateStatus::Partial);
    assert_eq!(dry.errors, row_errors(&dry, &["mem_gone"]));
}

/// A fetch refused whole, before any network, is `Rejected` and lists every
/// refusal.
#[test]
fn a_rejected_fetch_lists_each_refused_members_error_in_errors() {
    let temp = TempDir::new("rejected-fetch-errors");
    let backend = Git2Backend::without_credential_helpers();
    let _remotes = fetch_workspace(&backend, &temp);
    let policy = crate::OperationPolicy {
        remote: Some("nope".to_owned()),
        ..Default::default()
    };
    let meta = crate::RequestMeta {
        policy: Some(policy),
        ..request_meta_with_workspace()
    };

    let refused = fetch(&backend, &temp, meta);

    assert_eq!(
        refused.meta.aggregate_status,
        crate::AggregateStatus::Rejected
    );
    let ids = ["@root", "mem_good", "mem_broken", "mem_gone"];
    assert_eq!(statuses(&refused), ids.map(|id| (id, Rejected)));
    assert_eq!(refused.errors, row_errors(&refused, &ids));
}

/// While both remotes answer, the fetch succeeds and `errors` stays empty;
/// once both are gone nothing answers (the root has no remote), so the fetch
/// is `Failed` and lists each failure.
#[test]
fn a_failed_fetch_lists_each_failed_members_error_in_errors() {
    let temp = TempDir::new("failed-fetch-errors");
    let backend = Git2Backend::without_credential_helpers();
    let _remotes = two_member_workspace(&backend, &temp);

    let answered = fetch(&backend, &temp, request_meta_with_workspace());

    assert_eq!(answered.meta.aggregate_status, crate::AggregateStatus::Noop);
    assert_eq!(answered.errors, []);

    let absent = temp.path().join("absent.git");
    for path in ["repos/app", "repos/lib"] {
        git2::Repository::open(temp.path().join(path))
            .unwrap()
            .remote_set_url("origin", absent.to_str().unwrap())
            .unwrap();
    }

    let failed = fetch(&backend, &temp, request_meta_with_workspace());

    assert_eq!(failed.meta.aggregate_status, crate::AggregateStatus::Failed);
    let rows = [("@root", Noop), ("mem_app", Failed), ("mem_lib", Failed)];
    assert_eq!(statuses(&failed), rows);
    assert_eq!(failed.errors, row_errors(&failed, &["mem_app", "mem_lib"]));
}

/// `app`'s remote moved on, so its push is refused at push time; `lib` is one
/// commit ahead of its remote and publishes; the root is then not attempted.
#[test]
fn a_partial_push_lists_the_failed_member_and_the_refused_root_in_errors() {
    let temp = TempDir::new("partial-push-errors");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let root_remote = temp.path().join("root.git");
    init_bare_main(&root_remote);
    let root_url = root_remote.to_str().unwrap();
    backend.add_remote(temp.path(), "origin", root_url).unwrap();
    set_identity(temp.path());
    commit_file(temp.path(), "root.txt", "before", "before", &[]).unwrap();
    let main = "refs/heads/main:refs/heads/main";
    backend.push(temp.path(), "origin", main).unwrap();

    let [app, lib] = ["app", "lib"].map(|name| RemoteFixture::new(&format!("partial-push-{name}")));
    let mut locked = Vec::new();
    for (fixture, path) in [(&app, "repos/app"), (&lib, "repos/lib")] {
        let base =
            git2::Oid::from_str(&clone_member(&backend, temp.path(), fixture, path)).unwrap();
        let member = temp.path().join(path);
        locked.push(commit_file(&member, "README.md", "local", "local", &[base]).unwrap());
    }
    app.commit_and_push("README.md", "remote", "remote", &backend);
    write_pull_fixture(
        temp.path(),
        vec![
            ("mem_app", "repos/app", app.remote_url(), &locked[0]),
            ("mem_lib", "repos/lib", lib.remote_url(), &locked[1]),
        ],
    );
    // The fixture locks every member as `src_app`; the root proves its lock
    // against the manifest, which names `lib`'s source `src_lib`.
    let mut lock = crate::artifact::read_lock(temp.path()).unwrap();
    lock.members.get_mut("mem_lib").unwrap().source_id = Some("src_lib".to_owned());
    crate::artifact::write_lock(temp.path(), &lock).unwrap();
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    backend.commit(temp.path(), "publish lock", false).unwrap();
    let request = crate::PushRequest {
        meta: request_meta_with_workspace(),
        remote: None,
        refspec: None,
        remote_check: None,
    };

    let response = handle_push(&backend, temp.path(), request, "op_push")
        .unwrap()
        .response;

    assert_eq!(
        response.meta.aggregate_status,
        crate::AggregateStatus::Partial
    );
    let rows = [
        ("mem_app", Failed),
        ("mem_lib", crate::MemberStatus::Ok),
        ("@root", Rejected),
    ];
    assert_eq!(statuses(&response), rows);
    assert_eq!(
        response.errors,
        row_errors(&response, &["mem_app", "@root"])
    );
}

/// A push to a remote named `nope`, which no repository has, with the
/// manifest and lock committed in the root.
fn push_to_nope(
    backend: &Git2Backend,
    temp: &TempDir,
    meta: crate::RequestMeta,
) -> crate::ResponseEnvelope {
    set_identity(temp.path());
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    backend.commit(temp.path(), "publish lock", false).unwrap();
    let request = crate::PushRequest {
        meta,
        remote: Some("nope".to_owned()),
        refspec: None,
        remote_check: None,
    };
    handle_push(backend, temp.path(), request, "op_push")
        .unwrap()
        .response
}

/// Every repository is refused before any remote is contacted: the push is
/// `Rejected` and lists each refusal, the root's last as its row is.
#[test]
fn a_rejected_push_lists_each_refused_members_error_in_errors() {
    let temp = TempDir::new("rejected-push-errors");
    let backend = Git2Backend::without_credential_helpers();
    let _remotes = two_member_workspace(&backend, &temp);

    let refused = push_to_nope(&backend, &temp, request_meta_with_workspace());

    assert_eq!(
        refused.meta.aggregate_status,
        crate::AggregateStatus::Rejected
    );
    let ids = ["mem_app", "mem_lib", "@root"];
    assert_eq!(statuses(&refused), ids.map(|id| (id, Rejected)));
    assert_eq!(refused.errors, row_errors(&refused, &ids));
}

/// The rule is the non-success aggregates' alone: a push that skips every
/// member it cannot push succeeds as `Noop`, and the skipped rows keep their
/// errors on their entries only, as before.
#[test]
fn a_successful_push_with_skipped_members_keeps_errors_empty() {
    let temp = TempDir::new("skipped-push-errors");
    let backend = Git2Backend::without_credential_helpers();
    let _remotes = two_member_workspace(&backend, &temp);
    let meta = crate::RequestMeta {
        selection: Some(crate::Selection {
            exclude_targets: vec!["@root".to_owned()],
            ..Default::default()
        }),
        policy: Some(crate::OperationPolicy {
            unsupported_member: Some(crate::UnsupportedMemberBehavior::Skip),
            ..Default::default()
        }),
        ..request_meta_with_workspace()
    };

    let skipped = push_to_nope(&backend, &temp, meta);

    assert_eq!(skipped.meta.aggregate_status, crate::AggregateStatus::Noop);
    let ids = ["mem_app", "mem_lib"];
    assert_eq!(statuses(&skipped), ids.map(|id| (id, Skipped)));
    assert_eq!(row_errors(&skipped, &ids).len(), 2);
    assert_eq!(skipped.errors, []);
}

/// `gwz status` builds its envelope with the same builder: a member whose
/// source kind status cannot read is refused, and the `Rejected` result lists
/// that refusal.
#[test]
fn a_rejected_status_lists_each_refused_members_error_in_errors() {
    let temp = TempDir::new("rejected-status-errors");
    let backend = Git2Backend::without_credential_helpers();
    let _remotes = two_member_workspace(&backend, &temp);
    let mut manifest = crate::artifact::read_manifest(temp.path()).unwrap();
    manifest.members[1].source_kind = crate::artifact::ArtifactSourceKind::Archive;
    crate::artifact::write_manifest(temp.path(), &manifest).unwrap();
    let request = crate::StatusRequest {
        meta: request_meta_with_workspace(),
        ..Default::default()
    };

    let status = crate::status::handle_status(&backend, temp.path(), request, "op_status")
        .unwrap()
        .response;

    assert_eq!(
        status.meta.aggregate_status,
        crate::AggregateStatus::Rejected
    );
    let rows = [("mem_app", crate::MemberStatus::Ok), ("mem_lib", Rejected)];
    assert_eq!(statuses(&status), rows);
    assert_eq!(status.errors, row_errors(&status, &["mem_lib"]));
}
