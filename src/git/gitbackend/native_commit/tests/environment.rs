//! Git's own variables, in hermetic child processes: native and the Git CLI
//! see the same identity, dates and configuration locations.

use std::fs;
use std::path::Path;

use super::support::*;
use crate::git::gitbackend::native_commit::Route;
use crate::git::gitbackend::native_commit::route::CliReason;
use crate::git::gitbackend::*;
use crate::model::ErrorCode;

const PREFIX: &str = "git::gitbackend::native_commit::tests::environment::";

fn not_child() -> bool {
    !is_child()
}

fn stage(twin: &Twin) {
    twin.both(|path| {
        write(path, "tracked.txt", "env change\n");
        git(path, &["add", "tracked.txt"]);
    });
}

fn authors(path: &Path) -> String {
    git(
        path,
        &[
            "log",
            "-1",
            "--format=%an|%ae|%ad|%cn|%ce|%cd",
            "--date=raw",
        ],
    )
}

#[test]
fn author_and_committer_variables_match_git() {
    run_child(
        &format!("{PREFIX}author_and_committer_variables_child"),
        &[
            ("GIT_AUTHOR_NAME", "Env Author"),
            ("GIT_AUTHOR_EMAIL", "env-author@example.invalid"),
            ("GIT_COMMITTER_NAME", "Env Committer"),
            ("GIT_COMMITTER_EMAIL", "env-committer@example.invalid"),
        ],
    );
}

#[test]
fn author_and_committer_variables_child() {
    if not_child() {
        return;
    }
    let twin = twin(seeded);
    stage(&twin);
    assert!(matches!(
        twin.commit("env identity", false),
        Outcome::Done(_)
    ));
    assert_eq!(
        authors(&twin.native),
        "Env Author|env-author@example.invalid|981153306 +0530|Env Committer|env-committer@example.invalid|1015243567 -0700"
    );
    assert!(matches!(
        twin.tag_create("env-tag", Some("env tagger")),
        Outcome::Done(_)
    ));
    let tagger = git(&twin.native, &["cat-file", "-p", "refs/tags/env-tag"]);
    assert!(
        tagger.contains("tagger Env Committer <env-committer@example.invalid> 1015243567 -0700"),
        "{tagger}"
    );
}

#[test]
fn supported_date_forms_match_git() {
    for date in [
        "2001-02-03T04:05:06 +0530",
        "2001-02-03T04:05:06 -07:00",
        "2001-02-03T04:05:06 Z",
        "1015243567 -0700",
        "@1015243567 +0000",
    ] {
        run_child(
            &format!("{PREFIX}date_forms_child"),
            &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
        );
    }
}

#[test]
fn date_forms_child() {
    if not_child() {
        return;
    }
    let twin = twin(seeded);
    stage(&twin);
    assert!(matches!(twin.commit("dated", false), Outcome::Done(_)));
    twin.tag_create("dated-tag", Some("dated"));
}

#[test]
fn other_date_forms_go_to_git() {
    for date in [
        "yesterday",
        "2001-02-03",
        "2001-02-03 04:05:06 +0530",
        "@1015243567",
    ] {
        run_child(
            &format!("{PREFIX}other_date_forms_child"),
            &[("GIT_COMMITTER_DATE", date)],
        );
    }
}

#[test]
fn other_date_forms_child() {
    if not_child() {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let path = real(dir.path()).join("repo");
    let valid = "2001-02-03T04:05:06 +0530";
    unborn(&path);
    write(&path, "tracked.txt", "base\n");
    git(&path, &["add", "tracked.txt"]);
    let seed = std::process::Command::new("git")
        .arg("-C")
        .arg(&path)
        .args(["commit", "-q", "-m", "base"])
        .env("GIT_AUTHOR_DATE", valid)
        .env("GIT_COMMITTER_DATE", valid)
        .status()
        .unwrap();
    assert!(seed.success());
    write(&path, "tracked.txt", "again\n");
    git(&path, &["add", "tracked.txt"]);
    let backend = Git2Backend::new();
    assert!(matches!(
        backend.commit_route(&path, "m", false),
        Route::Cli(CliReason::Identity(_))
    ));
    // The odd date is Git's to read: here Git refuses or reads it, and the
    // backend returns what Git did.
    let result = backend.commit(&path, "git reads the date", false);
    let direct = std::process::Command::new("git")
        .arg("-C")
        .arg(&path)
        .args(["commit", "-q", "--allow-empty", "-m", "direct"])
        .output()
        .unwrap();
    assert_eq!(result.is_ok(), direct.status.success());
}

#[test]
fn global_identity_alone_matches_git() {
    run_child(&format!("{PREFIX}global_identity_child"), &[]);
}

#[test]
fn global_identity_child() {
    if not_child() {
        return;
    }
    let global = std::env::var_os("GIT_CONFIG_GLOBAL").unwrap();
    fs::write(
        global,
        "[user]\n\tname = Global User\n\temail = global@example.invalid\n",
    )
    .unwrap();
    let twin = twin(|path| {
        let backend = Git2Backend::new();
        backend.create_repo(path).unwrap();
        write(path, "tracked.txt", "base\n");
        git(path, &["add", "tracked.txt"]);
        git(path, &["commit", "-q", "-m", "base"]);
    });
    stage(&twin);
    assert!(matches!(
        twin.commit("global only", false),
        Outcome::Done(_)
    ));
    assert!(authors(&twin.native).starts_with("Global User|global@example.invalid|"));
    assert!(matches!(
        twin.tag_create("global-tag", Some("global")),
        Outcome::Done(_)
    ));
}

#[test]
fn configuration_given_in_the_environment_goes_to_git() {
    run_child(
        &format!("{PREFIX}environment_configuration_child"),
        &[
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "user.name"),
            ("GIT_CONFIG_VALUE_0", "Config From Environment"),
        ],
    );
}

#[test]
fn environment_configuration_child() {
    if not_child() {
        return;
    }
    let lab = lab();
    git(&lab.path, &["config", "--unset", "user.name"]);
    lab.stage_change();
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::Environment(_))
    ));
    lab.backend
        .commit(&lab.path, "named by the environment", false)
        .unwrap();
    assert!(git(&lab.path, &["log", "-1", "--format=%an"]).contains("Config From Environment"));
}

#[test]
fn missing_identity_fails_as_git_fails() {
    run_child(&format!("{PREFIX}missing_identity_child"), &[]);
}

#[test]
fn missing_identity_child() {
    if not_child() {
        return;
    }
    let twin = twin(|path| {
        seeded(path);
        git(path, &["config", "--unset", "user.name"]);
        git(path, &["config", "--unset", "user.email"]);
        git(path, &["config", "user.useConfigOnly", "true"]);
    });
    stage(&twin);
    let route = twin.auto.commit_route(&twin.native, "no identity", false);
    assert!(
        matches!(route, Route::Cli(CliReason::Identity(_))),
        "{route:?}"
    );
    let native = twin
        .auto
        .commit(&twin.native, "no identity", false)
        .unwrap_err();
    let cli = twin
        .cli_only
        .commit(&twin.cli, "no identity", false)
        .unwrap_err();
    assert_eq!(native.code, cli.code);
    assert_eq!(native.code, ErrorCode::GitCommandFailed);
    twin.assert_same("after the refused commit");
}

#[test]
fn email_variable_alone_goes_to_git() {
    run_child(
        &format!("{PREFIX}email_variable_child"),
        &[("EMAIL", "from-email-variable@example.invalid")],
    );
}

#[test]
fn email_variable_child() {
    if not_child() {
        return;
    }
    let twin = twin(|path| {
        seeded(path);
        git(path, &["config", "--unset", "user.email"]);
    });
    stage(&twin);
    assert!(matches!(
        twin.auto.commit_route(&twin.native, "email", false),
        Route::Cli(CliReason::Identity(_))
    ));
    let native = twin.auto.commit(&twin.native, "email", false).unwrap();
    let cli = twin.cli_only.commit(&twin.cli, "email", false).unwrap();
    assert_eq!(native.commit, cli.commit);
    assert!(authors(&twin.native).contains("from-email-variable@example.invalid"));
}

/// A system configuration file, named by `GIT_CONFIG_SYSTEM` with the system
/// file switched on, that sets what only Git would honour.
fn system_file(dir: &tempfile::TempDir, text: &str) -> std::path::PathBuf {
    let path = dir.path().join("system.gitconfig");
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn system_level_signing_picks_the_cli() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = system_file(
        &dir,
        "[commit]\n\tgpgSign = true\n[tag]\n\tgpgSign = true\n",
    );
    run_child(
        &format!("{PREFIX}system_signing_child"),
        &[
            ("GIT_CONFIG_NOSYSTEM", "0"),
            ("GIT_CONFIG_SYSTEM", file.to_str().unwrap()),
        ],
    );
}

#[test]
fn system_signing_child() {
    if not_child() {
        return;
    }
    let lab = lab();
    lab.stage_change();
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::Signing("commit.gpgSign"))
    ));
    assert!(matches!(
        lab.backend
            .tag_create_route(&lab.path, "v1", Some("m"), false),
        Route::Cli(CliReason::Signing("tag.gpgSign"))
    ));
}

#[test]
fn system_level_hooks_path_picks_the_cli_and_the_hook_runs() {
    let dir = tempfile::TempDir::new().unwrap();
    let hooks = dir.path().join("system-hooks");
    let marker = install_hook(&hooks, "pre-commit", dir.path());
    let file = system_file(
        &dir,
        &format!(
            "[core]\n\thooksPath = {}\n",
            hooks.to_string_lossy().replace('\\', "/")
        ),
    );
    run_child(
        &format!("{PREFIX}system_hooks_path_child"),
        &[
            ("GIT_CONFIG_NOSYSTEM", "0"),
            ("GIT_CONFIG_SYSTEM", file.to_str().unwrap()),
        ],
    );
    assert!(marker.exists(), "the system-level hook ran");
}

#[test]
fn system_hooks_path_child() {
    if not_child() {
        return;
    }
    let lab = lab();
    lab.stage_change();
    assert!(matches!(
        lab.commit_route(false),
        Route::Cli(CliReason::Hook(_))
    ));
    lab.backend
        .commit(&lab.path, "hooked by system config", false)
        .unwrap();
}
