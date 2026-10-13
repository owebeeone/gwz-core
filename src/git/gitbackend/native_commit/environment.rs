//! The part of the process environment that Git's commit and tag read.
//!
//! The backend captures it once, when it is built, and passes it to the route
//! decision, so the decision never reads the process environment at the point
//! of use and a test states the environment it means.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

/// Git variables whose value cannot change what a native commit or tag
/// writes. Any other `GIT_*` variable sends the operation to the Git CLI
/// (so do `GIT_ATTR_*`, which change how `commit -a` stores content, and
/// `GIT_TEST_*`, which Git defines to change its behaviour).
const IRRELEVANT_PREFIXES: &[&str] = &[
    "GIT_TRACE",
    "GIT_CURL_",
    "GIT_HTTP_",
    "GIT_SSL_",
    "GIT_SSH",
    "GIT_ASKPASS",
    "GIT_TERMINAL_PROMPT",
    "GIT_PAGER",
    "GIT_EDITOR",
    "GIT_SEQUENCE_EDITOR",
    "GIT_EXEC_PATH",
    "GIT_TEMPLATE_DIR",
    "GIT_PROTOCOL",
    "GIT_ALLOW_PROTOCOL",
    "GIT_FLUSH",
    "GIT_PROGRESS_DELAY",
    "GIT_MERGE_",
    "GIT_DIFF_",
    "GIT_EXTERNAL_DIFF",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_NO_LAZY_FETCH",
];

/// Other variables kept: the search path the CLI route finds `git` on, and
/// where Windows keeps Git's machine-wide configuration.
const SYSTEM_PATHS: &[&str] = &["PATH", "PROGRAMDATA"];

/// Variables the native route reads itself.
const READ_NAMES: &[&str] = &[
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_AUTHOR_DATE",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "GIT_COMMITTER_DATE",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_NOSYSTEM",
];

/// A snapshot of the `GIT_*` variables, `EMAIL`, `PATH` and `PROGRAMDATA`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitEnvironment {
    vars: BTreeMap<String, OsString>,
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl GitEnvironment {
            /// A snapshot with no variable set.
            pub fn none() -> Self {
                Self::default()
            }
        }
    }
}

impl GitEnvironment {
    /// A snapshot of the given variables; names are compared in upper case,
    /// as Windows does.
    pub fn from_vars<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let vars = vars
            .into_iter()
            .filter_map(|(name, value)| {
                let name = name.as_ref().to_str()?.to_ascii_uppercase();
                if name.starts_with("GIT_")
                    || name == "EMAIL"
                    || SYSTEM_PATHS.contains(&name.as_str())
                {
                    Some((name, value.as_ref().to_owned()))
                } else {
                    None
                }
            })
            .collect();
        Self { vars }
    }

    /// The live process environment, read once.
    pub fn process() -> Self {
        Self::from_vars(std::env::vars_os())
    }

    pub(super) fn get(&self, name: &str) -> Option<&OsStr> {
        self.vars.get(name).map(OsString::as_os_str)
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        self.vars.contains_key(name)
    }

    /// The first set variable that native cannot honour, if any.
    pub(super) fn unsupported(&self) -> Option<&str> {
        self.vars.keys().map(String::as_str).find(|name| {
            *name != "EMAIL"
                && !SYSTEM_PATHS.contains(name)
                && !READ_NAMES.contains(name)
                && !IRRELEVANT_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_git_variables_and_email_are_kept_in_upper_case() {
        let env = GitEnvironment::from_vars([
            ("git_author_name", "A"),
            ("HOME", "/home/x"),
            ("EMAIL", "e@x"),
            ("PATH", "/bin"),
        ]);
        assert_eq!(env.get("GIT_AUTHOR_NAME"), Some(OsStr::new("A")));
        assert_eq!(env.get("HOME"), None);
        assert_eq!(env.get("PATH"), Some(OsStr::new("/bin")));
        assert_eq!(env.unsupported(), None, "PATH is read, not refused");
        assert!(env.contains("EMAIL"));
    }

    #[test]
    fn unsupported_names_the_first_variable_native_cannot_honour() {
        let ok = GitEnvironment::from_vars([
            ("GIT_AUTHOR_NAME", "A"),
            ("GIT_CONFIG_GLOBAL", "/g"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_TRACE", "1"),
            ("EMAIL", "e@x"),
        ]);
        assert_eq!(ok.unsupported(), None);
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_NAMESPACE",
            "GIT_REFLOG_ACTION",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG",
            "GIT_NO_REPLACE_OBJECTS",
            "GIT_ATTR_SOURCE",
            "GIT_TEST_ASSUME_DIFFERENT_OWNER",
        ] {
            let env = GitEnvironment::from_vars([(name, "x")]);
            assert_eq!(env.unsupported(), Some(name));
        }
    }
}
