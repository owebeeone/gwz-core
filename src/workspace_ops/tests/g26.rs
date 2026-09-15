//! Push plan step 1.2 (`dev-docs/GwzUrlSchemePushPlan.md`): the tracking
//! backend as an observable publication seam, and today's publication reads
//! and pushes pinned on it. Steps 2.1, 3.3 and 3.5 move these pins on purpose.
//! Step 2.1 adds the read-URL cases of the plan's §3.2 and §3.3, step 3.3 the
//! check-once cases of §3.5 rule 1, and step 3.5 the contact cases of rules 2
//! and 3 (`contact_changed`).
use std::path::{Path, PathBuf};

use crate::git::{GitBackend, GitPreparedPush, UrlScheme};
use crate::model::ErrorCode;
use crate::operation::NullSink;
use crate::workspace_ops::url_scheme_state::{
    EffectiveUrlScheme, URL_SCHEME_STATE_PATH, UrlSchemeSource, record_workspace_url_scheme,
};

use super::g01::tracking_backend::{
    ConfiguredRemote, RemoteCall, TEST_COMMIT, TrackingBackend, calls_by_host,
};
use super::*;

mod concurrent_reads;
mod contact_changed;
mod tag_publication;

const MEMBERS: usize = 2;
const MAIN: &str = "refs/heads/main";
const ROOT_SSH: &str = "git@github.com:o/root.git";
const ROOT_HTTPS: &str = "https://github.com/o/root.git";
const APP_SSH: &str = "git@github.com:o/app.git";
const APP_HTTPS: &str = "https://github.com/o/app.git";
/// Another repository: a fork of `app`.
const APP_FORK: &str = "git@github.com:fork/app.git";
/// `app` on a host with no derivable forms, in both schemes.
const APP_UNKNOWN_HOST_SSH: &str = "git@git.example.com:o/app.git";
const APP_UNKNOWN_HOST_HTTPS: &str = "https://git.example.com/o/app.git";
const LIB_SSH: &str = "git@github.com:o/lib.git";
const LIB_HTTPS: &str = "https://github.com/o/lib.git";
const ROOT_BASE: &str = "1111111111111111111111111111111111111111";
const ROOT_HEAD: &str = "2222222222222222222222222222222222222222";
const APP_BASE: &str = "3333333333333333333333333333333333333333";
const APP_HEAD: &str = "4444444444444444444444444444444444444444";
const LIB_HEAD: &str = "5555555555555555555555555555555555555555";
/// One ahead of `app`'s locked `APP_HEAD`.
const APP_NEXT: &str = "6666666666666666666666666666666666666666";
/// A rewrite of `app` that does not contain `APP_HEAD`.
const APP_REWRITE: &str = "7777777777777777777777777777777777777777";
/// `app` through an SSH host alias for github.com, which §3.1 cannot tell is
/// the same repository.
const APP_ALIAS: &str = "git@github-work:o/app.git";
/// `app` without its `.git` suffix, which GitHub serves as the same repository.
const APP_BARE: &str = "git@github.com:o/app";
const FORCED: &str = "+refs/heads/main:refs/heads/main";

/// The double answers publication calls from what a test configured, keeps its
/// original answers for anything unconfigured, and records every read and push.
#[test]
fn tracking_backend_serves_configured_publication_state_and_records_calls() {
    let backend = TrackingBackend::new(1);
    let (root, app) = (Path::new("/ws"), Path::new("/ws/repos/app"));
    let branch = "refs/heads/main:refs/heads/main";
    assert!(backend.is_repository(app).unwrap());
    assert!(backend.is_ancestor(app, APP_HEAD, APP_BASE).unwrap());
    assert_eq!(
        backend.prepare_push(app, "origin", branch).unwrap().url,
        "ssh://app.invalid/repo.git"
    );
    assert_eq!(
        backend.ls_remote_url(app, APP_SSH, "origin", None).unwrap()[0].target,
        TEST_COMMIT
    );
    assert_eq!(
        backend
            .read_file_at_commit(root, ROOT_HEAD, "a.txt")
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedOperation
    );

    backend.commit_files(root, ROOT_HEAD, &[("a.txt", b"a".to_vec())]);
    assert_eq!(
        backend
            .read_file_at_commit(root, ROOT_HEAD, "a.txt")
            .unwrap(),
        Some(b"a".to_vec())
    );
    assert_eq!(
        backend
            .read_file_at_commit(root, ROOT_HEAD, "b.txt")
            .unwrap(),
        None
    );
    backend.set_head(app, "main", APP_HEAD);
    backend.add_remote_config(
        app,
        ConfiguredRemote {
            push_url: Some(APP_SSH.to_owned()),
            ..ConfiguredRemote::new("origin", APP_HTTPS)
        },
    );
    assert_eq!(
        backend.fetch_refspecs(app, "origin"),
        Some(vec!["+refs/heads/*:refs/remotes/origin/*".to_owned()])
    );
    let plan = backend.prepare_push(app, "origin", branch).unwrap();
    assert_eq!(plan.url, APP_SSH, "the push destination wins over the URL");
    assert_eq!(plan.refspecs, vec![format!("{APP_HEAD}:{MAIN}")]);

    // One store behind two spellings. An ordinary push that is not a
    // fast-forward moves nothing; a forced push moves what both advertise.
    backend.serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_BASE)]);
    for answer in [Ok(false), Err("shallow history")] {
        backend.set_ancestry(APP_BASE, APP_HEAD, answer);
        assert_eq!(
            backend.push_prepared(app, &plan).unwrap_err().code,
            ErrorCode::RemoteRejected
        );
    }
    assert_eq!(
        backend
            .is_ancestor(app, APP_BASE, APP_HEAD)
            .unwrap_err()
            .code,
        ErrorCode::GitCommandFailed
    );
    assert_eq!(
        backend.advertised_ref(APP_HTTPS, MAIN).as_deref(),
        Some(APP_BASE)
    );
    let forced = GitPreparedPush {
        refspecs: vec![format!("+{APP_HEAD}:{MAIN}")],
        ..plan
    };
    backend.push_prepared(app, &forced).unwrap();
    assert_eq!(
        backend
            .ls_remote_url(app, APP_HTTPS, "origin", Some(app))
            .unwrap()[0]
            .target,
        APP_HEAD
    );
    let deletion = backend
        .prepare_push(app, "origin", &format!(":{MAIN}"))
        .unwrap();
    backend.push_prepared(app, &deletion).unwrap();
    assert_eq!(backend.advertised_ref(APP_SSH, MAIN), None);

    backend.set_materialized(app, false);
    assert!(!backend.is_repository(app).unwrap());
    assert_eq!(backend.remote_reads().len(), 2);
    assert_eq!(
        backend.prepared_pushes().len(),
        4,
        "refused pushes are recorded too"
    );
    assert_eq!(backend.remote_calls().len(), 6);
}

/// Advertisement reads carry an overlap counter like fetches and pushes.
#[test]
fn tracking_backend_counts_overlapping_advertisement_reads() {
    let backend = TrackingBackend::new(1).with_read_overlap(2);
    std::thread::scope(|scope| {
        for url in [APP_SSH, LIB_SSH] {
            let backend = &backend;
            scope.spawn(move || {
                backend
                    .ls_remote_url(Path::new("/ws"), url, "origin", None)
                    .unwrap()
            });
        }
    });
    assert_eq!(backend.read_peak(), 2);
    assert_eq!(backend.remote_reads().len(), 2);
}

/// Step 1.2(a), moved by step 3.3 to §2.3's check-once counts: configured URLs
/// equal the committed URLs, and `app` and the root changed. Each destination
/// is read once. `lib` is already on origin, so it is `Noop` and not pushed.
/// The dependency preflight and the proof answer from those reads and from
/// `app`'s push: N+1 reads and 2 pushes, N+3 calls.
#[test]
fn whole_push_reads_each_destination_once_and_pushes_only_what_changed() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    let response = fixture.push(None);

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_SSH, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
            push_to(&app, APP_SSH, APP_HEAD),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ]
    );
    assert_eq!(fixture.backend.remote_calls().len(), MEMBERS + 3);
    let lib_row = &response.response.members[1];
    let planned = lib_row.planned.as_ref().unwrap();
    assert_eq!(
        (lib_row.status, planned.action, planned.message.as_deref()),
        (
            crate::MemberStatus::Noop,
            crate::PlannedAction::Noop,
            Some("already on origin")
        )
    );
    assert_eq!(
        fixture.backend.advertised_ref(APP_SSH, MAIN).as_deref(),
        Some(APP_HEAD)
    );
    assert_eq!(
        fixture.backend.advertised_ref(ROOT_SSH, MAIN).as_deref(),
        Some(ROOT_HEAD)
    );
}

/// Step 1.2(b), moved by step 3.3: a root-only push has no member read to reuse
/// and no member push to accept, so each dependency is read once before the
/// transfer, and the proof answers from those reads (D9): N+1 reads and one
/// push, N+2 calls.
#[test]
fn root_only_push_reads_every_dependency_once_before_the_transfer() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    // `app` was published before this push.
    fixture
        .backend
        .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    let response = fixture.push(Some(crate::Selection {
        targets: vec!["@root".to_owned()],
        ..Default::default()
    }));

    assert_eq!(
        response.response.members.single().status,
        crate::MemberStatus::Ok
    );
    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(root, ROOT_SSH, root),
            read_at(root, APP_SSH, &app),
            read_at(root, LIB_SSH, &lib),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ]
    );
    assert_eq!(fixture.backend.remote_calls().len(), MEMBERS + 2);
}

/// Step 1.2(c), flipped by step 2.1: member remotes are the https form of their
/// SSH committed URLs, so each dependency is read through its member's https
/// remote. The preflight answers from each member's own read, and the proof
/// from `app`'s push and `lib`'s read, since `lib` is already on origin (step
/// 3.3): N+1 reads, none of them SSH.
#[test]
fn https_member_remotes_read_every_dependency_through_its_https_remote() {
    let fixture = PublicationFixture::new(ROOT_HTTPS, APP_HTTPS, LIB_HTTPS);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    let response = fixture.push(None);

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_HTTPS, &app),
            read_at(&lib, LIB_HTTPS, &lib),
            read_at(root, ROOT_HTTPS, root),
            push_to(&app, APP_HTTPS, APP_HEAD),
            push_to(root, ROOT_HTTPS, ROOT_HEAD),
        ]
    );
    let reads = fixture.backend.remote_reads();
    let ssh = reads
        .iter()
        .filter(|call| matches!(call, RemoteCall::Read { url, .. } if crate::git::uses_ssh(url)))
        .count();
    assert_eq!((reads.len(), ssh), (MEMBERS + 1, 0));
    assert_eq!(fixture.backend.prepared_pushes().len(), 2);
}

/// Step 2.1: a fork push URL is another repository, so `app` is read at its
/// committed URL before the transfer and again for the proof, because its
/// push reached only the fork and the advertisement kept from before does not
/// show the commit. The committed repository lacks the commit, so the root is
/// refused. `lib` is already on origin (step 3.3).
#[test]
fn a_fork_push_url_reads_the_committed_url_and_gets_no_shortcut() {
    let fork = ConfiguredRemote {
        push_url: Some(APP_FORK.to_owned()),
        ..ConfiguredRemote::new("origin", APP_SSH)
    };
    let fixture = PublicationFixture::with_app(ROOT_SSH, APP_SSH, fork, LIB_SSH);
    fixture.backend.serve(&[APP_FORK], &[(MAIN, APP_BASE)]);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    let response = fixture.push(None);

    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_FORK, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
            read_at(root, APP_SSH, &app),
            push_to(&app, APP_FORK, APP_HEAD),
            read_at(root, APP_SSH, &app),
        ]
    );
    let root_row = response.response.members.last().unwrap();
    assert_eq!(
        (root_row.member_id.as_str(), root_row.status),
        ("@root", crate::MemberStatus::Rejected)
    );
}

/// Step 2.1 (D2): a remote that fetches over https and pushes over SSH is read
/// at its SSH push destination, the URL it is pushed to, so the dedup and the
/// shortcut both hit and nothing is read over https.
#[test]
fn a_remote_that_fetches_over_https_and_pushes_over_ssh_is_read_at_its_push_url() {
    let split = ConfiguredRemote {
        push_url: Some(APP_SSH.to_owned()),
        ..ConfiguredRemote::new("origin", APP_HTTPS)
    };
    let fixture = PublicationFixture::with_app(ROOT_SSH, APP_SSH, split, LIB_SSH);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    fixture.push(None);

    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_SSH, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
            push_to(&app, APP_SSH, APP_HEAD),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ]
    );
}

/// Step 2.1: on a host with no derivable forms an https remote is not provably
/// the committed repository, so `app` is read at its committed URL as before
/// step 2.1, with no dedup and no shortcut. Reads and pushes to two hosts may
/// overlap (step 3.4), so the reads keep a fixed order within each host.
#[test]
fn an_https_remote_on_an_unknown_host_reads_the_committed_url() {
    let fixture = PublicationFixture::with_app(
        ROOT_SSH,
        APP_UNKNOWN_HOST_SSH,
        ConfiguredRemote::new("origin", APP_UNKNOWN_HOST_HTTPS),
        LIB_SSH,
    );
    fixture.backend.serve(
        &[APP_UNKNOWN_HOST_SSH, APP_UNKNOWN_HOST_HTTPS],
        &[(MAIN, APP_BASE)],
    );
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    let response = fixture.push(None);

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        calls_by_host(fixture.backend.remote_reads()),
        calls_by_host(vec![
            read_at(&app, APP_UNKNOWN_HOST_HTTPS, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
            read_at(root, APP_UNKNOWN_HOST_SSH, &app),
            read_at(root, APP_UNKNOWN_HOST_SSH, &app),
        ])
    );
    // `app` and the root; `lib` is already on origin (step 3.3).
    assert_eq!(fixture.backend.prepared_pushes().len(), 2);
}

/// Step 2.1 (D3), moved by step 3.3: a dependency that is not materialized is
/// read through the workspace's recorded scheme, with no identity repository,
/// exactly once under the default and under `always`: the proof answers from
/// the read kept before the transfer.
#[test]
fn an_unmaterialized_dependency_is_read_through_the_recorded_url_scheme() {
    for remote_check in [None, Some(crate::RemoteCheck::Always)] {
        let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
        // `app` was published before this push.
        fixture
            .backend
            .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
        let root = fixture.root.as_path();
        let app = root.join("repos/app");
        fixture
            .backend
            .set_materialized(&root.join("repos/lib"), false);
        let recorded = EffectiveUrlScheme {
            scheme: UrlScheme::Https,
            source: UrlSchemeSource::Request,
        };
        record_workspace_url_scheme(root, recorded, "clone").unwrap();
        let lib_read = RemoteCall::Read {
            path: root.to_path_buf(),
            url: LIB_HTTPS.to_owned(),
            remote: "origin".to_owned(),
            identity_repo: None,
        };

        let response = fixture.push_with(root_only(), None, remote_check);

        assert_eq!(
            response.response.members.single().status,
            crate::MemberStatus::Ok
        );
        assert_eq!(
            fixture.backend.remote_calls(),
            vec![
                read_at(root, ROOT_SSH, root),
                read_at(root, APP_SSH, &app),
                lib_read,
                push_to(root, ROOT_SSH, ROOT_HEAD),
            ],
            "{remote_check:?}"
        );
    }
}

/// Step 2.1 (D3, §3.3): a recorded scheme that cannot be read refuses a push
/// only when a dependency needs it, before any read, and names the remedies a
/// push has rather than a materialize.
#[test]
fn an_unreadable_url_scheme_record_refuses_a_push_with_an_unmaterialized_dependency() {
    for materialized in [true, false] {
        let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
        fixture
            .backend
            .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
        let root = fixture.root.as_path();
        fixture
            .backend
            .set_materialized(&root.join("repos/lib"), materialized);
        let record = root.join(URL_SCHEME_STATE_PATH);
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, "schema: gwz.url-scheme/v1\nscheme: auto\n").unwrap();

        let response = fixture.push(root_only());

        let row = response.response.members.single();
        if materialized {
            assert_eq!(row.status, crate::MemberStatus::Ok);
            continue;
        }
        assert_eq!(row.status, crate::MemberStatus::Rejected);
        let message = &row.error.as_ref().unwrap().message;
        assert!(
            message.contains("url-scheme.yml is unreadable"),
            "{message}"
        );
        assert!(message.contains("delete or repair the file"), "{message}");
        assert!(!message.contains("materialize"), "{message}");
        assert!(!message.contains("--url-scheme"), "{message}");
        assert_eq!(fixture.backend.remote_calls(), Vec::new());
    }
}

/// Step 2.1 (§3.3): a proof refusal names the URL it read when that is not the
/// committed URL, and is worded as before when it is. Step 3.5 (§3.6) gives a
/// push under the default the remedies of a missing dependency.
#[test]
fn a_proof_refusal_names_the_read_url_when_it_is_not_the_committed_url() {
    let read_through = format!(" (read through {APP_HTTPS})");
    for (app_url, read_through) in [(APP_HTTPS, read_through.as_str()), (APP_SSH, "")] {
        let fixture = PublicationFixture::new(ROOT_SSH, app_url, LIB_SSH);

        let response = fixture.push(root_only());

        let row = response.response.members.single();
        assert_eq!(row.status, crate::MemberStatus::Rejected);
        assert_eq!(
            row.error.as_ref().unwrap().message,
            format!(
                "root publication blocked: cannot prove member mem_app commit {APP_HEAD} is available at its committed fetch remote origin{read_through}; publish the member by pushing a branch that contains the commit, fetch its advertised history and retry, or run gwz push --check-remotes, which re-checks the members and pushes those whose remote lacks their branch's commit"
            )
        );
        assert!(fixture.backend.prepared_pushes().is_empty());
    }
}

/// Step 3.3 (§2.3): nothing to publish. Each repository is read once and is
/// already on origin, so nothing is pushed and the root is proven from those
/// reads: N+1 calls, with every row and the aggregate `noop`.
#[test]
fn a_whole_push_with_nothing_to_publish_reads_each_repository_once_and_pushes_nothing() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    fixture
        .backend
        .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
    fixture.backend.serve(&[ROOT_SSH], &[(MAIN, ROOT_HEAD)]);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    let response = fixture.push(None);

    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_SSH, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
        ]
    );
    assert_eq!(fixture.backend.remote_calls().len(), MEMBERS + 1);
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Noop
    );
    for row in &response.response.members {
        let planned = row.planned.as_ref().unwrap();
        assert_eq!(
            (row.status, planned.action, planned.message.as_deref()),
            (
                crate::MemberStatus::Noop,
                crate::PlannedAction::Noop,
                Some("already on origin")
            ),
            "{}",
            row.member_id
        );
    }
}

/// Step 3.3: a root already on origin is still proven, from the evidence a
/// pushed root uses, as §3.5 rules 2 and 3 require of a root that rule 1 finds
/// already on origin. `app` was rewound below the commit that root names, so
/// the root is refused and nothing is pushed.
#[test]
fn a_root_already_on_origin_is_refused_when_a_dependency_is_missing() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    fixture.backend.serve(&[ROOT_SSH], &[(MAIN, ROOT_HEAD)]);
    let app = fixture.root.join("repos/app");
    fixture.backend.set_head(&app, "main", APP_BASE);

    let response = fixture.push_with(None, None, Some(crate::RemoteCheck::Always));

    assert_root_refused(&fixture, &response);
    assert_eq!(fixture.backend.prepared_pushes(), Vec::new());
}

/// Step 3.3 (§3.5 rule 1): `lib`'s remote lacks the destination ref, though it
/// may advertise `lib`'s commit under another ref, so `lib` is still pushed.
#[test]
fn an_advertisement_without_the_destination_ref_still_pushes() {
    for refs in [&[][..], &[("refs/heads/other", LIB_HEAD)][..]] {
        let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
        let lib = fixture.root.join("repos/lib");
        fixture.backend.serve(&[LIB_SSH, LIB_HTTPS], refs);

        let response = fixture.push(None);

        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok,
            "{refs:?}"
        );
        assert!(
            fixture
                .backend
                .prepared_pushes()
                .contains(&push_to(&lib, LIB_SSH, LIB_HEAD)),
            "{refs:?}"
        );
    }
}

/// Step 3.3 (D8): `app`'s head is one ahead of its locked commit. The accepted
/// push of that head proves the locked commit, its ancestor, with no read.
#[test]
fn a_member_ahead_of_its_lock_commit_is_proven_by_its_push_with_no_read() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));
    fixture.backend.set_head(&app, "main", APP_NEXT);
    for ancestor in [APP_BASE, APP_HEAD] {
        fixture.backend.set_ancestry(ancestor, APP_NEXT, Ok(true));
    }

    let response = fixture.push(None);

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_SSH, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
            push_to(&app, APP_SSH, APP_NEXT),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ]
    );
}

/// Step 3.3 (D8, D9): before `app` pushes a head that does not contain its
/// locked commit, its remote advertises that commit under another branch. A
/// destination this operation pushed to is proven by its push or by a read,
/// never by its advertisement from before the push, so it is read again.
#[test]
fn a_pushed_destination_is_proven_by_a_read_not_by_its_pre_push_advertisement() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));
    fixture.backend.serve(
        &[APP_SSH, APP_HTTPS],
        &[(MAIN, APP_BASE), ("refs/heads/keep", APP_HEAD)],
    );
    fixture.backend.set_head(&app, "main", APP_NEXT);
    fixture.backend.set_ancestry(APP_BASE, APP_NEXT, Ok(true));
    fixture.backend.set_ancestry(APP_HEAD, APP_NEXT, Ok(false));

    let response = fixture.push(None);

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_SSH, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
            push_to(&app, APP_SSH, APP_NEXT),
            read_at(root, APP_SSH, &app),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ]
    );
}

/// Step 3.3 (§3.5 rule 1): `lib` is not selected and reaches `app`'s
/// repository, through `app`'s own URL under `always`, or through the spelling
/// without `.git` under the default. `app`'s forced push rewinds that
/// repository, so `lib`'s advertisement kept from before no longer counts:
/// `lib` is read again and the root refused.
#[test]
fn a_forced_push_voids_the_kept_advertisement_of_another_member_of_its_repository() {
    for (lib_url, remote_check) in [
        (APP_SSH, Some(crate::RemoteCheck::Always)),
        (APP_BARE, None),
    ] {
        let fixture = PublicationFixture::with_members(
            ROOT_SSH,
            (APP_SSH, ConfiguredRemote::new("origin", APP_SSH)),
            (lib_url, ConfiguredRemote::new("origin", lib_url)),
        );
        let lib = fixture.root.join("repos/lib");
        fixture
            .backend
            .serve(&[APP_SSH, APP_HTTPS, APP_BARE], &[(MAIN, LIB_HEAD)]);
        fixture.backend.set_ancestry(LIB_HEAD, APP_HEAD, Ok(false));

        let response =
            fixture.push_with(selected(&["mem_app", "@root"]), Some(FORCED), remote_check);

        assert_eq!(
            fixture.backend.remote_calls().last(),
            Some(&read_at(&fixture.root, lib_url, &lib)),
            "{lib_url}"
        );
        assert_root_refused(&fixture, &response);
    }
}

/// Step 3.3 (§3.5 rule 1): `app` pushes through an SSH host alias that §3.1
/// cannot tell is its committed repository, so its dependency is read at the
/// committed URL under another key. A forced push through the alias rewinds
/// that repository, and the advertisement kept from before no longer counts,
/// under `always` and under the default.
#[test]
fn a_forced_push_through_a_host_alias_voids_the_committed_urls_kept_advertisement() {
    for remote_check in [Some(crate::RemoteCheck::Always), None] {
        let fixture = alias_fixture();
        let app = fixture.root.join("repos/app");
        fixture.backend.set_head(&app, "main", APP_REWRITE);
        fixture
            .backend
            .set_ancestry(APP_HEAD, APP_REWRITE, Ok(false));

        let response = fixture.push_with(None, Some(FORCED), remote_check);

        assert_eq!(
            fixture.backend.advertised_ref(APP_SSH, MAIN).as_deref(),
            Some(APP_REWRITE),
            "{remote_check:?}"
        );
        assert_root_refused(&fixture, &response);
    }
}

/// Step 3.3 (D9): the ordinary variant. `app` fast-forwards through the alias
/// from its locked commit, which the committed URL's kept advertisement showed
/// and still proves, so nothing is read after the transfer. The alias is
/// another host, whose calls may overlap github.com's (step 3.4), so the calls
/// keep a fixed order within each host.
#[test]
fn an_ordinary_push_through_a_host_alias_reuses_the_kept_advertisement() {
    let fixture = alias_fixture();
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));
    fixture.backend.set_head(&app, "main", APP_NEXT);
    fixture.backend.set_ancestry(APP_HEAD, APP_NEXT, Ok(true));

    let response = fixture.push(None);

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        calls_by_host(fixture.backend.remote_calls()),
        calls_by_host(vec![
            read_at(&app, APP_ALIAS, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
            read_at(root, APP_SSH, &app),
            push_to(&app, APP_ALIAS, APP_NEXT),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ])
    );
}

/// Step 3.3 (§3.5 rule 1): `app` and `lib` reach one repository through two
/// spellings, and only one of them is pushed with force. One host runs their
/// transfers in order, `app` first, so forcing each in turn runs the ordinary
/// push before and after the forced one. Before: the ordinary push's D8 proof
/// does not count, the dependency is read, and the root is refused. After: the
/// rewound repository refuses the ordinary push, and the root is not attempted.
#[test]
fn an_ordinary_push_proves_nothing_once_a_forced_push_reaches_its_repository() {
    for (forced, refusal) in [
        ("repos/lib", "cannot prove member mem_app"),
        ("repos/app", "member push failed: mem_lib"),
    ] {
        let fixture = PublicationFixture::with_members(
            ROOT_SSH,
            (APP_SSH, ConfiguredRemote::new("origin", APP_SSH)),
            (APP_BARE, ConfiguredRemote::new("origin", APP_BARE)),
        );
        fixture
            .backend
            .serve(&[APP_SSH, APP_HTTPS, APP_BARE], &[(MAIN, APP_BASE)]);
        fixture.backend.force_pushes(&fixture.root.join(forced));
        fixture.backend.set_ancestry(APP_HEAD, LIB_HEAD, Ok(false));
        fixture.backend.set_ancestry(LIB_HEAD, APP_HEAD, Ok(false));

        let response = fixture.push(None);

        let root_row = response.response.members.last().unwrap();
        let message = &root_row.error.as_ref().unwrap().message;
        assert!(message.contains(refusal), "{forced}: {message}");
        assert_root_refused(&fixture, &response);
    }
}

/// Step 3.3 (§3.5 rule 1, D8): `app`'s remote holds its locked commit, and a
/// forced push of a rewrite without that commit rewinds the remote. Neither the
/// advertisement kept from before nor the push proves the commit, so it is read
/// and the root refused.
#[test]
fn a_forced_push_that_rewinds_a_members_remote_refuses_the_root() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    let app = fixture.root.join("repos/app");
    fixture
        .backend
        .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
    fixture.backend.set_head(&app, "main", APP_REWRITE);
    fixture
        .backend
        .set_ancestry(APP_HEAD, APP_REWRITE, Ok(false));

    let response = fixture.push_with(None, Some(FORCED), None);

    assert_eq!(
        fixture.backend.advertised_ref(APP_SSH, MAIN).as_deref(),
        Some(APP_REWRITE)
    );
    assert_root_refused(&fixture, &response);
}

fn root_only() -> Option<crate::Selection> {
    selected(&["@root"])
}

fn selected(targets: &[&str]) -> Option<crate::Selection> {
    Some(crate::Selection {
        targets: targets.iter().map(|target| (*target).to_owned()).collect(),
        ..Default::default()
    })
}

/// The root row is refused, and the root was never pushed.
fn assert_root_refused(fixture: &PublicationFixture, response: &crate::PushResponse) {
    let row = response.response.members.last().unwrap();
    assert_eq!(
        (row.member_id.as_str(), row.status),
        ("@root", crate::MemberStatus::Rejected)
    );
    assert!(
        !fixture
            .backend
            .prepared_pushes()
            .iter()
            .any(|call| matches!(call, RemoteCall::Push { path, .. } if *path == fixture.root)),
        "{:?}",
        fixture.backend.remote_calls()
    );
}

/// `app` fetches from its committed URL and pushes through `APP_ALIAS`, and its
/// repository already holds its locked commit.
fn alias_fixture() -> PublicationFixture {
    let origin = ConfiguredRemote {
        push_url: Some(APP_ALIAS.to_owned()),
        ..ConfiguredRemote::new("origin", APP_SSH)
    };
    let fixture = PublicationFixture::with_app(ROOT_SSH, APP_SSH, origin, LIB_SSH);
    fixture
        .backend
        .serve(&[APP_SSH, APP_HTTPS, APP_ALIAS], &[(MAIN, APP_HEAD)]);
    fixture
}

/// Two members and the root as the double serves them. The root head commits a
/// lock whose `app` commit is one ahead of `app`'s remote and whose `lib` commit
/// is already on `lib`'s remote; the root head is one ahead of the root's
/// remote. Committed member URLs are SSH, the configured URLs are the
/// arguments, and each member's store answers at both spellings.
struct PublicationFixture {
    _temp: TempDir,
    root: PathBuf,
    backend: TrackingBackend,
}

impl PublicationFixture {
    fn new(root_url: &str, app_url: &str, lib_url: &str) -> Self {
        let app_origin = ConfiguredRemote::new("origin", app_url);
        Self::with_app(root_url, APP_SSH, app_origin, lib_url)
    }

    /// As `new`, with `app` committed at `app_committed` and configured with
    /// `app_origin`.
    fn with_app(
        root_url: &str,
        app_committed: &str,
        app_origin: ConfiguredRemote,
        lib_url: &str,
    ) -> Self {
        let lib_origin = ConfiguredRemote::new("origin", lib_url);
        Self::with_members(root_url, (app_committed, app_origin), (LIB_SSH, lib_origin))
    }

    /// As `new`, with each member committed at its URL and configured with its
    /// remote.
    fn with_members(
        root_url: &str,
        (app_committed, app_origin): (&str, ConfiguredRemote),
        (lib_committed, lib_origin): (&str, ConfiguredRemote),
    ) -> Self {
        let temp = TempDir::new("push-seam");
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        write_pull_fixture(
            temp.path(),
            vec![
                ("mem_app", "repos/app", app_committed, APP_HEAD),
                ("mem_lib", "repos/lib", lib_committed, LIB_HEAD),
            ],
        );
        // That fixture gives every lock entry `src_app`; publication checks it.
        let mut lock = crate::artifact::read_lock(temp.path()).unwrap();
        lock.members.get_mut("mem_lib").unwrap().source_id = Some("src_lib".to_owned());
        crate::artifact::write_lock(temp.path(), &lock).unwrap();
        let root = crate::workspace_ops::resolve_request_workspace_root(
            temp.path(),
            &request_meta_with_workspace(),
        )
        .unwrap();

        let backend = TrackingBackend::new(1);
        let committed = [
            crate::workspace::WORKSPACE_MANIFEST,
            crate::artifact::LOCK_PATH,
        ]
        .map(|file| (file, std::fs::read(root.join(file)).unwrap()));
        backend.commit_files(&root, ROOT_HEAD, &committed);
        for (path, origin, head) in [
            (
                root.clone(),
                ConfiguredRemote::new("origin", root_url),
                ROOT_HEAD,
            ),
            (root.join("repos/app"), app_origin, APP_HEAD),
            (root.join("repos/lib"), lib_origin, LIB_HEAD),
        ] {
            backend.set_head(&path, "main", head);
            backend.add_remote_config(&path, origin);
        }
        backend.serve(&[root_url], &[(MAIN, ROOT_BASE)]);
        backend.serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_BASE)]);
        backend.serve(&[LIB_SSH, LIB_HTTPS], &[(MAIN, LIB_HEAD)]);
        for (base, head) in [(ROOT_BASE, ROOT_HEAD), (APP_BASE, APP_HEAD)] {
            backend.set_ancestry(base, head, Ok(true));
            backend.set_ancestry(head, base, Ok(false));
        }
        Self {
            _temp: temp,
            root,
            backend,
        }
    }

    fn push(&self, selection: Option<crate::Selection>) -> crate::PushResponse {
        self.push_with(selection, None, None)
    }

    /// As `push`, with a request refspec and `remote_check`.
    fn push_with(
        &self,
        selection: Option<crate::Selection>,
        refspec: Option<&str>,
        remote_check: Option<crate::RemoteCheck>,
    ) -> crate::PushResponse {
        self.run(Self::request(selection, refspec, remote_check))
    }

    /// The request `push_with` sends, for a test to adjust before `run`.
    fn request(
        selection: Option<crate::Selection>,
        refspec: Option<&str>,
        remote_check: Option<crate::RemoteCheck>,
    ) -> crate::PushRequest {
        crate::PushRequest {
            meta: crate::RequestMeta {
                selection,
                // One connection per host keeps the recorded order stable.
                policy: Some(crate::OperationPolicy {
                    max_connections_per_host: Some(1),
                    ..Default::default()
                }),
                ..request_meta_with_workspace()
            },
            remote: None,
            refspec: refspec.map(str::to_owned),
            remote_check,
        }
    }

    fn run(&self, request: crate::PushRequest) -> crate::PushResponse {
        let world = crate::operation_context::TestWorld::physical();
        let services = world.context();
        handle_push_with_events_in(
            &services,
            &self.backend,
            &self.root,
            request,
            "op_push",
            &NullSink,
        )
        .unwrap()
    }
}

fn read_at(path: &Path, url: &str, identity_repo: &Path) -> RemoteCall {
    RemoteCall::Read {
        path: path.to_path_buf(),
        url: url.to_owned(),
        remote: "origin".to_owned(),
        identity_repo: Some(identity_repo.to_path_buf()),
    }
}

fn push_to(path: &Path, url: &str, commit: &str) -> RemoteCall {
    RemoteCall::Push {
        path: path.to_path_buf(),
        remote: "origin".to_owned(),
        url: url.to_owned(),
        refspecs: vec![format!("{commit}:{MAIN}")],
    }
}
