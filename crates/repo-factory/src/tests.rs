//! Tier A tests for clean and bare construction (design §4.2/§4.3).
//!
//! Every test drives [`construct`] through the recording
//! [`RecordingBuildPort`] fake: deterministic values, no filesystem, no
//! process, no real repository. The fake records the exact port calls and
//! keeps a small faithful model of what each destination ended up holding,
//! so a test can assert both the ordering the design fixes and the state
//! that survives a failure.

use std::path::PathBuf;

use gwz_repo_contract::contract_tests::oid;
use gwz_repo_contract::{HeadState, ObjectFormat, ObjectId, RepoKey};

use crate::test_support::{BuildCall, RecordingBuildPort};
use crate::{
    BuildError, BuiltRepo, CapturedRepo, FactoryError, FactoryMode, FactoryReport, FactoryRequest,
    construct, origin_is_kept,
};

const SHA1: ObjectFormat = ObjectFormat::Sha1;

// ---------------------------------------------------------------- fixtures

fn member(id: &str) -> RepoKey {
    RepoKey::Member { id: id.to_owned() }
}

fn source(path: &str) -> PathBuf {
    if path.is_empty() {
        PathBuf::from("/src")
    } else {
        PathBuf::from("/src").join(path)
    }
}

fn destination(path: &str) -> PathBuf {
    if path.is_empty() {
        PathBuf::from("/dest")
    } else {
        PathBuf::from("/dest").join(path)
    }
}

fn repo(key: RepoKey, path: &str, head: u8, branch: Option<&str>) -> CapturedRepo {
    CapturedRepo {
        key,
        source: source(path),
        destination: destination(path),
        object_format: SHA1,
        head: oid(SHA1, head),
        branch: branch.map(ToOwned::to_owned),
        origin: None,
    }
}

/// Root on `main` at 1, `app` on `main` at 2, `lib` on `topic` at 3.
fn family() -> Vec<CapturedRepo> {
    vec![
        repo(RepoKey::Root, "", 1, Some("main")),
        repo(member("app"), "app", 2, Some("main")),
        repo(member("lib"), "libs/lib", 3, Some("topic")),
    ]
}

fn request(mode: FactoryMode, vector: Vec<CapturedRepo>) -> FactoryRequest {
    FactoryRequest { mode, vector }
}

fn clean(vector: Vec<CapturedRepo>) -> FactoryRequest {
    request(FactoryMode::Clean { branch: None }, vector)
}

/// A port whose sources hold every frozen commit of `request`.
fn planted(request: &FactoryRequest) -> RecordingBuildPort {
    let mut port = RecordingBuildPort::new();
    for repo in &request.vector {
        port.set_object(&repo.source, repo.head.clone());
    }
    port
}

fn object_exists(path: &str, head: u8) -> BuildCall {
    BuildCall::ObjectExists {
        repository: source(path),
        oid: oid(SHA1, head),
    }
}

fn init(path: &str, bare: bool) -> BuildCall {
    BuildCall::InitRepository {
        destination: destination(path),
        bare,
    }
}

fn transfer(path: &str, head: u8) -> BuildCall {
    BuildCall::TransferObjects {
        source: source(path),
        destination: destination(path),
        refspecs: vec![oid(SHA1, head).to_hex()],
    }
}

fn set_head(path: &str, branch: &str, head: u8, checkout: bool) -> BuildCall {
    BuildCall::SetHead {
        repository: destination(path),
        branch: branch.to_owned(),
        target: oid(SHA1, head),
        checkout,
    }
}

fn bare_path(path: &str) -> PathBuf {
    destination(path).join(".git")
}

fn keys(report: &FactoryReport) -> Vec<RepoKey> {
    report.built.clone()
}

fn built(report: &FactoryReport, key: &RepoKey) -> BuiltRepo {
    report
        .repositories
        .iter()
        .find(|repo| repo.key == *key)
        .cloned()
        .unwrap_or_else(|| panic!("{key} is missing from the report"))
}

// ------------------------------------------------------------------ clean

#[test]
fn clean_construction_builds_every_repository_at_its_frozen_commit() {
    let request = clean(family());
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("clean construction");

    assert_eq!(
        port.calls(),
        [
            object_exists("", 1),
            object_exists("app", 2),
            object_exists("libs/lib", 3),
            init("", false),
            transfer("", 1),
            set_head("", "main", 1, true),
            init("app", false),
            transfer("app", 2),
            set_head("app", "main", 2, true),
            init("libs/lib", false),
            transfer("libs/lib", 3),
            set_head("libs/lib", "topic", 3, true),
        ]
    );
    assert_eq!(keys(&report), [RepoKey::Root, member("app"), member("lib")]);
    assert!(report.is_complete(&request));

    let lib = built(&report, &member("lib"));
    assert_eq!(lib.destination, destination("libs/lib"));
    assert_eq!(lib.git_dir, bare_path("libs/lib"));
    assert!(!lib.bare);
    assert_eq!(
        lib.head,
        HeadState::Attached {
            branch: "topic".to_owned(),
            target: oid(SHA1, 3),
        }
    );
    assert_eq!(lib.branches, ["topic".to_owned()]);
    assert_eq!(lib.origin, None);

    // The fake's model of the destination, not just the call log.
    let state = port.repository(destination("libs/lib")).expect("built");
    assert!(!state.bare);
    assert_eq!(state.object_format, Some(SHA1));
    assert_eq!(state.transferred, [oid(SHA1, 3).to_hex()]);
    assert_eq!(state.branches.get("topic"), Some(&oid(SHA1, 3)));
    assert!(state.head.is_some());
    assert!(state.holds_object(&oid(SHA1, 3)));
}

#[test]
fn the_installer_follow_ups_put_lock_recapture_first_and_the_manifest_last() {
    let request = clean(family());
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("clean construction");

    assert_eq!(
        report.follow_ups,
        [
            crate::InstallerFollowUp::RecaptureLock,
            crate::InstallerFollowUp::WriteFamilyRoot,
            crate::InstallerFollowUp::WriteManifestLast,
        ]
    );
}

#[test]
fn the_root_is_constructed_before_its_members_whatever_the_vector_order() {
    let mut vector = family();
    vector.rotate_left(1); // app, lib, root
    let request = clean(vector);
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("clean construction");

    assert_eq!(keys(&report), [RepoKey::Root, member("app"), member("lib")]);
    let inits: Vec<&BuildCall> = port
        .calls()
        .iter()
        .filter(|call| matches!(call, BuildCall::InitRepository { .. }))
        .collect();
    assert_eq!(
        inits,
        [
            &init("", false),
            &init("app", false),
            &init("libs/lib", false),
        ]
    );
}

#[test]
fn a_detached_source_head_produces_a_detached_destination() {
    let request = clean(vec![
        repo(RepoKey::Root, "", 1, Some("main")),
        repo(member("app"), "app", 2, None),
    ]);
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("clean construction");

    assert!(port.calls().contains(&BuildCall::SetDetachedHead {
        repository: destination("app"),
        target: oid(SHA1, 2),
        checkout: true,
    }));
    let app = built(&report, &member("app"));
    assert_eq!(
        app.head,
        HeadState::Detached {
            target: oid(SHA1, 2)
        }
    );
    assert!(app.branches.is_empty());
}

// ----------------------------------------------------------------- branch

#[test]
fn the_requested_branch_is_created_in_every_member_at_the_frozen_commit() {
    let request = request(
        FactoryMode::Clean {
            branch: Some("lane/x".to_owned()),
        },
        family(),
    );
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("clean construction");

    for (path, head) in [("", 1u8), ("app", 2), ("libs/lib", 3)] {
        assert!(
            port.calls().contains(&BuildCall::RefExists {
                repository: source(path),
                name: "refs/heads/lane/x".to_owned(),
            }),
            "{path} was not checked for the requested branch"
        );
        assert!(
            port.calls().contains(&set_head(path, "lane/x", head, true)),
            "{path} did not get lane/x at its frozen commit"
        );
        let state = port.repository(destination(path)).expect("built");
        assert_eq!(state.branches.get("lane/x"), Some(&oid(SHA1, head)));
        // Only the requested branch is created; the frozen branch is not
        // re-created behind the operator's back.
        assert_eq!(state.branches.len(), 1);
    }
    assert_eq!(
        report.follow_ups.first(),
        Some(&crate::InstallerFollowUp::RecaptureDesiredBranches {
            branch: "lane/x".to_owned(),
        })
    );
    assert_eq!(
        report.follow_ups.last(),
        Some(&crate::InstallerFollowUp::WriteManifestLast)
    );
}

#[test]
fn a_branch_that_already_exists_refuses_the_whole_construction_aggregating() {
    let request = request(
        FactoryMode::Clean {
            branch: Some("lane/x".to_owned()),
        },
        family(),
    );
    let mut port = planted(&request);
    port.set_ref(source("app"), "refs/heads/lane/x");
    port.set_ref(source("libs/lib"), "refs/heads/lane/x");

    let error = construct(&request, &mut port).unwrap_err();

    assert_eq!(
        error,
        FactoryError::BranchExists {
            conflicts: vec![
                (member("app"), "lane/x".to_owned()),
                (member("lib"), "lane/x".to_owned()),
            ],
        }
    );
    assert!(
        !port
            .calls()
            .iter()
            .any(|call| matches!(call, BuildCall::InitRepository { .. })),
        "the collision must refuse before anything is created"
    );
    assert!(port.repositories().is_empty());
    // Every repository is checked, not just the first conflict.
    assert_eq!(
        port.calls()
            .iter()
            .filter(|call| matches!(call, BuildCall::RefExists { .. }))
            .count(),
        3
    );
}

// ------------------------------------------------------------------- bare

#[test]
fn bare_construction_makes_real_bare_repositories_in_the_same_layout() {
    let request = request(FactoryMode::Bare, family());
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("bare construction");

    assert_eq!(
        port.calls(),
        [
            object_exists("", 1),
            object_exists("app", 2),
            object_exists("libs/lib", 3),
            BuildCall::InitRepository {
                destination: bare_path(""),
                bare: true,
            },
            BuildCall::TransferObjects {
                source: source(""),
                destination: bare_path(""),
                refspecs: vec![oid(SHA1, 1).to_hex()],
            },
            BuildCall::SetHead {
                repository: bare_path(""),
                branch: "main".to_owned(),
                target: oid(SHA1, 1),
                checkout: false,
            },
            BuildCall::InitRepository {
                destination: bare_path("app"),
                bare: true,
            },
            BuildCall::TransferObjects {
                source: source("app"),
                destination: bare_path("app"),
                refspecs: vec![oid(SHA1, 2).to_hex()],
            },
            BuildCall::SetHead {
                repository: bare_path("app"),
                branch: "main".to_owned(),
                target: oid(SHA1, 2),
                checkout: false,
            },
            BuildCall::InitRepository {
                destination: bare_path("libs/lib"),
                bare: true,
            },
            BuildCall::TransferObjects {
                source: source("libs/lib"),
                destination: bare_path("libs/lib"),
                refspecs: vec![oid(SHA1, 3).to_hex()],
            },
            BuildCall::SetHead {
                repository: bare_path("libs/lib"),
                branch: "topic".to_owned(),
                target: oid(SHA1, 3),
                checkout: false,
            },
        ]
    );
    let lib = built(&report, &member("lib"));
    assert!(lib.bare);
    // The workspace layout is the same; only the repository is bare.
    assert_eq!(lib.destination, destination("libs/lib"));
    assert_eq!(lib.git_dir, bare_path("libs/lib"));
    let state = port.repository(bare_path("libs/lib")).expect("built");
    assert!(state.bare);
    assert_eq!(
        state.head,
        Some(HeadState::Attached {
            branch: "topic".to_owned(),
            target: oid(SHA1, 3),
        })
    );
    // Nothing writes the hub's workspace metadata here.
    assert_eq!(
        report.follow_ups,
        [
            crate::InstallerFollowUp::RecaptureLock,
            crate::InstallerFollowUp::WriteFamilyRoot,
            crate::InstallerFollowUp::WriteManifestLast,
        ]
    );
}

#[test]
fn a_bare_destination_never_checks_out_a_worktree() {
    let request = request(
        FactoryMode::Bare,
        vec![repo(RepoKey::Root, "", 1, Some("main"))],
    );
    let mut port = planted(&request);

    construct(&request, &mut port).expect("bare construction");

    assert!(
        !port.calls().iter().any(|call| matches!(
            call,
            BuildCall::SetHead { checkout: true, .. }
                | BuildCall::SetDetachedHead { checkout: true, .. }
        )),
        "a bare repository has no worktree to check out"
    );
}

// ----------------------------------------------------------------- origins

#[test]
fn a_portable_origin_is_kept_and_a_filesystem_or_credential_origin_is_dropped() {
    let mut vector = family();
    vector[0].origin = Some("https://example.invalid/root.git".to_owned());
    vector[1].origin = Some("file:///src/app".to_owned());
    vector[2].origin = Some("https://gianni:token@example.invalid/lib.git".to_owned());
    let request = clean(vector);
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("clean construction");

    assert_eq!(
        port.calls()
            .iter()
            .filter(|call| matches!(call, BuildCall::SetOrigin { .. }))
            .collect::<Vec<_>>(),
        [&BuildCall::SetOrigin {
            repository: destination(""),
            url: "https://example.invalid/root.git".to_owned(),
        }]
    );
    let root = built(&report, &RepoKey::Root);
    assert_eq!(
        root.origin.as_deref(),
        Some("https://example.invalid/root.git")
    );
    assert_eq!(root.dropped_origin, None);
    let app = built(&report, &member("app"));
    assert_eq!(app.origin, None);
    assert_eq!(app.dropped_origin.as_deref(), Some("file:///src/app"));
    let lib = built(&report, &member("lib"));
    assert_eq!(lib.origin, None);
    assert_eq!(
        lib.dropped_origin.as_deref(),
        Some("https://gianni:token@example.invalid/lib.git")
    );
    assert_eq!(
        port.repository(destination("")).expect("built").origin,
        Some("https://example.invalid/root.git".to_owned())
    );
    assert_eq!(
        port.repository(destination("app")).expect("built").origin,
        None
    );
}

#[test]
fn origin_urls_are_classified_by_their_shape() {
    for url in [
        "https://example.invalid/x.git",
        "http://example.invalid/x.git",
        "ssh://git@example.invalid/x.git",
        "git@example.invalid:owner/x.git",
        "https://gianni@example.invalid/x.git", // a user without a secret
        "git://example.invalid/x.git",
    ] {
        assert!(origin_is_kept(url), "{url} should have been kept");
    }
    for url in [
        "file:///srv/git/x.git",
        "FILE:///srv/git/x.git",
        "/srv/git/x.git",
        "./hub",
        "../gwz-dev-hub",
        "~/hub",
        "hub",
        "",
        "C:\\git\\x",
        "https://gianni:token@example.invalid/x.git",
        "ssh://gianni:token@example.invalid/x.git",
        // A non-ASCII spelling must be classified, never panic on a byte
        // slice that is not a character boundary.
        "\u{e9}toile",
        "fil\u{e9}",
    ] {
        assert!(!origin_is_kept(url), "{url} should have been dropped");
    }
}

// ---------------------------------------------------------------- failures

#[test]
fn a_builder_failure_names_the_member_and_what_was_already_built() {
    let request = clean(family());
    let mut port = planted(&request);
    let failure = BuildError::Repository {
        path: destination("app"),
        detail: "no space left on device".to_owned(),
    };
    port.fail_next_for("transfer_objects", destination("app"), failure.clone());

    let error = construct(&request, &mut port).unwrap_err();

    assert_eq!(
        error,
        FactoryError::Build {
            key: member("app"),
            error: failure,
            built: vec![RepoKey::Root],
        }
    );
    // Construction stops at the failing call; `lib` is never touched.
    assert_eq!(
        port.calls().last(),
        Some(&transfer("app", 2)),
        "no work happens after the failure"
    );
    assert!(port.repository(destination("libs/lib")).is_none());
}

#[test]
fn a_failed_construction_is_retained_for_inspection() {
    let request = clean(family());
    let mut port = planted(&request);
    port.fail_next_for(
        "set_head",
        destination("app"),
        BuildError::Failed {
            detail: "ref lock".to_owned(),
        },
    );

    let error = construct(&request, &mut port).unwrap_err();

    assert!(matches!(error, FactoryError::Build { .. }));
    // Both the completed root and the half-built member are still there.
    assert_eq!(port.repositories().len(), 2);
    let app = port.repository(destination("app")).expect("retained");
    assert_eq!(app.transferred, [oid(SHA1, 2).to_hex()]);
    assert_eq!(app.head, None);
    assert!(
        port.repository(destination(""))
            .expect("retained")
            .head
            .is_some()
    );
    // The port has no removal operation at all: nothing can be cleaned up.
    assert!(port.calls().iter().all(|call| matches!(
        call,
        BuildCall::ObjectExists { .. }
            | BuildCall::RefExists { .. }
            | BuildCall::InitRepository { .. }
            | BuildCall::TransferObjects { .. }
            | BuildCall::SetHead { .. }
            | BuildCall::SetDetachedHead { .. }
            | BuildCall::SetOrigin { .. }
    )));
}

#[test]
fn a_port_failure_during_the_preflight_reports_nothing_built() {
    let request = clean(family());
    let mut port = planted(&request);
    port.fail_next_for(
        "object_exists",
        source("app"),
        BuildError::Failed {
            detail: "unreadable".to_owned(),
        },
    );

    let error = construct(&request, &mut port).unwrap_err();

    assert_eq!(
        error,
        FactoryError::Build {
            key: member("app"),
            error: BuildError::Failed {
                detail: "unreadable".to_owned(),
            },
            built: Vec::new(),
        }
    );
    assert!(port.repositories().is_empty());
}

#[test]
fn a_missing_frozen_object_refuses_typed_before_anything_is_created() {
    let request = clean(family());
    let mut port = RecordingBuildPort::new();
    // `app`'s frozen commit is never planted in its source.
    port.set_object(source(""), oid(SHA1, 1));
    port.set_object(source("libs/lib"), oid(SHA1, 3));

    let error = construct(&request, &mut port).unwrap_err();

    assert_eq!(
        error,
        FactoryError::ObjectMissing {
            missing: vec![(member("app"), oid(SHA1, 2))],
        }
    );
    assert!(port.repositories().is_empty());
    assert!(
        !port
            .calls()
            .iter()
            .any(|call| matches!(call, BuildCall::InitRepository { .. }))
    );
}

#[test]
fn a_branch_collision_is_reported_ahead_of_a_missing_object() {
    let request = request(
        FactoryMode::Clean {
            branch: Some("lane/x".to_owned()),
        },
        family(),
    );
    let mut port = RecordingBuildPort::new();
    port.set_object(source(""), oid(SHA1, 1));
    port.set_ref(source("libs/lib"), "refs/heads/lane/x");

    let error = construct(&request, &mut port).unwrap_err();

    assert!(matches!(error, FactoryError::BranchExists { .. }));
}

// -------------------------------------------------------------- validation

#[test]
fn an_invalid_request_refuses_before_any_port_call() {
    let cases: Vec<FactoryRequest> = vec![
        // Empty vector.
        clean(Vec::new()),
        // No root in the captured vector.
        clean(vec![repo(member("app"), "app", 2, Some("main"))]),
        // The same repository twice.
        clean(vec![
            repo(RepoKey::Root, "", 1, Some("main")),
            repo(RepoKey::Root, "other", 1, Some("main")),
        ]),
        // Two members onto one destination.
        clean(vec![
            repo(RepoKey::Root, "", 1, Some("main")),
            repo(member("app"), "app", 2, Some("main")),
            repo(member("dup"), "app", 3, Some("main")),
        ]),
        // A frozen commit whose format is not the repository's.
        clean(vec![CapturedRepo {
            object_format: ObjectFormat::Sha256,
            ..repo(RepoKey::Root, "", 1, Some("main"))
        }]),
        // A blank requested branch.
        request(
            FactoryMode::Clean {
                branch: Some("   ".to_owned()),
            },
            family(),
        ),
        // A full ref name where a branch name belongs.
        request(
            FactoryMode::Clean {
                branch: Some("refs/heads/lane".to_owned()),
            },
            family(),
        ),
        // A captured branch spelled as a full ref name.
        clean(vec![repo(RepoKey::Root, "", 1, Some("refs/heads/main"))]),
    ];

    for (index, request) in cases.iter().enumerate() {
        let mut port = planted(request);
        let error = construct(request, &mut port).unwrap_err();
        assert!(
            matches!(error, FactoryError::InvalidRequest { .. }),
            "case {index} produced {error:?}"
        );
        assert!(port.calls().is_empty(), "case {index} called the port");
    }
}

#[test]
fn a_report_is_complete_only_when_every_repository_was_built() {
    let request = clean(family());
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("clean construction");
    assert!(report.is_complete(&request));

    let mut partial = report.clone();
    partial.built.pop();
    partial.repositories.pop();
    assert!(!partial.is_complete(&request));

    let mut mislabelled = report;
    mislabelled.built[1] = member("other");
    assert!(!mislabelled.is_complete(&request));
}

#[test]
fn every_built_repository_carries_its_own_frozen_commit() {
    let request = clean(family());
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("clean construction");

    for repo in &request.vector {
        let built = built(&report, &repo.key);
        let target: &ObjectId = match &built.head {
            HeadState::Attached { target, .. } | HeadState::Detached { target } => target,
            HeadState::Unborn { .. } => panic!("a built repository is never unborn"),
        };
        assert_eq!(target, &repo.head);
    }
}

#[test]
fn each_repository_is_created_in_its_own_object_format() {
    let sha256 = ObjectFormat::Sha256;
    let request = clean(vec![
        repo(RepoKey::Root, "", 1, Some("main")),
        CapturedRepo {
            object_format: sha256,
            head: oid(sha256, 2),
            ..repo(member("app"), "app", 2, Some("main"))
        },
    ]);
    let mut port = planted(&request);

    let report = construct(&request, &mut port).expect("clean construction");

    assert_eq!(
        port.repository(destination(""))
            .expect("built")
            .object_format,
        Some(SHA1)
    );
    let app = port.repository(destination("app")).expect("built");
    assert_eq!(app.object_format, Some(sha256));
    // A 32-byte digest is transferred and placed as itself; nothing
    // assumes 40 hex characters.
    assert_eq!(app.transferred, [oid(sha256, 2).to_hex()]);
    assert_eq!(app.branches.get("main"), Some(&oid(sha256, 2)));
    assert_eq!(
        built(&report, &member("app")).head,
        HeadState::Attached {
            branch: "main".to_owned(),
            target: oid(sha256, 2),
        }
    );
}
