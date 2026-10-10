//! What differs between platforms in the environment of one helper child, and the environment itself: how names
//! compare, the neutral working directory and the ceiling above it, the null configuration source and how the
//! bytes of a configuration view become a value.
use super::*;

/// The complete environment of one helper. Names are replaced and removed by the platform's own comparison.
pub(super) struct Environment(Vec<(OsString, OsString)>);

impl Environment {
    /// The captured snapshot without the variables that would hand the helper another program or another
    /// repository.
    pub(super) fn snapshot(captured: &[(OsString, OsString)]) -> Self {
        let mut environment = Self(Vec::new());
        for (name, value) in captured {
            if [
                "GIT_ASKPASS",
                "SSH_ASKPASS",
                "GIT_DIR",
                "GIT_COMMON_DIR",
                "GIT_WORK_TREE",
            ]
            .iter()
            .any(|removed| names(name, removed))
            {
                continue;
            }
            environment.set(name, value);
        }
        environment
    }

    /// Sets `name`, replacing every variable the platform treats as the same name.
    pub(super) fn set(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
        let name = name.as_ref();
        self.0.retain(|(existing, _)| !names_os(existing, name));
        self.0.push((name.to_owned(), value.as_ref().to_owned()));
    }

    /// Removes `name`.
    pub(super) fn remove(&mut self, name: &str) {
        self.0.retain(|(existing, _)| !names(existing, name));
    }

    /// Removes every variable whose name starts with `prefix`.
    pub(super) fn remove_prefix(&mut self, prefix: &str) {
        self.0
            .retain(|(existing, _)| !starts_with(existing, prefix));
    }

    pub(super) fn into_pairs(self) -> Vec<(OsString, OsString)> {
        self.0
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl Environment {
            pub(super) fn get(&self, name: &str) -> Option<&OsStr> {
                self.0
                    .iter()
                    .find(|(existing, _)| names(existing, name))
                    .map(|(_, value)| value.as_os_str())
            }
        }
    }
}

fn names_os(left: &OsStr, right: &OsStr) -> bool {
    names_pair(left, right)
}

/// Whether the environment variable `key` is `name`.
pub(super) fn names(key: &OsStr, name: &str) -> bool {
    names_pair(key, OsStr::new(name))
}

/// Whether the environment variable `key` starts with `prefix`. Both compare as encoded bytes: a name that is
/// not Unicode still starts with an ASCII prefix when its first bytes are that prefix.
pub(super) fn starts_with(key: &OsStr, prefix: &str) -> bool {
    key.as_encoded_bytes()
        .get(..prefix.len())
        .is_some_and(|head| same_bytes(head, prefix.as_bytes()))
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;

        fn names_pair(left: &OsStr, right: &OsStr) -> bool {
            left == right
        }

        fn same_bytes(left: &[u8], right: &[u8]) -> bool {
            left == right
        }

        /// The neutral directory a helper starts in: no repository, no configuration, nothing writable by a user.
        pub(super) fn working_directory() -> Result<PathBuf, AuthError> {
            Ok(PathBuf::from("/"))
        }

        /// A directory above the working directory that Git must not search for a repository: `/` has none.
        pub(super) fn confine(_environment: &mut Environment, _directory: &Path) {}

        /// A configuration source that is always empty.
        pub(super) const NULL_CONFIG: &str = "/dev/null";

        /// A configuration view is bytes that Git wrote; the platform's environment holds them as they are.
        pub(super) fn parameters_value(bytes: &[u8]) -> Result<OsString, AuthError> {
            Ok(OsStr::from_bytes(bytes).to_owned())
        }
    } else if #[cfg(windows)] {
        /// Windows compares names without regard to ASCII case.
        fn names_pair(left: &OsStr, right: &OsStr) -> bool {
            left.eq_ignore_ascii_case(right)
        }

        fn same_bytes(left: &[u8], right: &[u8]) -> bool {
            left.eq_ignore_ascii_case(right)
        }

        /// The neutral directory a helper starts in: the Windows directory, which the operating system names (not
        /// the captured environment, which is the caller's). It has no repository and no configuration, and a user
        /// cannot write to it.
        pub(super) fn working_directory() -> Result<PathBuf, AuthError> {
            use std::os::windows::ffi::OsStringExt;
            use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
            let mut buffer = vec![0u16; 32768];
            // SAFETY: the buffer is initialized and its capacity matches the size passed.
            let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
            if length == 0 || length >= buffer.len() {
                return Err(AuthError::ConfigurationRefused);
            }
            Ok(PathBuf::from(OsString::from_wide(&buffer[..length])))
        }

        /// The Windows directory's parent is a drive root, where any user may create a folder, and Git searches
        /// upward for a repository: the parent is a ceiling Git does not chdir up into.
        pub(super) fn confine(environment: &mut Environment, directory: &Path) {
            if let Some(parent) = directory.parent() {
                environment.set("GIT_CEILING_DIRECTORIES", parent);
            }
        }

        /// A configuration source that is always empty.
        pub(super) const NULL_CONFIG: &str = "NUL";

        /// A configuration view is UTF-8 that Git wrote, and a Windows environment value is UTF-16. A view that is
        /// not UTF-8 is refused; it is never converted with a replacement character.
        pub(super) fn parameters_value(bytes: &[u8]) -> Result<OsString, AuthError> {
            std::str::from_utf8(bytes)
                .map(OsString::from)
                .map_err(|_| AuthError::ConfigurationRefused)
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;

            fn pairs(items: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
                items.iter().map(|(name, value)| ((*name).into(), (*value).into())).collect()
            }

            #[test]
            fn a_name_matches_exactly_and_the_platform_decides_about_capitalization() {
                assert!(names(OsStr::new("GIT_DIR"), "GIT_DIR"));
                assert!(!names(OsStr::new("GIT_DIRS"), "GIT_DIR"));
                assert!(!names(OsStr::new("GIT_DI"), "GIT_DIR"));
                assert_eq!(names(OsStr::new("git_dir"), "GIT_DIR"), cfg!(windows));
            }

            #[test]
            fn a_prefix_matches_a_name_that_starts_with_it() {
                assert!(starts_with(OsStr::new("GIT_CONFIG_KEY_0"), "GIT_CONFIG_KEY_"));
                assert!(!starts_with(OsStr::new("GIT_CONFIG_KEY"), "GIT_CONFIG_KEY_"));
                assert!(!starts_with(OsStr::new("X"), "GIT_CONFIG_KEY_"));
                assert_eq!(starts_with(OsStr::new("git_config_key_0"), "GIT_CONFIG_KEY_"), cfg!(windows));
            }

            #[cfg(windows)]
            #[test]
            fn a_name_that_is_not_unicode_still_starts_with_an_ascii_prefix() {
                use std::os::windows::ffi::OsStringExt;
                // GIT_CONFIG_KEY_ followed by a lone surrogate: not Unicode, but the prefix is there.
                let mut units: Vec<u16> = "git_config_key_".encode_utf16().collect();
                units.push(0xD800);
                let name = OsString::from_wide(&units);
                assert!(name.to_str().is_none());
                assert!(starts_with(&name, "GIT_CONFIG_KEY_"));
            }

            #[test]
            fn the_snapshot_drops_the_variables_that_name_another_program_or_repository() {
                let environment = Environment::snapshot(&pairs(&[
                    ("GIT_ASKPASS", "a"),
                    ("SSH_ASKPASS", "b"),
                    ("GIT_DIR", "c"),
                    ("GIT_COMMON_DIR", "d"),
                    ("GIT_WORK_TREE", "e"),
                    ("KEPT", "f"),
                ]));
                assert_eq!(environment.into_pairs(), pairs(&[("KEPT", "f")]));
            }

            #[test]
            fn setting_a_name_replaces_every_spelling_the_platform_treats_as_one() {
                let mut environment = Environment::snapshot(&pairs(&[("Git_Terminal_Prompt", "1"), ("KEPT", "k")]));
                environment.set("GIT_TERMINAL_PROMPT", "0");
                let all = environment.into_pairs();
                let prompts: Vec<_> = all
                    .iter()
                    .filter(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("GIT_TERMINAL_PROMPT"))
                    .collect();
                assert_eq!(prompts.len(), if cfg!(windows) { 1 } else { 2 });
                assert!(all.contains(&("GIT_TERMINAL_PROMPT".into(), "0".into())));
            }

            #[test]
            fn removing_a_prefix_removes_every_variable_that_starts_with_it() {
                let mut environment = Environment::snapshot(&pairs(&[
                    ("GIT_CONFIG_COUNT", "1"),
                    ("GIT_CONFIG_KEY_0", "k"),
                    ("GIT_CONFIG_VALUE_0", "v"),
                    ("GIT_CONFIG_GLOBAL", "g"),
                ]));
                environment.remove_prefix("GIT_CONFIG_KEY_");
                environment.remove_prefix("GIT_CONFIG_VALUE_");
                environment.remove("GIT_CONFIG_COUNT");
                assert_eq!(environment.into_pairs(), pairs(&[("GIT_CONFIG_GLOBAL", "g")]));
            }

            #[test]
            fn the_neutral_directory_is_absolute_and_has_a_ceiling_on_windows() {
                let directory = working_directory().unwrap();
                assert!(directory.is_absolute());
                let mut environment = Environment::snapshot(&pairs(&[("GIT_CEILING_DIRECTORIES", "caller")]));
                confine(&mut environment, &directory);
                if cfg!(windows) {
                    assert_eq!(
                        environment.get("GIT_CEILING_DIRECTORIES"),
                        directory.parent().map(|parent| parent.as_os_str())
                    );
                } else {
                    assert_eq!(directory, PathBuf::from("/"));
                    assert_eq!(environment.get("GIT_CEILING_DIRECTORIES"), Some(OsStr::new("caller")));
                }
            }

            #[test]
            fn a_view_that_the_environment_cannot_carry_is_refused_rather_than_converted() {
                assert_eq!(parameters_value(b"'a'='b'").unwrap(), OsString::from("'a'='b'"));
                assert_eq!(parameters_value(b"'a'='\xff'").is_err(), cfg!(windows));
            }
        }
    }
}
