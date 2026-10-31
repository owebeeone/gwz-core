//! The Linux probe: `/proc/<pid>/status` and the process's namespace links, read after the hold was taken and before it
//! is confirmed, so that a process that went or was replaced meanwhile is refused by the confirmation.
use super::super::unix::Hold;
use super::{PeerRefusal, SandboxFindings};
use std::{os::unix::fs::MetadataExt, path::Path};

/// The peer held by `hold`.
pub(crate) fn of_peer(hold: &Hold) -> Result<SandboxFindings, PeerRefusal> {
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

/// The findings of the process whose `/proc` directory is `dir`. Any file that cannot be read, and a status without a
/// seccomp mode (a kernel built without seccomp), refuses.
pub(crate) fn probe_dir(dir: &Path) -> Result<SandboxFindings, PeerRefusal> {
    let status = std::fs::read_to_string(dir.join("status"))
        .map_err(|_| PeerRefusal::Unreadable("the peer's process status"))?;
    Ok(SandboxFindings {
        seccomp: seccomp_mode(&status).ok_or(PeerRefusal::Unreadable("the peer's seccomp mode"))?,
        user_namespace: Some(inode(&dir.join("ns/user"), "the peer's user namespace")?),
        mount_namespace: Some(inode(&dir.join("ns/mnt"), "the peer's mount namespace")?),
        profile: false,
    })
}
