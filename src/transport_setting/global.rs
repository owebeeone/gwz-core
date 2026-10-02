//! The global configuration form (design §2): git's `--global` files, read
//! from the driver's snapshot, the XDG file first and `$HOME/.gitconfig`,
//! whose value wins, last.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{Found, Location, Refusal, Setting, Source, Transport, last_value};
use crate::session_host::EnvironmentSnapshot;

/// The transport the global files select, or the default when none holds the
/// key, with the files read and those skipped.
pub(super) fn resolve(environment: &EnvironmentSnapshot) -> Result<Setting, Refusal> {
    let mut read = Vec::new();
    let mut skipped = Vec::new();
    let mut decided = None;
    for file in files(environment) {
        match state(&file) {
            State::Absent => {}
            State::Unreadable => skipped.push(file),
            State::Readable => {
                let found = last_value(&file).map_err(|error| Refusal::Unparsable {
                    file: file.clone(),
                    cause: error.message().to_owned(),
                })?;
                if let Some(Found { included, value }) = found {
                    decided = Some((
                        Location {
                            file: file.clone(),
                            included,
                        },
                        value,
                    ));
                }
                read.push(file);
            }
        }
    }
    let Some((location, value)) = decided else {
        return Ok(Setting {
            transport: Transport::Gwz,
            source: Source::Default,
            read,
            skipped,
        });
    };
    match value.as_deref().and_then(Transport::named) {
        Some(transport) => Ok(Setting {
            transport,
            source: Source::GlobalConfiguration(location),
            read,
            skipped,
        }),
        None => Err(Refusal::Configuration {
            location,
            value: value.map(|value| String::from_utf8_lossy(&value).into_owned()),
        }),
    }
}

/// git's `--global` files in the order read, where `git config --global`
/// writes them (design §2, D5): `$XDG_CONFIG_HOME/git/config`, or
/// `$HOME/.config/git/config` when that variable is unset or empty, then
/// `$HOME/.gitconfig`. Both variables come from the snapshot. A `HOME` that is
/// empty or relative names no file, and neither does a relative
/// `XDG_CONFIG_HOME`, since the caller's directory, perhaps a workspace, would
/// otherwise supply the value. No passwd entry is consulted.
fn files(environment: &EnvironmentSnapshot) -> Vec<PathBuf> {
    let directory = |name: &str| {
        environment
            .get(name)
            .map(|value| PathBuf::from(value.as_os_str()))
    };
    let home = directory("HOME").filter(|home| home.is_absolute());
    let xdg = match directory("XDG_CONFIG_HOME").filter(|base| !base.as_os_str().is_empty()) {
        Some(base) => base.is_absolute().then(|| base.join("git/config")),
        None => home.as_ref().map(|home| home.join(".config/git/config")),
    };
    xdg.into_iter()
        .chain(home.map(|home| home.join(".gitconfig")))
        .collect()
}

/// Whether a global file is read.
enum State {
    /// It does not exist, so it carries no value.
    Absent,
    /// It exists but cannot be read, so it carries no value and is reported as
    /// skipped, as git and libgit2 skip it (`config_file.c:114-124`; D4).
    Unreadable,
    /// libgit2 reads it.
    Readable,
}

/// The file's state. A file that is not a regular file once links are
/// followed, such as a directory or a FIFO, cannot be read as configuration:
/// it is skipped without being opened, so a FIFO cannot block the
/// resolution. A file is otherwise readable when it opens for reading.
fn state(file: &Path) -> State {
    match fs::metadata(file) {
        Err(error) if absent(&error) => State::Absent,
        Err(_) => State::Unreadable,
        Ok(metadata) if !metadata.is_file() => State::Unreadable,
        Ok(_) => match fs::File::open(file) {
            Ok(_) => State::Readable,
            Err(error) if absent(&error) => State::Absent,
            Err(_) => State::Unreadable,
        },
    }
}

/// Whether `error` says the file is not there: libgit2 takes `ENOENT` and
/// `ENOTDIR` to mean a missing file (`config.c`'s `git_config_add_file_ondisk`).
fn absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}
