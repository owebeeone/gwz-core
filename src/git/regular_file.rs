//! Opening a regular file without blocking on a special one (GwzTransportWindowsParityPlan.md, step 1.3).
//!
//! The transport reads a `known_hosts`, a private-key file or an identity file only after it has shown the path
//! names a regular file. A path can instead name a FIFO, a device or a named pipe, and opening or reading one of
//! those can block a thread that the transport must keep responsive. [`open`] refuses anything but a regular file
//! with `InvalidInput`, and never blocks to find out.
use cfg_if::cfg_if;
use std::{fs::File, io, path::Path};

/// Opens `path` for reading if it names a regular file.
///
/// `InvalidInput` means the path names something else: a directory, a FIFO, a device, a pipe. Any other error is the
/// operating system's own (`NotFound`, `PermissionDenied`).
pub(crate) fn open(path: &Path) -> io::Result<File> {
    sys::open(path)
}

/// Whether `text`, a path, names a device or a pipe by its spelling: Windows opens `CON` and `\\.\pipe\x` as
/// devices, and some of them block. Compiled everywhere so that every platform's test run covers the table that
/// only the Windows arm of [`open`] consults.
#[allow(dead_code)]
fn refuses_device_path(text: &str) -> bool {
    let is_separator = |c: char| c == '\\' || c == '/';
    let starts = |prefixes: &[&str]| prefixes.iter().find_map(|prefix| text.strip_prefix(prefix));
    // The NT namespace is never a path a user writes, and the Win32 device namespace holds `\\.\NUL`,
    // `\\.\pipe\x` and `\\.\C:`.
    if text.starts_with("\\??\\") || starts(&["\\\\.\\", "//./", "\\\\./", "//.\\"]).is_some() {
        return true;
    }
    let mut rest = text;
    // The extended-length namespace names files as well as devices: only a drive or a UNC path is a file.
    if let Some(after) = starts(&["\\\\?\\", "//?/", "\\\\?/", "//?\\"]) {
        let bytes = after.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && is_separator(bytes[2] as char);
        let unc = after
            .get(..4)
            .is_some_and(|head| head.eq_ignore_ascii_case("UNC\\"));
        if !drive && !unc {
            return true;
        }
        rest = after;
    }
    rest.split(is_separator)
        .enumerate()
        .any(|(index, component)| {
            // A leading drive, `C:` or the drive-relative `C:NUL`, is not a colon in a name. Any other colon names an
            // alternate data stream or a device (`NUL:`).
            let name = match component.as_bytes() {
                [letter, b':', ..] if index == 0 && letter.is_ascii_alphabetic() => &component[2..],
                _ => component,
            };
            name.contains(':') || reserved_name(name)
        })
}

/// Whether the file name `name` is one of the reserved DOS device names, with or without an extension.
fn reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    let upper = stem.to_uppercase();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => return true,
        _ => {}
    }
    // COM1 to COM9 and LPT1 to LPT9, with the superscript digits Windows also reserves.
    let digits = [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "\u{b9}", "\u{b2}", "\u{b3}",
    ];
    ["COM", "LPT"].iter().any(|device| {
        upper
            .strip_prefix(device)
            .is_some_and(|number| digits.contains(&number))
    })
}

cfg_if! {
    if #[cfg(unix)] {
        mod sys {
            use std::{
                fs::{File, OpenOptions},
                io,
                os::unix::fs::OpenOptionsExt,
                path::Path,
            };

            /// `O_NONBLOCK` keeps the open of a FIFO from waiting for a writer; `fstat` then shows it is not a
            /// regular file.
            pub(super) fn open(path: &Path) -> io::Result<File> {
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(path)?;
                super::ensure_regular(&file)?;
                Ok(file)
            }
        }

        /// Whether `file`, just opened, is a regular file.
        fn ensure_regular(file: &File) -> io::Result<()> {
            if file.metadata()?.file_type().is_file() {
                Ok(())
            } else {
                Err(io::ErrorKind::InvalidInput.into())
            }
        }
    } else if #[cfg(windows)] {
        mod sys {
            use std::{
                fs::{File, OpenOptions},
                io,
                path::Path,
            };

            /// The name is judged before the open, because opening `CON` or a pipe path is itself what can block;
            /// the handle's type is judged after, because a name can lead anywhere.
            pub(super) fn open(path: &Path) -> io::Result<File> {
                if super::refuses_device_path(&path.to_string_lossy()) {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
                let file = match OpenOptions::new().read(true).open(path) {
                    Ok(file) => file,
                    // Windows refuses to open a directory as a file with "access denied".
                    Err(error)
                        if error.kind() == io::ErrorKind::PermissionDenied
                            && std::fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()) =>
                    {
                        return Err(io::ErrorKind::InvalidInput.into());
                    }
                    Err(error) => return Err(error),
                };
                super::ensure_regular(&file)?;
                Ok(file)
            }
        }

        /// Whether `file`, just opened, is a disk file: not a pipe, a character device or an unknown handle.
        fn ensure_regular(file: &File) -> io::Result<()> {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_DISK, GetFileType};
            if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            if file.metadata()?.is_file() {
                Ok(())
            } else {
                Err(io::ErrorKind::InvalidInput.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Read, sync::mpsc, thread, time::Duration};
    use tempfile::TempDir;

    fn read(path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        open(path)?.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Runs `work` on a thread and fails the test, rather than hanging it, if it does not return in time.
    fn without_blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, result) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(work());
        });
        result
            .recv_timeout(Duration::from_secs(10))
            .expect("the open blocked on a special file")
    }

    #[test]
    fn a_regular_file_reads() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("known_hosts");
        fs::write(&path, b"host key\n").unwrap();
        assert_eq!(read(&path).unwrap(), b"host key\n");
    }

    #[test]
    fn an_empty_file_reads_as_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty");
        fs::write(&path, b"").unwrap();
        assert!(read(&path).unwrap().is_empty());
    }

    #[test]
    fn paths_with_spaces_and_non_ascii_characters_read() {
        let dir = TempDir::new().unwrap();
        for name in [
            "with spaces",
            "caf\u{e9} \u{65e5}\u{672c}\u{8a9e}",
            "  padded  .txt",
        ] {
            let path = dir.path().join(name);
            fs::write(&path, name.as_bytes()).unwrap();
            assert_eq!(read(&path).unwrap(), name.as_bytes(), "{name}");
        }
        let nested = dir.path().join("sub dir \u{e9}");
        fs::create_dir(&nested).unwrap();
        let path = nested.join("key file");
        fs::write(&path, b"k").unwrap();
        assert_eq!(read(&path).unwrap(), b"k");
    }

    #[test]
    fn a_directory_is_refused() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            open(dir.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            open(&dir.path().join("absent")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn a_file_that_is_replaced_by_a_directory_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("swapped");
        fs::write(&path, b"x").unwrap();
        assert!(open(&path).is_ok());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert_eq!(open(&path).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::{ffi::CString, os::unix::{ffi::OsStrExt, fs::symlink}};

            fn fifo(path: &Path) {
                let name = CString::new(path.as_os_str().as_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            }

            #[test]
            fn a_fifo_is_refused_without_blocking() {
                use std::time::Instant;
                let dir = TempDir::new().unwrap();
                let path = dir.path().join("fifo");
                fifo(&path);
                let started = Instant::now();
                let error = without_blocking(move || open(&path)).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
                assert!(started.elapsed() < Duration::from_secs(5));
            }

            #[test]
            fn a_symlink_to_a_fifo_is_refused_and_one_to_a_file_reads() {
                let dir = TempDir::new().unwrap();
                let pipe = dir.path().join("fifo");
                fifo(&pipe);
                let file = dir.path().join("file");
                fs::write(&file, b"v").unwrap();
                let (to_pipe, to_file) = (dir.path().join("to-pipe"), dir.path().join("to-file"));
                symlink(&pipe, &to_pipe).unwrap();
                symlink(&file, &to_file).unwrap();
                assert_eq!(
                    without_blocking(move || open(&to_pipe)).unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
                assert_eq!(read(&to_file).unwrap(), b"v");
            }

            #[test]
            fn a_device_is_refused() {
                // /dev/null is a character device: reading it never blocks, but it is not a regular file.
                assert_eq!(
                    open(Path::new("/dev/null")).unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
            }
        } else if #[cfg(windows)] {
            #[test]
            fn device_names_are_refused_without_blocking() {
                for name in ["CON", "con", "NUL", "nul.txt", "COM1", "LPT9", "AUX", "PRN", "CONIN$", "CONOUT$", "\\\\.\\NUL", "\\\\.\\CON"] {
                    let owned = name.to_owned();
                    let error = without_blocking(move || open(Path::new(&owned))).unwrap_err();
                    assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name}");
                }
            }

            #[test]
            fn a_pipe_path_with_no_server_is_refused_without_blocking() {
                let error = without_blocking(|| open(Path::new("\\\\.\\pipe\\gwz-regular-file-absent"))).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            }

            #[test]
            fn a_pipe_handle_is_not_a_regular_file() {
                use std::{os::windows::io::{FromRawHandle, RawHandle}, time::{SystemTime, UNIX_EPOCH}};
                use windows_sys::Win32::{
                    Foundation::INVALID_HANDLE_VALUE,
                    Storage::FileSystem::PIPE_ACCESS_DUPLEX,
                    System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT},
                };
                let name = format!(
                    "\\\\.\\pipe\\gwz-regular-file-{}-{}",
                    std::process::id(),
                    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
                );
                let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
                let handle = unsafe {
                    CreateNamedPipeW(wide.as_ptr(), PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, std::ptr::null())
                };
                assert_ne!(handle, INVALID_HANDLE_VALUE, "{}", io::Error::last_os_error());
                let pipe = unsafe { File::from_raw_handle(handle as RawHandle) };
                let error = without_blocking(move || ensure_regular(&pipe)).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            }
        }
    }

    #[test]
    fn device_path_classes_are_recognised_by_name_alone() {
        for refused in [
            "CON",
            "con",
            "Nul",
            "NUL.txt",
            "nul .txt",
            "COM1",
            "com9.log",
            "LPT1",
            "lpt9",
            "AUX",
            "PRN.x",
            "CONIN$",
            "CONOUT$",
            "COM\u{b9}",
            "LPT\u{b3}",
            "C:\\dir\\NUL",
            "C:\\dir\\com3.txt",
            "C:NUL",
            "dir/CON/file",
            "\\\\.\\NUL",
            "\\\\.\\pipe\\x",
            "//./pipe/x",
            "\\\\?\\GLOBALROOT\\Device\\Null",
            "\\\\?\\pipe\\x",
            "\\??\\C:\\x",
            "\\\\?\\C:\\dir\\CON",
            "C:\\dir\\file:stream",
            "NUL:",
            "\\\\server\\share\\AUX",
        ] {
            assert!(refuses_device_path(refused), "{refused:?} must be refused");
        }
        for accepted in [
            "C:\\Users\\a b\\known_hosts",
            "known_hosts",
            "CONSOLE",
            "COM0",
            "COM10",
            "LPT0",
            "nulls",
            "aux1",
            "console.txt",
            "C:\\dir\\.ssh\\id_ed25519",
            "\\\\?\\C:\\dir\\known_hosts",
            "\\\\?\\UNC\\server\\share\\known_hosts",
            "\\\\server\\share\\known_hosts",
            "..\\up\\file",
            "./rel/file",
            "dir/file.txt",
            "caf\u{e9}",
            "COMX",
        ] {
            assert!(
                !refuses_device_path(accepted),
                "{accepted:?} must be accepted"
            );
        }
    }
}
