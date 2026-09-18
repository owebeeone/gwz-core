//! `gwz fetch` (gwz-cli dev-docs/GwzFetchPlan.md step 1.3): the handler
//! against real local bare remotes. A repository whose remote moved, one that
//! did not, one with no upstream at all, a remote that refuses, and the N+1
//! reads running concurrently.

use std::path::Path;

use crate::git::{Git2Backend, GitBackend};

use super::*;

fn fetch_request() -> crate::FetchRequest {
    crate::FetchRequest {
        meta: request_meta_with_workspace(),
    }
}

fn fetch_request_for(targets: &[&str]) -> crate::FetchRequest {
    crate::FetchRequest {
        meta: crate::RequestMeta {
            selection: Some(crate::Selection {
                targets: targets.iter().map(|target| (*target).to_owned()).collect(),
                ..Default::default()
            }),
            ..request_meta_with_workspace()
        },
    }
}

/// A workspace member cloned from a local bare remote, so a later push to that
/// remote is something the member can genuinely fetch.
fn add_member_from_remote(
    backend: &Git2Backend,
    root: &Path,
    fixture: &RemoteFixture,
    name: &str,
) -> std::path::PathBuf {
    let member_path = root.join(name);
    backend
        .clone_repo(fixture.remote_url(), &member_path)
        .unwrap();
    handle_add_existing_repo(
        backend,
        root,
        crate::AddExistingRepoRequest {
            meta: request_meta_with_workspace(),
            repository_path: member_path.to_string_lossy().into_owned(),
            member_path: None,
            member_id: None,
            source_id: None,
        },
        "op_add",
    )
    .unwrap();
    member_path
}

fn summary<'a>(response: &'a crate::FetchResponse, member_id: &str) -> &'a crate::FetchRepoSummary {
    response
        .repos
        .as_ref()
        .expect("fetch always reports repos")
        .iter()
        .find(|row| row.member_id == member_id)
        .unwrap_or_else(|| panic!("no row for {member_id}"))
}

fn statuses(response: &crate::FetchResponse) -> Vec<(String, crate::MemberStatus)> {
    response
        .response
        .members
        .iter()
        .map(|row| (row.member_id.clone(), row.status))
        .collect()
}

/// A member whose remote gained a commit reports `updated`, the exact object
/// ids either side of the fetch, and how far behind the local branch now is.
/// Nothing integrates: HEAD is where it was.
#[test]
fn a_member_whose_remote_moved_reports_the_ref_it_moved_from_and_to() {
    let temp = TempDir::new("fetch-ahead");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    let fixture = RemoteFixture::new("fetch-ahead-source");
    fixture.commit_and_push("README.md", "one", "initial", &backend);
    let member_path = add_member_from_remote(&backend, temp.path(), &fixture, "app");
    let head_before = backend.head(&member_path).unwrap().commit.unwrap();

    // The upstream moves after the member was cloned.
    let advanced = fixture.commit_and_push("README.md", "two", "second", &backend);

    let response = handle_fetch(
        &backend,
        temp.path(),
        fetch_request_for(&["@all", "@root"]),
        "op_fetch",
    )
    .unwrap();

    let row = summary(&response, "mem_app");
    assert_eq!(row.result, crate::FetchResult::Updated);
    assert_eq!(row.remote.as_deref(), Some("origin"));
    assert_eq!(row.branch.as_deref(), Some("main"));
    assert_eq!(row.before.as_deref(), Some(head_before.as_str()));
    assert_eq!(row.after.as_deref(), Some(advanced.as_str()));
    assert_eq!(row.upstream.as_deref(), Some("refs/remotes/origin/main"));
    assert_eq!((row.ahead, row.behind), (Some(0), Some(1)));

    assert_eq!(
        backend.head(&member_path).unwrap().commit,
        Some(head_before),
        "fetch integrates nothing: HEAD must not move"
    );
}

/// A member whose remote did not move reports `unchanged`, with the same
/// object id either side and level counts.
#[test]
fn a_member_whose_remote_did_not_move_reports_no_change() {
    let temp = TempDir::new("fetch-unchanged");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    let fixture = RemoteFixture::new("fetch-unchanged-source");
    let committed = fixture.commit_and_push("README.md", "one", "initial", &backend);
    add_member_from_remote(&backend, temp.path(), &fixture, "app");

    let response = handle_fetch(&backend, temp.path(), fetch_request(), "op_fetch").unwrap();

    let row = summary(&response, "mem_app");
    assert_eq!(row.result, crate::FetchResult::Unchanged);
    assert_eq!(row.before.as_deref(), Some(committed.as_str()));
    assert_eq!(row.after.as_deref(), Some(committed.as_str()));
    assert_eq!((row.ahead, row.behind), (Some(0), Some(0)));
}

/// A member with no fetch remote is a reported row, not an error, and it does
/// not change the outcome of the members that were contacted (plan D2).
#[test]
fn a_member_with_no_upstream_is_reported_and_does_not_fail_the_request() {
    let temp = TempDir::new("fetch-no-upstream");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    let local_path = temp.path().join("local");
    backend.create_repo(&local_path).unwrap();
    commit_file(&local_path, "README.md", "one", "initial", &[]).unwrap();
    handle_add_existing_repo(
        &backend,
        temp.path(),
        crate::AddExistingRepoRequest {
            meta: request_meta_with_workspace(),
            repository_path: local_path.to_string_lossy().into_owned(),
            member_path: None,
            member_id: None,
            source_id: None,
        },
        "op_add_local",
    )
    .unwrap();

    let fixture = RemoteFixture::new("fetch-no-upstream-source");
    fixture.commit_and_push("README.md", "one", "initial", &backend);
    add_member_from_remote(&backend, temp.path(), &fixture, "app");

    let response = handle_fetch(&backend, temp.path(), fetch_request(), "op_fetch").unwrap();

    let local = summary(&response, "mem_local");
    assert_eq!(local.result, crate::FetchResult::NoUpstream);
    assert_eq!(local.remote, None);
    assert_eq!(local.before, None);
    assert_eq!(local.after, None);

    assert_eq!(
        summary(&response, "mem_app").result,
        crate::FetchResult::Unchanged,
        "the contacted member still answers"
    );
    assert!(
        !matches!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Failed | crate::AggregateStatus::Partial
        ),
        "a no-upstream member is not a failure: {:?}",
        response.response.meta.aggregate_status
    );
}

/// A remote that refuses fails its own row and leaves the others alone: the
/// batch is `Partial`, which the CLI exits 1 on, so a script can tell an
/// incomplete report from a complete one (plan D3).
#[test]
fn a_refusing_remote_fails_its_own_row_and_the_batch_is_partial() {
    let temp = TempDir::new("fetch-refused");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    let good = RemoteFixture::new("fetch-refused-good");
    good.commit_and_push("README.md", "one", "initial", &backend);
    add_member_from_remote(&backend, temp.path(), &good, "good");

    let broken = RemoteFixture::new("fetch-refused-broken");
    broken.commit_and_push("README.md", "one", "initial", &backend);
    let broken_path = add_member_from_remote(&backend, temp.path(), &broken, "broken");
    // Repoint the member at a destination that is not there: the connection
    // is refused at fetch time, which is the failure this row must carry.
    let absent = temp.path().join("absent.git");
    git2::Repository::open(&broken_path)
        .unwrap()
        .remote_set_url("origin", absent.to_str().unwrap())
        .unwrap();

    let response = handle_fetch(&backend, temp.path(), fetch_request(), "op_fetch").unwrap();

    assert_eq!(
        summary(&response, "mem_broken").result,
        crate::FetchResult::Failed
    );
    assert_eq!(
        summary(&response, "mem_good").result,
        crate::FetchResult::Unchanged
    );
    let failed = response
        .response
        .members
        .iter()
        .find(|row| row.member_id == "mem_broken")
        .unwrap();
    assert_eq!(failed.status, crate::MemberStatus::Failed);
    assert!(failed.error.is_some(), "a failed row carries its reason");
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Partial
    );
}

/// The root is selected by default (plan D1) and is read alongside the members:
/// N members plus the root is N+1 rows, the root first, each contacted exactly
/// once even when `--jobs 1` serializes them.
#[test]
fn the_root_and_every_member_are_read_once_each_in_one_pass() {
    let temp = TempDir::new("fetch-n-plus-one");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    // Give the root a real remote of its own, so it is genuinely the +1.
    let root_remote = temp.path().join("root.git");
    init_bare_main(&root_remote);
    backend
        .add_remote(temp.path(), "origin", root_remote.to_str().unwrap())
        .unwrap();
    set_identity(temp.path());
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    backend.commit(temp.path(), "captured", false).unwrap();
    backend
        .push(temp.path(), "origin", "refs/heads/main:refs/heads/main")
        .unwrap();

    let mut members = Vec::new();
    for name in ["app", "lib", "docs"] {
        let fixture = RemoteFixture::new(&format!("fetch-n-{name}"));
        fixture.commit_and_push("README.md", "one", "initial", &backend);
        add_member_from_remote(&backend, temp.path(), &fixture, name);
        members.push(fixture);
    }

    for jobs in [1_i64, 4] {
        let response = handle_fetch(
            &backend,
            temp.path(),
            crate::FetchRequest {
                meta: crate::RequestMeta {
                    policy: Some(crate::OperationPolicy {
                        concurrency: Some(jobs),
                        ..Default::default()
                    }),
                    ..request_meta_with_workspace()
                },
            },
            "op_fetch",
        )
        .unwrap();

        let rows = statuses(&response);
        assert_eq!(rows.len(), 4, "N+1 rows at --jobs {jobs}: {rows:?}");
        assert_eq!(rows[0].0, "@root", "the root is the first row");
        let ids: Vec<_> = rows.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["@root", "mem_app", "mem_lib", "mem_docs"]);
        assert_eq!(
            response.repos.as_ref().unwrap().len(),
            rows.len(),
            "the report rows are parallel to the envelope rows"
        );
        for (id, _) in &rows {
            assert_eq!(
                summary(&response, id).result,
                crate::FetchResult::Unchanged,
                "{id} at --jobs {jobs}"
            );
        }
    }
}

/// `--target` narrows the batch exactly as it does under push, and excluding
/// the root leaves the members.
#[test]
fn target_selection_narrows_the_batch() {
    let temp = TempDir::new("fetch-target");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    for name in ["app", "lib"] {
        let fixture = RemoteFixture::new(&format!("fetch-target-{name}"));
        fixture.commit_and_push("README.md", "one", "initial", &backend);
        add_member_from_remote(&backend, temp.path(), &fixture, name);
    }

    let one = handle_fetch(
        &backend,
        temp.path(),
        fetch_request_for(&["mem_app"]),
        "op_fetch_one",
    )
    .unwrap();
    assert_eq!(
        statuses(&one)
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        ["mem_app"]
    );

    let without_root = handle_fetch(
        &backend,
        temp.path(),
        crate::FetchRequest {
            meta: crate::RequestMeta {
                selection: Some(crate::Selection {
                    targets: vec!["@all".to_owned()],
                    exclude_targets: vec!["@root".to_owned()],
                    ..Default::default()
                }),
                ..request_meta_with_workspace()
            },
        },
        "op_fetch_members",
    )
    .unwrap();
    assert_eq!(
        statuses(&without_root)
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        ["mem_app", "mem_lib"]
    );
}

/// `--dry-run` resolves the selection and reports the rows it would contact,
/// contacting nothing: the remote's later commit is still invisible after it
/// (plan D9).
#[test]
fn a_dry_run_reports_the_rows_it_would_contact_and_contacts_nothing() {
    let temp = TempDir::new("fetch-dry-run");
    let backend = Git2Backend::without_credential_helpers();
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

    let fixture = RemoteFixture::new("fetch-dry-run-source");
    fixture.commit_and_push("README.md", "one", "initial", &backend);
    let member_path = add_member_from_remote(&backend, temp.path(), &fixture, "app");
    let tracking_before = backend
        .read_ref(&member_path, "refs/remotes/origin/main")
        .unwrap();
    fixture.commit_and_push("README.md", "two", "second", &backend);

    let response = handle_fetch(
        &backend,
        temp.path(),
        crate::FetchRequest {
            meta: crate::RequestMeta {
                dry_run: Some(true),
                ..request_meta_with_workspace()
            },
        },
        "op_fetch_dry",
    )
    .unwrap();

    let row = response
        .response
        .members
        .iter()
        .find(|row| row.member_id == "mem_app")
        .unwrap();
    assert_eq!(row.status, crate::MemberStatus::Planned);
    assert_eq!(
        summary(&response, "mem_app").result,
        crate::FetchResult::Planned,
        "a dry-run row carries no result from any remote, so it cannot be Unchanged"
    );
    assert_eq!(
        summary(&response, "mem_app").remote.as_deref(),
        Some("origin")
    );
    assert_eq!(
        backend
            .read_ref(&member_path, "refs/remotes/origin/main")
            .unwrap(),
        tracking_before,
        "a dry run contacts nothing, so the tracking ref cannot move"
    );
}
