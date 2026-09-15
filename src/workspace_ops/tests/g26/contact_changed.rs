//! Push plan step 3.5 (`dev-docs/GwzUrlSchemePushPlan.md`): contact only what
//! changed. Unless a push asks to check every remote, each selected repository
//! is classified against its last-known ref before any read (§3.5 rule 2): one
//! that equals that ref or is behind it is `noop`, with its reason, and is not
//! contacted. A contacted root still proves every dependency in the operation
//! (D10), and `always` reads every repository once (rule 3).
use super::*;

const UP_TO_DATE: &str = "up to date with origin/main as of the last fetch or push";
const BEHIND: &str = "behind origin/main as of the last fetch or push";
const ON_ORIGIN: &str = "already on origin";

/// Step 3.5 (§2.3, §3.6): nothing changed since the last fetch or push, so the
/// push contacts no repository. Every row and the aggregate are `noop`, each
/// row with its reason.
#[test]
fn nothing_changed_makes_no_transport_calls() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH).unchanged();

    let response = fixture.push(None);

    assert_eq!(fixture.backend.remote_calls(), Vec::new());
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Noop
    );
    assert_eq!(
        rows(&response),
        [
            noop("mem_app", UP_TO_DATE),
            noop("mem_lib", UP_TO_DATE),
            noop("@root", UP_TO_DATE),
        ]
    );
}

/// Step 3.5 (§2.3): `app` and the root changed, so only those two are read and
/// pushed. `lib` is not contacted, and is read once as the root's dependency
/// (D10): N+3 calls. `lib`'s row keeps its reason, which both CLIs count in
/// their summary line as not checked for changes.
#[test]
fn one_member_and_the_root_changed_contacts_those_two_and_reads_the_other_dependency_once() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH).app_and_root_changed();
    let root = fixture.root.as_path();
    let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));

    let response = fixture.push(None);

    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_SSH, &app),
            read_at(root, ROOT_SSH, root),
            read_at(root, LIB_SSH, &lib),
            push_to(&app, APP_SSH, APP_HEAD),
            push_to(root, ROOT_SSH, ROOT_HEAD),
        ]
    );
    assert_eq!(fixture.backend.remote_calls().len(), MEMBERS + 3);
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        rows(&response),
        [
            pushed("mem_app"),
            noop("mem_lib", UP_TO_DATE),
            pushed("@root")
        ]
    );
    assert_eq!(counted_as_unchecked(&response), 1);
}

/// Step 3.5 (§3.5 rule 3): `always` skips the classification, so the counts
/// are step 3.3's whatever the last-known refs say. With nothing changed, each
/// repository is read once, nothing is pushed and the root is proven from those
/// reads: N+1 calls. With `app` and the root changed: N+3 calls.
#[test]
fn always_gives_the_check_once_counts() {
    for changed in [false, true] {
        let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
        let fixture = if changed {
            fixture.app_and_root_changed()
        } else {
            fixture.unchanged()
        };
        let root = fixture.root.as_path();
        let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));
        let mut calls = vec![
            read_at(&app, APP_SSH, &app),
            read_at(&lib, LIB_SSH, &lib),
            read_at(root, ROOT_SSH, root),
        ];
        let mut expected = [
            noop("mem_app", ON_ORIGIN),
            noop("mem_lib", ON_ORIGIN),
            noop("@root", ON_ORIGIN),
        ];
        if changed {
            calls.extend([
                push_to(&app, APP_SSH, APP_HEAD),
                push_to(root, ROOT_SSH, ROOT_HEAD),
            ]);
            expected[0] = pushed("mem_app");
            expected[2] = pushed("@root");
        }

        let response = fixture.push_with(None, None, Some(crate::RemoteCheck::Always));

        assert_eq!(fixture.backend.remote_calls(), calls, "changed: {changed}");
        assert_eq!(rows(&response), expected, "changed: {changed}");
    }
}

/// Step 3.5 (§3.5, row status): a dry run reports the classification with no
/// transport calls, the unchanged row with status `noop` and its reason.
/// `always` skips the classification, so every row stays planned.
#[test]
fn a_dry_run_reports_the_classification_with_no_transport_calls() {
    for (remote_check, lib_row) in [
        (None, noop("mem_lib", UP_TO_DATE)),
        (Some(crate::RemoteCheck::Always), planned("mem_lib")),
    ] {
        let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH).app_and_root_changed();
        let mut request = PublicationFixture::request(None, None, remote_check);
        request.meta.dry_run = Some(true);

        let response = fixture.run(request);

        assert_eq!(
            fixture.backend.remote_calls(),
            Vec::new(),
            "{remote_check:?}"
        );
        assert_eq!(
            rows(&response),
            [planned("mem_app"), lib_row, planned("@root")],
            "{remote_check:?}"
        );
    }
}

/// Step 3.5 (D6): `policy.destructive = Allow` does not make a push forced, so
/// a branch behind its last-known ref is `noop` without contact.
#[test]
fn a_branch_behind_its_last_known_ref_is_noop_without_contact_when_destruction_is_allowed() {
    let fixture = behind_fixture();
    let mut request = PublicationFixture::request(selected(&["mem_app"]), None, None);
    let policy = request.meta.policy.as_mut().unwrap();
    policy.destructive = Some(crate::DestructiveBehavior::Allow);

    let response = fixture.run(request);

    assert_eq!(rows(&response), [noop("mem_app", BEHIND)]);
    assert_eq!(fixture.backend.remote_calls(), Vec::new());
}

/// Step 3.5 (§3.5 rule 2): the same branch pushed with a `+` refspec is
/// contacted, and its forced push rewinds the remote.
#[test]
fn a_forced_refspec_on_a_branch_behind_its_last_known_ref_is_contacted() {
    let fixture = behind_fixture();
    let app = fixture.root.join("repos/app");

    fixture.push_with(selected(&["mem_app"]), Some(FORCED), None);

    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_SSH, &app),
            forced_push_to(&app, APP_SSH, APP_BASE),
        ]
    );
}

/// Step 3.5 (§3.5 rule 2): a `+` refspec is contacted even when its last-known
/// ref equals the source. The remote has moved on to a descendant, so the
/// forced push goes ahead and rewinds it.
#[test]
fn a_forced_refspec_with_an_equal_last_known_ref_is_contacted_and_pushed() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    let app = fixture.root.join("repos/app");
    fixture
        .backend
        .set_last_known(&app, "origin", MAIN, APP_HEAD);
    fixture
        .backend
        .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_NEXT)]);

    fixture.push_with(selected(&["mem_app"]), Some(FORCED), None);

    assert_eq!(
        fixture.backend.remote_calls(),
        vec![
            read_at(&app, APP_SSH, &app),
            forced_push_to(&app, APP_SSH, APP_HEAD),
        ]
    );
    assert_eq!(
        fixture.backend.advertised_ref(APP_SSH, MAIN).as_deref(),
        Some(APP_HEAD)
    );
}

/// Step 3.5 (§3.5 rule 3): under `always`, a published root is proven after the
/// member transfers, from the reads made before them where those show each
/// commit available (D9). `app` was rewound, locally and on its remote, below
/// the commit the published root names, so its dependency is read again and the
/// root is refused with today's message, and nothing is pushed, in a whole push
/// and in a root-only push. Without the rewind the root is `noop`, with no extra
/// read.
#[test]
fn always_proves_a_published_root_and_refuses_it_when_a_dependency_was_rewound() {
    for rewound in [true, false] {
        for whole in [true, false] {
            let case = format!("rewound: {rewound}, whole push: {whole}");
            let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH).unchanged();
            let root = fixture.root.as_path();
            let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));
            if rewound {
                fixture.backend.set_head(&app, "main", APP_BASE);
                fixture
                    .backend
                    .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_BASE)]);
                fixture
                    .backend
                    .set_last_known(&app, "origin", MAIN, APP_BASE);
            }
            let (selection, mut reads) = if whole {
                let reads = vec![
                    read_at(&app, APP_SSH, &app),
                    read_at(&lib, LIB_SSH, &lib),
                    read_at(root, ROOT_SSH, root),
                ];
                (None, reads)
            } else {
                let reads = vec![
                    read_at(root, ROOT_SSH, root),
                    read_at(root, APP_SSH, &app),
                    read_at(root, LIB_SSH, &lib),
                ];
                (root_only(), reads)
            };
            if rewound {
                reads.push(read_at(root, APP_SSH, &app));
            }

            let response = fixture.push_with(selection, None, Some(crate::RemoteCheck::Always));

            assert_eq!(fixture.backend.remote_calls(), reads, "{case}");
            if rewound {
                assert_root_refused(&fixture, &response);
                let row = response.response.members.last().unwrap();
                let message = &row.error.as_ref().unwrap().message;
                assert!(
                    message.ends_with(
                        "; publish the member, or fetch its advertised history and retry"
                    ),
                    "{case}: {message}"
                );
            } else {
                assert_eq!(
                    rows(&response).last(),
                    Some(&noop("@root", ON_ORIGIN)),
                    "{case}"
                );
            }
        }
    }
}

/// Step 3.5 (§3.5): `app` is contacted when it has no last-known ref, when its
/// push URL reaches another repository than its fetch URL, and when an ancestry
/// answer fails, though each last-known ref would otherwise leave it unchanged.
#[test]
fn no_last_known_ref_a_push_url_to_another_repository_or_an_ancestry_error_contacts_the_remote() {
    let plain = ConfiguredRemote::new("origin", APP_SSH);
    let fork = ConfiguredRemote {
        push_url: Some(APP_FORK.to_owned()),
        ..plain.clone()
    };
    for (case, origin, last_known, ancestry_error, read_url) in [
        ("no last-known ref", plain.clone(), None, false, APP_SSH),
        ("a fork push URL", fork, Some(APP_HEAD), false, APP_FORK),
        ("an ancestry error", plain, Some(APP_BASE), true, APP_SSH),
    ] {
        let fixture = PublicationFixture::with_app(ROOT_SSH, APP_SSH, origin, LIB_SSH);
        let app = fixture.root.join("repos/app");
        if let Some(object) = last_known {
            fixture.backend.set_last_known(&app, "origin", MAIN, object);
        }
        if ancestry_error {
            fixture
                .backend
                .set_ancestry(APP_BASE, APP_HEAD, Err("shallow history"));
        }

        fixture.push_with(selected(&["mem_app"]), None, None);

        assert_eq!(
            fixture.backend.remote_calls().first(),
            Some(&read_at(&app, read_url, &app)),
            "{case}"
        );
    }
}

/// Step 3.5 (§3.5 "same repository, between destinations"): a push URL that is
/// the https form of the SSH fetch URL reaches the same repository, so an equal
/// last-known ref still leaves `app` uncontacted.
#[test]
fn a_push_url_in_the_other_scheme_reaches_the_same_repository_and_is_not_contacted() {
    let origin = ConfiguredRemote {
        push_url: Some(APP_HTTPS.to_owned()),
        ..ConfiguredRemote::new("origin", APP_SSH)
    };
    let fixture = PublicationFixture::with_app(ROOT_SSH, APP_SSH, origin, LIB_SSH);
    let app = fixture.root.join("repos/app");
    fixture
        .backend
        .set_last_known(&app, "origin", MAIN, APP_HEAD);

    let response = fixture.push_with(selected(&["mem_app"]), None, None);

    assert_eq!(rows(&response), [noop("mem_app", UP_TO_DATE)]);
    assert_eq!(fixture.backend.remote_calls(), Vec::new());
}

/// Step 3.5 (§3.5 rule 2, D10, §3.6): the root's last-known ref is behind its
/// head, so the root is contacted, and rule 1 finds it already on origin. It is
/// still proven: `app` and `lib` are not contacted, and each is read once as a
/// dependency before the transfers. When `app`'s remote lacks the locked
/// commit, that read does not prove it, so it is read again (D9) and the root
/// is refused with the §3.6 remedies, and `app` stays `noop` with its reason.
#[test]
fn a_contacted_root_already_on_origin_is_proven_and_refused_when_a_dependency_is_missing() {
    for missing in [false, true] {
        let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
        let root = fixture.root.as_path();
        let (app, lib) = (root.join("repos/app"), root.join("repos/lib"));
        let app_commit = if missing { APP_BASE } else { APP_HEAD };
        fixture.backend.set_head(&app, "main", app_commit);
        fixture
            .backend
            .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, app_commit)]);
        fixture.backend.serve(&[ROOT_SSH], &[(MAIN, ROOT_HEAD)]);
        fixture.last_known([ROOT_BASE, app_commit, LIB_HEAD]);
        let mut calls = vec![
            read_at(root, ROOT_SSH, root),
            read_at(root, APP_SSH, &app),
            read_at(root, LIB_SSH, &lib),
        ];
        if missing {
            calls.push(read_at(root, APP_SSH, &app));
        }

        let response = fixture.push(None);

        assert_eq!(fixture.backend.remote_calls(), calls, "missing: {missing}");
        let rows = rows(&response);
        assert_eq!(
            rows[..2],
            [noop("mem_app", UP_TO_DATE), noop("mem_lib", UP_TO_DATE)],
            "missing: {missing}"
        );
        if missing {
            assert_root_refused(&fixture, &response);
            let row = response.response.members.last().unwrap();
            let message = &row.error.as_ref().unwrap().message;
            assert!(
                message.contains(&format!(
                    "cannot prove member mem_app commit {APP_HEAD} is available"
                )),
                "{message}"
            );
            assert!(message.contains("gwz push --check-remotes"), "{message}");
        } else {
            assert_eq!(rows[2], noop("@root", ON_ORIGIN));
        }
    }
}

/// Step 3.5 (§3.7): a last-known ref left behind the remote costs one contact,
/// which finds the head already on origin and pushes nothing.
#[test]
fn a_last_known_ref_left_behind_contacts_the_remote_then_finds_nothing_to_push() {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    let app = fixture.root.join("repos/app");
    fixture
        .backend
        .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
    fixture
        .backend
        .set_last_known(&app, "origin", MAIN, APP_BASE);

    let response = fixture.push_with(selected(&["mem_app"]), None, None);

    assert_eq!(
        fixture.backend.remote_calls(),
        vec![read_at(&app, APP_SSH, &app)]
    );
    assert_eq!(rows(&response), [noop("mem_app", ON_ORIGIN)]);
}

/// `app`'s branch is one behind its last-known ref, which its remote holds.
fn behind_fixture() -> PublicationFixture {
    let fixture = PublicationFixture::new(ROOT_SSH, APP_SSH, LIB_SSH);
    let app = fixture.root.join("repos/app");
    fixture.backend.set_head(&app, "main", APP_BASE);
    fixture
        .backend
        .set_last_known(&app, "origin", MAIN, APP_HEAD);
    fixture
        .backend
        .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
    fixture
}

impl PublicationFixture {
    /// Record `main`'s last-known ref for the root, `app` and `lib`, in turn.
    fn last_known(&self, objects: [&str; 3]) {
        let paths = [
            self.root.clone(),
            self.root.join("repos/app"),
            self.root.join("repos/lib"),
        ];
        for (path, object) in paths.iter().zip(objects) {
            self.backend.set_last_known(path, "origin", MAIN, object);
        }
    }

    /// Nothing changed since the last push: each remote holds its repository's
    /// head, and each last-known ref records it.
    fn unchanged(self) -> Self {
        self.backend
            .serve(&[APP_SSH, APP_HTTPS], &[(MAIN, APP_HEAD)]);
        self.backend.serve(&[ROOT_SSH], &[(MAIN, ROOT_HEAD)]);
        self.last_known([ROOT_HEAD, APP_HEAD, LIB_HEAD]);
        self
    }

    /// `app` and the root are one ahead of what their remotes hold and last
    /// reported; `lib` is unchanged.
    fn app_and_root_changed(self) -> Self {
        self.last_known([ROOT_BASE, APP_BASE, LIB_HEAD]);
        self
    }
}

/// A row's target, status, planned action and reason.
type Row = (
    String,
    crate::MemberStatus,
    Option<crate::PlannedAction>,
    Option<String>,
);

fn rows(response: &crate::PushResponse) -> Vec<Row> {
    response
        .response
        .members
        .iter()
        .map(|row| {
            let planned = row.planned.as_ref();
            (
                row.member_id.clone(),
                row.status,
                planned.map(|planned| planned.action),
                planned.and_then(|planned| planned.message.clone()),
            )
        })
        .collect()
}

fn noop(target: &str, reason: &str) -> Row {
    (
        target.to_owned(),
        crate::MemberStatus::Noop,
        Some(crate::PlannedAction::Noop),
        Some(reason.to_owned()),
    )
}

fn planned(target: &str) -> Row {
    (
        target.to_owned(),
        crate::MemberStatus::Planned,
        Some(crate::PlannedAction::Push),
        Some("push to origin".to_owned()),
    )
}

fn pushed(target: &str) -> Row {
    (target.to_owned(), crate::MemberStatus::Ok, None, None)
}

/// The rows both CLIs count in their summary line as not checked for changes:
/// gwz-cli picks `noop` rows by status (`gwz-cli/src/pushargs.rs`), gwz-py by
/// planned action (`gwz-py/src/gwz/cli_render_parts/push.py`), and both by the
/// reason's ending.
fn counted_as_unchecked(response: &crate::PushResponse) -> usize {
    rows(response)
        .into_iter()
        .filter(|(_, status, action, reason)| {
            *status == crate::MemberStatus::Noop
                && *action == Some(crate::PlannedAction::Noop)
                && reason
                    .as_deref()
                    .is_some_and(|reason| reason.ends_with("as of the last fetch or push"))
        })
        .count()
}

fn forced_push_to(path: &Path, url: &str, commit: &str) -> RemoteCall {
    RemoteCall::Push {
        path: path.to_path_buf(),
        remote: "origin".to_owned(),
        url: url.to_owned(),
        refspecs: vec![format!("+{commit}:{MAIN}")],
    }
}
