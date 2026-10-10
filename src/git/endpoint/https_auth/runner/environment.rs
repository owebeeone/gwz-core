//! What differs between platforms in the environment of one helper child: how names compare, the neutral working
//! directory, the null configuration source and how the bytes of a configuration view become a value.
use super::*;

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;

        /// Whether the environment variable `key` is `name`.
        pub(super) fn names(key: &OsStr, name: &str) -> bool {
            key == OsStr::new(name)
        }

        /// Whether the environment variable `key` starts with `prefix`.
        pub(super) fn starts_with(key: &OsStr, prefix: &str) -> bool {
            key.as_bytes().starts_with(prefix.as_bytes())
        }

        /// The neutral directory a helper starts in: no repository, no configuration, nothing writable by a user.
        pub(super) fn working_directory(_config: &Config) -> Result<PathBuf, AuthError> {
            Ok(PathBuf::from("/"))
        }

        /// A configuration source that is always empty.
        pub(super) const NULL_CONFIG: &str = "/dev/null";

        /// A configuration view is bytes that Git wrote; the platform's environment holds them as they are.
        pub(super) fn parameters_value(bytes: &[u8]) -> Result<OsString, AuthError> {
            Ok(OsStr::from_bytes(bytes).to_owned())
        }
    } else if #[cfg(windows)] {
        /// Whether the environment variable `key` is `name`: Windows compares names without regard to ASCII case.
        pub(super) fn names(key: &OsStr, name: &str) -> bool {
            key.eq_ignore_ascii_case(name)
        }

        /// Whether the environment variable `key` starts with `prefix`, without regard to ASCII case. A name that
        /// is not Unicode cannot start with an ASCII prefix.
        pub(super) fn starts_with(key: &OsStr, prefix: &str) -> bool {
            key.to_str()
                .and_then(|key| key.get(..prefix.len()))
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        }

        /// The neutral directory a helper starts in: the Windows directory the captured environment names. It has no
        /// repository and no configuration, and a user cannot write to it. A snapshot without one is refused:
        /// the lookup never falls back to the directory the process happens to be in.
        pub(super) fn working_directory(config: &Config) -> Result<PathBuf, AuthError> {
            config
                .environment
                .iter()
                .find(|(name, _)| names(name, "SystemRoot"))
                .map(|(_, value)| PathBuf::from(value))
                .filter(|directory| directory.is_absolute())
                .ok_or(AuthError::ConfigurationRefused)
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

            #[test]
            fn the_neutral_directory_is_absolute() {
                let mut config = Config {
                    executable: PathBuf::from("git"),
                    environment: vec![("SYSTEMROOT".into(), "relative".into())],
                };
                if cfg!(windows) {
                    assert!(matches!(
                        working_directory(&config),
                        Err(AuthError::ConfigurationRefused)
                    ));
                    config.environment.clear();
                    assert!(matches!(
                        working_directory(&config),
                        Err(AuthError::ConfigurationRefused)
                    ));
                    config.environment.push(("SYSTEMROOT".into(), "C:\\Windows".into()));
                    assert_eq!(
                        working_directory(&config).unwrap(),
                        PathBuf::from("C:\\Windows")
                    );
                } else {
                    assert_eq!(working_directory(&config).unwrap(), PathBuf::from("/"));
                }
            }

            #[test]
            fn a_view_that_the_environment_cannot_carry_is_refused_rather_than_converted() {
                assert_eq!(
                    parameters_value(b"'a'='b'").unwrap(),
                    OsString::from("'a'='b'")
                );
                assert_eq!(parameters_value(b"'a'='\xff'").is_err(), cfg!(windows));
            }
        }
    }
}
