//! The route rule's inputs beyond hooks files and signing: the system file of
//! the `git` on `PATH`, hooks defined in configuration, `include.path` forms,
//! `core.fsync`, `commit.cleanup`, `commit -a` and the variables that are not
//! harmless (step 5.6 remediation, lane A).

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use super::support::*;
use crate::git::gitbackend::native_commit::Route;
use crate::git::gitbackend::native_commit::route::CliReason;
use crate::git::gitbackend::native_commit::{GitEnvironment, RoutePolicy};
use crate::git::gitbackend::*;

/// A seeded repository with the default backend (`lab` is shadowed by the rows' own).
fn fresh() -> Lab {
    lab()
}

/// A `git` that does nothing, under `<prefix>/bin`; returns that directory.
fn fake_git(prefix: &Path) -> PathBuf {
    let bin = prefix.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    fs::write(bin.join(name), "#!/bin/sh\nexit 0\n").unwrap();
    make_executable(&bin.join(name));
    bin
}

/// The backend, seeing only `PATH`.
fn on_path(lab: &Lab, path: &Path) -> Git2Backend {
    lab.backend
        .with_environment(GitEnvironment::from_vars([("PATH", path.as_os_str())]))
}

/// Empty the global configuration the hermetic child uses, so a key a row
/// set there (some of which Git itself cannot expand) reaches no later row.
fn reset_global() {
    fs::write(std::env::var_os("GIT_CONFIG_GLOBAL").unwrap(), "").unwrap();
}

fn system_file_route(route: &Route, expected: &Path) -> bool {
    matches!(route, Route::Cli(CliReason::SystemFile { path, .. }) if path == expected)
}

#[test]
fn a_system_file_of_the_git_on_path_picks_the_cli() {
    hermetic!();
    let lab = fresh();
    lab.stage_change();
    let dir = tempfile::TempDir::new().unwrap();
    // A prefix no fixed list knows (Linuxbrew, MacPorts, Nix, conda ...).
    let prefix = real(dir.path()).join("opt").join("elsewhere");
    let bin = fake_git(&prefix);
    let etc = prefix.join("etc");
    fs::create_dir_all(&etc).unwrap();
    fs::write(etc.join("gitconfig"), "[commit]\n\tgpgSign = true\n").unwrap();
    let backend = on_path(&lab, &bin);
    let commit = backend.commit_route(&lab.path, "m", false);
    assert!(
        system_file_route(&commit, &etc.join("gitconfig")),
        "{commit:?}"
    );
    let tag = backend.tag_create_route(&lab.path, "v1", Some("m"), false);
    assert!(system_file_route(&tag, &etc.join("gitconfig")), "{tag:?}");
    let delete = backend.tag_delete_route(&lab.path, "v1");
    assert!(
        system_file_route(&delete, &etc.join("gitconfig")),
        "{delete:?}"
    );
}

cfg_if::cfg_if! {
    if #[cfg(not(windows))] {
        #[test]
        fn the_share_git_core_form_of_the_system_file_counts_too() {
            // Apple's command-line tools keep theirs at <prefix>/share/git-core/gitconfig.
            hermetic!();
            let lab = fresh();
            lab.stage_change();
            let dir = tempfile::TempDir::new().unwrap();
            let prefix = real(dir.path()).join("opt").join("elsewhere");
            let bin = fake_git(&prefix);
            let share = prefix.join("share").join("git-core");
            fs::create_dir_all(&share).unwrap();
            fs::write(share.join("gitconfig"), "[core]\n\thooksPath = /corp/hooks\n").unwrap();
            let commit = on_path(&lab, &bin).commit_route(&lab.path, "m", false);
            assert!(system_file_route(&commit, &share.join("gitconfig")), "{commit:?}");
        }
    }
}

#[test]
fn a_git_whose_system_files_are_absent_stays_native_unless_native_reads_one() {
    hermetic!();
    let lab = fresh();
    lab.stage_change();
    let dir = tempfile::TempDir::new().unwrap();
    let bin = fake_git(&real(dir.path()).join("opt").join("homebrew"));
    let route = on_path(&lab, &bin).commit_route(&lab.path, "m", false);
    if git2::Config::find_system().is_ok() {
        // libgit2 reads a system file this git does not: the CLI decides.
        assert!(
            matches!(route, Route::Cli(CliReason::SystemFile { .. })),
            "{route:?}"
        );
    } else {
        assert_eq!(route, Route::Native);
    }
}

#[test]
fn no_resolvable_git_on_path_picks_the_cli() {
    hermetic!();
    let lab = fresh();
    lab.stage_change();
    let dir = tempfile::TempDir::new().unwrap();
    let empty = real(dir.path()).join("empty");
    fs::create_dir_all(&empty).unwrap();
    let bin = fake_git(&real(dir.path()).join("prefix"));
    let relative = std::env::join_paths([Path::new("relative"), bin.as_path()]).unwrap();
    for (what, backend) in [
        (
            "no PATH",
            lab.backend.with_environment(GitEnvironment::none()),
        ),
        ("no git in PATH", on_path(&lab, &empty)),
        ("a relative entry", on_path(&lab, Path::new(&relative))),
    ] {
        let unresolved = Route::Cli(CliReason::SystemConfig("no git on PATH"));
        assert_eq!(
            backend.commit_route(&lab.path, "m", false),
            unresolved,
            "{what}"
        );
        assert_eq!(
            backend.tag_create_route(&lab.path, "v1", None, false),
            unresolved,
            "{what}: tag"
        );
    }
}

#[test]
fn no_system_skips_the_files_of_the_git_on_path() {
    hermetic!();
    let lab = fresh();
    lab.stage_change();
    let dir = tempfile::TempDir::new().unwrap();
    let prefix = real(dir.path()).join("opt").join("elsewhere");
    let bin = fake_git(&prefix);
    fs::create_dir_all(prefix.join("etc")).unwrap();
    fs::write(
        prefix.join("etc").join("gitconfig"),
        "[commit]\n\tgpgSign = true\n",
    )
    .unwrap();
    let backend = lab.backend.with_environment(GitEnvironment::from_vars([
        ("PATH", bin.as_os_str()),
        ("GIT_CONFIG_NOSYSTEM", OsStr::new("1")),
    ]));
    // The snapshot names no %PROGRAMDATA% (the one file libgit2 reads whatever NOSYSTEM says).
    assert_eq!(backend.commit_route(&lab.path, "m", false), Route::Native);
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[test]
        fn no_system_is_no_proof_while_programdata_config_exists() {
            hermetic!();
            let lab = fresh();
            lab.stage_change();
            let dir = tempfile::TempDir::new().unwrap();
            let programdata = real(dir.path()).join("programdata");
            let git = programdata.join("Git");
            fs::create_dir_all(&git).unwrap();
            let bin = fake_git(&real(dir.path()).join("prefix"));
            let environment = |programdata: &Path| GitEnvironment::from_vars([
                ("PATH", bin.as_os_str()),
                ("GIT_CONFIG_NOSYSTEM", OsStr::new("1")),
                ("PROGRAMDATA", programdata.as_os_str()),
            ]);
            assert_eq!(
                lab.backend.with_environment(environment(&programdata)).commit_route(&lab.path, "m", false),
                Route::Native,
                "no ProgramData file"
            );
            fs::write(git.join("config"), "[commit]\n\tgpgSign = true\n").unwrap();
            let route = lab.backend.with_environment(environment(&programdata)).commit_route(&lab.path, "m", false);
            assert!(system_file_route(&route, &git.join("config")), "{route:?}");
        }
    }
}

#[test]
fn config_defined_hooks_pick_the_cli_for_commit_and_tag() {
    hermetic!();
    for scope in ["--local", "--global"] {
        for (event, command) in [
            ("pre-commit", true),
            ("commit-msg", true),
            ("reference-transaction", true),
            ("pre-commit", false),
        ] {
            reset_global();
            let lab = fresh();
            lab.stage_change();
            assert_eq!(lab.commit_route(false), Route::Native);
            git(&lab.path, &["config", scope, "hook.guard.event", event]);
            if command {
                git(
                    &lab.path,
                    &["config", scope, "hook.guard.command", "exit 1"],
                );
            }
            let commit = lab.commit_route(false);
            assert!(
                matches!(commit, Route::Cli(CliReason::Hook(_))),
                "{scope} {event}: {commit:?}"
            );
            for route in [
                lab.backend.tag_create_route(&lab.path, "v1", None, false),
                lab.backend
                    .tag_create_route(&lab.path, "v2", Some("m"), false),
                lab.backend.tag_delete_route(&lab.path, "v1"),
            ] {
                assert!(
                    matches!(route, Route::Cli(CliReason::Hook(_))),
                    "{scope} {event}: tag {route:?}"
                );
            }
        }
    }
    reset_global();
}

/// The version `git --version` reports, as (major, minor).
fn git_version() -> (u32, u32) {
    let output = std::process::Command::new("git")
        .arg("--version")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut numbers = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().unwrap());
    (numbers.next().unwrap(), numbers.next().unwrap())
}

#[test]
fn a_config_hook_rejects_the_commit_when_git_runs_config_hooks() {
    hermetic!();
    if git_version() < (2, 54) {
        // Git before 2.54 ignores hook.<name>.* keys; the route still sends them to Git (above).
        return;
    }
    let lab = fresh();
    lab.stage_change();
    git(&lab.path, &["config", "hook.guard.event", "pre-commit"]);
    git(&lab.path, &["config", "hook.guard.command", "exit 1"]);
    let before = lab.head();
    lab.backend
        .commit(&lab.path, "refused by a configuration hook", false)
        .unwrap_err();
    assert_eq!(
        lab.head(),
        before,
        "the hook that Git runs refused the commit"
    );
}

#[test]
fn commit_all_always_picks_the_cli() {
    hermetic!();
    let lab = fresh();
    let all = |lab: &Lab| lab.commit_route(true);
    let is_all = |route: Route| matches!(route, Route::Cli(CliReason::AllCommit(_)));
    // A clean tree, a dirty one, and a staged one.
    assert!(is_all(all(&lab)), "clean");
    lab.dirty_change();
    assert!(is_all(all(&lab)), "dirty");
    lab.stage_change();
    assert!(is_all(all(&lab)), "staged");
    // An unborn branch.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("unborn");
    unborn(&path);
    write(&path, "a.txt", "a\n");
    git(&path, &["add", "a.txt"]);
    assert!(is_all(lab.backend.commit_route(&path, "m", true)), "unborn");
    // Without `-a` the same repository is native.
    assert_eq!(lab.commit_route(false), Route::Native);
    // The native `-a` code is kept for later; only an explicit policy reaches it.
    assert_eq!(
        lab.backend
            .with_route_policy(RoutePolicy::NATIVE_ALL)
            .commit_route(&lab.path, "m", true),
        Route::Native
    );
}

#[test]
fn include_path_forms_libgit2_does_not_expand_pick_the_cli() {
    hermetic!();
    for scope in ["--local", "--global"] {
        for refused in [
            "~alice/shared.gitconfig",
            "%(prefix)/etc/shared.gitconfig",
            "",
        ] {
            reset_global();
            let lab = fresh();
            lab.stage_change();
            // Git itself cannot expand these, so nothing below runs it again.
            git(&lab.path, &["config", scope, "include.path", refused]);
            assert!(
                matches!(lab.commit_route(false), Route::Cli(CliReason::Config(_))),
                "{scope} include.path={refused:?}"
            );
            assert!(
                matches!(
                    lab.backend.tag_create_route(&lab.path, "v1", None, false),
                    Route::Cli(CliReason::Config(_))
                ),
                "{scope} include.path={refused:?}: tag"
            );
        }
        for accepted in [
            "shared.gitconfig",
            "../shared.gitconfig",
            "~/shared.gitconfig",
        ] {
            reset_global();
            let lab = fresh();
            lab.stage_change();
            git(&lab.path, &["config", scope, "include.path", accepted]);
            assert_eq!(
                lab.commit_route(false),
                Route::Native,
                "{scope} include.path={accepted:?}"
            );
        }
    }
    reset_global();
    let lab = fresh();
    lab.stage_change();
    let absolute = lab.path.parent().unwrap().join("shared.gitconfig");
    git(
        &lab.path,
        &["config", "include.path", absolute.to_str().unwrap()],
    );
    assert_eq!(lab.commit_route(false), Route::Native, "an absolute path");
    // The path of an includeIf is judged the same way.
    reset_global();
    let lab = fresh();
    lab.stage_change();
    let condition = format!(
        "includeIf.gitdir:{}/.path",
        real(&lab.path).to_string_lossy().replace('\\', "/")
    );
    git(
        &lab.path,
        &[
            "config",
            "--global",
            &condition,
            "~alice/by-directory.gitconfig",
        ],
    );
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::Config(_))
    ));
    reset_global();
}

#[test]
fn attribute_and_test_variables_are_not_harmless() {
    hermetic!();
    let lab = fresh();
    lab.stage_change();
    for name in [
        "GIT_ATTR_SOURCE",
        "GIT_ATTR_NOSYSTEM",
        "GIT_TEST_ASSUME_DIFFERENT_OWNER",
    ] {
        let backend = lab
            .backend
            .with_environment(GitEnvironment::from_vars([(name, "x")]));
        assert_eq!(
            backend.commit_route(&lab.path, "m", false),
            Route::Cli(CliReason::Environment(name.to_owned())),
            "{name}"
        );
        assert_eq!(
            backend.tag_create_route(&lab.path, "v1", None, false),
            Route::Cli(CliReason::Environment(name.to_owned())),
            "{name}: tag"
        );
    }
}

#[test]
fn commit_cleanup_is_judged_in_git_s_case() {
    hermetic!();
    for value in [
        "Strip",
        "STRIP",
        "Default",
        "Whitespace",
        "VERBATIM",
        "Scissors",
    ] {
        let lab = fresh();
        lab.stage_change();
        lab.config("commit.cleanup", value);
        assert!(
            matches!(lab.commit_route(false), Route::Cli(CliReason::Config(_))),
            "{value}"
        );
        // Git refuses the value; the CLI route reports Git's refusal.
        let before = lab.head();
        lab.backend.commit(&lab.path, "m", false).expect_err(value);
        assert_eq!(lab.head(), before, "{value}");
    }
    for value in ["strip", "default", "whitespace", "verbatim", "scissors"] {
        let lab = fresh();
        lab.stage_change();
        lab.config("commit.cleanup", value);
        assert_eq!(lab.commit_route(false), Route::Native, "{value}");
    }
}

#[test]
fn core_fsync_settings_pick_the_cli() {
    hermetic!();
    for (key, value) in [
        ("core.fsync", "committed"),
        ("core.fsync", "all"),
        ("core.fsync", "none"),
        ("core.fsync", "loose-object,-pack"),
        ("core.fsyncMethod", "batch"),
        ("core.fsyncMethod", "fsync"),
    ] {
        let lab = fresh();
        lab.stage_change();
        lab.config(key, value);
        assert!(
            matches!(lab.commit_route(false), Route::Cli(CliReason::Config(_))),
            "{key}={value}"
        );
        assert!(
            matches!(
                lab.backend.tag_create_route(&lab.path, "v1", None, false),
                Route::Cli(CliReason::Config(_))
            ),
            "{key}={value}: tag"
        );
        assert!(
            matches!(
                lab.backend.tag_delete_route(&lab.path, "v1"),
                Route::Cli(CliReason::Config(_))
            ),
            "{key}={value}: delete"
        );
    }
}
