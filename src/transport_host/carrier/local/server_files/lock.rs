//! The file rule, the exclusive lock and the record's reads and writes (parked server design §4 "Files the host
//! opens" and "One server per address", §11 "One server per address"). The rule is one for every file the server
//! locks: `ws-<key>.lock` and the per-directory `id-<key>.lock` (split doc §4.2 "One server per directory").
//!
//! | Rule | Linux, macOS | Windows |
//! | --- | --- | --- |
//! | The directory | opened once (`O_DIRECTORY`, `O_NOFOLLOW`), checked through its descriptor: a directory this user owns with no group or other bit; every file is opened relative to it | opened once (`FILE_FLAG_BACKUP_SEMANTICS`, `FILE_FLAG_OPEN_REPARSE_POINT`), checked through the handle: a directory, not a reparse point, this user (or the token's default owner) owns it |
//! | The directory still published | its path still names the directory opened (`lstat` of the path against `fstat` of the descriptor), for the link check | its path still opens the directory with the identity read at open |
//! | A link at the file | never followed: `openat` with `O_NOFOLLOW` refuses it | never followed: `FILE_FLAG_OPEN_REPARSE_POINT` opens the link itself, which is refused |
//! | A FIFO, socket or device at the file | never blocks: opened with `O_NONBLOCK` (cleared after), then refused as not a regular file | refused as not a regular file |
//! | An existing file | used only if `fstat` shows a regular file with one link (a hard link is a second name of another file), this user owns it, with no group or other bit; never repaired | used only if it has one link and this user (or the token's default owner) owns it |
//! | A new file | created with mode 0600 (`O_CREAT \| O_EXCL`), and the mode set on it | inherits the directory's ACL |
//! | Truncation | never at open (no `O_TRUNC`); the log is opened append-only; only the lock's own record is rewritten, through the held descriptor | the same; the log is opened with `FILE_APPEND_DATA` only |
//! | The lock | `flock(LOCK_EX \| LOCK_NB)` on the lock file's descriptor | `LockFileEx`, exclusive and failing at once, on the byte at [`RECORD_MAX`], past every record, so readers still read it |
//!
//! An entry that fails is refused ([`FileFault`]) and is never followed, truncated or written.
//!
//! The lock is on the open file description, so `O_CLOEXEC` does not release it at a fork: a child that another
//! thread of the process forks (or `vfork`s, or `posix_spawn`s) while the lock is held has its own copy of the
//! descriptor until it executes, and closing this process's file alone leaves the lock held. The clean exit therefore
//! releases each lock explicitly before it closes the file ([`release`]: the record cleared, then `flock(LOCK_UN)`,
//! which releases the lock for every copy of the description; the precedent is `filesystem/native/handles.rs`), so a
//! child in its fork-to-exec window no longer keeps a lock the server gave up. What remains is the exit that does not
//! run: a crash, or a [`super::Claim`] dropped before it bound, releases by closing only, and a child forked by then
//! holds the lock until it executes or exits (a start in that window is refused as held, and after a crash the record
//! still names the dead server). A process that forks and does not execute must close every inherited descriptor
//! except the one it needs (the fork-only obligation, carried to step 2.3r-2). On Windows the files are opened without
//! inheritance, so no child holds them, and `LockFileEx` locks belong to the handle; [`release`] unlocks there too
//! (`UnlockFileEx`), so that the exit is one sequence on every platform.
use super::super::LocalRefusal;
use super::FilesRefusal;
use super::record::{RECORD_MAX, Record, ServerRecord};
use std::{
    fmt,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// What is wrong with a file the server opens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileFault {
    /// A symbolic link (Unix).
    Link,
    /// A reparse point: a symbolic link, a junction or a mount point (Windows).
    ReparsePoint,
    /// A FIFO, a socket, a device or a directory.
    NotRegular,
    /// A file with more than one link: another name of a file elsewhere, which a write would change (State S-1).
    HardLink,
    OtherOwner,
    GroupOrOtherAccess,
    /// Nothing there (a reader's open; the server's own opens create).
    Missing,
    Os {
        kind: io::ErrorKind,
        code: Option<i32>,
    },
}

impl FileFault {
    fn os(error: &io::Error) -> Self {
        FileFault::Os {
            kind: error.kind(),
            code: error.raw_os_error(),
        }
    }
}

impl fmt::Display for FileFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileFault::Link => f.write_str("it is a symbolic link, which the server never follows"),
            FileFault::ReparsePoint => f.write_str(
                "it is a reparse point (a link, a junction or a mount point), which the server never follows",
            ),
            FileFault::NotRegular => f.write_str("it is not a regular file"),
            FileFault::HardLink => f.write_str(
                "it has more than one name (a hard link), so writing it would change a file elsewhere; remove it if \
                 nothing uses it",
            ),
            FileFault::OtherOwner => f.write_str("another user owns it"),
            FileFault::GroupOrOtherAccess => f.write_str(
                "group or other users have access to it; the server needs mode 0600 and does not change the file",
            ),
            FileFault::Missing => f.write_str("it does not exist"),
            FileFault::Os { kind, code: Some(code) } => write!(f, "it cannot be opened: {kind} (OS error {code})"),
            FileFault::Os { kind, code: None } => write!(f, "it cannot be opened: {kind}"),
        }
    }
}

/// How a file is opened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Use {
    /// The lock file: read and write, created when missing.
    Lock,
    /// A reader of a record: read only, never created.
    Read,
    /// The log: appended to only, created when missing.
    Log,
}

/// Reads the record in `file`: at most [`RECORD_MAX`] bytes from its start.
pub(crate) fn read_record(mut file: &File) -> io::Result<Record> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(RECORD_MAX).read_to_end(&mut bytes)?;
    Ok(Record::parse(&bytes))
}

/// The proof that a record was written (round 2 Code P3-N1 / State N-1): it can be made only by [`write_record`] (its
/// field is private to this module), and the bind step of a start takes it, so a start cannot bind a socket whose record
/// it has not written, and the order is held by the types and not by a call site's care.
#[derive(Debug)]
pub(crate) struct Recorded(());

/// Replaces the record in the held lock file with `record`; the returned [`Recorded`] says it was written.
pub(crate) fn write_record(file: &File, record: &ServerRecord) -> Result<Recorded, FilesRefusal> {
    replace(file, &record.encode().map_err(FilesRefusal::Record)?)?;
    Ok(Recorded(()))
}

/// The clean exit's release of a held lock file: its record cleared, then the lock itself released, then (when the
/// caller drops the file) the file closed. The unlock is explicit because a lock on Unix belongs to the open file
/// description: closing this process's descriptor releases it only when no forked child still holds a copy, and
/// `LOCK_UN` releases it for every copy. A failure of either step is ignored: the file closes next, which releases the
/// lock when nothing else holds a copy, and the record is only a claim.
pub(crate) fn release(file: &File) {
    let _ = clear_record(file);
    let _ = unlock(file);
}

/// Clears the record in the held lock file.
pub(crate) fn clear_record(file: &File) -> Result<(), FilesRefusal> {
    replace(file, &[])
}

fn replace(mut file: &File, bytes: &[u8]) -> Result<(), FilesRefusal> {
    let os = |error: io::Error| FilesRefusal::Os(LocalRefusal::os("write the record", &error));
    file.set_len(0).map_err(os)?;
    file.seek(SeekFrom::Start(0)).map_err(os)?;
    file.write_all(bytes).map_err(os)?;
    file.flush().map_err(os)
}

/// Opens `name` in `dir` under the file rule, mapping a fault to the refusal that names the file.
pub(crate) fn open_named(dir: &FilesDir, name: &str, how: Use) -> Result<File, FilesRefusal> {
    open_file(dir, name, how).map_err(|fault| FilesRefusal::File {
        path: dir.path().join(name),
        fault,
    })
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use super::super::DirectoryFault;
        use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags};
        use rustix::io::Errno;
        use std::os::fd::{AsFd, OwnedFd};

        /// A file's identity: what the link check compares (the 30 s check, split doc :241).
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub(crate) struct FileId {
            dev: libc::dev_t,
            ino: libc::ino_t,
        }

        impl FileId {
            fn of(stat: &rustix::fs::Stat) -> Self {
                FileId { dev: stat.st_dev, ino: stat.st_ino }
            }
        }

        fn euid() -> u32 {
            // SAFETY: `geteuid` has no preconditions and cannot fail.
            unsafe { libc::geteuid() }
        }

        /// The directory the server's files are in, open, so that every file is opened, checked and removed through
        /// one descriptor.
        pub(crate) struct FilesDir {
            fd: OwnedFd,
            path: PathBuf,
        }

        impl FilesDir {
            /// Opens `path` without following a link at its leaf and checks it through the descriptor: a directory this
            /// user owns, with no group or other bit.
            pub(crate) fn open(path: &Path) -> Result<Self, LocalRefusal> {
                let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
                let fd = rustix::fs::open(path, flags, Mode::empty()).map_err(|errno| match errno {
                    Errno::NOENT => LocalRefusal::Directory(DirectoryFault::Missing),
                    Errno::LOOP => LocalRefusal::Directory(DirectoryFault::Link),
                    Errno::NOTDIR => LocalRefusal::Directory(DirectoryFault::NotADirectory),
                    other => LocalRefusal::os("open", &io::Error::from(other)),
                })?;
                let stat = rustix::fs::fstat(&fd).map_err(|errno| LocalRefusal::os("fstat", &io::Error::from(errno)))?;
                let fault = if stat.st_uid != euid() {
                    Some(DirectoryFault::OtherOwner)
                } else if stat.st_mode & 0o077 != 0 {
                    Some(DirectoryFault::GroupOrOtherAccess)
                } else {
                    None
                };
                if let Some(fault) = fault {
                    return Err(LocalRefusal::Directory(fault));
                }
                Ok(Self { fd, path: path.to_path_buf() })
            }

            /// A directory the caller names and has walked (an explicit `--log`'s): opened as a directory, its leaf
            /// link not followed, with no owner or mode rule, which applies to the per-user directory only.
            pub(crate) fn open_given(path: &Path) -> Result<Self, LocalRefusal> {
                let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
                let fd = rustix::fs::open(path, flags, Mode::empty()).map_err(|errno| match errno {
                    Errno::NOENT => LocalRefusal::Directory(DirectoryFault::Missing),
                    Errno::LOOP => LocalRefusal::Directory(DirectoryFault::Link),
                    Errno::NOTDIR => LocalRefusal::Directory(DirectoryFault::NotADirectory),
                    other => LocalRefusal::os("open", &io::Error::from(other)),
                })?;
                Ok(Self { fd, path: path.to_path_buf() })
            }

            pub(crate) fn path(&self) -> &Path {
                &self.path
            }

            /// Whether the directory's path still names the directory opened (its leaf link not followed): `false` once
            /// it was renamed, removed or replaced, when the published address no longer reaches this server.
            pub(crate) fn still_published(&self) -> io::Result<bool> {
                let held = FileId::of(&rustix::fs::fstat(&self.fd)?);
                match rustix::fs::statat(rustix::fs::CWD, &self.path, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(stat) => Ok(FileId::of(&stat) == held),
                    Err(Errno::NOENT | Errno::NOTDIR) => Ok(false),
                    Err(other) => Err(other.into()),
                }
            }

            /// The identity of the entry `name`, its link not followed; `None` when nothing is there.
            pub(crate) fn entry(&self, name: &str) -> io::Result<Option<(FileId, rustix::fs::Stat)>> {
                match rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(stat) => Ok(Some((FileId::of(&stat), stat))),
                    Err(Errno::NOENT) => Ok(None),
                    Err(other) => Err(other.into()),
                }
            }

            /// Removes the entry `name` (not a directory).
            pub(crate) fn remove(&self, name: &str) -> io::Result<()> {
                rustix::fs::unlinkat(&self.fd, name, AtFlags::empty()).map_err(io::Error::from)
            }
        }

        /// Whether the entry `stat` describes is a socket this user owns with no group or other bit (a link is not).
        pub(crate) fn is_own_socket(stat: &rustix::fs::Stat) -> bool {
            FileType::from_raw_mode(stat.st_mode) == FileType::Socket
                && stat.st_uid == euid()
                && stat.st_mode & 0o077 == 0
        }

        /// The identity of an open file.
        pub(crate) fn file_id(file: &File) -> io::Result<FileId> {
            Ok(FileId::of(&rustix::fs::fstat(file)?))
        }

        fn classify(errno: Errno) -> FileFault {
            match errno {
                Errno::LOOP => FileFault::Link,
                // A FIFO with no reader (for writing), a socket: never opened, never waited for.
                Errno::NXIO | Errno::OPNOTSUPP => FileFault::NotRegular,
                Errno::NOENT => FileFault::Missing,
                other => FileFault::os(&io::Error::from(other)),
            }
        }

        /// Opens `name` in `dir` under the file rule (module table).
        pub(crate) fn open_file(dir: &FilesDir, name: &str, how: Use) -> Result<File, FileFault> {
            let access = match how {
                Use::Lock => OFlags::RDWR,
                Use::Read => OFlags::RDONLY,
                Use::Log => OFlags::WRONLY | OFlags::APPEND,
            };
            let base = access | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
            let created = match how {
                Use::Read => None,
                Use::Lock | Use::Log => {
                    match rustix::fs::openat(&dir.fd, name, base | OFlags::CREATE | OFlags::EXCL, Mode::from_raw_mode(0o600)) {
                        Ok(fd) => Some(fd),
                        Err(Errno::EXIST) => None,
                        Err(other) => return Err(classify(other)),
                    }
                }
            };
            let new = created.is_some();
            let fd = match created {
                Some(fd) => fd,
                None => rustix::fs::openat(&dir.fd, name, base, Mode::empty()).map_err(classify)?,
            };
            let stat = rustix::fs::fstat(&fd).map_err(classify)?;
            if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
                return Err(FileFault::NotRegular);
            }
            if stat.st_nlink != 1 {
                return Err(FileFault::HardLink);
            }
            if stat.st_uid != euid() {
                return Err(FileFault::OtherOwner);
            }
            if new {
                rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o600)).map_err(classify)?;
            } else if stat.st_mode & 0o077 != 0 {
                return Err(FileFault::GroupOrOtherAccess);
            }
            let flags = rustix::fs::fcntl_getfl(&fd).map_err(classify)?;
            rustix::fs::fcntl_setfl(&fd, flags - OFlags::NONBLOCK).map_err(classify)?;
            Ok(File::from(fd))
        }

        /// Takes the exclusive lock on the lock file without waiting: `false` when another holder has it.
        pub(crate) fn try_lock(file: &File) -> io::Result<bool> {
            match rustix::fs::flock(file.as_fd(), FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => Ok(true),
                Err(Errno::WOULDBLOCK) => Ok(false),
                Err(other) => Err(other.into()),
            }
        }

        /// Releases the lock on the lock file for every descriptor of its open file description (`LOCK_UN`), not only
        /// this one.
        fn unlock(file: &File) -> io::Result<()> {
            rustix::fs::flock(file.as_fd(), FlockOperation::Unlock).map_err(io::Error::from)
        }
    } else if #[cfg(windows)] {
        use std::os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        };
        use windows_sys::Win32::{
            Foundation::{ERROR_LOCK_VIOLATION, HANDLE, LocalFree},
            Security::{
                Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
                EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER,
                TokenOwner, TokenUser,
            },
            Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, FILE_APPEND_DATA, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
                FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, LOCKFILE_EXCLUSIVE_LOCK,
                LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, READ_CONTROL, SYNCHRONIZE, UnlockFileEx,
            },
            System::{
                IO::OVERLAPPED,
                Threading::{GetCurrentProcess, OpenProcessToken},
            },
        };

        use super::super::DirectoryFault;
        use crate::workspace_identity::{WorkspaceIdentity, handle_identity};

        /// A file's identity: the 64-bit volume serial number and the 128-bit file ID (`FileIdInfo`, unique on ReFS
        /// as the 64-bit index is not).
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub(crate) struct FileId(WorkspaceIdentity);

        fn information(handle: HANDLE) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
            // SAFETY: plain data, written by the call below.
            let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
            // SAFETY: a live handle and a writable structure.
            if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(info)
        }

        /// The identity of an open file.
        pub(crate) fn file_id(file: &File) -> io::Result<FileId> {
            handle_identity(file.as_raw_handle()).map(FileId)
        }

        /// Opens the directory at `path` itself (a reparse point there is opened, not followed), for its attributes,
        /// its owner and its identity.
        fn open_directory(path: &Path) -> io::Result<File> {
            std::fs::OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
        }

        /// The directory the server's files are in, opened once and checked through its handle; the files in it are
        /// opened by path (Windows has no `openat`), and the link check compares the path with the identity read here.
        pub(crate) struct FilesDir {
            path: PathBuf,
            handle: File,
            id: FileId,
        }

        impl FilesDir {
            /// The per-user directory: a directory, not a reparse point, owned by this user or the token's default
            /// owner (the Unix arm's owner rule; the access list of server design §11 is the per-user directory's
            /// creator's to set, recorded in the 2.3f round-1 RemPlan).
            pub(crate) fn open(path: &Path) -> Result<Self, LocalRefusal> {
                let dir = Self::open_given(path)?;
                let owned = owned_by_caller(dir.handle.as_raw_handle())
                    .map_err(|error| LocalRefusal::os("GetSecurityInfo", &error))?;
                if !owned {
                    return Err(LocalRefusal::Directory(DirectoryFault::OtherOwner));
                }
                Ok(dir)
            }

            /// A directory the caller names and has walked (an explicit `--log`'s): a directory and not a reparse
            /// point, with no owner rule, which applies to the per-user directory only, as on Unix.
            pub(crate) fn open_given(path: &Path) -> Result<Self, LocalRefusal> {
                let handle = open_directory(path).map_err(|error| match error.kind() {
                    io::ErrorKind::NotFound => LocalRefusal::Directory(DirectoryFault::Missing),
                    _ => LocalRefusal::os("CreateFileW", &error),
                })?;
                let info = information(handle.as_raw_handle())
                    .map_err(|error| LocalRefusal::os("GetFileInformationByHandle", &error))?;
                if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err(LocalRefusal::Directory(DirectoryFault::Link));
                }
                if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
                    return Err(LocalRefusal::Directory(DirectoryFault::NotADirectory));
                }
                let id = file_id(&handle).map_err(|error| LocalRefusal::os("GetFileInformationByHandleEx", &error))?;
                Ok(Self { path: path.to_path_buf(), handle, id })
            }

            pub(crate) fn path(&self) -> &Path {
                &self.path
            }

            /// Whether the directory's path still opens the directory opened: `false` once it was renamed, removed or
            /// replaced, when the published address no longer reaches this server.
            pub(crate) fn still_published(&self) -> io::Result<bool> {
                match open_directory(&self.path) {
                    Ok(now) => Ok(file_id(&now)? == self.id),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                    Err(error) => Err(error),
                }
            }
        }

        /// A buffer for `GetTokenInformation` of `class` on this process's token.
        fn token_information(class: i32) -> io::Result<Vec<u64>> {
            let mut token: HANDLE = std::ptr::null_mut();
            // SAFETY: the pseudo handle of this process; `token` is written on success.
            if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the token handle was just opened and is closed when this guard drops.
            let _token = unsafe { OwnedHandle::from_raw_handle(token) };
            let mut buffer = vec![0u64; 128];
            let mut length = 0u32;
            // SAFETY: the buffer is writable for its stated length in bytes.
            let ok = unsafe {
                GetTokenInformation(token, class, buffer.as_mut_ptr().cast(), (buffer.len() * 8) as u32, &mut length)
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(buffer)
        }

        /// Whether the owner of the open file `handle` is this process's user, or its token's default owner (the
        /// owner an elevated token gives new files).
        fn owned_by_caller(handle: HANDLE) -> io::Result<bool> {
            let mut owner: *mut core::ffi::c_void = std::ptr::null_mut();
            let mut descriptor: *mut core::ffi::c_void = std::ptr::null_mut();
            // SAFETY: a live handle opened with READ_CONTROL; the call allocates `descriptor`, freed below.
            let status = unsafe {
                GetSecurityInfo(handle, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &mut owner, std::ptr::null_mut(),
                    std::ptr::null_mut(), std::ptr::null_mut(), &mut descriptor)
            };
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            let user = token_information(TokenUser);
            let default_owner = token_information(TokenOwner);
            // SAFETY: each buffer holds the structure its class names; the SIDs they point at live in the buffers.
            let same = |buffer: &io::Result<Vec<u64>>, sid: fn(&[u64]) -> *mut core::ffi::c_void| {
                buffer.as_ref().is_ok_and(|buffer| unsafe { EqualSid(owner, sid(buffer)) } != 0)
            };
            let owned = same(&user, |buffer| unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid })
                || same(&default_owner, |buffer| unsafe { (*buffer.as_ptr().cast::<TOKEN_OWNER>()).Owner });
            // SAFETY: allocated by `GetSecurityInfo` above; `owner` points into it and is not used after.
            unsafe { LocalFree(descriptor) };
            user.map(|_| owned)
        }

        fn classify_failed_open(path: &Path, error: &io::Error) -> FileFault {
            match std::fs::symlink_metadata(path) {
                Err(missing) if missing.kind() == io::ErrorKind::NotFound => FileFault::Missing,
                Ok(meta) if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 => FileFault::ReparsePoint,
                Ok(meta) if meta.is_dir() => FileFault::NotRegular,
                _ => FileFault::os(error),
            }
        }

        /// Opens `name` in `dir` under the file rule (module table).
        pub(crate) fn open_file(dir: &FilesDir, name: &str, how: Use) -> Result<File, FileFault> {
            let path = dir.path.join(name);
            let mut options = std::fs::OpenOptions::new();
            options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
            match how {
                Use::Lock => options.read(true).write(true).create(true),
                Use::Read => options.read(true),
                Use::Log => options
                    .append(true)
                    .create(true)
                    .access_mode(FILE_APPEND_DATA | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE),
            };
            let file = options.open(&path).map_err(|error| classify_failed_open(&path, &error))?;
            let info = information(file.as_raw_handle()).map_err(|error| FileFault::os(&error))?;
            if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(FileFault::ReparsePoint);
            }
            if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                return Err(FileFault::NotRegular);
            }
            if info.nNumberOfLinks != 1 {
                return Err(FileFault::HardLink);
            }
            if !owned_by_caller(file.as_raw_handle()).map_err(|error| FileFault::os(&error))? {
                return Err(FileFault::OtherOwner);
            }
            Ok(file)
        }

        /// Takes the exclusive lock without waiting, on the byte at [`RECORD_MAX`]: `false` when another holder has it.
        pub(crate) fn try_lock(file: &File) -> io::Result<bool> {
            // SAFETY: plain data; the offset is the locked byte's.
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            overlapped.Anonymous.Anonymous.Offset = RECORD_MAX as u32;
            // SAFETY: a live, synchronous handle; the call completes before it returns.
            let ok = unsafe {
                LockFileEx(file.as_raw_handle(), LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY, 0, 1, 0, &mut overlapped)
            };
            if ok != 0 {
                return Ok(true);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
                return Ok(false);
            }
            Err(error)
        }

        /// Releases the lock taken by [`try_lock`], on the same byte.
        fn unlock(file: &File) -> io::Result<()> {
            // SAFETY: plain data; the offset is the locked byte's.
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            overlapped.Anonymous.Anonymous.Offset = RECORD_MAX as u32;
            // SAFETY: a live handle; the call completes before it returns.
            if unsafe { UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut overlapped) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
}
