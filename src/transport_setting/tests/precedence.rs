//! Precedence and values (design §2; §9's "precedence" and "values" rows).

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::fixture::{GWZ, NATIVE, home, snapshot, write};
use crate::test_support::TempDir;
use crate::transport_setting::{Location, Refusal, Setting, Source, Transport, resolve};

/// Writes `contents` to the `.gitconfig` in `home`, and returns its path.
fn gitconfig(home: &Path, contents: &str) -> PathBuf {
    let file = home.join(".gitconfig");
    write(&file, contents);
    file
}

/// A setting that `file` decided with a value it holds itself.
fn global(transport: Transport, file: &Path) -> Setting {
    Setting {
        transport,
        source: Source::GlobalConfiguration(Location {
            file: file.to_path_buf(),
            included: false,
        }),
        read: vec![file.to_path_buf()],
        skipped: Vec::new(),
    }
}

/// A setting that a form above the global configuration decided.
fn decided(transport: Transport, source: Source) -> Setting {
    Setting {
        transport,
        source,
        read: Vec::new(),
        skipped: Vec::new(),
    }
}

/// A snapshot with `HOME` at `home` and `GWZ_TRANSPORT` set to `value`.
fn with_variable(home: &Path, value: &OsStr) -> crate::session_host::EnvironmentSnapshot {
    snapshot([("HOME", home.as_os_str()), ("GWZ_TRANSPORT", value)])
}

#[test]
fn with_no_form_gwz_applies_by_default() {
    let dir = TempDir::new("setting-default");
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(decided(Transport::Gwz, Source::Default))
    );
}

#[test]
fn each_form_alone_decides() {
    let dir = TempDir::new("setting-alone");
    for transport in [Transport::Gwz, Transport::Native] {
        assert_eq!(
            resolve(Some(transport), &home(dir.path())),
            Ok(decided(transport, Source::Flag))
        );
        assert_eq!(
            resolve(
                None,
                &with_variable(dir.path(), OsStr::new(transport.name()))
            ),
            Ok(decided(transport, Source::Environment))
        );
    }
    for (contents, transport) in [(GWZ, Transport::Gwz), (NATIVE, Transport::Native)] {
        let file = gitconfig(dir.path(), contents);
        assert_eq!(
            resolve(None, &home(dir.path())),
            Ok(global(transport, &file))
        );
    }
}

/// Each pair of forms, both ways round: the higher form decides, whichever
/// value each holds.
#[test]
fn of_each_pair_of_forms_the_higher_decides_both_ways() {
    let dir = TempDir::new("setting-pairs");
    for (higher, lower) in [
        (Transport::Gwz, Transport::Native),
        (Transport::Native, Transport::Gwz),
    ] {
        let lower_variable = with_variable(dir.path(), OsStr::new(lower.name()));
        assert_eq!(
            resolve(Some(higher), &lower_variable),
            Ok(decided(higher, Source::Flag)),
            "the flag over the variable"
        );
        gitconfig(
            dir.path(),
            &format!("[gwz]\n\ttransport = {}\n", lower.name()),
        );
        assert_eq!(
            resolve(Some(higher), &home(dir.path())),
            Ok(decided(higher, Source::Flag)),
            "the flag over the global configuration"
        );
        let higher_variable = with_variable(dir.path(), OsStr::new(higher.name()));
        assert_eq!(
            resolve(None, &higher_variable),
            Ok(decided(higher, Source::Environment)),
            "the variable over the global configuration"
        );
    }
}

/// A form below the deciding one is neither read nor checked: a malformed
/// variable under the flag, and under the flag or the variable a global
/// file that cannot be parsed, one whose value is malformed and one that
/// cannot be read. None of them is read, refused or listed as skipped.
#[test]
fn a_form_below_the_deciding_one_is_neither_read_nor_checked() {
    let dir = TempDir::new("setting-unread");
    for value in [
        OsStr::new("bogus"),
        OsStr::new("true"),
        OsStr::from_bytes(b"nat\xffive"),
    ] {
        assert_eq!(
            resolve(Some(Transport::Native), &with_variable(dir.path(), value)),
            Ok(decided(Transport::Native, Source::Flag))
        );
    }
    let variable = with_variable(dir.path(), OsStr::new("native"));
    for contents in [
        "[gwz\n",
        "[gwz]\n\ttransport = bogus\n",
        "[gwz]\n\ttransport\n",
    ] {
        gitconfig(dir.path(), contents);
        assert_eq!(
            resolve(Some(Transport::Gwz), &home(dir.path())),
            Ok(decided(Transport::Gwz, Source::Flag))
        );
        assert_eq!(
            resolve(None, &variable),
            Ok(decided(Transport::Native, Source::Environment))
        );
    }
    let file = gitconfig(dir.path(), NATIVE);
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    assert_eq!(
        resolve(Some(Transport::Gwz), &home(dir.path())),
        Ok(decided(Transport::Gwz, Source::Flag))
    );
    assert_eq!(
        resolve(None, &variable),
        Ok(decided(Transport::Native, Source::Environment))
    );
}

#[test]
fn the_variable_ignores_ascii_case_and_surrounding_white_space() {
    let dir = TempDir::new("setting-variable-case");
    for (value, transport) in [
        ("native", Transport::Native),
        ("NATIVE", Transport::Native),
        ("Native", Transport::Native),
        (" native ", Transport::Native),
        ("\tnative\n", Transport::Native),
        ("gwz", Transport::Gwz),
        ("GWZ", Transport::Gwz),
        (" Gwz\t", Transport::Gwz),
    ] {
        assert_eq!(
            resolve(None, &with_variable(dir.path(), OsStr::new(value))),
            Ok(decided(transport, Source::Environment)),
            "GWZ_TRANSPORT={value:?}"
        );
    }
}

/// An empty variable, or one of white space alone, is the same as unset: the
/// global configuration decides.
#[test]
fn an_empty_variable_is_unset() {
    let dir = TempDir::new("setting-variable-empty");
    let file = gitconfig(dir.path(), NATIVE);
    for value in ["", " ", " \t\n"] {
        assert_eq!(
            resolve(None, &with_variable(dir.path(), OsStr::new(value))),
            Ok(global(Transport::Native, &file)),
            "GWZ_TRANSPORT={value:?}"
        );
    }
}

/// Any other value is refused, git's other boolean spellings included, and
/// the refusal holds the value without its surrounding white space.
#[test]
fn the_variable_refuses_any_other_value() {
    let dir = TempDir::new("setting-variable-other");
    gitconfig(dir.path(), NATIVE);
    for (value, shown) in [
        ("bogus", "bogus"),
        (" bo gus\t", "bo gus"),
        ("native2", "native2"),
        ("nativ", "nativ"),
        ("true", "true"),
        ("false", "false"),
        ("1", "1"),
        ("on", "on"),
        ("yes", "yes"),
        ("n\u{0430}tive", "n\u{0430}tive"),
    ] {
        assert_eq!(
            resolve(None, &with_variable(dir.path(), OsStr::new(value))),
            Err(Refusal::Environment {
                value: shown.to_owned()
            }),
            "GWZ_TRANSPORT={value:?}"
        );
    }
}

#[test]
fn a_variable_that_is_not_utf8_is_refused() {
    let dir = TempDir::new("setting-variable-utf8");
    gitconfig(dir.path(), NATIVE);
    for value in [&b"native\xff"[..], b"\xff", b" \xfe "] {
        assert_eq!(
            resolve(None, &with_variable(dir.path(), OsStr::from_bytes(value))),
            Err(Refusal::EnvironmentNotUtf8)
        );
    }
}

/// The key's value ignores case, and so, as git reads it, does the key's
/// name.
#[test]
fn the_key_ignores_case() {
    let dir = TempDir::new("setting-key-case");
    for (contents, transport) in [
        ("[gwz]\n\ttransport = NATIVE\n", Transport::Native),
        ("[gwz]\n\ttransport = NaTiVe\n", Transport::Native),
        ("[gwz]\n\ttransport = GWZ\n", Transport::Gwz),
        ("[GWZ]\n\tTransport = native\n", Transport::Native),
        ("[gwz]\n\ttransport = \"native\"\n", Transport::Native),
    ] {
        let file = gitconfig(dir.path(), contents);
        assert_eq!(
            resolve(None, &home(dir.path())),
            Ok(global(transport, &file)),
            "{contents:?}"
        );
    }
}

/// Any other value is refused, git's other booleans included, and so are an
/// empty value and a key with no value. The key, unlike the variable, is not
/// trimmed: a quoted value keeps its white space.
#[test]
fn the_key_refuses_any_other_value_an_empty_value_and_a_key_with_no_value() {
    let dir = TempDir::new("setting-key-other");
    for (line, value) in [
        ("transport = bogus", Some("bogus")),
        ("transport = true", Some("true")),
        ("transport = 1", Some("1")),
        ("transport = yes", Some("yes")),
        ("transport = on", Some("on")),
        ("transport = native2", Some("native2")),
        ("transport =", Some("")),
        ("transport = \" native\"", Some(" native")),
        ("transport", None),
    ] {
        let file = gitconfig(dir.path(), &format!("[gwz]\n\t{line}\n"));
        assert_eq!(
            resolve(None, &home(dir.path())),
            Err(Refusal::Configuration {
                location: Location {
                    file,
                    included: false
                },
                value: value.map(str::to_owned),
            }),
            "{line:?}"
        );
    }
    let file = gitconfig(dir.path(), "");
    write(&file, b"[gwz]\n\ttransport = nat\xffive\n");
    assert_eq!(
        resolve(None, &home(dir.path())),
        Err(Refusal::Configuration {
            location: Location {
                file,
                included: false
            },
            value: Some("nat\u{fffd}ive".to_owned()),
        })
    );
}

/// Within a file the last value wins, as git reads one key, and only that
/// value is checked.
#[test]
fn within_a_file_the_last_value_wins() {
    let dir = TempDir::new("setting-key-last");
    let file = gitconfig(
        dir.path(),
        "[gwz]\n\ttransport = bogus\n\ttransport = native\n",
    );
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(global(Transport::Native, &file))
    );
    for (contents, value) in [
        (
            "[gwz]\n\ttransport = native\n[gwz]\n\ttransport = bogus\n",
            Some("bogus"),
        ),
        ("[gwz]\n\ttransport = native\n\ttransport\n", None),
    ] {
        let file = gitconfig(dir.path(), contents);
        assert_eq!(
            resolve(None, &home(dir.path())),
            Err(Refusal::Configuration {
                location: Location {
                    file,
                    included: false
                },
                value: value.map(str::to_owned),
            }),
            "{contents:?}"
        );
    }
}
