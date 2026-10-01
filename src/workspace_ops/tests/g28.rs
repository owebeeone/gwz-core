//! A `Partial` result lists each failed member's error in its top-level
//! `errors` (TR2.3, OD7 of dev-docs/GwzTransportReleasePlan.md; the contract is
//! gwz-cli docs/MachineOutput.md, "Partial results"). Before, a partial fetch
//! or push kept its failures on the member entries alone, so a caller that
//! read `errors` to decide success saw none.

use std::path::Path;

use crate::MemberStatus::{Failed, Noop, Rejected};
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

/// The rule is `Partial`'s alone: a fetch refused whole is `Rejected`, and its
/// member failures stay on their entries only, as before.
#[test]
fn a_fetch_that_is_not_partial_keeps_member_failures_on_their_entries() {
    let temp = TempDir::new("partial-fetch-rejected");
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
    assert_eq!(row_errors(&refused, &ids).len(), 4);
    assert_eq!(refused.errors, []);
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
