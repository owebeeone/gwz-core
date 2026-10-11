//! `tag_create` and `tag_delete`: every condition of the route rule picks the
//! Git CLI.

use super::support::*;
use crate::git::gitbackend::native_commit::Route;
use crate::git::gitbackend::native_commit::RoutePolicy;
use crate::git::gitbackend::native_commit::route::CliReason;
use crate::git::gitbackend::*;

fn create(lab: &Lab, name: &str, message: Option<&str>, signed: bool) -> Route {
    lab.backend
        .tag_create_route(&lab.path, name, message, signed)
}

#[test]
fn plain_tags_are_native() {
    hermetic!();
    let lab = lab();
    assert_eq!(create(&lab, "v1", None, false), Route::Native);
    assert_eq!(create(&lab, "v1", Some("note"), false), Route::Native);
    assert_eq!(lab.backend.tag_delete_route(&lab.path, "v1"), Route::Native);
    let forced = lab.backend.with_route_policy(RoutePolicy::CLI_ONLY);
    assert_eq!(
        forced.tag_create_route(&lab.path, "v1", None, false),
        Route::Cli(CliReason::Forced)
    );
    assert_eq!(
        forced.tag_delete_route(&lab.path, "v1"),
        Route::Cli(CliReason::Forced)
    );
}

#[test]
fn reference_transaction_picks_the_cli_for_create_and_delete_and_runs() {
    hermetic!();
    let lab = lab();
    let marker = lab.hook("reference-transaction");
    let hook = Route::Cli(CliReason::Hook("reference-transaction".to_owned()));
    assert_eq!(create(&lab, "light", None, false), hook);
    assert_eq!(create(&lab, "note", Some("m"), false), hook);
    assert_eq!(lab.backend.tag_delete_route(&lab.path, "light"), hook);
    lab.backend
        .tag_create(&lab.path, "light", None, false)
        .unwrap();
    assert!(marker.exists(), "the hook ran for the create");
    std::fs::remove_file(&marker).unwrap();
    lab.backend.tag_delete(&lab.path, "light").unwrap();
    assert!(marker.exists(), "the hook ran for the delete");
}

#[test]
fn ordinary_commit_hooks_do_not_run_for_tags() {
    hermetic!();
    let lab = lab();
    let markers: Vec<_> = [
        "pre-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
    ]
    .into_iter()
    .map(|name| lab.hook(name))
    .collect();
    assert_eq!(create(&lab, "light", None, false), Route::Native);
    assert_eq!(create(&lab, "note", Some("m"), false), Route::Native);
    lab.backend
        .tag_create(&lab.path, "note", Some("m"), false)
        .unwrap();
    lab.backend.tag_delete(&lab.path, "note").unwrap();
    assert!(markers.iter().all(|marker| !marker.exists()));
}

#[test]
fn signing_picks_the_cli_for_tags() {
    hermetic!();
    let lab = lab();
    assert!(matches!(
        create(&lab, "s", Some("m"), true),
        Route::Cli(CliReason::Signing(_))
    ));
    lab.config("tag.forceSignAnnotated", "true");
    assert!(matches!(
        create(&lab, "a", Some("m"), false),
        Route::Cli(CliReason::Signing(_))
    ));
    assert_eq!(
        create(&lab, "l", None, false),
        Route::Native,
        "forceSignAnnotated leaves lightweight tags alone"
    );
    lab.config("tag.forceSignAnnotated", "false");
    lab.config("tag.gpgSign", "true");
    for message in [None, Some("m")] {
        assert!(matches!(
            create(&lab, "g", message, false),
            Route::Cli(CliReason::Signing(_))
        ));
    }
}

#[test]
fn tag_names_native_and_git_may_judge_differently_pick_the_cli() {
    hermetic!();
    let lab = lab();
    for name in [
        "",
        "HEAD",
        "-leading",
        ".hidden",
        "trailing.",
        "a..b",
        "a b",
        "x.lock",
        "a//b",
        "a/.b",
        "a@{b}",
        "caf\u{e9}",
        "tilde~1",
        "star*",
        "colon:name",
    ] {
        assert_eq!(
            create(&lab, name, None, false),
            Route::Cli(CliReason::TagName),
            "{name:?}"
        );
        assert_eq!(
            lab.backend.tag_delete_route(&lab.path, name),
            Route::Cli(CliReason::TagName),
            "delete {name:?}"
        );
    }
    for name in ["v1", "release-1.0_rc+1", "nested/name"] {
        assert_eq!(create(&lab, name, None, false), Route::Native, "{name:?}");
    }
}

#[test]
fn an_unborn_head_and_odd_messages_pick_the_cli() {
    hermetic!();
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("unborn");
    unborn(&path);
    let backend = Git2Backend::new();
    assert_eq!(
        backend.tag_create_route(&path, "v1", None, false),
        Route::Cli(CliReason::HeadShape)
    );
    let lab = lab();
    for message in ["", "  \n", "# only comment\n", "nul\0byte"] {
        assert!(
            matches!(
                create(&lab, "t", Some(message), false),
                Route::Cli(CliReason::Message(_))
            ),
            "{message:?}"
        );
    }
    lab.config("core.commentChar", ";");
    assert!(matches!(
        create(&lab, "t", Some("m"), false),
        Route::Cli(CliReason::Message(_))
    ));
    assert_eq!(create(&lab, "t", None, false), Route::Native);
}
