use super::super::directory_mutation::sync_directory_edge;
use crate::checked_artifact::capability::CheckedFsError;
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::{OwnershipMarkerV1, managed_marker_name};
use crate::filesystem::FsDirectory as Dir;
use crate::filesystem::FsKind;
use std::io::{Seek, SeekFrom, Write};

use super::*;

/// The staged component's ownership marker, written once or rewritten in place.
///
/// Rewriting is the *scratch* case only: the marker leaf lives inside this
/// action's own deterministic staging row, which carries no authority until the
/// sealed primitive publishes it, and the bytes are re-derived from the same
/// intent every drive derives. The file is opened `create_new` when absent and
/// existing-only when present, so this never creates a marker at an unexpected
/// name and never adopts a symlink or a non-file in its place.
pub(crate) fn write_or_rewrite_marker(
    staged: &Dir,
    marker: &OwnershipMarkerV1,
) -> Result<(), CheckedFsError> {
    let name = os_name(&managed_marker_name());
    let create_new = match staged.entry_metadata(&name) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => true,
        Err(source) => {
            return Err(CheckedFsError::io(
                "observe managed ownership marker",
                source,
            ));
        }
        Ok(metadata) if metadata.kind == FsKind::File && metadata.kind != FsKind::Symlink => false,
        Ok(_) => {
            return Err(managed_error(
                "staged ownership marker is not a canonical regular file",
            ));
        }
    };
    let options = super::super::directory_mutation::durable_write_options(create_new);
    let mut file = staged
        .open_file(&name, &options)
        .map_err(|source| CheckedFsError::io("open managed ownership marker", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::ManagedBootstrapOwnershipMarkerCreate,
    );
    if !create_new {
        file.set_len(0)
            .map_err(|source| CheckedFsError::io("truncate managed ownership marker", source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind managed ownership marker", source))?;
    }
    file.write_all(&marker.encode_canonical())
        .map_err(|source| CheckedFsError::io("write managed ownership marker", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::ManagedBootstrapOwnershipMarkerWrite,
    );
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush managed ownership marker", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::ManagedBootstrapOwnershipMarkerFlush,
    );
    drop(file);
    sync_directory_edge(staged, "flush managed staging interior")?;
    // The first of `staging_directory_flush`'s two boundaries; the second is the
    // managed parent's flush in `stage_component`, which only a creating drive
    // reaches. See the note there.
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::ManagedBootstrapStagingDirectoryFlush,
    );
    Ok(())
}
