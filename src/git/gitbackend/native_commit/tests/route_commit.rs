//! `commit`: every condition of the route rule picks the Git CLI.

use std::fs;

use super::support::*;
use crate::git::gitbackend::native_commit::Route;
use crate::git::gitbackend::native_commit::route::CliReason;
use crate::git::gitbackend::native_commit::{GitEnvironment, RoutePolicy};
use crate::git::gitbackend::*;

fn is_cli(route: &Route, expected: &CliReason) -> bool {
    matches!(route, Route::Cli(reason) if reason == expected)
}

#[test]
fn a_plain_repository_commits_natively() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    assert_eq!(lab.commit_route(false), Route::Native);
    lab.dirty_change();
    assert!(
        matches!(lab.commit_route(true), Route::Cli(CliReason::AllCommit(_))),
        "commit -a is Git's whatever the repository holds"
    );
    assert_eq!(
        lab.native_all_route(),
        Route::Native,
        "the kept native code reads it"
    );
}

#[test]
fn the_cli_only_policy_forces_the_cli() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    let forced = lab.backend.with_route_policy(RoutePolicy::CLI_ONLY);
    assert!(is_cli(
        &forced.commit_route(&lab.path, "m", false),
        &CliReason::Forced
    ));
}

#[test]
fn each_commit_hook_picks_the_cli_and_the_hook_runs() {
    hermetic!();
    for name in [
        "pre-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
        "reference-transaction",
    ] {
        let lab = lab();
        let marker = lab.hook(name);
        lab.stage_change();
        assert!(
            is_cli(&lab.commit_route(false), &CliReason::Hook(name.to_owned())),
            "{name}"
        );
        let before = lab.head();
        lab.backend.commit(&lab.path, "with hook", false).unwrap();
        assert_ne!(lab.head(), before, "{name}: commit happened");
        assert!(marker.exists(), "{name}: the hook ran");
    }
}

#[test]
fn post_index_change_also_picks_the_cli() {
    hermetic!();
    let lab = lab();
    lab.hook("post-index-change");
    lab.stage_change();
    assert!(is_cli(
        &lab.commit_route(false),
        &CliReason::Hook("post-index-change".to_owned())
    ));
}

#[test]
fn sample_hooks_never_count() {
    hermetic!();
    let lab = lab();
    for name in [
        "pre-commit.sample",
        "commit-msg.sample",
        "reference-transaction.sample",
    ] {
        let hooks = lab.path.join(".git").join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(hooks.join(name), "#!/bin/sh\nexit 1\n").unwrap();
        make_executable(&hooks.join(name));
    }
    lab.stage_change();
    assert_eq!(lab.commit_route(false), Route::Native);
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn on_unix_a_hook_counts_only_when_executable() {
            use std::os::unix::fs::PermissionsExt;
            hermetic!();
            let lab = lab();
            let marker = lab.hook("pre-commit");
            let hook = lab.path.join(".git/hooks/pre-commit");
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o644)).unwrap();
            lab.stage_change();
            assert_eq!(lab.commit_route(false), Route::Native);
            lab.backend.commit(&lab.path, "ignored hook", false).unwrap();
            assert!(!marker.exists(), "git ignores a hook that is not executable");
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(is_cli(
                &lab.commit_route(false),
                &CliReason::Hook("pre-commit".to_owned())
            ));
        }
    } else {
        #[test]
        fn on_windows_any_file_with_the_hook_name_counts() {
            hermetic!();
            let lab = lab();
            let hooks = lab.path.join(".git").join("hooks");
            fs::create_dir_all(&hooks).unwrap();
            fs::write(hooks.join("commit-msg"), "not even a script\n").unwrap();
            lab.stage_change();
            assert!(is_cli(
                &lab.commit_route(false),
                &CliReason::Hook("commit-msg".to_owned())
            ));
            fs::remove_file(hooks.join("commit-msg")).unwrap();
            fs::write(hooks.join("pre-commit.exe"), "stub\n").unwrap();
            assert!(is_cli(
                &lab.commit_route(false),
                &CliReason::Hook("pre-commit".to_owned())
            ));
        }
    }
}

#[test]
fn hooks_path_replaces_the_default_directory() {
    hermetic!();
    let lab = lab();
    lab.hook("pre-commit");
    let empty = lab.path.join("empty-hooks");
    fs::create_dir_all(&empty).unwrap();
    lab.config("core.hooksPath", empty.to_str().unwrap());
    lab.stage_change();
    assert_eq!(
        lab.commit_route(false),
        Route::Native,
        "the default directory is not consulted when core.hooksPath is set"
    );

    lab.config("core.hooksPath", ".githooks");
    install_hook(&lab.path.join(".githooks"), "commit-msg", &lab.path);
    assert!(is_cli(
        &lab.commit_route(false),
        &CliReason::Hook("commit-msg".to_owned())
    ));

    let outside = lab.path.parent().unwrap().join("outside-hooks");
    install_hook(&outside, "post-commit", &lab.path);
    lab.config("core.hooksPath", outside.to_str().unwrap());
    assert!(is_cli(
        &lab.commit_route(false),
        &CliReason::Hook("post-commit".to_owned())
    ));

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap();
    install_hook(
        &std::path::Path::new(&home).join("tilde-hooks"),
        "pre-commit",
        &lab.path,
    );
    lab.config("core.hooksPath", "~/tilde-hooks");
    assert!(is_cli(
        &lab.commit_route(false),
        &CliReason::Hook("pre-commit".to_owned())
    ));
    lab.config("core.hooksPath", "~nobody/hooks");
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::Config(_))
    ));
}

#[test]
fn a_linked_worktree_checks_the_shared_hooks_directory() {
    hermetic!();
    let lab = lab();
    let link = lab.path.parent().unwrap().join("linked");
    git(
        &lab.path,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            link.to_str().unwrap(),
        ],
    );
    write(&link, "tracked.txt", "linked\n");
    git(&link, &["add", "tracked.txt"]);
    assert_eq!(lab.backend.commit_route(&link, "m", false), Route::Native);
    let marker = lab.hook("pre-commit");
    assert!(is_cli(
        &lab.backend.commit_route(&link, "m", false),
        &CliReason::Hook("pre-commit".to_owned())
    ));
    lab.backend.commit(&link, "linked commit", false).unwrap();
    assert!(
        marker.exists(),
        "the shared hook ran for the linked worktree"
    );
}

#[test]
fn signing_picks_the_cli() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    lab.config("commit.gpgSign", "false");
    assert_eq!(lab.commit_route(false), Route::Native);
    lab.config("commit.gpgSign", "true");
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::Signing(_))
    ));
    lab.config("commit.gpgSign", "maybe");
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::Signing(_))
    ));
}

#[test]
fn merge_cherry_pick_revert_and_rebase_state_picks_the_cli() {
    hermetic!();
    for (name, directory) in [
        ("MERGE_HEAD", false),
        ("CHERRY_PICK_HEAD", false),
        ("REVERT_HEAD", false),
        ("rebase-merge", true),
        ("rebase-apply", true),
        ("sequencer", true),
        ("MERGE_MSG", false),
        ("SQUASH_MSG", false),
    ] {
        let lab = lab();
        lab.stage_change();
        let target = lab.path.join(".git").join(name);
        if directory {
            fs::create_dir_all(&target).unwrap();
        } else {
            fs::write(&target, format!("{}\n", lab.head())).unwrap();
        }
        assert!(
            matches!(
                lab.commit_route(false),
                Route::Cli(CliReason::InProgress(_))
            ),
            "{name}"
        );
    }
}

#[test]
fn unmerged_entries_and_intent_to_add_pick_the_cli() {
    hermetic!();
    let lab = lab();
    git(&lab.path, &["checkout", "-q", "-b", "side"]);
    write(&lab.path, "tracked.txt", "side\n");
    git(&lab.path, &["commit", "-q", "-a", "-m", "side"]);
    git(&lab.path, &["checkout", "-q", "main"]);
    write(&lab.path, "tracked.txt", "main\n");
    git(&lab.path, &["commit", "-q", "-a", "-m", "main"]);
    let merge = std::process::Command::new("git")
        .arg("-C")
        .arg(&lab.path)
        .args(["merge", "side"])
        .output()
        .unwrap();
    assert!(!merge.status.success(), "the merge must conflict");
    for name in ["MERGE_HEAD", "MERGE_MSG", "MERGE_MODE", "AUTO_MERGE"] {
        let _ = fs::remove_file(lab.path.join(".git").join(name));
    }
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::IndexState("unmerged entries"))
    ));

    let lab = lab_with_intent_to_add();
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::IndexState("intent-to-add entry"))
    ));
}

fn lab_with(key: &str, value: &str) -> Lab {
    let lab = lab();
    lab.stage_change();
    lab.config(key, value);
    lab
}

fn lab_with_intent_to_add() -> Lab {
    let lab = lab();
    lab.stage_change();
    write(&lab.path, "later.txt", "later\n");
    git(&lab.path, &["add", "-N", "later.txt"]);
    lab
}

#[test]
fn an_index_lock_is_refused_like_git() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    fs::write(lab.path.join(".git/index.lock"), "").unwrap();
    assert!(is_cli(
        &lab.commit_route(false),
        &CliReason::IndexState("index.lock")
    ));
    let before = lab.head();
    let error = lab.backend.commit(&lab.path, "locked", false).unwrap_err();
    assert_eq!(error.code, crate::model::ErrorCode::GitCommandFailed);
    assert_eq!(lab.head(), before);
}

#[test]
fn staging_for_all_picks_the_cli_when_content_could_change() {
    hermetic!();
    type Setup = fn(&Lab);
    let cases: [(&str, Setup); 9] = [
        ("filter configuration", |lab| {
            lab.config("filter.x.clean", "cat")
        }),
        ("autocrlf true", |lab| lab.config("core.autocrlf", "true")),
        ("autocrlf input", |lab| lab.config("core.autocrlf", "input")),
        ("eol crlf", |lab| lab.config("core.eol", "crlf")),
        ("text attribute", |lab| {
            write(&lab.path, ".gitattributes", "*.txt text\n")
        }),
        ("eol attribute", |lab| {
            write(&lab.path, ".gitattributes", "*.txt eol=crlf\n")
        }),
        ("filter attribute", |lab| {
            write(&lab.path, ".gitattributes", "*.txt filter=x\n")
        }),
        ("ident attribute", |lab| {
            write(&lab.path, ".gitattributes", "*.txt ident\n")
        }),
        ("fsmonitor", |lab| lab.config("core.fsmonitor", "true")),
    ];
    for (what, setup) in cases {
        let lab = lab();
        setup(&lab);
        lab.dirty_change();
        assert!(
            matches!(lab.native_all_route(), Route::Cli(CliReason::AllCommit(_))),
            "{what}: -a"
        );
        lab.stage_change();
        let without_all = lab.commit_route(false);
        assert!(
            !matches!(without_all, Route::Cli(CliReason::AllCommit(_))),
            "{what}: staging rules apply to -a only, got {without_all:?}"
        );
    }
}

#[test]
fn indexed_state_native_cannot_stage_picks_the_cli_for_all() {
    hermetic!();
    type Setup = fn(&Lab);
    let cases: [(&str, Setup); 3] = [
        ("assume-unchanged", |lab| {
            git(
                &lab.path,
                &["update-index", "--assume-unchanged", "other.txt"],
            );
        }),
        ("skip-worktree", |lab| {
            git(&lab.path, &["update-index", "--skip-worktree", "other.txt"]);
        }),
        ("submodule entry", |lab| {
            let head = lab.head();
            git(
                &lab.path,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("160000,{head},sub"),
                ],
            );
        }),
    ];
    for (what, setup) in cases {
        let lab = lab();
        setup(&lab);
        lab.dirty_change();
        assert!(
            matches!(lab.native_all_route(), Route::Cli(CliReason::AllCommit(_))),
            "{what}"
        );
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn a_typechange_picks_the_cli_for_all() {
            hermetic!();
            let lab = lab();
            fs::remove_file(lab.path.join("tracked.txt")).unwrap();
            std::os::unix::fs::symlink("other.txt", lab.path.join("tracked.txt")).unwrap();
            assert!(matches!(lab.native_all_route(), Route::Cli(CliReason::AllCommit(_))));
        }
    }
}

#[test]
fn sparse_and_split_index_configuration_pick_the_cli_for_every_commit() {
    hermetic!();
    for (key, value) in [
        ("core.sparseCheckout", "true"),
        ("core.splitIndex", "true"),
        ("index.sparse", "true"),
        ("extensions.worktreeConfig", "true"),
        ("core.sharedRepository", "group"),
        ("core.logAllRefUpdates", "false"),
        ("core.logAllRefUpdates", "always"),
    ] {
        let lab = lab();
        lab.stage_change();
        lab.config(key, value);
        assert!(
            matches!(lab.commit_route(false), Route::Cli(CliReason::Config(_))),
            "{key}={value}"
        );
    }
}

#[test]
fn unsupported_include_conditions_pick_the_cli() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    assert_eq!(lab.commit_route(false), Route::Native);
    lab.config(
        "includeIf.hasconfig:remote.*.url:https://example.invalid/**.path",
        "/dev/null",
    );
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::Config(_))
    ));
    let onbranch = lab_with("includeIf.onbranch:main.path", "/dev/null");
    assert!(matches!(
        onbranch.commit_route(false),
        Route::Cli(CliReason::Config(_))
    ));
    let gitdir = lab_with("includeIf.gitdir:/nowhere/.path", "/dev/null");
    cfg_if::cfg_if! {
        if #[cfg(windows)] {
            assert!(matches!(gitdir.commit_route(false), Route::Cli(CliReason::Config(_))));
        } else {
            assert_eq!(gitdir.commit_route(false), Route::Native);
        }
    }
}

#[test]
fn environment_native_does_not_honour_picks_the_cli() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_NAMESPACE",
        "GIT_REFLOG_ACTION",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        let env = GitEnvironment::from_vars([(name, "x")]);
        let backend = lab.backend.with_environment(env);
        assert!(
            is_cli(
                &backend.commit_route(&lab.path, "m", false),
                &CliReason::Environment(name.to_owned())
            ),
            "{name}"
        );
    }
    let harmless = GitEnvironment::from_vars([
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_TRACE", "0"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ]);
    assert_eq!(
        lab.backend
            .with_environment(harmless)
            .commit_route(&lab.path, "m", false),
        Route::Native
    );
}

#[test]
fn identity_native_cannot_reproduce_picks_the_cli() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    let with = |pairs: &[(&str, &str)]| {
        lab.backend
            .with_environment(GitEnvironment::from_vars(
                pairs.iter().copied().chain([("GIT_CONFIG_NOSYSTEM", "1")]),
            ))
            .commit_route(&lab.path, "m", false)
    };
    assert_eq!(with(&[("GIT_AUTHOR_NAME", "Someone Else")]), Route::Native);
    assert!(matches!(
        with(&[("GIT_AUTHOR_NAME", "Dr. Who Jr.")]),
        Route::Cli(CliReason::Identity(_))
    ));
    assert!(matches!(
        with(&[("GIT_COMMITTER_EMAIL", "<angle>")]),
        Route::Cli(CliReason::Identity(_))
    ));
    assert!(matches!(
        with(&[("GIT_AUTHOR_DATE", "yesterday")]),
        Route::Cli(CliReason::Identity(_))
    ));
    git(&lab.path, &["config", "--unset", "user.name"]);
    assert!(matches!(
        with(&[("EMAIL", "e@example.invalid")]),
        Route::Cli(CliReason::Identity(_))
    ));
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::Identity(_))
    ));
}

#[test]
fn messages_native_cannot_clean_exactly_pick_the_cli() {
    hermetic!();
    let lab = lab();
    lab.stage_change();
    let route = |message: &str| lab.backend.commit_route(&lab.path, message, false);
    for empty in ["", "   ", "\n\n"] {
        assert!(
            matches!(route(empty), Route::Cli(CliReason::Message(_))),
            "{empty:?}"
        );
    }
    assert!(matches!(
        route("nul\0inside"),
        Route::Cli(CliReason::Message(_))
    ));
    lab.config("commit.cleanup", "strip");
    lab.config("core.commentChar", ";");
    assert!(matches!(route("m"), Route::Cli(CliReason::Message(_))));
    lab.config("commit.cleanup", "whitespace");
    assert_eq!(
        route("m"),
        Route::Native,
        "the comment character is moot without strip"
    );
    lab.config("commit.cleanup", "no-such-mode");
    assert!(matches!(route("m"), Route::Cli(CliReason::Config(_))));
    lab.config("commit.cleanup", "verbatim");
    lab.config("i18n.commitEncoding", "ISO-8859-1");
    assert!(matches!(route("m"), Route::Cli(CliReason::Config(_))));
}

#[test]
fn repositories_libgit2_cannot_serve_pick_the_cli() {
    hermetic!();
    let dir = tempfile::TempDir::new().unwrap();
    let bare = dir.path().join("bare.git");
    git(
        dir.path(),
        &["init", "-q", "--bare", bare.to_str().unwrap()],
    );
    let backend = Git2Backend::new();
    assert!(matches!(
        backend.commit_route(&bare, "m", false),
        Route::Cli(CliReason::Bare)
    ));
    let sha256 = dir.path().join("sha256");
    git(
        dir.path(),
        &[
            "init",
            "-q",
            "--object-format=sha256",
            sha256.to_str().unwrap(),
        ],
    );
    assert!(matches!(
        backend.commit_route(&sha256, "m", false),
        Route::Cli(CliReason::ObjectFormat | CliReason::CannotOpen)
    ));
    assert!(matches!(
        backend.commit_route(&dir.path().join("missing"), "m", false),
        Route::Cli(CliReason::CannotOpen)
    ));
}
