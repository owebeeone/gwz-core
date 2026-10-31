//! The Linux probe: `/proc/<pid>/status` (every thread's) and the process's namespace links, read after the hold was
//! taken and before it is confirmed. The hold is a `pidfd` the kernel recorded for the connection, so a process that
//! went or was replaced meanwhile is refused by the confirmation, and the ID read here names the held process for as
//! long as the confirmation holds.
use super::super::unix::Hold;
use super::{PeerRefusal, SandboxFindings};
use std::{
    io,
    os::{fd::RawFd, unix::fs::MetadataExt},
    path::Path,
};

/// The peer held by `hold`. The socket is not read: the hold alone names the process.
pub(crate) fn of_peer(hold: &Hold, _fd: RawFd) -> Result<SandboxFindings, PeerRefusal> {
    probe_dir(Path::new(&format!("/proc/{}", hold.pid())))
}

/// This process.
pub(crate) fn of_self() -> Result<SandboxFindings, PeerRefusal> {
    probe_dir(Path::new("/proc/self"))
}

/// The `Seccomp:` line of a `/proc/<pid>/status` text. Not `Seccomp_filters:`.
fn seccomp_mode(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Seccomp:"))?
        .trim()
        .parse()
        .ok()
}

/// The inode `stat` reports for a namespace link (which follows it to the namespace's own file).
fn inode(link: &Path, what: &'static str) -> Result<u64, PeerRefusal> {
    std::fs::metadata(link)
        .map(|meta| meta.ino())
        .map_err(|_| PeerRefusal::Unreadable(what))
}

/// Whether `error` says the file is gone because its thread is.
fn vanished(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

/// The highest `Seccomp:` mode among the process's threads: a filter is per thread, and the process's own `status` shows
/// the first thread's. A thread that ended between the listing and the read has no filter left to find; any other
/// thread that cannot be read refuses.
fn thread_seccomp(tasks: &Path) -> Result<u32, PeerRefusal> {
    let unreadable = PeerRefusal::Unreadable("the process's threads");
    let mut highest = 0;
    for task in std::fs::read_dir(tasks).map_err(|_| unreadable.clone())? {
        let task = task.map_err(|_| unreadable.clone())?;
        match std::fs::read_to_string(task.path().join("status")) {
            Ok(status) => {
                let mode = seccomp_mode(&status)
                    .ok_or(PeerRefusal::Unreadable("a thread's seccomp mode"))?;
                highest = highest.max(mode);
            }
            Err(error) if vanished(&error) => {}
            Err(_) => return Err(unreadable),
        }
    }
    Ok(highest)
}

/// The findings of the process whose `/proc` directory is `dir`. Any file that cannot be read, and a status without a
/// seccomp mode (a kernel built without seccomp), refuses.
pub(crate) fn probe_dir(dir: &Path) -> Result<SandboxFindings, PeerRefusal> {
    let status = std::fs::read_to_string(dir.join("status"))
        .map_err(|_| PeerRefusal::Unreadable("the process status"))?;
    let leader =
        seccomp_mode(&status).ok_or(PeerRefusal::Unreadable("the process's seccomp mode"))?;
    Ok(SandboxFindings {
        seccomp: leader.max(thread_seccomp(&dir.join("task"))?),
        user_namespace: Some(inode(&dir.join("ns/user"), "the process's user namespace")?),
        mount_namespace: Some(inode(&dir.join("ns/mnt"), "the process's mount namespace")?),
        profile: false,
    })
}
