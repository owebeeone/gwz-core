use std::time::UNIX_EPOCH;

use super::*;

pub(super) fn enforce(root: &Path) -> ModelResult<()> {
    let mut ordinary = Vec::new();
    for path in record_files(&root.join(DONE_DIR))? {
        // Both installed envelopes, for the reason `store::gc::collect` reads
        // them: under the v0-only reader every v1 archive fell to the `Err`
        // arm, which retains forever, so no v1 archive was ever classified.
        let owns_backup_ref = match read_seam_record(&path, RecordLocation::Archived, true) {
            Ok((_, record)) => record
                .participants
                .values()
                .flat_map(|participant| &participant.preservation)
                .chain(
                    record
                        .publication
                        .iter()
                        .flat_map(|publication| &publication.root_preservation),
                )
                .any(|row| row.backup_ref.is_some()),
            Err(_) => match validated_future_cleanup(root, &path) {
                Some(value) => value,
                None => continue, // Unknown/corrupt archives may own evidence: retain them.
            },
        };
        if owns_backup_ref {
            continue;
        }
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        ordinary.push((modified, path));
    }
    ordinary.sort_by(|left, right| right.cmp(left));
    for (_, path) in ordinary.into_iter().skip(ORDINARY_RETENTION) {
        fs::remove_file(path).map_err(io_error)?;
    }
    let done = root.join(DONE_DIR);
    if path_exists(&done)? {
        sync_dir(&done).map_err(io_error)
    } else {
        Ok(())
    }
}

#[cfg(test)]
fn validated_future_cleanup(root: &Path, path: &Path) -> Option<bool> {
    let merge_id = path.file_stem()?.to_str()?;
    let locations =
        super::super::record_wire::acquire_canonical_merge_locations(root, merge_id).ok()?;
    let (_, bytes, _) = locations.archived().exact()?;
    super::super::record_wire::decode_archived(bytes.as_slice(), merge_id)
        .ok()
        .map(|record| !record.cleanup().backup_refs().is_empty())
}

#[cfg(not(test))]
fn validated_future_cleanup(_root: &Path, _path: &Path) -> Option<bool> {
    None
}
