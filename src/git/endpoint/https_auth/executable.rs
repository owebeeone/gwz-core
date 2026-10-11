//! Executable discovery uses only the captured environment.
use super::*;

pub(super) fn resolve(config: &Config) -> Result<PathBuf, AuthError> {
    if config.executable.is_absolute() {
        return if program::runnable(&config.executable) {
            Ok(config.executable.clone())
        } else {
            Err(AuthError::MissingExecutable)
        };
    }
    if config.executable.as_os_str().is_empty() || config.executable.components().count() != 1 {
        return Err(AuthError::MissingExecutable);
    }
    let name = program::file_name(&config.executable).ok_or(AuthError::MissingExecutable)?;
    let path = config
        .environment
        .iter()
        .find(|(name, _)| runner::environment::names(name, "PATH"))
        .map(|(_, value)| value)
        .ok_or(AuthError::MissingExecutable)?;
    for directory in std::env::split_paths(path) {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(&name);
        if program::executable_file(&candidate) {
            return Ok(candidate);
        }
    }
    Err(AuthError::MissingExecutable)
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// What a program is on Unix: a file with an execute bit, under the name it is asked for.
        mod program {
            use std::{ffi::OsString, path::Path};

            /// Whether the absolute path `path` may be run: any program may.
            pub(super) fn runnable(_: &Path) -> bool {
                true
            }

            /// The name a directory of `PATH` holds `name` under.
            pub(super) fn file_name(name: &Path) -> Option<OsString> {
                Some(name.as_os_str().to_owned())
            }

            pub(super) fn executable_file(path: &Path) -> bool {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(path).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            }
        }
    } else if #[cfg(windows)] {
        /// What a program is on Windows: an `.exe`, and nothing else. The loader's own search is not used: there
        /// is no `PATHEXT` (so no `.cmd`, `.bat` or `.com`, and no script a shell would run), no current directory
        /// and no system directory ahead of the captured `PATH`.
        mod program {
            use std::{ffi::OsString, path::Path};

            /// Whether `path` names an `.exe`, whatever the case of its extension.
            fn is_exe(path: &Path) -> bool {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            }

            /// Whether the absolute path `path` may be run: an `.exe`.
            pub(super) fn runnable(path: &Path) -> bool {
                is_exe(path)
            }

            /// The name a directory of `PATH` holds `name` under: `name.exe` for a name with no extension, `name`
            /// itself when it is an `.exe`, and nothing for any other extension, which is never run.
            pub(super) fn file_name(name: &Path) -> Option<OsString> {
                if is_exe(name) {
                    return Some(name.as_os_str().to_owned());
                }
                if name.extension().is_some() || name.as_os_str().as_encoded_bytes().ends_with(b".") {
                    return None;
                }
                let mut with_extension = name.as_os_str().to_owned();
                with_extension.push(".exe");
                Some(with_extension)
            }

            pub(super) fn executable_file(path: &Path) -> bool {
                path.is_file()
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            use std::fs;

            cfg_if::cfg_if! {
                if #[cfg(unix)] {
                    /// The file a directory on `PATH` holds for the bare name `git`.
                    const GIT: &str = "git";

                    /// Makes `path` a program.
                    fn install(path: &std::path::Path) {
                        use std::os::unix::fs::PermissionsExt;
                        fs::write(path, "").unwrap();
                        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
                    }
                } else if #[cfg(windows)] {
                    const GIT: &str = "git.exe";

                    fn install(path: &std::path::Path) {
                        fs::write(path, "").unwrap();
                    }
                }
            }

            fn config(executable: &str, path_name: &str, path: OsString) -> Config {
                Config {
                    executable: PathBuf::from(executable),
                    environment: vec![
                        ("OTHER".into(), "value".into()),
                        (path_name.into(), path),
                    ],
                }
            }

            fn joined(entries: &[&std::path::Path]) -> OsString {
                std::env::join_paths(entries).unwrap()
            }

            #[test]
            fn a_bare_name_is_found_in_a_directory_of_the_captured_path() {
                let root = tempfile::tempdir().unwrap();
                for name in ["bin", "with spaces", "caf\u{e9} \u{65e5}\u{672c}\u{8a9e}"] {
                    let directory = root.path().join(name);
                    fs::create_dir(&directory).unwrap();
                    install(&directory.join(GIT));
                    let other = root.path().join("empty");
                    fs::create_dir_all(&other).unwrap();
                    let config = config("git", "PATH", joined(&[&other, &directory]));
                    assert_eq!(resolve(&config).unwrap(), directory.join(GIT));
                }
            }

            #[test]
            fn a_missing_program_and_a_name_with_a_directory_are_missing() {
                let root = tempfile::tempdir().unwrap();
                install(&root.path().join(GIT));
                for executable in ["", "sub/git", "./git", "../git"] {
                    let config = config(executable, "PATH", joined(&[root.path()]));
                    assert_eq!(resolve(&config).err(), Some(AuthError::MissingExecutable), "{executable:?}");
                }
                let config = config("git", "PATH", joined(&[&root.path().join("elsewhere")]));
                assert_eq!(resolve(&config).err(), Some(AuthError::MissingExecutable));
                let none = Config { executable: "git".into(), environment: Vec::new() };
                assert_eq!(resolve(&none).err(), Some(AuthError::MissingExecutable));
            }

            /// The current directory is never searched: not as an empty entry, not as `.`, and not as any
            /// relative entry.
            #[test]
            fn a_relative_or_empty_entry_never_finds_the_current_directory() {
                let here = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
                install(&here.path().join(GIT));
                let relative = PathBuf::from(here.path().file_name().unwrap());
                let entries: [OsString; 4] = [
                    OsString::new(),
                    ".".into(),
                    relative.clone().into(),
                    format!("{}{}", relative.display(), std::path::MAIN_SEPARATOR).into(),
                ];
                for entry in entries {
                    let config = config("git", "PATH", entry.clone());
                    assert_eq!(resolve(&config).err(), Some(AuthError::MissingExecutable), "{entry:?}");
                }
            }

            #[test]
            fn an_absolute_executable_is_used_as_it_is() {
                let root = tempfile::tempdir().unwrap();
                let absolute = root.path().join("with spaces").join(GIT);
                let config = Config { executable: absolute.clone(), environment: Vec::new() };
                assert_eq!(resolve(&config).unwrap(), absolute);
            }

            cfg_if::cfg_if! {
                if #[cfg(windows)] {
                    /// A Windows program's environment is not case sensitive, and the key the snapshot holds is
                    /// whichever the process that made it was given.
                    #[test]
                    fn the_path_variable_is_found_whatever_its_case() {
                        let root = tempfile::tempdir().unwrap();
                        install(&root.path().join(GIT));
                        for name in ["PATH", "Path", "path", "pAtH"] {
                            let config = config("git", name, joined(&[root.path()]));
                            assert_eq!(resolve(&config).unwrap(), root.path().join(GIT), "{name}");
                        }
                    }

                    /// Only an `.exe` is run: no `PATHEXT`, so no `.cmd`, `.bat`, `.com` or script, and a
                    /// program of the same name without an extension is not one.
                    #[test]
                    fn only_an_exe_is_ever_found() {
                        let root = tempfile::tempdir().unwrap();
                        for name in ["git.cmd", "git.bat", "git.com", "git.ps1", "git", "git.exe.txt", "other.exe"] {
                            install(&root.path().join(name));
                        }
                        let path = joined(&[root.path()]);
                        for executable in ["git", "git.cmd", "git.bat", "git.com", "git.ps1", "git.exe.txt", "git."] {
                            let config = config(executable, "PATH", path.clone());
                            let found = resolve(&config);
                            assert!(
                                matches!(found, Err(AuthError::MissingExecutable)),
                                "{executable}: {found:?}"
                            );
                        }
                    }

                    #[test]
                    fn a_name_that_is_an_exe_is_found_as_it_is_whatever_its_case() {
                        let root = tempfile::tempdir().unwrap();
                        install(&root.path().join("tool.exe"));
                        for executable in ["tool.exe", "TOOL.EXE", "Tool.Exe", "tool"] {
                            let config = config(executable, "PATH", joined(&[root.path()]));
                            let found = resolve(&config).unwrap();
                            // The file system finds `tool.exe` under any capitalization; the path is spelled as asked.
                            assert_eq!(found.parent(), Some(root.path()), "{executable}");
                            assert!(found.file_name().unwrap().eq_ignore_ascii_case("tool.exe"), "{executable}");
                        }
                    }

                    #[test]
                    fn an_absolute_executable_that_is_not_an_exe_is_refused() {
                        let root = tempfile::tempdir().unwrap();
                        for name in ["helper.cmd", "helper.bat", "helper.ps1", "helper", "helper.exe.cmd"] {
                            let config = Config { executable: root.path().join(name), environment: Vec::new() };
                            assert_eq!(resolve(&config).err(), Some(AuthError::MissingExecutable), "{name}");
                        }
                        for name in ["helper.exe", "HELPER.EXE", "helper.Exe"] {
                            let config = Config { executable: root.path().join(name), environment: Vec::new() };
                            assert_eq!(resolve(&config).unwrap(), root.path().join(name), "{name}");
                        }
                    }

                    /// A directory whose name holds the separator is quoted in `PATH`, as the Windows loader
                    /// reads it.
                    #[test]
                    fn a_quoted_entry_with_a_separator_in_its_name_is_found() {
                        let root = tempfile::tempdir().unwrap();
                        let directory = root.path().join("semi;colon dir");
                        fs::create_dir(&directory).unwrap();
                        install(&directory.join(GIT));
                        let config = config("git", "Path", format!("\"{}\"", directory.display()).into());
                        assert_eq!(resolve(&config).unwrap(), directory.join(GIT));
                    }

                    /// A drive-relative or root-relative entry names a directory by the current drive or
                    /// directory, which is the caller's and not the snapshot's.
                    #[test]
                    fn a_drive_relative_or_root_relative_entry_is_never_searched() {
                        let root = tempfile::tempdir().unwrap();
                        install(&root.path().join(GIT));
                        let drive = match root.path().components().next() {
                            Some(std::path::Component::Prefix(prefix)) => prefix.as_os_str().to_string_lossy().into_owned(),
                            other => panic!("a temporary directory has a drive: {other:?}"),
                        };
                        let without_drive = root.path().strip_prefix(format!("{drive}\\")).unwrap();
                        let rooted = format!("\\{}", without_drive.display());
                        let drive_relative = format!("{drive}{}", without_drive.display());
                        for entry in [rooted, drive_relative] {
                            let config = config("git", "PATH", entry.clone().into());
                            assert_eq!(resolve(&config).err(), Some(AuthError::MissingExecutable), "{entry}");
                        }
                    }
                }
            }
        }
    }
}
