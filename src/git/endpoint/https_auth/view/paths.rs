//! The paths of the configuration view are bytes that Git wrote, and this is where they become paths and back
//! (GwzTransportWindowsHelperOwnerContract-DRAFT.md, section 7; plan step 4.3).
//!
//! A Unix path is bytes, and the bytes are the path. A Windows path is UTF-16, and the bytes Git writes for one are
//! UTF-8: a path that is not UTF-8, or whose UTF-16 has no UTF-8 (a lone surrogate), is refused with
//! `ConfigurationRefused`. It is never converted with a replacement character, which would name another file.
use super::super::*;
use std::path::Path;

/// The path that `bytes` name.
pub(in crate::git::endpoint::https_auth) fn path(bytes: &[u8]) -> Result<&Path, AuthError> {
    sys::path(bytes)
}

/// The bytes that name `path`, as Git is given them and as it writes them.
pub(in crate::git::endpoint::https_auth) fn bytes(path: &Path) -> Result<&[u8], AuthError> {
    sys::bytes(path)
}

/// The bytes of an environment value that names a path.
pub(in crate::git::endpoint::https_auth) fn value_bytes(value: &OsStr) -> Result<&[u8], AuthError> {
    sys::value_bytes(value)
}

/// The directory a relative path is joined to when its source has none of its own: the helper's working directory
/// on Unix, `/`. Windows has none (its neutral directory is the Windows directory, and a path with no anchor there
/// has no meaning), so a relative path is refused.
pub(in crate::git::endpoint::https_auth) const ROOT: &[u8] = sys::ROOT;

/// Whether `byte` ends a directory name.
pub(in crate::git::endpoint::https_auth) fn is_separator(byte: u8) -> bool {
    sys::is_separator(byte)
}

/// Whether `path`, which is not absolute, is a plain relative path. On Windows a path with a root and no drive
/// (`\dir`) or a drive and no root (`C:dir`) is not: it depends on the current drive or directory, which Git
/// decides and the view does not know.
pub(in crate::git::endpoint::https_auth) fn is_plain_relative(path: &Path) -> bool {
    sys::is_plain_relative(path)
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod sys {
            use super::*;
            use std::os::unix::ffi::OsStrExt;

            pub(super) fn path(bytes: &[u8]) -> Result<&Path, AuthError> {
                Ok(Path::new(OsStr::from_bytes(bytes)))
            }

            pub(super) fn bytes(path: &Path) -> Result<&[u8], AuthError> {
                Ok(path.as_os_str().as_bytes())
            }

            pub(super) fn value_bytes(value: &OsStr) -> Result<&[u8], AuthError> {
                Ok(value.as_bytes())
            }

            pub(super) const ROOT: &[u8] = b"/";

            pub(super) fn is_separator(byte: u8) -> bool {
                byte == b'/'
            }

            pub(super) fn is_plain_relative(_: &Path) -> bool {
                true
            }
        }
    } else if #[cfg(windows)] {
        mod sys {
            use super::*;
            use std::path::Component;

            pub(super) fn path(bytes: &[u8]) -> Result<&Path, AuthError> {
                std::str::from_utf8(bytes)
                    .map(Path::new)
                    .map_err(|_| AuthError::ConfigurationRefused)
            }

            pub(super) fn bytes(path: &Path) -> Result<&[u8], AuthError> {
                path.to_str()
                    .map(str::as_bytes)
                    .ok_or(AuthError::ConfigurationRefused)
            }

            pub(super) fn value_bytes(value: &OsStr) -> Result<&[u8], AuthError> {
                value
                    .to_str()
                    .map(str::as_bytes)
                    .ok_or(AuthError::ConfigurationRefused)
            }

            pub(super) const ROOT: &[u8] = b"";

            pub(super) fn is_separator(byte: u8) -> bool {
                byte == b'/' || byte == b'\\'
            }

            pub(super) fn is_plain_relative(path: &Path) -> bool {
                !matches!(
                    path.components().next(),
                    Some(Component::Prefix(_) | Component::RootDir)
                )
            }
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;

            cfg_if::cfg_if! {
                if #[cfg(unix)] {
                    const ABSOLUTE: &str = "/dir with spaces/caf\u{e9} \u{65e5}\u{672c}\u{8a9e}";

                    #[test]
                    fn bytes_that_are_not_utf8_are_a_path_on_unix() {
                        let raw = b"/dir/\xff\xfe";
                        assert_eq!(bytes(path(raw).unwrap()).unwrap(), raw);
                        assert_eq!(value_bytes(OsStr::new("a")).unwrap(), b"a");
                        assert_eq!(ROOT, b"/");
                        assert!(is_separator(b'/') && !is_separator(b'\\'));
                        assert!(is_plain_relative(Path::new("/rooted")));
                    }
                } else if #[cfg(windows)] {
                    use std::{ffi::OsString, os::windows::ffi::OsStringExt};

                    const ABSOLUTE: &str = "C:/dir with spaces/caf\u{e9} \u{65e5}\u{672c}\u{8a9e}";

                    /// A lone surrogate is a legal Windows name, with no UTF-8.
                    fn lone_surrogate() -> OsString {
                        let mut units: Vec<u16> = "C:\\dir\\".encode_utf16().collect();
                        units.push(0xD800);
                        OsString::from_wide(&units)
                    }

                    #[test]
                    fn bytes_that_are_not_utf8_are_refused_on_windows() {
                        assert_eq!(path(b"C:/\xff").err(), Some(AuthError::ConfigurationRefused));
                        assert_eq!(path(b"\xc0\xaf").err(), Some(AuthError::ConfigurationRefused));
                    }

                    #[test]
                    fn a_path_with_a_lone_surrogate_has_no_bytes() {
                        let name = lone_surrogate();
                        assert_eq!(bytes(Path::new(&name)).err(), Some(AuthError::ConfigurationRefused));
                        assert_eq!(value_bytes(&name).err(), Some(AuthError::ConfigurationRefused));
                    }

                    #[test]
                    fn separators_roots_and_prefixes_are_told_apart() {
                        assert!(is_separator(b'/') && is_separator(b'\\') && !is_separator(b'a'));
                        assert_eq!(ROOT, b"");
                        for relative in ["dir", "dir\\file", ".\\file", "..\\file", "dir/file"] {
                            assert!(is_plain_relative(Path::new(relative)), "{relative}");
                        }
                        for other in ["\\rooted", "/rooted", "C:drive-relative", "\\\\server\\share"] {
                            assert!(!is_plain_relative(Path::new(other)), "{other}");
                        }
                    }
                }
            }

            #[test]
            fn a_path_with_spaces_and_non_ascii_characters_round_trips() {
                let carried = path(ABSOLUTE.as_bytes()).unwrap();
                assert!(carried.is_absolute());
                assert_eq!(bytes(carried).unwrap(), ABSOLUTE.as_bytes());
                assert_eq!(value_bytes(carried.as_os_str()).unwrap(), ABSOLUTE.as_bytes());
            }
        }
    }
}
