//! Push plan step 3.4 (`dev-docs/GwzUrlSchemePushPlan.md`): a push's
//! pre-transfer reads and its root proof's reads run through
//! `par_map_per_host`, under the policy of its transfers. The rows, the
//! refusals and the failure aggregation are those of reads made in turn, and a
//! destination is read once however many targets need it.
use super::*;

/// `app` on a second host, committed and configured at this URL.
const APP_URL: &str = APP_UNKNOWN_HOST_SSH;

/// Reads to two hosts overlap, and the rows keep their order and outcomes: the
/// members in selection order, then the root.
#[test]
fn concurrent_reads_keep_the_report_order() {
    let fixture = two_host_fixture().hold_reads(2, 1);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    let response = fixture.push_under(None, policy(None, 1));

    let rows: Vec<_> = response
        .response
        .members
        .iter()
        .map(|row| (row.member_id.as_str(), row.status))
        .collect();
    assert_eq!(
        rows,
        [
            ("mem_app", crate::MemberStatus::Ok),
            ("mem_lib", crate::MemberStatus::Noop),
            ("@root", crate::MemberStatus::Ok),
        ]
    );
    assert_eq!(
        calls_by_host(fixture.backend.remote_calls()),
        calls_by_host(vec![
            read_at(&app, APP_URL, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
            push_to(&app, APP_URL, APP_HEAD),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ])
    );
    assert_eq!(fixture.backend.read_peak(), 2);
}

/// Every failed read is reported on each target that needs its destination, as
/// the first failure in the order that target needs its reads, and nothing is
/// transferred. `app`'s destination is its own and the root's dependency: it is
/// read once, and its failure refuses both rows. The root reads its own
/// destination first, so when that fails too, the root reports it.
#[test]
fn every_failed_read_is_reported_and_no_transfer_starts() {
    let (app_failure, root_failure) = ("app unreachable", "root unreachable");
    for (failing, root_reports) in [
        (&[(APP_URL, app_failure)][..], app_failure),
        (
            &[(APP_URL, app_failure), (ROOT_SSH, root_failure)][..],
            root_failure,
        ),
    ] {
        let fixture = two_host_fixture().hold_reads(2, 1);
        for (url, detail) in failing {
            fixture.backend.fail_reads(url, detail);
        }

        let response = fixture.push_under(None, policy(None, 1));

        let rows: Vec<_> = response
            .response
            .members
            .iter()
            .map(|row| {
                let message = row.error.as_ref().map(|error| error.message.clone());
                (row.member_id.as_str(), row.status, message)
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "mem_app",
                    crate::MemberStatus::Rejected,
                    Some(format!("member 'mem_app' at 'repos/app': {app_failure}")),
                ),
                ("mem_lib", crate::MemberStatus::Planned, None),
                (
                    "@root",
                    crate::MemberStatus::Rejected,
                    Some(format!("workspace root '@root' at '.': {root_reports}")),
                ),
            ],
            "{failing:?}"
        );
        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Rejected
        );
        assert_eq!(fixture.backend.prepared_pushes(), Vec::new(), "{failing:?}");
        for (url, _) in failing {
            assert_eq!(reads_of(&fixture, url), 1, "{url} in {failing:?}");
        }
        assert_eq!(fixture.backend.read_peak(), 2, "{failing:?}");
    }
}

/// The reads run under the policy of the transfers: at most `--max-per-host` at
/// once to one host, and `--jobs` in all. `app` is read at its https remote and
/// at its committed URL on one host, and `lib` and the root on github.com, so
/// four reads could overlap. Where a limit binds, reads are held until one more
/// read than it allows has started, so a broken limit shows in the peak.
#[test]
fn concurrent_reads_hold_the_per_host_limit_and_jobs() {
    for (jobs, per_host, hold, peak) in [(None, 1, 3, 2), (Some(1), 2, 2, 1), (None, 2, 4, 4)] {
        let fixture = PublicationFixture::with_app(
            ROOT_SSH,
            APP_UNKNOWN_HOST_SSH,
            ConfiguredRemote::new("origin", APP_UNKNOWN_HOST_HTTPS),
            LIB_SSH,
        )
        .hold_reads(hold, 1);
        fixture.backend.serve(
            &[APP_UNKNOWN_HOST_SSH, APP_UNKNOWN_HOST_HTTPS],
            &[(MAIN, APP_BASE)],
        );

        let response = fixture.push_under(None, policy(jobs, per_host));

        let case = format!("jobs {jobs:?}, per host {per_host}");
        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok,
            "{case}"
        );
        assert_eq!(fixture.backend.remote_reads().len(), 5, "{case}");
        assert_eq!(fixture.backend.read_peak(), peak, "{case}");
    }
}

/// A forced push voids the evidence from before the transfers, so the root
/// proof reads every dependency after them. `app`'s read and `lib`'s, on two
/// hosts, overlap, and still come after the member transfers and before the
/// root's.
#[test]
fn root_proof_reads_run_concurrently_after_the_member_transfers() {
    let fixture = two_host_fixture().hold_reads(1, 2);
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    let response = fixture.push_under(Some(FORCED), policy(None, 1));

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    let forced = |path: &Path, url: &str, commit: &str| RemoteCall::Push {
        path: path.to_path_buf(),
        remote: "origin".to_owned(),
        url: url.to_owned(),
        refspecs: vec![format!("+{commit}:{MAIN}")],
    };
    assert_eq!(
        calls_by_host(fixture.backend.remote_calls()),
        calls_by_host(vec![
            read_at(&app, APP_URL, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
            forced(&app, APP_URL, APP_HEAD),
            read_at(root, APP_URL, &app),
            read_at(root, LIB_SSH, &lib),
            forced(root, ROOT_SSH, ROOT_HEAD),
        ])
    );
    assert_eq!(fixture.backend.post_push_read_peak(), 2);
}

/// `app` committed and configured at `APP_URL`, whose repository holds
/// `APP_BASE`: its own read and its dependency read are one destination on
/// git.example.com, and `lib` and the root are read on github.com.
fn two_host_fixture() -> PublicationFixture {
    let fixture = PublicationFixture::with_members(
        ROOT_SSH,
        (APP_URL, ConfiguredRemote::new("origin", APP_URL)),
        (LIB_SSH, ConfiguredRemote::new("origin", LIB_SSH)),
    );
    fixture.backend.serve(&[APP_URL], &[(MAIN, APP_BASE)]);
    fixture
}

/// `--jobs`, the default when `None`, and `--max-per-host`.
fn policy(jobs: Option<i64>, per_host: i64) -> crate::OperationPolicy {
    crate::OperationPolicy {
        concurrency: jobs,
        max_connections_per_host: Some(per_host),
        ..Default::default()
    }
}

/// How many advertisement reads of `url` the double recorded.
fn reads_of(fixture: &PublicationFixture, url: &str) -> usize {
    fixture
        .backend
        .remote_reads()
        .iter()
        .filter(|call| matches!(call, RemoteCall::Read { url: read, .. } if read == url))
        .count()
}

impl PublicationFixture {
    /// Hold the reads made before any push until `before` of them have
    /// started, and those made after one until `after` have.
    fn hold_reads(mut self, before: usize, after: usize) -> Self {
        self.backend = self
            .backend
            .with_read_overlap(before)
            .with_post_push_read_overlap(after);
        self
    }

    /// Push the whole workspace with a request refspec, under `policy`.
    fn push_under(
        &self,
        refspec: Option<&str>,
        policy: crate::OperationPolicy,
    ) -> crate::PushResponse {
        let world = crate::operation_context::TestWorld::physical();
        handle_push_with_events_in(
            &world.context(),
            &self.backend,
            &self.root,
            crate::PushRequest {
                meta: crate::RequestMeta {
                    policy: Some(policy),
                    ..request_meta_with_workspace()
                },
                remote: None,
                refspec: refspec.map(str::to_owned),
                remote_check: None,
            },
            "op_push",
            &NullSink,
        )
        .unwrap()
    }
}
