//! The SSH home: the one directory that `known_hosts`, `~/` identity paths, the global Git configuration and the helper
//! configuration view's `~/` includes are read under (design `GwzTransportWindowsParityDesign.md` §3; plan step 3.1).
//!
//! The choice is a pure function of the captured variables, the captured current directory and a probe for which
//! directories exist, so it is decided once, from the snapshot, and a later change of the process directory or of the
//! live environment cannot redirect trust. Nothing here reads the process: the driver that captures the environment
//! captures the current directory beside it, and passes both in.
//!
//! **Windows** follows libgit2's order, which 1.0.17 uses (`sysdir.c`, `find_win32_dirs`): `HOME`, then `HOMEDRIVE`
//! joined to `HOMEPATH`, then `USERPROFILE`. The first of them that is set and names an existing directory is the
//! home. Executed on 1.0.17 and kept (RS/b04-*):
//! - an unset variable, or a candidate that is not an existing directory, is skipped (B03 and B04: `HOME` unset
//!   works through `HOMEDRIVE` and `HOMEPATH`, or through `USERPROFILE` alone);
//! - an explicitly empty `HOME` refuses with a home diagnostic, even when a later candidate is good; it is not the
//!   same as unset;
//! - a relative candidate that exists under the captured current directory is used, as that absolute path;
//! - once a home is chosen, nothing else is tried: the caller never falls through to another home's trust file
//!   because the chosen home has no usable `known_hosts`.
//!
//! **Elsewhere** `HOME` is the home, as libgit2 reads it on Unix: it is taken as written, an absolute path, without
//! asking whether it exists (the file read that follows says so), and a relative one names no home. Where `HOME` is
//! unset, `USERPROFILE` is the fallback on every platform (operator, 2026-10-11: with no `HOME`, `~/` falls back to
//! `USERPROFILE`).
//!
//! Paths keep their native form: the choice moves `OsString` values and never converts them through UTF-8, so a
//! Unicode home or one with spaces, and on Unix a home that is not UTF-8, reaches the file operations unchanged (TD6).
use crate::session_host::EnvironmentSnapshot;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

/// The captured variables a home is chosen from. Names follow the platform's rules (the snapshot's).
pub(crate) trait Variables {
    /// The value of `name`, if it is set (it may be empty).
    fn get(&self, name: &str) -> Option<OsString>;
}

impl Variables for EnvironmentSnapshot {
    fn get(&self, name: &str) -> Option<OsString> {
        EnvironmentSnapshot::get(self, name).map(|value| value.as_os_str().to_owned())
    }
}

/// Which directories exist: the one question the choice asks of the file system.
pub(crate) trait Probe {
    fn is_directory(&self, path: &Path) -> bool;
}

/// The file system itself.
pub(crate) struct Disk;

impl Probe for Disk {
    fn is_directory(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
    }
}

/// A probe for a reader that never touches the file system: every directory exists, so the first candidate that is set
/// and usable is the home. The helper configuration view reads paths as Git wrote them and is the one such reader; Git
/// takes `HOME` as written, and so does the view.
pub(crate) struct Assumed;

impl Probe for Assumed {
    fn is_directory(&self, _: &Path) -> bool {
        true
    }
}

/// The rules of one platform: which variables are candidates, in what order, and whether a candidate must exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rules {
    /// libgit2's order on Windows: `HOME`, `HOMEDRIVE` and `HOMEPATH`, `USERPROFILE`; each must exist.
    Windows,
    /// `HOME`, then `USERPROFILE`; taken as written, and absolute.
    Unix,
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// The rules of the platform this build is for.
        pub(crate) const PLATFORM: Rules = Rules::Windows;
    } else {
        /// The rules of the platform this build is for.
        pub(crate) const PLATFORM: Rules = Rules::Unix;
    }
}

/// Where a home came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HomeSource {
    Home,
    HomeDrivePath,
    UserProfile,
}

impl HomeSource {
    /// The variable or variables that name this source, as a message words them.
    pub(crate) fn variables(self) -> &'static str {
        match self {
            Self::Home => "HOME",
            Self::HomeDrivePath => "HOMEDRIVE and HOMEPATH",
            Self::UserProfile => "USERPROFILE",
        }
    }
}

/// The chosen home, and the variable it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Home {
    /// Absolute.
    pub(crate) path: PathBuf,
    pub(crate) source: HomeSource,
}

/// The `known_hosts` file under a home, and the variable that named the home: what a trust message prints, so that a
/// user is told which file gwz read and where its home came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KnownHostsFile {
    pub(crate) path: PathBuf,
    pub(crate) source: HomeSource,
}

impl KnownHostsFile {
    /// The file the SSH endpoint reads under `home`. Its path is built by components, so that what a message prints
    /// uses the platform's separators.
    pub(crate) fn under(home: &Path, source: HomeSource) -> Self {
        Self {
            path: home.join(".ssh").join("known_hosts"),
            source,
        }
    }
}

/// Why no home was chosen. Each words what needs correcting, so a user is not left to guess which variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// `HOME` is set and empty.
    EmptyHome,
    /// A candidate is not a usable path (it holds a NUL).
    Malformed(HomeSource),
    /// Nothing named an acceptable home.
    NoHome(Rules),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyHome => f.write_str(
                "the SSH home cannot be chosen: HOME is set but empty. Unset HOME, or set it to your home \
                 directory",
            ),
            Self::Malformed(source) => write!(
                f,
                "the SSH home cannot be chosen: {} does not hold a usable path. Correct it, or unset it",
                source.variables()
            ),
            Self::NoHome(Rules::Windows) => f.write_str(
                "no SSH home was found: none of HOME, HOMEDRIVE and HOMEPATH, or USERPROFILE names an existing \
                 directory. Set HOME to your home directory",
            ),
            Self::NoHome(Rules::Unix) => f.write_str(
                "no SSH home was found: HOME is unset or not an absolute path, and USERPROFILE does not name one. \
                 Set HOME to your home directory",
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// The home that `variables` name under `rules`. `cwd` is the current directory as it was captured, against which a
/// relative candidate is resolved where the rules allow one; without it a relative candidate is skipped.
pub(crate) fn resolve(
    rules: Rules,
    variables: &impl Variables,
    cwd: Option<&Path>,
    probe: &impl Probe,
) -> Result<Home, Refusal> {
    let order: &[HomeSource] = match rules {
        Rules::Windows => &[
            HomeSource::Home,
            HomeSource::HomeDrivePath,
            HomeSource::UserProfile,
        ],
        Rules::Unix => &[HomeSource::Home, HomeSource::UserProfile],
    };
    for &source in order {
        let Some(raw) = candidate(variables, source) else {
            continue;
        };
        if raw.is_empty() {
            if source == HomeSource::Home {
                return Err(Refusal::EmptyHome);
            }
            continue;
        }
        if raw.as_encoded_bytes().contains(&0) {
            return Err(Refusal::Malformed(source));
        }
        let path = PathBuf::from(raw);
        let Some(path) = anchored(rules, path, cwd) else {
            continue;
        };
        let chosen = match rules {
            Rules::Windows => probe.is_directory(&path),
            Rules::Unix => true,
        };
        if chosen {
            return Ok(Home { path, source });
        }
    }
    Err(Refusal::NoHome(rules))
}

/// The raw value of `source`'s variable or variables, if set.
fn candidate(variables: &impl Variables, source: HomeSource) -> Option<OsString> {
    match source {
        HomeSource::Home => variables.get("HOME"),
        HomeSource::UserProfile => variables.get("USERPROFILE"),
        HomeSource::HomeDrivePath => {
            let mut joined = variables.get("HOMEDRIVE")?;
            joined.push(variables.get("HOMEPATH")?);
            Some(joined)
        }
    }
}

/// `path` as an absolute path, or `None` when the rules leave it out: Windows resolves a relative one against `cwd`,
/// and drops one that is still not absolute (`C:dir`, which names the current directory of another drive).
fn anchored(rules: Rules, path: PathBuf, cwd: Option<&Path>) -> Option<PathBuf> {
    if path.is_absolute() {
        return Some(path);
    }
    match rules {
        Rules::Unix => None,
        Rules::Windows => {
            let joined = cwd?.join(path);
            joined.is_absolute().then_some(joined)
        }
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
