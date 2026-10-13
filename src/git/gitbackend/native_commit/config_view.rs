//! A read-only view of a repository's configuration for the route decision.

use super::ident::{ConfigStrings, IdentRefusal};

/// What a boolean key holds. `Unknown` is a value Git would parse in a way
/// native does not trust (or one that does not parse at all).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flag {
    Unset,
    True,
    False,
    Unknown,
}

impl Flag {
    /// Set to true, or to something native cannot read as false.
    pub(super) fn may_be_true(self) -> bool {
        matches!(self, Self::True | Self::Unknown)
    }
}

/// A configuration value that does not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Unreadable;

/// A snapshot of the repository's configuration, levels merged.
pub(super) struct ConfigView {
    config: git2::Config,
}

impl ConfigView {
    pub(super) fn new(repo: &git2::Repository) -> Result<Self, git2::Error> {
        Ok(Self {
            config: repo.config()?.snapshot()?,
        })
    }

    pub(super) fn flag(&self, key: &str) -> Flag {
        match self.config.get_bool(key) {
            Ok(true) => Flag::True,
            Ok(false) => Flag::False,
            Err(error) if error.code() == git2::ErrorCode::NotFound => Flag::Unset,
            Err(_) => Flag::Unknown,
        }
    }

    /// The last value of a string key; `Err(Unreadable)` when it cannot be read.
    pub(super) fn text(&self, key: &str) -> Result<Option<String>, Unreadable> {
        match self.config.get_string(key) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(_) => Err(Unreadable),
        }
    }

    pub(super) fn is_set(&self, key: &str) -> bool {
        !matches!(self.text(key), Ok(None))
    }

    /// A path value with a leading `~/` expanded; `None` when unset or when
    /// the form is not expanded.
    pub(super) fn path(&self, key: &str) -> Option<std::path::PathBuf> {
        self.config.get_path(key).ok()
    }

    /// Every entry whose key matches the regular expression, with its value
    /// (`None` for a key written without one).
    pub(super) fn values_matching(
        &self,
        pattern: &str,
    ) -> Result<Vec<(String, Option<String>)>, Unreadable> {
        let mut entries = self.config.entries(Some(pattern)).map_err(|_| Unreadable)?;
        let mut values = Vec::new();
        while let Some(entry) = entries.next() {
            let entry = entry.map_err(|_| Unreadable)?;
            let name = entry.name().map_err(|_| Unreadable)?.to_owned();
            let value = if entry.has_value() {
                Some(entry.value().map_err(|_| Unreadable)?.to_owned())
            } else {
                None
            };
            values.push((name, value));
        }
        Ok(values)
    }

    /// Every key matching the regular expression, lower-cased by the reader.
    pub(super) fn names_matching(&self, pattern: &str) -> Result<Vec<String>, Unreadable> {
        let mut entries = self.config.entries(Some(pattern)).map_err(|_| Unreadable)?;
        let mut names = Vec::new();
        while let Some(entry) = entries.next() {
            let entry = entry.map_err(|_| Unreadable)?;
            names.push(entry.name().map_err(|_| Unreadable)?.to_owned());
        }
        Ok(names)
    }
}

impl ConfigStrings for ConfigView {
    fn string(&self, key: &str) -> Result<Option<String>, IdentRefusal> {
        self.text(key)
            .map_err(|Unreadable| IdentRefusal::Unusual("configuration value"))
    }
}
