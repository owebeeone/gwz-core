//! The macOS probe, `NoTrigger` and the connect and bind by the parsed address (movement-only split of `walk_unix.rs`,
//! step 2.3e2; the module documentation of [`super`] says what they do).
use super::*;
use std::{
    ffi::{CStr, CString, c_char, c_int},
    os::fd::{AsRawFd, FromRawFd, RawFd},
};

/// The grammar this host applies.
pub(crate) const HOST: Platform = Platform::Macos;

/// A walk with this host's probe.
pub(crate) type HostWalked = Walked<MacDir, ()>;

/// A walked directory, and the flags of the mount it is on.
#[derive(Debug)]
pub(crate) struct MacDir {
    fd: OwnedFd,
    flags: u32,
}

impl AsFd for MacDir {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Sets the bound socket's mode through the walked directory (`fchmodat` with `AT_SYMLINK_NOFOLLOW`), once its
/// entry there is shown to be the socket bound.
pub(super) fn set_mode(bound: &BoundSocket, mode: rustix::fs::Mode) -> Result<(), UseError> {
    let name = OsStr::from_bytes(&bound.name);
    let os = |error: rustix::io::Errno| UseError::Io(error.into());
    if !bound
        .is_bound(&rustix::fs::statat(&bound.dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(os)?)
    {
        return Err(UseError::Io(io::Error::from(io::ErrorKind::NotFound)));
    }
    rustix::fs::chmodat(&bound.dir, name, mode, AtFlags::SYMLINK_NOFOLLOW).map_err(os)
}

fn word(buffer: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes([buffer[at], buffer[at + 1], buffer[at + 2], buffer[at + 3]])
}

fn long(buffer: &[u8], at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&buffer[at..at + 8]);
    u64::from_ne_bytes(bytes)
}

fn name_of(name: &[u8]) -> Result<CString, Kind> {
    CString::new(name).map_err(|_| Kind::Unreadable {
        call: "getattrlistat",
        errno: None,
    })
}

/// The attributes of `name` in `dir`, never following a link or crossing a trigger. The buffer is packed in
/// bit order, invalid attributes included (`FSOPT_PACK_INVAL_ATTRS`): its length, the returned set (20 bytes),
/// `ATTR_CMN_FSID` (8), `ATTR_CMN_OBJTYPE` (4), `ATTR_CMN_FILEID` (8) and `ATTR_DIR_MOUNTSTATUS` (4).
fn attributes(dir: RawFd, name: &CString) -> io::Result<MacAttributes> {
    let mut list = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS
            | libc::ATTR_CMN_FSID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_FILEID,
        volattr: 0,
        dirattr: libc::ATTR_DIR_MOUNTSTATUS,
        fileattr: 0,
        forkattr: 0,
    };
    let mut buffer = [0u8; 64];
    let options = libc::FSOPT_NOFOLLOW | libc::FSOPT_PACK_INVAL_ATTRS;
    // SAFETY: `list` and `buffer` are valid for the call; the kernel writes at most `buffer.len()` bytes.
    let result = unsafe {
        libc::getattrlistat(
            dir,
            name.as_ptr(),
            (&raw mut list).cast(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc::c_ulong::from(options),
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    let common = libc::ATTR_CMN_FSID | libc::ATTR_CMN_OBJTYPE | libc::ATTR_CMN_FILEID;
    if word(&buffer, 4) & common != common {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    let mut fsid = [0u8; 8];
    fsid.copy_from_slice(&buffer[24..32]);
    Ok(MacAttributes {
        fsid,
        objtype: word(&buffer, 32),
        fileid: long(&buffer, 36),
        mount_status: (word(&buffer, 12) & libc::ATTR_DIR_MOUNTSTATUS != 0)
            .then(|| word(&buffer, 44)),
    })
}

/// The file-system and file IDs of an open descriptor (`fgetattrlist`).
fn identity(fd: RawFd) -> io::Result<([u8; 8], u64)> {
    let mut list = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_FSID | libc::ATTR_CMN_FILEID,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut buffer = [0u8; 64];
    // SAFETY: as in `attributes`.
    let result = unsafe {
        libc::fgetattrlist(
            fd,
            (&raw mut list).cast(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc::FSOPT_PACK_INVAL_ATTRS,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut fsid = [0u8; 8];
    fsid.copy_from_slice(&buffer[24..32]);
    Ok((fsid, long(&buffer, 32)))
}

/// The mount `fd` is on: its flags and its type name. Its `f_fsid` is not compared with the probe's file-system ID:
/// on the sealed system volume the two differ (`/System`: `f_fsid` is the data volume's), as `st_dev` does.
fn mount_of(fd: RawFd) -> io::Result<(u32, Vec<u8>)> {
    // SAFETY: `statfs` is plain data, written by the call.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `stat` is valid for writes.
    if unsafe { libc::fstatfs(fd, &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let name: Vec<u8> = stat
        .f_fstypename
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c: &c_char| c as u8)
        .collect();
    Ok((stat.f_flags, name))
}

pub(super) fn open_at(dir: RawFd, name: &CStr, flags: c_int) -> io::Result<OwnedFd> {
    // SAFETY: `name` is NUL-terminated; the descriptor returned is owned by the caller.
    let fd = unsafe { libc::openat(dir, name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and is owned by no one else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Probe for HostProbe {
    type Dir = MacDir;
    type Leaf = ();

    fn root(&mut self) -> Result<MacDir, Kind> {
        let fd = open_at(libc::AT_FDCWD, c"/", libc::O_SEARCH | libc::O_CLOEXEC)
            .map_err(|error| unreadable("open", &error))?;
        let (flags, _) = mount_of(fd.as_raw_fd()).map_err(|error| unreadable("fstatfs", &error))?;
        Ok(MacDir { fd, flags })
    }

    fn look(&mut self, dir: &MacDir, name: &[u8]) -> Result<Seen<MacDir, ()>, Kind> {
        let name = name_of(name)?;
        let seen = match attributes(dir.fd.as_raw_fd(), &name) {
            Ok(seen) => seen,
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
                return Ok(Seen::Missing);
            }
            Err(error) => {
                return Err(unreadable("getattrlistat", &error));
            }
        };
        match judge_probe(&seen)? {
            MacNext::ReadLink => {
                let mut target = vec![0u8; libc::PATH_MAX as usize + 1];
                // SAFETY: `target` is valid for `target.len()` bytes.
                let length = unsafe {
                    libc::readlinkat(
                        dir.fd.as_raw_fd(),
                        name.as_ptr(),
                        target.as_mut_ptr().cast(),
                        target.len(),
                    )
                };
                let Ok(length) = usize::try_from(length) else {
                    return Err(unreadable("readlinkat", &io::Error::last_os_error()));
                };
                if length >= target.len() {
                    return Err(Kind::Unreadable {
                        call: "readlinkat",
                        errno: None,
                    });
                }
                target.truncate(length);
                Ok(Seen::Link(target))
            }
            MacNext::Other => Ok(Seen::Other(())),
            MacNext::Open => {
                let flags = libc::O_SEARCH | libc::O_NOFOLLOW | libc::O_CLOEXEC;
                let fd = match open_at(dir.fd.as_raw_fd(), &name, flags) {
                    Ok(fd) => fd,
                    Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
                        return Ok(Seen::Missing);
                    }
                    Err(error) => {
                        return Err(unreadable("openat", &error));
                    }
                };
                let opened =
                    identity(fd.as_raw_fd()).map_err(|error| unreadable("fgetattrlist", &error))?;
                let (flags, type_name) =
                    mount_of(fd.as_raw_fd()).map_err(|error| unreadable("fstatfs", &error))?;
                judge_opened(&seen, opened, flags, &type_name)?;
                Ok(Seen::Directory(MacDir { fd, flags }))
            }
        }
    }

    fn socket_directory(&mut self, dir: &MacDir) -> Result<(), Kind> {
        if dir.flags & MNT_IGNORE_OWNERSHIP != 0 {
            return Err(Kind::IgnoresOwnership);
        }
        Ok(())
    }
}

impl Walked<MacDir, ()> {
    /// One non-blocking connect to the parsed address, which fits `sun_path` (there is no `connectat`), once
    /// the walk has ended holding the address's final component. Through a [`Walk`], under its [`NoTrigger`].
    pub(super) fn connect(&self) -> Result<UnixStream, UseError> {
        self.socket()?;
        self.connect_by(self.address.clone())
    }

    /// Binds the parsed address, once the walk has judged the socket's own directory and its final name is not a
    /// link. [`Walk::listen`] sets the socket's mode.
    pub(super) fn bind(&self) -> Result<UnixListener, UseError> {
        self.bindable()?;
        self.bind_by(self.address.clone())
    }

    /// The socket file at the name in the walked directory, its link not followed (refused as [`Self::connect`]
    /// is).
    pub(crate) fn socket_stat(&self) -> Result<rustix::fs::Stat, UseError> {
        self.socket()?;
        rustix::fs::statat(
            &self.parent.fd,
            OsStr::from_bytes(&self.name),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|error| UseError::Io(error.into()))
    }

    /// The walked directory and the name looked up in it.
    pub(super) fn into_directory(self) -> (OwnedFd, Vec<u8>) {
        (self.parent.fd, self.name)
    }
}
