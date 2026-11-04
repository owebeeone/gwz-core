//! The walk's pure judges and type tables (movement-only split of `walk_unix.rs`, step 2.3e2): the Linux file-system
//! types, the macOS mount table and the macOS probe's decisions, judged from injected values on any host.
use super::Kind;

/// Linux `PROC_SUPER_MAGIC`.
pub(crate) const PROC_SUPER_MAGIC: u32 = 0x9FA0;

/// The Linux file-system types a walk refuses (`f_type`, magic.h v7.2 and coreutils' out-of-tree list): every
/// automounter, network and cluster file system, and every FUSE file system, whose subtype whoever mounts it chooses.
const LINUX_REFUSED: [(u32, &str); 25] = [
    (0x0187, "autofs"),
    (0x6969, "nfs"),
    (0x517B, "smbfs"),
    (0xFF53_4D42, "cifs"),
    (0xFE53_4D42, "smb2"),
    (0x5346_414F, "openafs"),
    (0x6B41_4653, "kafs"),
    (0x00C3_6400, "ceph"),
    (0x0102_1997, "9p"),
    (0x7375_7245, "coda"),
    (0x564C, "ncp"),
    (0x7461_636F, "ocfs2"),
    (0x0116_1970, "gfs2"),
    (0x2003_0528, "orangefs"),
    (0x786F_4256, "vboxsf"),
    (0x6573_5546, "fuse"),
    (0x0BD0_0BD0, "lustre"),
    (0x4750_4653, "gpfs"),
    (0x1983_0326, "beegfs"),
    (0xAAD7_AAEA, "panfs"),
    (0x0131_11A8, "ibrix"),
    (0xBEEF_DEAD, "stornext"),
    (0x6163_6673, "acfs"),
    (0x7C7C_6673, "prl_fs"),
    (0xBACB_ACBC, "vmhgfs"),
];

/// The name of a refused Linux file-system type, or `None` for one a socket may sit on.
pub(crate) fn linux_refused_type(f_type: u32) -> Option<&'static str> {
    LINUX_REFUSED
        .iter()
        .find(|(magic, _)| *magic == f_type)
        .map(|(_, name)| *name)
}

/// macOS `MNT_LOCAL`, `MNT_IGNORE_OWNERSHIP` and `MNT_AUTOMOUNTED` (`<sys/mount.h>`), so the table is judged on any host.
pub(crate) const MNT_LOCAL: u32 = 0x0000_1000;
pub(crate) const MNT_IGNORE_OWNERSHIP: u32 = 0x0020_0000;
pub(crate) const MNT_AUTOMOUNTED: u32 = 0x0040_0000;
/// macOS `DIR_MNTSTATUS_TRIGGER` (`<sys/attr.h>`) and the vnode types `VDIR` and `VLNK` (`<sys/vnode.h>`), so the
/// probe's decisions are judged on any host.
pub(crate) const DIR_MNTSTATUS_TRIGGER: u32 = 0x0000_0002;
pub(crate) const VDIR: u32 = 2;
pub(crate) const VLNK: u32 = 5;

/// The macOS file-system type names a walk refuses, and the prefixes macFUSE's `fstypename=NAME` gives. The names are
/// checked as well as the flags because macFUSE's `-o local` sets `MNT_LOCAL`.
const MACOS_REFUSED: [&[u8]; 12] = [
    b"autofs",
    b"nfs",
    b"smbfs",
    b"afpfs",
    b"webdav",
    b"ftp",
    b"acfs",
    b"virtiofs",
    b"fusefs",
    b"osxfusefs",
    b"osxfuse",
    b"macfuse",
];
const MACOS_REFUSED_PREFIXES: [&[u8]; 2] = [b"macfuse_", b"osxfuse_"];

/// Why a macOS mount with `flags` and type name `name` is refused, or `None` for one a socket may sit on.
pub(crate) fn macos_refused_mount(flags: u32, name: &[u8]) -> Option<Kind> {
    if MACOS_REFUSED.contains(&name)
        || MACOS_REFUSED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
    {
        return Some(Kind::FileSystem(String::from_utf8_lossy(name).into_owned()));
    }
    if flags & MNT_LOCAL == 0 {
        return Some(Kind::NotLocal);
    }
    if flags & MNT_AUTOMOUNTED != 0 {
        return Some(Kind::Automounted);
    }
    None
}

/// What macOS's `getattrlistat` showed of one component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MacAttributes {
    pub(crate) fsid: [u8; 8],
    pub(crate) fileid: u64,
    pub(crate) objtype: u32,
    /// `ATTR_DIR_MOUNTSTATUS`, when the call returned it.
    pub(crate) mount_status: Option<u32>,
}

/// What the macOS probe may do next with a component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MacNext {
    /// A link: read it.
    ReadLink,
    /// Neither a link nor a directory: nothing is opened.
    Other,
    /// A directory that is not a trigger: open it.
    Open,
}

/// Design §3 macOS steps 1 to 3, before anything is opened: a trigger is refused, and so is a directory whose mount
/// status was not returned, since any `open` of a trigger mounts it.
pub(crate) fn judge_probe(seen: &MacAttributes) -> Result<MacNext, Kind> {
    if seen
        .mount_status
        .is_some_and(|status| status & DIR_MNTSTATUS_TRIGGER != 0)
    {
        return Err(Kind::Trigger);
    }
    match seen.objtype {
        VLNK => Ok(MacNext::ReadLink),
        VDIR if seen.mount_status.is_none() => Err(Kind::Unreadable {
            call: "getattrlistat",
            errno: None,
        }),
        VDIR => Ok(MacNext::Open),
        _ => Ok(MacNext::Other),
    }
}

/// Design §3 macOS step 3, after the open: the descriptor's file-system and file IDs (`opened`) must be the probe's,
/// and its mount (`flags`, `type_name`) must pass [`macos_refused_mount`].
pub(crate) fn judge_opened(
    seen: &MacAttributes,
    opened: ([u8; 8], u64),
    flags: u32,
    type_name: &[u8],
) -> Result<(), Kind> {
    if opened != (seen.fsid, seen.fileid) {
        return Err(Kind::Changed);
    }
    match macos_refused_mount(flags, type_name) {
        Some(kind) => Err(kind),
        None => Ok(()),
    }
}

/// Linux `S_ISVTX` and `S_IWOTH` (`<sys/stat.h>`), so that [`protected_link`] is judged on any host.
const STICKY: u32 = 0o1000;
const OTHERS_WRITE: u32 = 0o002;

/// Whether Linux, under `fs.protected_symlinks` (1 on most distributions), refuses to follow a link owned by `link_uid`
/// for the follower `follower`, in a directory with mode `dir_mode` owned by `dir_uid` (`may_follow_link`, fs/namei.c):
/// only in a sticky directory that others may write, and only when the link's owner is neither the follower nor the
/// directory's owner. The walk refuses such a link whatever the setting (2.3e round 3, Code N3-3).
pub(crate) fn protected_link(dir_mode: u32, dir_uid: u32, link_uid: u32, follower: u32) -> bool {
    dir_mode & (STICKY | OTHERS_WRITE) == STICKY | OTHERS_WRITE
        && link_uid != follower
        && link_uid != dir_uid
}
