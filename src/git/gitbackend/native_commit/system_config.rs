//! System-level Git configuration the native read must cover.
//!
//! libgit2 reads one system file (`/etc/gitconfig`, or the Git for Windows one
//! it finds, and always `%PROGRAMDATA%/Git/config` on Windows) while the `git`
//! the CLI route would run may read others: Homebrew's
//! `/opt/homebrew/etc/gitconfig`, a Linuxbrew, MacPorts, Nix or conda prefix,
//! Apple's command-line-tools file, Git for Windows' `etc/gitconfig`. A key
//! set in a file native does not read (a hooks path, signing, a filter, an
//! identity) would change the result with no warning, and so would a key in a
//! file only native reads. So the decision starts from the `git` itself: the
//! first one on the snapshot's `PATH`, which is the one the CLI route runs.
//! Native goes ahead only when every system file that `git` may read, and every
//! file libgit2 reads, is the same file on both sides, and each can be read.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::environment::GitEnvironment;
use super::route::CliReason;

/// Why the CLI is used when no `git` can be resolved.
const NO_GIT: &str = "no git on PATH";
/// Why the CLI is used when the `git` found has no derivable prefix.
const NO_PREFIX: &str = "no prefix for the git on PATH";
const GIT_ONLY: &str = "the git on PATH reads it and native does not";
const NATIVE_ONLY: &str = "native reads it and the git on PATH does not";
const ALWAYS: &str = "native reads it whatever GIT_CONFIG_NOSYSTEM says";
const UNREADABLE: &str = "system file unreadable";

/// The platform conventions a `git`'s system file follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Host {
    Unix,
    MacOs,
    Windows,
}

impl Host {
    pub(super) fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Unix
        }
    }
}

/// The system files of one `git`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct GitFiles {
    /// Files this `git` certainly reads. A file libgit2 reads that is not one
    /// of these may change only the native result.
    reads: Vec<PathBuf>,
    /// Files this `git` may read: `reads` and the other places a prefix of
    /// this `git` keeps a system file. One that exists and that libgit2 does
    /// not read may change only the CLI's result.
    may_read: Vec<PathBuf>,
}

/// What the route decision knows of the system configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SystemView {
    /// The `git` the CLI route would run; `None` when none resolves.
    git: Option<GitFiles>,
    /// Why `git` is `None`.
    git_missing: &'static str,
    /// Files libgit2 reads.
    covered: Vec<PathBuf>,
    /// Files libgit2 reads even when `GIT_CONFIG_NOSYSTEM` is true.
    always: Vec<PathBuf>,
}

fn is_program(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => {
            cfg_if::cfg_if! {
                if #[cfg(unix)] {
                    use std::os::unix::fs::PermissionsExt;
                    meta.is_file() && meta.permissions().mode() & 0o111 != 0
                } else {
                    meta.is_file()
                }
            }
        }
        Err(_) => false,
    }
}

/// The first `git` on `path`, as the CLI route's `Command::new("git")` finds
/// it. A directory that depends on the working directory, met before any
/// `git`, leaves the answer open (`None`).
fn find_git(path: Option<&OsStr>, host: Host) -> Option<PathBuf> {
    let name = if host == Host::Windows {
        "git.exe"
    } else {
        "git"
    };
    for directory in std::env::split_paths(path?) {
        if !directory.is_absolute() {
            return None;
        }
        let candidate = directory.join(name);
        if is_program(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// The directory above the one holding `program`.
fn prefix_of(program: &Path) -> Option<&Path> {
    program.parent()?.parent()
}

/// Where a `git` built for `prefix` keeps its system file. A build for `/usr`
/// keeps it in `/etc`, as Git's own makefile does.
fn system_etc(prefix: &Path) -> PathBuf {
    if prefix == Path::new("/usr") || prefix == Path::new("/") {
        PathBuf::from("/etc/gitconfig")
    } else {
        prefix.join("etc").join("gitconfig")
    }
}

/// Apple's `git` (behind the `/usr/bin/git` stub) reads the command-line
/// tools' or Xcode's files.
fn apple_files() -> Vec<PathBuf> {
    [
        "/Library/Developer/CommandLineTools/usr/etc/gitconfig",
        "/Library/Developer/CommandLineTools/usr/share/git-core/gitconfig",
        "/Applications/Xcode.app/Contents/Developer/usr/etc/gitconfig",
        "/Applications/Xcode.app/Contents/Developer/usr/share/git-core/gitconfig",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

fn push_unique(files: &mut Vec<PathBuf>, file: PathBuf) {
    if !files.contains(&file) {
        files.push(file);
    }
}

/// The system files of the `git` at `program` (its symbolic links resolved to
/// `resolved`, when they lead elsewhere). `None` when no prefix can be derived.
pub(super) fn derive(
    program: &Path,
    resolved: Option<&Path>,
    host: Host,
    program_data: Option<&Path>,
) -> Option<GitFiles> {
    let prefix = prefix_of(program)?;
    let mut files = GitFiles::default();
    match host {
        Host::Windows => {
            // Git for Windows keeps `etc/gitconfig` at the root of the install,
            // above `cmd`, `bin` and `mingw64/bin`.
            const ARCH: [&str; 3] = ["mingw64", "mingw32", "clangarm64"];
            let is_arch = |dir: &Path| {
                dir.file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| ARCH.contains(&name.to_ascii_lowercase().as_str()))
            };
            let root = if is_arch(prefix) {
                prefix.parent().unwrap_or(prefix)
            } else {
                prefix
            };
            let etc = root.join("etc").join("gitconfig");
            files.reads.push(etc.clone());
            files.may_read.push(etc);
            push_unique(&mut files.may_read, prefix.join("etc").join("gitconfig"));
            for arch in ARCH {
                push_unique(
                    &mut files.may_read,
                    root.join(arch).join("etc").join("gitconfig"),
                );
            }
            if let Some(data) = program_data {
                let config = data.join("Git").join("config");
                files.reads.push(config.clone());
                files.may_read.push(config);
            }
        }
        Host::Unix | Host::MacOs => {
            let apple = host == Host::MacOs && program == Path::new("/usr/bin/git");
            if apple {
                for file in apple_files() {
                    files.reads.push(file.clone());
                    files.may_read.push(file);
                }
            } else {
                files.reads.push(system_etc(prefix));
            }
            push_unique(&mut files.may_read, system_etc(prefix));
            push_unique(&mut files.may_read, prefix.join("etc").join("gitconfig"));
            push_unique(
                &mut files.may_read,
                prefix.join("share").join("git-core").join("gitconfig"),
            );
            if let Some(other) = resolved.filter(|resolved| *resolved != program)
                && let Some(prefix) = prefix_of(other)
            {
                push_unique(&mut files.may_read, system_etc(prefix));
                push_unique(&mut files.may_read, prefix.join("etc").join("gitconfig"));
                push_unique(
                    &mut files.may_read,
                    prefix.join("share").join("git-core").join("gitconfig"),
                );
            }
        }
    }
    Some(files)
}

fn program_data(env: &GitEnvironment) -> Option<PathBuf> {
    env.get("PROGRAMDATA").map(PathBuf::from)
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// Files libgit2 reads: the one it finds, and `%PROGRAMDATA%/Git/config`.
        fn covered(env: &GitEnvironment) -> Vec<PathBuf> {
            let mut files: Vec<PathBuf> = git2::Config::find_system().into_iter().collect();
            files.extend(always(env));
            files
        }

        /// libgit2 reads `%PROGRAMDATA%/Git/config` whatever `GIT_CONFIG_NOSYSTEM` says.
        fn always(env: &GitEnvironment) -> Vec<PathBuf> {
            program_data(env)
                .map(|root| root.join("Git").join("config"))
                .into_iter()
                .collect()
        }
    } else {
        /// Files libgit2 reads: the one it finds.
        fn covered(_env: &GitEnvironment) -> Vec<PathBuf> {
            git2::Config::find_system().into_iter().collect()
        }

        /// Nothing is read once `GIT_CONFIG_NOSYSTEM` is true.
        fn always(_env: &GitEnvironment) -> Vec<PathBuf> {
            Vec::new()
        }
    }
}

impl SystemView {
    /// Read what the decision needs from the snapshot and this machine.
    pub(super) fn read(env: &GitEnvironment) -> Self {
        let host = Host::current();
        let program = find_git(env.get("PATH"), host);
        let (git, git_missing) = match &program {
            None => (None, NO_GIT),
            Some(program) => {
                let resolved = program.canonicalize().ok();
                let data = program_data(env);
                match derive(program, resolved.as_deref(), host, data.as_deref()) {
                    Some(files) => (Some(files), ""),
                    None => (None, NO_PREFIX),
                }
            }
        };
        Self {
            git,
            git_missing,
            covered: covered(env),
            always: always(env),
        }
    }
}

fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn readable(path: &Path) -> bool {
    std::fs::File::open(path).is_ok()
}

fn exists(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

/// How `GIT_CONFIG_NOSYSTEM` reads; `None` when Git and libgit2 might not agree.
fn no_system(env: &GitEnvironment) -> Option<bool> {
    match env.get("GIT_CONFIG_NOSYSTEM") {
        None => Some(false),
        Some(value) => match value.to_str()?.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "" | "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        },
    }
}

fn system_file(why: &'static str, path: &Path) -> CliReason {
    CliReason::SystemFile {
        why,
        path: path.to_owned(),
    }
}

/// Fail safe: native goes ahead only when the system files the `git` on `PATH`
/// may read and the files libgit2 reads are the same files, and each that
/// exists can be read.
pub(super) fn check(env: &GitEnvironment, view: &SystemView) -> Result<(), CliReason> {
    let Some(skip) = no_system(env) else {
        return Err(CliReason::SystemConfig("GIT_CONFIG_NOSYSTEM value"));
    };
    if skip {
        // Git reads no system file. libgit2 also reads none, except a file
        // it always reads (Windows: `%PROGRAMDATA%/Git/config`).
        return match view.always.iter().find(|path| exists(path)) {
            Some(path) => Err(system_file(ALWAYS, path)),
            None => Ok(()),
        };
    }
    if let Some(named) = env.get("GIT_CONFIG_SYSTEM") {
        // Git reads exactly this file, and so does libgit2 given the variable.
        let path = Path::new(named);
        return if !exists(path) || readable(path) {
            Ok(())
        } else {
            Err(CliReason::SystemConfig("GIT_CONFIG_SYSTEM file unreadable"))
        };
    }
    let Some(git) = &view.git else {
        return Err(CliReason::SystemConfig(view.git_missing));
    };
    for path in git.may_read.iter().filter(|path| exists(path)) {
        if !readable(path) {
            return Err(system_file(UNREADABLE, path));
        }
        if !view.covered.iter().any(|known| same_file(known, path)) {
            return Err(system_file(GIT_ONLY, path));
        }
    }
    for path in view.covered.iter().filter(|path| exists(path)) {
        if !readable(path) {
            return Err(system_file(UNREADABLE, path));
        }
        if !git.reads.iter().any(|known| same_file(known, path)) {
            return Err(system_file(NATIVE_ONLY, path));
        }
    }
    Ok(())
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}
