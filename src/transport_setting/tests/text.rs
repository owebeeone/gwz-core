//! §10's words: each refusal in each driver's words, the pieces that the
//! drivers' notes share (`<where>`, `<remove>`, `<entry>` and `<scope>`),
//! E1's quoting and escaping, and the printed commands run as given (§9's
//! "includes" row, and the quoting rows applied to the module's own strings).
//! The printed commands have a form for each platform (decision D11): POSIX
//! single quotes, run through `sh`, and double quotes on Windows, run through
//! cmd.exe and through PowerShell.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::fixture::{GWZ, NATIVE, SHELLS, home, lines, write};
use crate::test_support::TempDir;
use crate::transport_setting::text::windows_quoted;
use crate::transport_setting::{
    Driver, IgnoredValue, Location, Refusal, Scope, Source, Transport, path_text, resolve,
};

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;

        /// A user's home directory, as a path of this platform.
        const HOME_DIR: &str = "/home/u";
        /// The platform's path separator.
        const SEP: char = '/';

        /// `path` as the printed command quotes it.
        fn quoted(path: &str) -> String {
            format!("'{path}'")
        }

        /// A directory name that holds the characters the quoting must survive, and would run a command if the
        /// quoting failed: `$(touch marker)` is a command substitution in `sh`.
        const AWKWARD_NAME: &str = "it's $(touch marker)";

        /// The quoting of an awkward path.
        fn awkward_quoted(file: &str) -> String {
            format!("'{}'", file.replace('\'', r"'\''"))
        }

        /// A path that is not UTF-8.
        fn not_utf8() -> PathBuf {
            PathBuf::from(std::ffi::OsStr::from_bytes(b"/home/u\xff/.gitconfig"))
        }
    } else {
        use std::os::windows::ffi::OsStringExt;

        const HOME_DIR: &str = r"C:\Users\u";
        const SEP: char = '\\';

        fn quoted(path: &str) -> String {
            format!("\"{path}\"")
        }

        /// The characters the quoting must survive on Windows: an apostrophe, parentheses, a space and the `&` at
        /// which cmd.exe would run a second command (`mkdir marker`), none of which a double quote lets a shell read.
        const AWKWARD_NAME: &str = "it's (x) & mkdir marker & y";

        fn awkward_quoted(file: &str) -> String {
            format!("\"{file}\"")
        }

        /// A path that is not UTF-8: an unpaired surrogate.
        fn not_utf8() -> PathBuf {
            let mut units: Vec<u16> = r"C:\Users\u".encode_utf16().collect();
            units.push(0xD800);
            units.extend(r"\.gitconfig".encode_utf16());
            PathBuf::from(std::ffi::OsString::from_wide(&units))
        }
    }
}

/// A file of the home directory, as a path of this platform.
fn in_home(name: &str) -> String {
    format!("{HOME_DIR}{SEP}{name}")
}

/// `path` as text with forward slashes and no `\\?\` prefix, which git's `includeIf "gitdir:"` patterns and its
/// `file:` lines use on every platform.
fn slashed(path: &Path) -> String {
    let text = path.display().to_string();
    text.trim_start_matches(r"\\?\").replace('\\', "/")
}

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
    let file = in_home(".gitconfig");
    let inc = in_home("inc");
    let held = location(&file, false);
    let included = location(&file, true);
    let unset = format!(
        "git config --file {} --unset-all gwz.transport",
        quoted(&file)
    );
    let locate = format!(
        "git -C / config --file {} --includes --show-origin --get-all gwz.transport shows which file holds it; remove it there",
        quoted(&file)
    );
    let cause = format!("failed to parse config file: missing ']' in section header (in {inc}:3)");
    let cases = [
        (
            Refusal::Environment {
                value: "bogus".to_owned(),
            },
            r#"GWZ_TRANSPORT must be gwz or native, not "bogus"; --transport decides without it"#
                .to_owned(),
            r#"GWZ_TRANSPORT must be gwz or native, not "bogus""#.to_owned(),
        ),
        (
            Refusal::EnvironmentNotUtf8,
            "GWZ_TRANSPORT is not valid UTF-8; --transport decides without it".to_owned(),
            "GWZ_TRANSPORT is not valid UTF-8".to_owned(),
        ),
        (
            Refusal::Configuration {
                location: held.clone(),
                value: Some("true".to_owned()),
            },
            format!(
                "gwz.transport in {file} must be gwz or native, not \"true\"; remove it with {unset}, or fix it; \
                 --transport or GWZ_TRANSPORT decides without it"
            ),
            format!(
                "gwz.transport in {file} must be gwz or native, not \"true\"; remove it with {unset}, or fix it; \
                 GWZ_TRANSPORT decides without it"
            ),
        ),
        (
            Refusal::Configuration {
                location: held,
                value: None,
            },
            format!(
                "gwz.transport in {file} must be gwz or native, and has no value; remove it with {unset}, or fix \
                 it; --transport or GWZ_TRANSPORT decides without it"
            ),
            format!(
                "gwz.transport in {file} must be gwz or native, and has no value; remove it with {unset}, or fix \
                 it; GWZ_TRANSPORT decides without it"
            ),
        ),
        (
            Refusal::Configuration {
                location: included,
                value: Some(String::new()),
            },
            format!(
                "gwz.transport in a file included by {file} must be gwz or native, not \"\"; {locate}, or fix it; \
                 --transport or GWZ_TRANSPORT decides without it"
            ),
            format!(
                "gwz.transport in a file included by {file} must be gwz or native, not \"\"; {locate}, or fix it; \
                 GWZ_TRANSPORT decides without it"
            ),
        ),
        (
            Refusal::Unparsable {
                file: PathBuf::from(&file),
                cause: cause.clone(),
            },
            format!(
                "could not parse {file}, or a file it includes, for gwz.transport: {cause}; fix the file and line \
                 it names; --transport or GWZ_TRANSPORT decides without it"
            ),
            format!(
                "could not parse {file}, or a file it includes, for gwz.transport: {cause}; fix the file and line \
                 it names; GWZ_TRANSPORT decides without it"
            ),
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
    let file = in_home(".gitconfig");
    let held = location(&file, false);
    assert_eq!(held.where_text(), file);
    assert_eq!(
        held.remove_text(),
        format!(
            "remove it with git config --file {} --unset-all gwz.transport",
            quoted(&file)
        )
    );
    let included = location(&file, true);
    assert_eq!(included.where_text(), format!("a file included by {file}"));
    assert_eq!(
        included.remove_text(),
        format!(
            "git -C / config --file {} --includes --show-origin --get-all gwz.transport shows which file holds \
             it; remove it there",
            quoted(&file)
        )
    );
    let awkward = in_home(&format!("{AWKWARD_NAME}{SEP}.gitconfig"));
    let held = location(&awkward, false);
    assert_eq!(held.where_text(), awkward);
    assert_eq!(
        held.remove_text(),
        format!(
            "remove it with git config --file {} --unset-all gwz.transport",
            awkward_quoted(&awkward)
        )
    );
}

/// The Windows quoting (decision D11), a pure function of the path's text so that it is tested on every platform: a
/// drive path in double quotes, and the words (no command) for a path some shell would read differently.
#[test]
fn windows_quotes_a_drive_path_in_double_quotes_and_falls_back_for_anything_a_shell_expands() {
    for (path, quoted) in [
        (
            r"C:\Users\x y\.gitconfig",
            Some(r#""C:\Users\x y\.gitconfig""#),
        ),
        (r"d:\a b\c\.gitconfig", Some(r#""d:\a b\c\.gitconfig""#)),
        ("C:/Users/x/.gitconfig", Some(r#""C:/Users/x/.gitconfig""#)),
        // Characters that no shell reads inside double quotes.
        (
            r"C:\it's (x) & y ^ z ; {a}, [b] #\.gitconfig",
            Some(r#""C:\it's (x) & y ^ z ; {a}, [b] #\.gitconfig""#),
        ),
        (
            "C:\\Users\\caf\u{e9} \u{4e2d}\\.gitconfig",
            Some("\"C:\\Users\\caf\u{e9} \u{4e2d}\\.gitconfig\""),
        ),
        // A character that PowerShell, bash or cmd.exe expands, or reads as a quote, inside double quotes.
        (r"C:\Users\a$b\.gitconfig", None),
        (r"C:\Users\$(x)\.gitconfig", None),
        ("C:\\Users\\a`b\\.gitconfig", None),
        (r"C:\Users\100%\.gitconfig", None),
        (r"C:\Users\%USERPROFILE%\.gitconfig", None),
        (r"C:\Users\hi!\.gitconfig", None),
        ("C:\\Users\\a\u{201c}b\\.gitconfig", None),
        ("C:\\Users\\a\u{201d}b\\.gitconfig", None),
        ("C:\\Users\\a\u{201e}b\\.gitconfig", None),
        ("C:\\Users\\a\"b\\.gitconfig", None),
        // A control character, a path that is not a drive path, and the double backslash that bash halves.
        ("C:\\Users\\a\nb\\.gitconfig", None),
        ("C:\\Users\\a\tb\\.gitconfig", None),
        (r"\\server\share\.gitconfig", None),
        (r"\\?\C:\Users\x\.gitconfig", None),
        (r"C:\Users\\x\.gitconfig", None),
        (r"Users\x\.gitconfig", None),
        (r"\Users\x\.gitconfig", None),
        (r"C:.gitconfig", None),
        ("", None),
    ] {
        assert_eq!(windows_quoted(path).as_deref(), quoted, "{path:?}");
    }
}

/// A path that holds a control character is shown with it escaped, on one
/// line, and gets no command, as do a path that is not UTF-8 and one that is
/// not absolute, which no command could name (E1).
#[test]
fn a_path_no_command_can_name_gets_none_and_one_escaped_line() {
    for (file, shown) in [
        (
            in_home("u\nx") + &format!("{SEP}.gitconfig"),
            in_home("u\\nx") + &format!("{SEP}.gitconfig"),
        ),
        (
            in_home("u\tx\r") + &format!("{SEP}.gitconfig"),
            in_home("u\\tx\\r") + &format!("{SEP}.gitconfig"),
        ),
        (
            in_home("\u{1b}[31mred") + &format!("{SEP}.gitconfig"),
            in_home("\\u{1b}[31mred") + &format!("{SEP}.gitconfig"),
        ),
        (
            in_home("u\u{7f}\u{85}") + &format!("{SEP}.gitconfig"),
            in_home("u\\u{7f}\\u{85}") + &format!("{SEP}.gitconfig"),
        ),
    ] {
        assert_eq!(path_text(std::path::Path::new(&file)), shown);
        let held = location(&file, false);
        assert_eq!(held.where_text(), shown);
        assert_eq!(held.remove_text(), "remove it from that file");
        let included = location(&file, true);
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
                file: PathBuf::from(&file),
                cause: format!("failed (in {file}:1)"),
            },
        ] {
            let message = refusal.message(Driver::Cli);
            assert!(!message.contains(char::is_control), "{message:?}");
        }
    }
    let not_utf8 = Location {
        file: not_utf8(),
        included: false,
    };
    assert_eq!(
        not_utf8.where_text(),
        format!("{HOME_DIR}\u{fffd}{SEP}.gitconfig")
    );
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

/// The removal command, run as printed in each shell of the platform, removes every line of the key from the file it
/// names, and nothing else.
#[test]
fn the_removal_command_run_as_given_removes_every_line_of_the_key() {
    for shell in SHELLS {
        let dir = TempDir::new("setting-remove");
        let gitconfig = dir.path().join(".gitconfig");
        write(
            &gitconfig,
            "[gwz]\n\ttransport = native\n[user]\n\tname = U\n[gwz]\n\ttransport = native\n",
        );
        let remove = decided_location(dir.path()).remove_text();
        lines(&shell.run(
            remove.strip_prefix("remove it with ").unwrap(),
            dir.path(),
            dir.path(),
        ));
        assert_eq!(
            resolve(None, &home(dir.path())).unwrap().source,
            Source::Default,
            "{shell:?}"
        );
        assert!(fs::read_to_string(&gitconfig).unwrap().contains("name = U"));
    }
}

/// For a value that only a file `~/.gitconfig` includes holds, the locating
/// command, run as printed from the workspace root and from the home
/// directory, prints one line, naming that file, even when an `includeIf`
/// that matches the workspace includes a second file holding the key; and
/// `git config --file` on the named file with `--unset-all` then removes the
/// value.
#[test]
fn the_locating_command_names_the_one_included_file_gwz_read() {
    for shell in SHELLS {
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
                slashed(&workspace)
            ),
        );
        let location = decided_location(&user);
        assert!(location.included);
        let remove = location.remove_text();
        let command = remove
            .strip_suffix(" shows which file holds it; remove it there")
            .unwrap();
        // git C-quotes a path that holds a backslash (`file:"E:\\dir\\inc/first.inc"`), as on Windows.
        let named = |line: &str| {
            let path = line
                .strip_prefix("file:")
                .and_then(|line| line.strip_suffix("\tnative"))
                .unwrap_or_else(|| panic!("{line:?}"));
            match path
                .strip_prefix('"')
                .and_then(|path| path.strip_suffix('"'))
            {
                Some(quoted) => quoted.replace("\\\\", "\\"),
                None => path.to_owned(),
            }
        };
        let first = fs::canonicalize(&first).unwrap();
        for cwd in [&workspace, &user] {
            let found = lines(&shell.run(command, cwd, &user));
            assert_eq!(
                found.len(),
                1,
                "{found:?} from {} in {shell:?}",
                cwd.display()
            );
            assert_eq!(fs::canonicalize(named(&found[0])).unwrap(), first);
        }
        // In the workspace, without `-C /`, git applies the includeIf and lists
        // the second file too, which gwz never reads.
        let unanchored = command.replacen("git -C / config", "git config", 1);
        assert_eq!(
            lines(&shell.run(&unanchored, &workspace, &user)).len(),
            2,
            "{shell:?}"
        );
        let file = named(&lines(&shell.run(command, &user, &user))[0]);
        let unset = Command::new("git")
            .args(["config", "--file", &file, "--unset-all", "gwz.transport"])
            .env("HOME", &user)
            .env("USERPROFILE", &user)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(unset.success());
        assert_eq!(resolve(None, &home(&user)).unwrap().source, Source::Default);
    }
}

/// A path holding the characters that would run something if the quoting failed (`AWKWARD_NAME`): each command, run
/// through each shell of the platform, does its work and runs nothing else, so no `marker` appears.
#[test]
fn a_path_holding_the_awkward_characters_runs_nothing_else() {
    for shell in SHELLS {
        let dir = TempDir::new("setting-quote");
        let user = dir.path().join(AWKWARD_NAME);
        write(&user.join(".gitconfig"), NATIVE);
        let remove = decided_location(&user).remove_text();
        let file = format!("{}{SEP}{AWKWARD_NAME}{SEP}.gitconfig", dir.path().display());
        assert_eq!(
            remove,
            format!(
                "remove it with git config --file {} --unset-all gwz.transport",
                awkward_quoted(&file)
            )
        );
        lines(&shell.run(
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
        assert_eq!(
            lines(&shell.run(command, dir.path(), &user)).len(),
            1,
            "{shell:?}"
        );
        assert!(!dir.path().join("marker").exists(), "{shell:?}");
        assert!(!user.join("marker").exists(), "{shell:?}");
    }
}

// A newline in a name is Unix's: Windows forbids one (operator decision D9, 2026-10-12).
cfg_if::cfg_if! {
    if #[cfg(unix)] {
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
    }
}

// A directory name that a Windows shell would expand gives the words, not a command, and the refusal still names the
// file: the fallback of decision D11, on a real file.
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[test]
        fn a_global_file_under_an_expanding_name_gives_the_words_and_no_command() {
            let dir = TempDir::new("setting-expanding");
            for name in ["100%", "a$b", "back`tick", "hi!"] {
                let user = dir.path().join(name);
                write(&user.join(".gitconfig"), "[gwz]\n\ttransport = bogus\n");
                let refusal = resolve(None, &home(&user)).unwrap_err();
                assert_eq!(
                    refusal.message(Driver::Cli),
                    format!(
                        "gwz.transport in {}\\{name}\\.gitconfig must be gwz or native, not \"bogus\"; remove it \
                         from that file, or fix it; --transport or GWZ_TRANSPORT decides without it",
                        dir.path().display()
                    ),
                    "{name}"
                );
            }
        }
    }
}
