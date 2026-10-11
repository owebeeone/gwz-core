//! `commit`: native on one repository, the Git CLI on an identical twin.

use std::fs;

use super::support::*;
use crate::git::gitbackend::GitRepository as _;
use crate::git::gitbackend::native_commit::Route;
use crate::model::ErrorCode;

fn stage_both(twin: &Twin, files: &[(&str, &str)]) {
    twin.both(|path| {
        for (name, text) in files {
            write(path, name, text);
        }
        let names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
        let mut args = vec!["add"];
        args.extend(names);
        git(path, &args);
    });
}

#[test]
fn initial_commit_on_an_unborn_branch_matches_git() {
    hermetic!();
    let twin = twin(unborn);
    stage_both(&twin, &[("a.txt", "a\n"), ("dir/b.txt", "b\n")]);
    let Outcome::Done(first) = twin.commit("first", false) else {
        panic!("initial commit failed");
    };
    assert_eq!(
        git(&twin.native, &["rev-parse", "HEAD"]),
        first,
        "HEAD is the new commit"
    );
    let log = fs::read_to_string(twin.native.join(".git/logs/HEAD")).unwrap();
    assert!(log.contains("\tcommit (initial): first"), "{log}");
}

#[test]
fn empty_index_on_an_unborn_branch_is_refused_like_git() {
    hermetic!();
    let twin = twin(unborn);
    assert_eq!(
        twin.commit("nothing", false),
        Outcome::Failed(ErrorCode::GitCommandFailed)
    );
}

#[test]
fn successive_commits_and_reflog_subjects_match_git() {
    hermetic!();
    let twin = twin(seeded);
    let messages = [
        "plain",
        "multi\nline\n\nbody",
        "\n\n  leading blanks and indent  \n\n\n\nafter run\n\n",
        "# comment kept\nunder default cleanup",
        "trailing spaces   \nnext\t\n",
        "no newline at end",
        "unicode \u{e9}\u{4e2d}\u{1f600} subject",
        "tab\tinside\r\ncrlf line\r\n",
        "\ttab leading subject",
        "a very long subject line that goes on and on and on and on and on and on and on and on and on and on and on and on and on",
    ];
    for (round, message) in messages.iter().enumerate() {
        stage_both(&twin, &[("tracked.txt", &format!("round {round}\n"))]);
        assert!(matches!(twin.commit(message, false), Outcome::Done(_)));
    }
}

#[test]
fn commit_cleanup_modes_match_git() {
    hermetic!();
    let message = "\n  body  \n# comment\nnext  \n\n\nlast\n# ------------------------ >8 ------------------------\ncut\n";
    for mode in [
        None,
        Some("default"),
        Some("whitespace"),
        Some("strip"),
        Some("verbatim"),
        Some("scissors"),
    ] {
        let twin = twin(seeded);
        if let Some(mode) = mode {
            twin.both(|path| {
                git(path, &["config", "commit.cleanup", mode]);
            });
        }
        stage_both(&twin, &[("tracked.txt", "changed\n")]);
        assert!(
            matches!(twin.commit(message, false), Outcome::Done(_)),
            "cleanup {mode:?}"
        );
        assert!(matches!(twin.commit("again", false), Outcome::Failed(_)));
    }
}

#[test]
fn nothing_to_commit_is_refused_like_git_with_and_without_all() {
    hermetic!();
    let twin = twin(seeded);
    for all in [false, true] {
        assert_eq!(
            twin.commit("empty", all),
            Outcome::Failed(ErrorCode::GitCommandFailed)
        );
    }
    write(&twin.native, "untracked.txt", "u\n");
    write(&twin.cli, "untracked.txt", "u\n");
    assert_eq!(
        twin.commit("untracked only", true),
        Outcome::Failed(ErrorCode::GitCommandFailed)
    );
}

#[test]
fn commit_uses_the_index_and_all_stages_tracked_changes_only() {
    hermetic!();
    let twin = twin(seeded);
    stage_both(&twin, &[("tracked.txt", "staged\n")]);
    twin.both(|path| write(path, "tracked.txt", "unstaged on top\n"));
    assert!(matches!(twin.commit("indexed", false), Outcome::Done(_)));

    stage_both(&twin, &[("added.txt", "added and staged\n")]);
    twin.both(|path| {
        write(path, "tracked.txt", "all this\n");
        write(path, "untracked.txt", "never staged\n");
        fs::remove_file(path.join("other.txt")).unwrap();
    });
    assert!(matches!(twin.commit("all", true), Outcome::Done(_)));
    assert_eq!(
        git(&twin.native, &["ls-files"]),
        git(&twin.cli, &["ls-files"])
    );
    assert!(!git(&twin.native, &["ls-files"]).contains("untracked.txt"));
    assert!(git(&twin.native, &["ls-files"]).contains("added.txt"));
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn all_records_a_mode_change_like_git() {
            use std::os::unix::fs::PermissionsExt;
            hermetic!();
            let twin = twin(seeded);
            twin.both(|path| {
                let file = path.join("tracked.txt");
                fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
            });
            assert!(matches!(twin.commit("chmod", true), Outcome::Done(_)));
            assert!(git(&twin.native, &["ls-tree", "HEAD", "tracked.txt"]).starts_with("100755"));
        }
    }
}

#[test]
fn commit_on_a_detached_head_matches_git() {
    hermetic!();
    let twin = twin(seeded);
    twin.both(|path| {
        git(path, &["checkout", "-q", "--detach"]);
    });
    stage_both(&twin, &[("tracked.txt", "detached\n")]);
    assert!(matches!(
        twin.commit("on detached head", false),
        Outcome::Done(_)
    ));
    assert_eq!(
        git(&twin.native, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "HEAD"
    );
}

#[test]
fn local_identity_keys_and_include_if_match_git() {
    hermetic!();
    let twin = twin(seeded);
    twin.both(|path| {
        git(path, &["config", "author.name", "Author Only"]);
        git(
            path,
            &[
                "config",
                "committer.email",
                "committer-only@example.invalid",
            ],
        );
    });
    stage_both(&twin, &[("tracked.txt", "role keys\n")]);
    assert!(matches!(twin.commit("roles", false), Outcome::Done(_)));

    let include = twin.native.parent().unwrap().join("included.gitconfig");
    fs::write(
        &include,
        "[user]\n\tname = Included Name\n\temail = included@example.invalid\n",
    )
    .unwrap();
    twin.both(|path| {
        git(path, &["config", "--unset", "author.name"]);
        git(path, &["config", "--unset", "committer.email"]);
        git(path, &["config", "--unset", "user.name"]);
        git(path, &["config", "--unset", "user.email"]);
        let condition = format!(
            "includeIf.gitdir:{}/.path",
            path.to_string_lossy().replace('\\', "/")
        );
        git(path, &["config", &condition, include.to_str().unwrap()]);
    });
    stage_both(&twin, &[("tracked.txt", "include if\n")]);
    cfg_if::cfg_if! {
        if #[cfg(windows)] {
            assert!(matches!(
                twin.auto.commit_route(&twin.native, "include", false),
                Route::Cli(crate::git::gitbackend::native_commit::route::CliReason::Config(_))
            ));
            let native = twin.auto.commit(&twin.native, "include", false).unwrap();
            let cli = twin.cli_only.commit(&twin.cli, "include", false).unwrap();
            assert_eq!(native.commit, cli.commit);
        } else {
            assert!(matches!(twin.commit("include", false), Outcome::Done(_)));
        }
    }
    assert!(git(&twin.native, &["log", "-1", "--format=%an <%ae>"]).contains("Included Name"));
}

#[test]
fn commit_in_a_linked_worktree_matches_git() {
    hermetic!();
    let twin = twin(seeded);
    let links: Vec<_> = [("native-wt", &twin.native), ("cli-wt", &twin.cli)]
        .iter()
        .map(|(parent, path)| {
            let link = path.parent().unwrap().join(parent).join("linked");
            git(
                path,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "linked",
                    link.to_str().unwrap(),
                ],
            );
            write(&link, "tracked.txt", "linked change\n");
            git(&link, &["add", "tracked.txt"]);
            link
        })
        .collect();
    assert_eq!(
        twin.auto.commit_route(&links[0], "linked", false),
        Route::Native
    );
    let native = twin.auto.commit(&links[0], "linked", false).unwrap();
    let cli = twin.cli_only.commit(&links[1], "linked", false).unwrap();
    assert_eq!(native.commit, cli.commit);
    assert_eq!(snapshot(&links[0]), snapshot(&links[1]));
    twin.assert_same("after the linked worktree commit");
}
