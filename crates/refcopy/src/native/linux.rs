//! The Linux `FICLONE` wrapper, gated to the targets that have it.

#[cfg(all(
    target_os = "linux",
    not(any(target_arch = "sparc", target_arch = "sparc64"))
))]
pub(crate) mod imp {
    //! `ioctl(destination, FICLONE, source)`.
    //!
    //! FICLONE clones into an *open* destination, so this wrapper creates the
    //! temporary itself and removes it again unless the clone succeeded. The
    //! engine therefore finds either a complete clone or no file at all, and
    //! never a stale tail to append to.

    use std::fs::{self, File, OpenOptions};
    use std::path::Path;

    use gwz_copy_contract::CopyErrorCategory;
    use rustix::fs::ioctl_ficlone;

    use crate::NativeMechanism;
    use crate::native::Outcome;

    pub(crate) const MECHANISM: NativeMechanism = NativeMechanism::LinuxFiclone;

    pub(crate) fn clone_regular_file(source: &File, temporary: &Path) -> Outcome {
        let destination = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary)
        {
            Ok(destination) => destination,
            Err(error) => {
                return Outcome::Failed(
                    CopyErrorCategory::DestinationUnwritable,
                    format!("the temporary could not be created: {error}"),
                );
            }
        };
        let outcome = match ioctl_ficlone(&destination, source) {
            Ok(()) => Outcome::Cloned,
            Err(errno) => crate::native::classify::describe(
                crate::native::classify::ficlone(errno),
                errno,
                "FICLONE",
            ),
        };
        // Close before the engine renames or recreates the name.
        drop(destination);
        if outcome != Outcome::Cloned {
            // Reset the file this call created, so the fallback starts from
            // nothing rather than appending to a partial attempt.
            let _ = fs::remove_file(temporary);
        }
        outcome
    }
}
