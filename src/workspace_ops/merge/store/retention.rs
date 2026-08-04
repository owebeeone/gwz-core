use std::time::UNIX_EPOCH;

use super::*;

pub(super) fn enforce(root: &Path) -> ModelResult<()> {
    let mut ordinary = Vec::new();
    for path in record_files(&root.join(DONE_DIR))? {
        let Ok((_, record)) = read_record(&path, RecordLocation::Archived) else {
            continue; // Unknown/corrupt archives may own evidence: fail safe by retaining them.
        };
        if record
            .participants
            .values()
            .any(|participant| !participant.preservation.is_empty())
            || record
                .publication
                .as_ref()
                .is_some_and(|publication| !publication.root_preservation.is_empty())
        {
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
