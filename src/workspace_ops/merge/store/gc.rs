use super::*;

pub(super) fn collect(root: &Path, merge_id: Option<&str>) -> ModelResult<()> {
    let Some(merge_id) = merge_id else {
        return super::retention::enforce(root);
    };
    validate_merge_id(merge_id)?;
    let path = done_path(root, merge_id);
    if !path_exists(&path)? {
        return Err(ModelError::new(
            ErrorCode::OperationNotFound,
            format!("archived merge record '{merge_id}' was not found"),
        ));
    }
    // The archive's last read before unlink, over BOTH installed envelopes:
    // the v0-only seam reader left every v1 archive unreadable here, so this
    // gate could never let one go. Its every refusal is kept.
    read_seam_record(&path, RecordLocation::Archived, true)?;
    fs::remove_file(&path).map_err(io_error)?;
    sync_dir(&root.join(DONE_DIR)).map_err(io_error)
}
