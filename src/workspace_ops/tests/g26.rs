//! Push plan step 1.2 (`dev-docs/GwzUrlSchemePushPlan.md`): the tracking
//! backend as an observable publication seam, and today's publication reads
//! and pushes pinned on it. Steps 2.1, 3.3 and 3.5 move these pins on purpose.
use std::path::{Path, PathBuf};

use crate::git::{GitBackend, GitPreparedPush};
use crate::model::ErrorCode;
use crate::operation::NullSink;

use super::g01::tracking_backend::{ConfiguredRemote, RemoteCall, TEST_COMMIT, TrackingBackend};
use super::*;

const MEMBERS: usize = 2;
const MAIN: &str = "refs/heads/main";
const ROOT_SSH: &str = "git@github.com:o/root.git";
const ROOT_HTTPS: &str = "https://github.com/o/root.git";
const APP_SSH: &str = "git@github.com:o/app.git";
const APP_HTTPS: &str = "https://github.com/o/app.git";
const LIB_SSH: &str = "git@github.com:o/lib.git";
const LIB_HTTPS: &str = "https://github.com/o/lib.git";
const ROOT_BASE: &str = "1111111111111111111111111111111111111111";
const ROOT_HEAD: &str = "2222222222222222222222222222222222222222";
const APP_BASE: &str = "3333333333333333333333333333333333333333";
const APP_HEAD: &str = "4444444444444444444444444444444444444444";
const LIB_HEAD: &str = "5555555555555555555555555555555555555555";

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

/// Step 1.2(a): configured URLs equal the committed URLs. The dependency
/// preflight reuses each member's own read and the proof accepts each member's
/// push, so a whole push makes N+1 reads and N+1 pushes. `lib` is pushed
/// although its remote already holds its commit.
#[test]
fn whole_push_reads_and_pushes_each_destination_once_when_remotes_equal_committed_urls() {
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
            push_to(&lib, LIB_SSH, LIB_HEAD),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ]
    );
    assert_eq!(fixture.backend.remote_reads().len(), MEMBERS + 1);
    assert_eq!(fixture.backend.prepared_pushes().len(), MEMBERS + 1);
    assert_eq!(
        fixture.backend.advertised_ref(APP_SSH, MAIN).as_deref(),
        Some(APP_HEAD)
    );
    assert_eq!(
        fixture.backend.advertised_ref(ROOT_SSH, MAIN).as_deref(),
        Some(ROOT_HEAD)
    );
}

/// Step 1.2(b): a root-only push has no member read to reuse and no member push
/// to accept, so every dependency is read before the transfer and again after
/// it: 2N+1 reads and one push.
#[test]
fn root_only_push_reads_every_dependency_before_and_after_the_transfer() {
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
            read_at(root, APP_SSH, &app),
            read_at(root, LIB_SSH, &lib),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ]
    );
    assert_eq!(fixture.backend.remote_reads().len(), 2 * MEMBERS + 1);
    assert_eq!(fixture.backend.prepared_pushes().len(), 1);
}

/// Step 1.2(c), the gap step 2.1 closes: member remotes are the https form of
/// their SSH committed URLs. Both shortcuts compare URLs exactly and miss, so
/// the preflight and the proof each read every dependency over SSH: 3N+1
/// reads, 2N of them SSH.
#[test]
fn https_member_remotes_still_read_every_dependency_at_its_ssh_committed_url() {
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
            read_at(root, APP_SSH, &app),
            read_at(root, LIB_SSH, &lib),
            push_to(&app, APP_HTTPS, APP_HEAD),
            push_to(&lib, LIB_HTTPS, LIB_HEAD),
            read_at(root, APP_SSH, &app),
            read_at(root, LIB_SSH, &lib),
            push_to(root, ROOT_HTTPS, ROOT_HEAD),
        ]
    );
    let reads = fixture.backend.remote_reads();
    let ssh = reads
        .iter()
        .filter(|call| matches!(call, RemoteCall::Read { url, .. } if crate::git::uses_ssh(url)))
        .count();
    assert_eq!((reads.len(), ssh), (3 * MEMBERS + 1, 2 * MEMBERS));
    assert_eq!(fixture.backend.prepared_pushes().len(), MEMBERS + 1);
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
        let temp = TempDir::new("push-seam");
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        write_pull_fixture(
            temp.path(),
            vec![
                ("mem_app", "repos/app", APP_SSH, APP_HEAD),
                ("mem_lib", "repos/lib", LIB_SSH, LIB_HEAD),
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
        for (path, url, head) in [
            (root.clone(), root_url, ROOT_HEAD),
            (root.join("repos/app"), app_url, APP_HEAD),
            (root.join("repos/lib"), lib_url, LIB_HEAD),
        ] {
            backend.set_head(&path, "main", head);
            backend.add_remote_config(&path, ConfiguredRemote::new("origin", url));
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
        let world = crate::operation_context::TestWorld::physical();
        let services = world.context();
        handle_push_with_events_in(
            &services,
            &self.backend,
            &self.root,
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
                ..Default::default()
            },
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
