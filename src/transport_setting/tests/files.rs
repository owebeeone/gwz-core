//! The global files (design §2; §9's "files" row, its unreadable and
//! unparsable files, and the value-reporting half of its "includes" row).

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use super::fixture::{GWZ, NATIVE, fifo, home, libgit2_home, snapshot, within, write};
use crate::test_support::TempDir;
use crate::transport_setting::{Location, Refusal, Setting, Source, Transport, resolve};

/// A setting that `file` decided, with the files read and none skipped.
fn decided_by(transport: Transport, file: &Path, included: bool, read: &[&Path]) -> Setting {
    Setting {
        transport,
        source: Source::GlobalConfiguration(Location {
            file: file.to_path_buf(),
            included,
        }),
        read: read.iter().map(|file| file.to_path_buf()).collect(),
        skipped: Vec::new(),
    }
}

/// The default, having read `read`.
fn by_default(read: &[&Path]) -> Setting {
    Setting {
        transport: Transport::Gwz,
        source: Source::Default,
        read: read.iter().map(|file| file.to_path_buf()).collect(),
        skipped: Vec::new(),
    }
}

#[test]
fn the_xdg_file_is_read_then_gitconfig_whose_value_wins() {
    let dir = TempDir::new("setting-xdg-order");
    let (user, xdg_home) = (dir.path().join("home"), dir.path().join("xdg"));
    let (xdg, gitconfig) = (xdg_home.join("git/config"), user.join(".gitconfig"));
    let environment = snapshot([
        ("HOME", user.as_os_str()),
        ("XDG_CONFIG_HOME", xdg_home.as_os_str()),
    ]);
    write(&xdg, NATIVE);
    assert_eq!(
        resolve(None, &environment),
        Ok(decided_by(Transport::Native, &xdg, false, &[&xdg]))
    );
    write(&gitconfig, "[user]\n\tname = U\n");
    assert_eq!(
        resolve(None, &environment),
        Ok(decided_by(
            Transport::Native,
            &xdg,
            false,
            &[&xdg, &gitconfig]
        )),
        "a .gitconfig without the key leaves the XDG file's value"
    );
    write(&gitconfig, GWZ);
    assert_eq!(
        resolve(None, &environment),
        Ok(decided_by(
            Transport::Gwz,
            &gitconfig,
            false,
            &[&xdg, &gitconfig]
        ))
    );
    write(&xdg, GWZ);
    write(&gitconfig, NATIVE);
    assert_eq!(
        resolve(None, &environment),
        Ok(decided_by(
            Transport::Native,
            &gitconfig,
            false,
            &[&xdg, &gitconfig]
        ))
    );
}

#[test]
fn an_unset_or_empty_xdg_config_home_reads_home_dot_config() {
    let dir = TempDir::new("setting-xdg-empty");
    let xdg = dir.path().join(".config/git/config");
    write(&xdg, NATIVE);
    for environment in [
        home(dir.path()),
        snapshot([
            ("HOME", dir.path().as_os_str()),
            ("XDG_CONFIG_HOME", OsStr::new("")),
        ]),
    ] {
        assert_eq!(
            resolve(None, &environment),
            Ok(decided_by(Transport::Native, &xdg, false, &[&xdg]))
        );
    }
}

/// A set `XDG_CONFIG_HOME` replaces `$HOME/.config` as git's does: an
/// absolute one names its own file, and a relative one names none.
#[test]
fn a_set_xdg_config_home_replaces_home_dot_config() {
    let dir = TempDir::new("setting-xdg-set");
    write(&dir.path().join(".config/git/config"), NATIVE);
    let elsewhere = dir.path().join("elsewhere");
    for xdg_home in [
        elsewhere.as_os_str(),
        OsStr::new("xdg"),
        OsStr::new("./xdg"),
    ] {
        let environment = snapshot([
            ("HOME", dir.path().as_os_str()),
            ("XDG_CONFIG_HOME", xdg_home),
        ]);
        assert_eq!(
            resolve(None, &environment),
            Ok(by_default(&[])),
            "{xdg_home:?}"
        );
    }
    let gitconfig = dir.path().join(".gitconfig");
    write(&gitconfig, GWZ);
    let relative = snapshot([
        ("HOME", dir.path().as_os_str()),
        ("XDG_CONFIG_HOME", OsStr::new("xdg")),
    ]);
    assert_eq!(
        resolve(None, &relative),
        Ok(decided_by(Transport::Gwz, &gitconfig, false, &[&gitconfig])),
        "a relative XDG_CONFIG_HOME leaves $HOME/.gitconfig"
    );
}

/// An empty, relative or unset `HOME` names no file, so neither
/// `$HOME/.gitconfig` nor `$HOME/.config/git/config` is read; an absolute
/// `XDG_CONFIG_HOME` still names its file.
#[test]
fn an_empty_relative_or_unset_home_names_no_file() {
    let dir = TempDir::new("setting-home-none");
    for environment in [
        snapshot([("HOME", OsStr::new(""))]),
        snapshot([("HOME", OsStr::new("home"))]),
        snapshot([("HOME", OsStr::new("./"))]),
        snapshot([("PATH", OsStr::new("/usr/bin"))]),
    ] {
        assert_eq!(resolve(None, &environment), Ok(by_default(&[])));
    }
    let xdg_home = dir.path().join("xdg");
    let xdg = xdg_home.join("git/config");
    write(&xdg, NATIVE);
    for user in ["", "home"] {
        let environment = snapshot([
            ("HOME", OsStr::new(user)),
            ("XDG_CONFIG_HOME", xdg_home.as_os_str()),
        ]);
        assert_eq!(
            resolve(None, &environment),
            Ok(decided_by(Transport::Native, &xdg, false, &[&xdg])),
            "HOME={user:?}"
        );
    }
}

/// `GIT_CONFIG_GLOBAL`, which git honours and libgit2 does not, and the
/// command scope's variables are not read, even when they hold the key.
#[test]
fn git_config_global_and_the_command_scope_are_not_read() {
    let dir = TempDir::new("setting-other-scopes");
    let named = dir.path().join("named.gitconfig");
    write(&named, NATIVE);
    let environment = snapshot([
        ("HOME", dir.path().as_os_str()),
        ("GIT_CONFIG_GLOBAL", named.as_os_str()),
        (
            "GIT_CONFIG_PARAMETERS",
            OsStr::new("'gwz.transport'='native'"),
        ),
        ("GIT_CONFIG_COUNT", OsStr::new("1")),
        ("GIT_CONFIG_KEY_0", OsStr::new("gwz.transport")),
        ("GIT_CONFIG_VALUE_0", OsStr::new("native")),
    ]);
    assert_eq!(resolve(None, &environment), Ok(by_default(&[])));
}

/// The snapshot's `HOME`, not the process's, names the files.
#[test]
fn a_snapshot_whose_home_differs_from_the_processs_reads_the_snapshots() {
    let dir = TempDir::new("setting-home-snapshot");
    assert_ne!(
        std::env::var_os("HOME").as_deref(),
        Some(dir.path().as_os_str())
    );
    let gitconfig = dir.path().join(".gitconfig");
    write(&gitconfig, NATIVE);
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(decided_by(
            Transport::Native,
            &gitconfig,
            false,
            &[&gitconfig]
        ))
    );
}

/// `include.path` is followed, relative to the including file, and a value
/// reached through it is reported as included, with `file` naming the global
/// file that includes it. Within the file the last value wins, wherever it
/// came from.
#[test]
fn include_path_is_followed_and_its_value_reported_as_included() {
    let dir = TempDir::new("setting-include");
    let gitconfig = dir.path().join(".gitconfig");
    write(&dir.path().join("more/included.inc"), NATIVE);
    write(&gitconfig, "[include]\n\tpath = more/included.inc\n");
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(decided_by(
            Transport::Native,
            &gitconfig,
            true,
            &[&gitconfig]
        ))
    );
    write(
        &gitconfig,
        "[include]\n\tpath = more/included.inc\n[gwz]\n\ttransport = gwz\n",
    );
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(decided_by(Transport::Gwz, &gitconfig, false, &[&gitconfig]))
    );
    write(
        &gitconfig,
        "[gwz]\n\ttransport = gwz\n[include]\n\tpath = more/included.inc\n",
    );
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(decided_by(
            Transport::Native,
            &gitconfig,
            true,
            &[&gitconfig]
        ))
    );
    let xdg = dir.path().join(".config/git/config");
    fs::remove_file(&gitconfig).unwrap();
    write(&xdg, "[include]\n\tpath = ../../more/included.inc\n");
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(decided_by(Transport::Native, &xdg, true, &[&xdg]))
    );
}

/// A `~/` include resolves against libgit2's process-wide home, not the
/// snapshot's `HOME`.
#[test]
fn a_tilde_include_resolves_against_libgit2s_process_wide_home() {
    let dir = TempDir::new("setting-include-tilde");
    let user = dir.path().join("home");
    let gitconfig = user.join(".gitconfig");
    let target = dir.path().join("target.inc");
    write(&target, NATIVE);
    // Walk up from libgit2's home to the root, then down to the target, on
    // real paths, so the walk does not depend on any link on the way.
    let process_home = fs::canonicalize(libgit2_home()).unwrap();
    let up = process_home
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count();
    let down = fs::canonicalize(&target).unwrap();
    let down = down.strip_prefix("/").unwrap().to_str().unwrap();
    write(
        &gitconfig,
        format!("[include]\n\tpath = ~/{}{down}\n", "../".repeat(up)),
    );
    assert_eq!(
        resolve(None, &home(&user)),
        Ok(decided_by(
            Transport::Native,
            &gitconfig,
            true,
            &[&gitconfig]
        ))
    );
    // A name that only the snapshot's HOME holds is not found.
    let name = format!("{}.inc", dir.path().file_name().unwrap().to_str().unwrap());
    write(&user.join(&name), NATIVE);
    write(&gitconfig, format!("[include]\n\tpath = ~/{name}\n"));
    assert_eq!(resolve(None, &home(&user)), Ok(by_default(&[&gitconfig])));
}

/// The files are opened without a repository, so no `includeIf` applies,
/// whatever its condition.
#[test]
fn an_include_if_that_sets_native_is_not_applied() {
    let dir = TempDir::new("setting-include-if");
    let gitconfig = dir.path().join(".gitconfig");
    write(&dir.path().join("conditional.inc"), NATIVE);
    write(
        &gitconfig,
        "[includeIf \"gitdir:/\"]\n\tpath = conditional.inc\n\
         [includeIf \"gitdir/i:/\"]\n\tpath = conditional.inc\n\
         [includeIf \"onbranch:*\"]\n\tpath = conditional.inc\n",
    );
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(by_default(&[&gitconfig]))
    );
}

/// A file that cannot be read carries no value and is listed as skipped,
/// and the other file is still read.
#[test]
fn an_unreadable_file_carries_no_value_and_is_listed_as_skipped() {
    let dir = TempDir::new("setting-unreadable");
    let gitconfig = dir.path().join(".gitconfig");
    write(&gitconfig, NATIVE);
    fs::set_permissions(&gitconfig, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(&gitconfig).is_ok() {
        eprintln!("skipped: this process reads a file of mode 000, as root does");
        return;
    }
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(Setting {
            transport: Transport::Gwz,
            source: Source::Default,
            read: Vec::new(),
            skipped: vec![gitconfig.clone()],
        })
    );
    let xdg = dir.path().join(".config/git/config");
    write(&xdg, NATIVE);
    fs::set_permissions(&xdg, fs::Permissions::from_mode(0o000)).unwrap();
    fs::set_permissions(&gitconfig, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        resolve(None, &home(dir.path())),
        Ok(Setting {
            transport: Transport::Native,
            source: Source::GlobalConfiguration(Location {
                file: gitconfig.clone(),
                included: false,
            }),
            read: vec![gitconfig.clone()],
            skipped: vec![xdg.clone()],
        })
    );
    // A directory that cannot be searched hides the file it holds.
    fs::set_permissions(&gitconfig, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(
        dir.path().join(".config/git"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    let skipped = resolve(None, &home(dir.path())).unwrap().skipped;
    fs::set_permissions(
        dir.path().join(".config/git"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert_eq!(skipped, vec![xdg]);
}

/// A global file that is not a regular file once links are followed, such as
/// a directory or a FIFO, cannot be read as configuration: it is skipped
/// without being opened, so a FIFO cannot block. A link to a regular file is
/// followed, as git follows it.
#[test]
fn a_global_file_that_is_not_a_regular_file_is_skipped_without_blocking() {
    let dir = TempDir::new("setting-not-regular");
    let gitconfig = dir.path().join(".gitconfig");
    fs::create_dir_all(&gitconfig).unwrap();
    let xdg = dir.path().join(".config/git/config");
    fifo(&xdg);
    let user = dir.path().to_path_buf();
    let setting = within(move || resolve(None, &home(&user)));
    assert_eq!(
        setting,
        Ok(Setting {
            transport: Transport::Gwz,
            source: Source::Default,
            read: Vec::new(),
            skipped: vec![xdg, gitconfig.clone()],
        })
    );
    fs::remove_dir(&gitconfig).unwrap();
    let real = dir.path().join("dotfiles/gitconfig");
    write(&real, NATIVE);
    std::os::unix::fs::symlink(&real, &gitconfig).unwrap();
    let setting = resolve(None, &home(dir.path())).unwrap();
    assert_eq!(
        (setting.transport, setting.read),
        (Transport::Native, vec![gitconfig])
    );
}

/// A file that cannot be parsed is refused with libgit2's message, which
/// names the file and line, the included file's for an error in an included
/// file.
#[test]
fn an_unparsable_file_is_refused_with_libgit2s_file_and_line() {
    let dir = TempDir::new("setting-unparsable");
    let gitconfig = dir.path().join(".gitconfig");
    write(&gitconfig, "[user]\n\tname = U\n[gwz\n");
    let Err(Refusal::Unparsable { file, cause }) = resolve(None, &home(dir.path())) else {
        panic!("not refused as unparsable");
    };
    assert_eq!(file, gitconfig);
    assert!(
        cause.contains(&format!("{}:3", gitconfig.display())),
        "{cause}"
    );
    let included: PathBuf = dir.path().join("broken.inc");
    write(&included, "[gwz]\n\ttransport = native\n[broken\n");
    write(&gitconfig, "[include]\n\tpath = broken.inc\n");
    let Err(Refusal::Unparsable { file, cause }) = resolve(None, &home(dir.path())) else {
        panic!("not refused as unparsable");
    };
    assert_eq!(file, gitconfig);
    assert!(
        cause.contains(&format!("{}:3", included.display())),
        "{cause}"
    );
    // The XDG file is read too, so its error refuses even when .gitconfig
    // holds a value.
    write(&gitconfig, NATIVE);
    let xdg = dir.path().join(".config/git/config");
    write(&xdg, "[gwz\n");
    let Err(Refusal::Unparsable { file, .. }) = resolve(None, &home(dir.path())) else {
        panic!("not refused as unparsable");
    };
    assert_eq!(file, xdg);
}
