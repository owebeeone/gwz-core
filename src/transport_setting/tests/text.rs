//! §10's words: each refusal in each driver's words, the pieces that the
//! drivers' notes share (`<where>`, `<remove>`, `<entry>` and `<scope>`),
//! E1's quoting and escaping, and the printed commands run as given (§9's
//! "includes" row, and the quoting rows applied to the module's own strings).

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;

use super::fixture::{GWZ, NATIVE, home, lines, sh, write};
use crate::test_support::TempDir;
use crate::transport_setting::{
    Driver, IgnoredValue, Location, Refusal, Scope, Source, Transport, path_text, resolve,
};

fn location(file: &str, included: bool) -> Location {
    Location {
        file: PathBuf::from(file),
        included,
    }
}

/// The location that decided, from a resolution through the global
/// configuration.
fn decided_location(home_dir: &std::path::Path) -> Location {
    match resolve(None, &home(home_dir)).unwrap().source {
        Source::GlobalConfiguration(location) => location,
        other => panic!("decided by {other:?}"),
    }
}

#[test]
fn each_refusal_takes_section_10s_words_in_each_driver() {
    let held = location("/home/u/.gitconfig", false);
    let included = location("/home/u/.gitconfig", true);
    let cases = [
        (
            Refusal::Environment {
                value: "bogus".to_owned(),
            },
            r#"GWZ_TRANSPORT must be gwz or native, not "bogus"; --transport decides without it"#,
            r#"GWZ_TRANSPORT must be gwz or native, not "bogus""#,
        ),
        (
            Refusal::EnvironmentNotUtf8,
            "GWZ_TRANSPORT is not valid UTF-8; --transport decides without it",
            "GWZ_TRANSPORT is not valid UTF-8",
        ),
        (
            Refusal::Configuration {
                location: held.clone(),
                value: Some("true".to_owned()),
            },
            "gwz.transport in /home/u/.gitconfig must be gwz or native, not \"true\"; remove it with \
             git config --file '/home/u/.gitconfig' --unset-all gwz.transport, or fix it; \
             --transport or GWZ_TRANSPORT decides without it",
            "gwz.transport in /home/u/.gitconfig must be gwz or native, not \"true\"; remove it with \
             git config --file '/home/u/.gitconfig' --unset-all gwz.transport, or fix it; \
             GWZ_TRANSPORT decides without it",
        ),
        (
            Refusal::Configuration {
                location: held,
                value: None,
            },
            "gwz.transport in /home/u/.gitconfig must be gwz or native, and has no value; remove it \
             with git config --file '/home/u/.gitconfig' --unset-all gwz.transport, or fix it; \
             --transport or GWZ_TRANSPORT decides without it",
            "gwz.transport in /home/u/.gitconfig must be gwz or native, and has no value; remove it \
             with git config --file '/home/u/.gitconfig' --unset-all gwz.transport, or fix it; \
             GWZ_TRANSPORT decides without it",
        ),
        (
            Refusal::Configuration {
                location: included,
                value: Some(String::new()),
            },
            "gwz.transport in a file included by /home/u/.gitconfig must be gwz or native, not \"\"; \
             git -C / config --file '/home/u/.gitconfig' --includes --show-origin --get-all \
             gwz.transport shows which file holds it; remove it there, or fix it; --transport or \
             GWZ_TRANSPORT decides without it",
            "gwz.transport in a file included by /home/u/.gitconfig must be gwz or native, not \"\"; \
             git -C / config --file '/home/u/.gitconfig' --includes --show-origin --get-all \
             gwz.transport shows which file holds it; remove it there, or fix it; GWZ_TRANSPORT \
             decides without it",
        ),
        (
            Refusal::Unparsable {
                file: PathBuf::from("/home/u/.gitconfig"),
                cause: "failed to parse config file: missing ']' in section header (in \
                        /home/u/inc:3)"
                    .to_owned(),
            },
            "could not parse /home/u/.gitconfig, or a file it includes, for gwz.transport: failed \
             to parse config file: missing ']' in section header (in /home/u/inc:3); fix the file \
             and line it names; --transport or GWZ_TRANSPORT decides without it",
            "could not parse /home/u/.gitconfig, or a file it includes, for gwz.transport: failed \
             to parse config file: missing ']' in section header (in /home/u/inc:3); fix the file \
             and line it names; GWZ_TRANSPORT decides without it",
        ),
    ];
    for (refusal, cli, python) in cases {
        assert_eq!(refusal.message(Driver::Cli), cli);
        assert_eq!(refusal.message(Driver::Python), python);
    }
}

/// A value is quoted, with its control characters, quotes and backslashes
/// escaped, as `GWZ_URL_SCHEME`'s message shows a value (E1).
#[test]
fn a_value_is_shown_quoted_and_escaped() {
    let refusal = Refusal::Environment {
        value: "a\"b\\c\nd\u{1b}".to_owned(),
    };
    assert_eq!(
        refusal.message(Driver::Python),
        r#"GWZ_TRANSPORT must be gwz or native, not "a\"b\\c\nd\u{1b}""#
    );
}

#[test]
fn where_and_remove_name_a_held_value_and_an_included_one() {
    let held = location("/home/u/.gitconfig", false);
    assert_eq!(held.where_text(), "/home/u/.gitconfig");
    assert_eq!(
        held.remove_text(),
        "remove it with git config --file '/home/u/.gitconfig' --unset-all gwz.transport"
    );
    let included = location("/home/u/.gitconfig", true);
    assert_eq!(
        included.where_text(),
        "a file included by /home/u/.gitconfig"
    );
    assert_eq!(
        included.remove_text(),
        "git -C / config --file '/home/u/.gitconfig' --includes --show-origin --get-all \
         gwz.transport shows which file holds it; remove it there"
    );
    let quoted = location("/home/it's $(x)/.gitconfig", false);
    assert_eq!(quoted.where_text(), "/home/it's $(x)/.gitconfig");
    assert_eq!(
        quoted.remove_text(),
        r"remove it with git config --file '/home/it'\''s $(x)/.gitconfig' --unset-all gwz.transport"
    );
}

/// A path that holds a control character is shown with it escaped, on one
/// line, and gets no command, as do a path that is not UTF-8 and one that is
/// not absolute, which no command could name (E1).
#[test]
fn a_path_no_command_can_name_gets_none_and_one_escaped_line() {
    for (file, shown) in [
        ("/home/u\nx/.gitconfig", r"/home/u\nx/.gitconfig"),
        ("/home/u\tx\r/.gitconfig", r"/home/u\tx\r/.gitconfig"),
        (
            "/home/\u{1b}[31mred/.gitconfig",
            r"/home/\u{1b}[31mred/.gitconfig",
        ),
        (
            "/home/u\u{7f}\u{85}/.gitconfig",
            r"/home/u\u{7f}\u{85}/.gitconfig",
        ),
    ] {
        assert_eq!(path_text(std::path::Path::new(file)), shown);
        let held = location(file, false);
        assert_eq!(held.where_text(), shown);
        assert_eq!(held.remove_text(), "remove it from that file");
        let included = location(file, true);
        assert_eq!(included.where_text(), format!("a file included by {shown}"));
        assert_eq!(
            included.remove_text(),
            "remove it from the file that holds it"
        );
        for refusal in [
            Refusal::Configuration {
                location: held,
                value: Some("x\ny".to_owned()),
            },
            Refusal::Unparsable {
                file: PathBuf::from(file),
                cause: format!("failed (in {file}:1)"),
            },
        ] {
            let message = refusal.message(Driver::Cli);
            assert!(!message.contains(char::is_control), "{message:?}");
        }
    }
    let not_utf8 = Location {
        file: PathBuf::from(OsStr::from_bytes(b"/home/u\xff/.gitconfig")),
        included: false,
    };
    assert_eq!(not_utf8.where_text(), "/home/u\u{fffd}/.gitconfig");
    assert_eq!(not_utf8.remove_text(), "remove it from that file");
    assert_eq!(
        location("home/.gitconfig", false).remove_text(),
        "remove it from that file"
    );
}

#[test]
fn entry_scope_and_the_json_words() {
    let ignored = |value: Option<&str>| IgnoredValue {
        scope: Scope::Root,
        location: location("/ws/.git/config", false),
        value: value.map(str::to_owned),
    };
    assert_eq!(
        ignored(Some("native")).entry_text(),
        r#"gwz.transport = "native""#
    );
    assert_eq!(ignored(Some("")).entry_text(), r#"gwz.transport = """#);
    assert_eq!(
        ignored(Some("x\ty")).entry_text(),
        r#"gwz.transport = "x\ty""#
    );
    assert_eq!(ignored(None).entry_text(), "gwz.transport");
    assert_eq!(Scope::Root.text(), "root");
    assert_eq!(Scope::Member("mem_a".to_owned()).text(), "member mem_a");
    assert_eq!(
        Scope::Member("mem_a\nb".to_owned()).text(),
        r"member mem_a\nb"
    );
    assert_eq!(
        (Scope::Root.name(), Scope::Root.member_id()),
        ("root", None)
    );
    let member = Scope::Member("mem_a".to_owned());
    assert_eq!(
        (member.name(), member.member_id()),
        ("member", Some("mem_a"))
    );
    assert_eq!(
        [Transport::Gwz.name(), Transport::Native.name()],
        ["gwz", "native"]
    );
    assert_eq!(
        [
            Source::Flag.name(),
            Source::Environment.name(),
            Source::GlobalConfiguration(location("/home/u/.gitconfig", false)).name(),
            Source::Default.name(),
        ],
        ["flag", "environment", "global_configuration", "default"]
    );
}

/// The removal command, run as printed, removes every line of the key from
/// the file it names, and nothing else.
#[test]
fn the_removal_command_run_as_given_removes_every_line_of_the_key() {
    let dir = TempDir::new("setting-remove");
    let gitconfig = dir.path().join(".gitconfig");
    write(
        &gitconfig,
        "[gwz]\n\ttransport = native\n[user]\n\tname = U\n[gwz]\n\ttransport = native\n",
    );
    let remove = decided_location(dir.path()).remove_text();
    lines(&sh(
        remove.strip_prefix("remove it with ").unwrap(),
        dir.path(),
        dir.path(),
    ));
    assert_eq!(
        resolve(None, &home(dir.path())).unwrap().source,
        Source::Default
    );
    assert!(fs::read_to_string(&gitconfig).unwrap().contains("name = U"));
}

/// For a value that only a file `~/.gitconfig` includes holds, the locating
/// command, run as printed from the workspace root and from the home
/// directory, prints one line, naming that file, even when an `includeIf`
/// that matches the workspace includes a second file holding the key; and
/// `git config --file` on the named file with `--unset-all` then removes the
/// value.
#[test]
fn the_locating_command_names_the_one_included_file_gwz_read() {
    let dir = TempDir::new("setting-locate");
    let user = dir.path().join("home");
    git2::Repository::init(dir.path().join("ws")).unwrap();
    let workspace = fs::canonicalize(dir.path().join("ws")).unwrap();
    let first = user.join("inc/first.inc");
    write(&first, NATIVE);
    write(&user.join("inc/second.inc"), GWZ);
    write(
        &user.join(".gitconfig"),
        format!(
            "[include]\n\tpath = inc/first.inc\n[includeIf \"gitdir:{}/\"]\n\tpath = inc/second.inc\n",
            workspace.display()
        ),
    );
    let location = decided_location(&user);
    assert!(location.included);
    let remove = location.remove_text();
    let command = remove
        .strip_suffix(" shows which file holds it; remove it there")
        .unwrap();
    let named = |line: &str| {
        let path = line
            .strip_prefix("file:")
            .and_then(|line| line.strip_suffix("\tnative"));
        path.unwrap_or_else(|| panic!("{line:?}")).to_owned()
    };
    let first = fs::canonicalize(&first).unwrap();
    for cwd in [&workspace, &user] {
        let found = lines(&sh(command, cwd, &user));
        assert_eq!(found.len(), 1, "{found:?} from {}", cwd.display());
        assert_eq!(fs::canonicalize(named(&found[0])).unwrap(), first);
    }
    // In the workspace, without `-C /`, git applies the includeIf and lists
    // the second file too, which gwz never reads.
    let unanchored = command.replacen("git -C / config", "git config", 1);
    assert_eq!(lines(&sh(&unanchored, &workspace, &user)).len(), 2);
    let file = named(&lines(&sh(command, &user, &user))[0]);
    let unset = Command::new("git")
        .args(["config", "--file", &file, "--unset-all", "gwz.transport"])
        .env("HOME", &user)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(unset.success());
    assert_eq!(resolve(None, &home(&user)).unwrap().source, Source::Default);
}

/// A path holding a space, a `'` and `$(` together: each command, run
/// through `sh`, does its work and runs nothing else, so no `marker` appears.
#[test]
fn a_path_holding_a_quote_a_space_and_dollar_paren_runs_nothing_else() {
    let dir = TempDir::new("setting-quote");
    let user = dir.path().join("it's $(touch marker)");
    write(&user.join(".gitconfig"), NATIVE);
    let remove = decided_location(&user).remove_text();
    assert_eq!(
        remove,
        format!(
            r"remove it with git config --file '{}/it'\''s $(touch marker)/.gitconfig' --unset-all gwz.transport",
            dir.path().display()
        )
    );
    lines(&sh(
        remove.strip_prefix("remove it with ").unwrap(),
        dir.path(),
        &user,
    ));
    assert_eq!(resolve(None, &home(&user)).unwrap().source, Source::Default);
    write(&user.join("inc.inc"), NATIVE);
    write(&user.join(".gitconfig"), "[include]\n\tpath = inc.inc\n");
    let remove = decided_location(&user).remove_text();
    let command = remove
        .strip_suffix(" shows which file holds it; remove it there")
        .unwrap();
    assert_eq!(lines(&sh(command, dir.path(), &user)).len(), 1);
    assert!(!dir.path().join("marker").exists());
    assert!(!user.join("marker").exists());
}

/// A global file under a path that holds a newline: the refusal is one line,
/// with the path escaped and no command.
#[test]
fn a_global_file_under_a_newline_gives_one_escaped_line_and_no_command() {
    let dir = TempDir::new("setting-newline");
    let user = dir.path().join("a\nb");
    write(&user.join(".gitconfig"), "[gwz]\n\ttransport = bogus\n");
    let refusal = resolve(None, &home(&user)).unwrap_err();
    assert_eq!(
        refusal.message(Driver::Cli),
        format!(
            "gwz.transport in {}/a\\nb/.gitconfig must be gwz or native, not \"bogus\"; remove it \
             from that file, or fix it; --transport or GWZ_TRANSPORT decides without it",
            dir.path().display()
        )
    );
}
