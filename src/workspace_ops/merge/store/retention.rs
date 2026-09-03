use std::time::UNIX_EPOCH;

use super::*;

/// The ordinary retention sweep — run by the id-less `--gc` AND by every
/// v0-lifecycle archive operation (`store::archived`), not by `--gc` alone.
///
/// REACH, disclosed 2026-09-02: now that v1 archives are classified below, a
/// workspace holding more than `ORDINARY_RETENTION` archives sweeps its v1 ones
/// on the next ordinary merge that archives, not only on an explicit
/// `gwz merge --gc`. Policy-consistent — such an archive owns no backup ref, no
/// ref or stash is deleted, and it is the cap v0 has always had — but
/// user-visible, so it is stated here rather than left to be discovered.
pub(super) fn enforce(root: &Path) -> ModelResult<()> {
    let mut ordinary = Vec::new();
    for path in record_files(&root.join(DONE_DIR))? {
        // Both installed envelopes, for the reason `store::gc::collect` reads
        // them: under the v0-only reader every v1 archive fell to the `Err`
        // arm, which retains forever, so no v1 archive was ever classified.
        let owns_backup_ref = match read_archived_record(&path) {
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
        // CAPABILITY-FREE EXCEPTION, §10 row `:275`: GC retention enforcement, same list, same permanent carve (2026-09-02, GwzM5-8R2E-CapabilityFreeAmendment.md §3).
        fs::remove_file(path).map_err(io_error)?;
    }
    let done = root.join(DONE_DIR);
    if path_exists(&done)? {
        sync_dir(&done).map_err(io_error)
    } else {
        Ok(())
    }
}

/// An archive this binary cannot read may own preservation evidence, so it is
/// retained rather than classified. That class is not only malformed YAML: the
/// shared decoder's unknown-field manifest refuses a well-formed archive whose
/// `operation_drift` identity is duplicated, which the projection alone would
/// accept — such an archive is retained here forever, uniformly with
/// `--gc <id>`, which reads through the same decoder (round-2 review
/// [R2-P3-3], 2026-09-02).
///
/// This had a `#[cfg(test)]` twin that classified such an archive by decoding
/// it with `decode_archived` and asking whether its cleanup worklist owned a
/// backup ref. It MASKED this site: under `cargo test` a v1 archive was
/// classified whether or not the read above could see it, so no row could
/// guard the cure. It was TEST-ONLY — no shipped build ever contained it — so
/// deleting it changes no production behaviour and makes test builds execute
/// the arm that ships. Its two consumers, `store::tests`'s retention rows, are
/// served by the read above, which classifies their v1 archives directly.
fn validated_future_cleanup(_root: &Path, _path: &Path) -> Option<bool> {
    None
}
