//! The Apple `clonefile` wrapper, gated to the targets that have it.

#[cfg(target_vendor = "apple")]
pub(crate) mod imp {
    //! `fclonefileat(source, destination_directory, name, 0)`.
    //!
    //! `clonefile` creates its destination and fails with `EEXIST` if the
    //! name is taken, which suits the engine exactly: it clones into the
    //! sibling temporary name it would have written ordinarily and renames
    //! afterwards, so an interrupted entry never appears under its final
    //! name.
    //!
    //! The clone carries the source's mode, timestamps, ACLs and extended
    //! attributes. The engine still applies the source's permission bits
    //! itself, so a native copy and an ordinary copy are observably the same
    //! copy.

    use std::fs::{self, File};
    use std::path::Path;

    use gwz_copy_contract::CopyErrorCategory;
    use rustix::fs::{CloneFlags, fclonefileat};

    use crate::NativeMechanism;
    use crate::native::Outcome;

    pub(crate) const MECHANISM: NativeMechanism = NativeMechanism::AppleClonefile;

    pub(crate) fn clone_regular_file(source: &File, temporary: &Path) -> Outcome {
        let (Some(parent), Some(name)) = (temporary.parent(), temporary.file_name()) else {
            return Outcome::Failed(
                CopyErrorCategory::DestinationUnwritable,
                format!("{} has no directory to be created in", temporary.display()),
            );
        };
        // The directory handle lives only for this call, so the copier still
        // holds at most one source and one destination-side handle at a time.
        let directory = match File::open(parent) {
            Ok(directory) => directory,
            Err(error) => {
                return Outcome::Failed(
                    CopyErrorCategory::DestinationUnwritable,
                    format!("the destination directory could not be opened: {error}"),
                );
            }
        };
        // No flags: `CLONE_NOFOLLOW` is about a source *path*, and this call
        // takes an already-open source; `CLONE_NOOWNERCOPY` only matters to
        // a superuser.
        match fclonefileat(source, &directory, name, CloneFlags::empty()) {
            Ok(()) => Outcome::Cloned,
            Err(errno) => {
                // A failed clone may still have created the destination.
                // Remove it before the engine falls back — only ever this
                // temporary, never an entry already under its final name.
                let _ = fs::remove_file(temporary);
                crate::native::classify::describe(
                    crate::native::classify::clonefile(errno),
                    errno,
                    "clonefile",
                )
            }
        }
    }
}
