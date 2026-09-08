use std::io;
use std::path::Path;

use crate::filesystem::{FileSystem, RenameMode, make_filesystem};

/// Atomically move `source` to an absent `destination` without replacement.
#[allow(
    dead_code,
    reason = "v1 lifecycle archive is production-disabled until A1"
)]
pub(crate) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    make_filesystem().rename(source, destination, RenameMode::NoReplace)
}

/// Flush directory-entry changes where the platform exposes that operation.
///
/// Windows supplies the persistence barrier through `MOVEFILE_WRITE_THROUGH`;
/// opening a directory as a normal `File` and syncing it fails with access denied.
pub(crate) fn sync_dir(path: &Path) -> io::Result<()> {
    make_filesystem().sync_directory(path)
}
