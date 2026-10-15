//! What differs between platforms in the environment of one helper child, and the environment itself: how names
//! compare, the neutral working directory and the ceiling above it, the null configuration source and how the
//! bytes of a configuration view become a value.
use super::*;
use crate::git::endpoint::https_auth::wiped::{Probe, Wiped};
use process_tree::{EnvValue, Unit};

/// The complete environment of one helper. Names are replaced and removed by the platform's own comparison. Every
/// value is in a wiping owner: the configuration view among them can carry a credential.
pub(super) struct Environment(Vec<(OsString, EnvValue)>);

/// The environment of one helper: the captured snapshot without what would hand it another program or repository,
/// the neutral working directory's ceiling and, with a configuration view, the view as the only configuration. The
/// view's bytes go from the borrowed `parameters` into an owner of the platform's units that wipes them; no
/// `String`, `OsString` or growing buffer holds them on the way. `probe` is how a test sees that owner drop.
pub(super) fn helper_environment(
    captured: &[(OsString, OsString)],
    directory: &Path,
    parameters: Option<&[u8]>,
    probe: Probe,
) -> Result<Environment, AuthError> {
    let mut variables = Environment::snapshot(captured);
    variables.set("GIT_TERMINAL_PROMPT", "0");
    confine(&mut variables, directory);
    if let Some(parameters) = parameters {
        variables.remove("GIT_CONFIG_COUNT");
        variables.remove_prefix("GIT_CONFIG_KEY_");
        variables.remove_prefix("GIT_CONFIG_VALUE_");
        variables.remove("GIT_CONFIG_SYSTEM");
        variables.set("GIT_CONFIG_GLOBAL", NULL_CONFIG);
        variables.set("GIT_CONFIG_NOSYSTEM", "1");
        variables.set_value(
            "GIT_CONFIG_PARAMETERS",
            parameters_value(parameters, probe)?,
        );
        if parameters.is_empty() {
            variables.remove("GIT_CONFIG_PARAMETERS");
        }
    }
    Ok(variables)
}

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

    /// Sets `name` to a copy of `value` in an owner of its own, replacing every variable the platform treats as
    /// the same name.
    pub(super) fn set(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
        self.set_value(name, EnvValue::from_os(value.as_ref(), Probe::inert()));
    }

    /// Sets `name` to `value`, which moves in with its owner, replacing every variable the platform treats as the
    /// same name.
    pub(super) fn set_value(&mut self, name: impl AsRef<OsStr>, value: EnvValue) {
        let name = name.as_ref();
        self.0.retain(|(existing, _)| !names_os(existing, name));
        self.0.push((name.to_owned(), value));
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

    pub(super) fn into_pairs(self) -> Vec<(OsString, EnvValue)> {
        self.0
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl Environment {
            pub(super) fn get(&self, name: &str) -> Option<OsString> {
                self.0
                    .iter()
                    .find(|(existing, _)| names(existing, name))
                    .map(|(_, value)| value.to_os_string())
            }

            /// The pairs as plain strings, for a comparison or a test's own child. Not for the product.
            pub(super) fn plain(&self) -> Vec<(OsString, OsString)> {
                self.0
                    .iter()
                    .map(|(name, value)| (name.clone(), value.to_os_string()))
                    .collect()
            }
        }

        /// `pairs` as a helper's environment exactly as given, nothing removed or added.
        pub(in crate::git::endpoint::https_auth) fn exact(pairs: &[(OsString, OsString)]) -> Vec<(OsString, EnvValue)> {
            pairs
                .iter()
                .map(|(name, value)| (name.clone(), EnvValue::from_os(value, Probe::inert())))
                .collect()
        }
    }
}

fn names_os(left: &OsStr, right: &OsStr) -> bool {
    names_pair(left, right)
}

/// Whether the environment variable `key` is `name`.
pub(in crate::git::endpoint::https_auth) fn names(key: &OsStr, name: &str) -> bool {
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

        /// A configuration view is bytes that Git wrote; the platform's environment holds them as they are, here in
        /// one owner of exactly their length.
        pub(super) fn parameters_value(bytes: &[u8], probe: Probe) -> Result<EnvValue, AuthError> {
            let mut owner = Wiped::<Unit>::zeroed(bytes.len(), probe);
            owner.as_mut_slice().copy_from_slice(bytes);
            Ok(EnvValue::from_owner(owner))
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

        /// A configuration source that is always empty. Lower case: Git for Windows 2.56.0.windows.1 refused
        /// `NUL` as a configuration file ("unable to access 'NUL'", git-for-windows/git#6449, fixed in
        /// 2.56.0.windows.2) and accepts `nul` in every version.
        pub(super) const NULL_CONFIG: &str = "nul";

        /// A configuration view is UTF-8 that Git wrote, and a Windows environment value is UTF-16. A view that is
        /// not UTF-8 is refused; it is never converted with a replacement character. The UTF-16 is written straight
        /// from the borrowed text into one owner sized beforehand: no `String`, `OsString` or growing `Vec` holds it.
        pub(super) fn parameters_value(bytes: &[u8], probe: Probe) -> Result<EnvValue, AuthError> {
            let text = std::str::from_utf8(bytes).map_err(|_| AuthError::ConfigurationRefused)?;
            let mut owner = Wiped::<Unit>::zeroed(text.encode_utf16().count(), probe);
            for (slot, unit) in owner.as_mut_slice().iter_mut().zip(text.encode_utf16()) {
                *slot = unit;
            }
            Ok(EnvValue::from_owner(owner))
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            use crate::git::endpoint::https_auth::wiped::Records;

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
                assert_eq!(environment.plain(), pairs(&[("KEPT", "f")]));
            }

            #[test]
            fn setting_a_name_replaces_every_spelling_the_platform_treats_as_one() {
                let mut environment = Environment::snapshot(&pairs(&[("Git_Terminal_Prompt", "1"), ("KEPT", "k")]));
                environment.set("GIT_TERMINAL_PROMPT", "0");
                let all = environment.plain();
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
                assert_eq!(environment.plain(), pairs(&[("GIT_CONFIG_GLOBAL", "g")]));
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
                        directory.parent().map(|parent| parent.as_os_str().to_owned())
                    );
                } else {
                    assert_eq!(directory, PathBuf::from("/"));
                    assert_eq!(environment.get("GIT_CEILING_DIRECTORIES"), Some(OsString::from("caller")));
                }
            }

            #[test]
            fn the_empty_configuration_is_spelled_as_every_git_for_windows_accepts() {
                // Git for Windows 2.56.0.windows.1 refuses `NUL`; `nul` and `/dev/null` work in it and before.
                assert!(NULL_CONFIG == "nul" || NULL_CONFIG == "/dev/null", "{NULL_CONFIG}");
            }

            /// P3-3: the view reaches the helper's environment in one owner that is overwritten when the environment
            /// drops, and nothing else holds it: the value is as long as the view in the platform's units, and the
            /// probe sees it all zero. A plain `String` or `OsString` copy would have no probe to record.
            #[test]
            fn the_configuration_view_is_held_in_a_wiping_owner_until_the_environment_drops() {
                let view = b"'http.extraheader'='Authorization: Bearer token-0123456789'";
                let records = Records::default();
                let environment = helper_environment(
                    &pairs(&[("KEPT", "f"), ("GIT_CONFIG_PARAMETERS", "caller's own view")]),
                    &working_directory().unwrap(),
                    Some(view),
                    Probe::recording(&records),
                )
                .unwrap();
                assert_eq!(
                    environment.get("GIT_CONFIG_PARAMETERS"),
                    Some(OsString::from(std::str::from_utf8(view).unwrap()))
                );
                assert!(records.lock().unwrap().is_empty(), "the owner is alive while the environment is");
                let child_pairs = environment.into_pairs();
                assert!(records.lock().unwrap().is_empty(), "moving to the command moves the owner, it copies nothing");
                drop(child_pairs);
                assert_eq!(*records.lock().unwrap(), vec![(view.len(), true)]);
            }

            #[test]
            fn an_empty_view_leaves_no_parameters_variable_and_its_owner_is_wiped_at_once() {
                let records = Records::default();
                let environment = helper_environment(
                    &pairs(&[("KEPT", "f")]),
                    &working_directory().unwrap(),
                    Some(b""),
                    Probe::recording(&records),
                )
                .unwrap();
                assert_eq!(environment.get("GIT_CONFIG_PARAMETERS"), None);
                assert_eq!(*records.lock().unwrap(), vec![(0, true)]);
            }

            #[test]
            fn a_view_that_replaces_a_value_wipes_the_value_it_replaced() {
                let records = Records::default();
                let mut environment = Environment::snapshot(&pairs(&[("KEPT", "f")]));
                environment.set_value("A", EnvValue::from_os(OsStr::new("first"), Probe::recording(&records)));
                environment.set_value("A", EnvValue::from_os(OsStr::new("second"), Probe::inert()));
                assert_eq!(*records.lock().unwrap(), vec![(5, true)]);
            }

            #[test]
            fn a_view_that_the_environment_cannot_carry_is_refused_rather_than_converted() {
                let value = parameters_value(b"'a'='b'", Probe::inert()).unwrap();
                assert_eq!(value.to_os_string(), OsString::from("'a'='b'"));
                assert_eq!(parameters_value(b"'a'='\xff'", Probe::inert()).is_err(), cfg!(windows));
            }
        }
    }
}
