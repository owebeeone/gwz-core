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
    let (_, record) = read_record(&path)?;
    ensure_terminal_for_archive(&record)?;
    fs::remove_file(&path).map_err(io_error)?;
    sync_dir(&root.join(DONE_DIR))
}
