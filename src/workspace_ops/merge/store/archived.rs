use super::*;

pub(super) fn load(root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
    validate_merge_id(merge_id)?;
    let path = done_path(root, merge_id);
    if path_exists(&path)? {
        read_record(&path, RecordLocation::Archived).map(|(_, record)| record)
    } else {
        Err(ModelError::new(
            ErrorCode::OperationNotFound,
            format!("archived merge record '{merge_id}' was not found"),
        ))
    }
}

pub(super) fn archive(root: &Path, merge_id: &str) -> ModelResult<()> {
    validate_merge_id(merge_id)?;
    let source = open_path(root, merge_id);
    let destination = done_path(root, merge_id);
    if !path_exists(&source)? {
        if path_exists(&destination)? {
            read_record(&destination, RecordLocation::Archived)?;
            return super::retention::enforce(root);
        }
        return Err(ModelError::new(
            ErrorCode::OperationNotFound,
            format!("merge record '{merge_id}' was not found"),
        ));
    }
    let (source_raw, record) = read_record(&source, RecordLocation::Open)?;
    ensure_terminal_for_archive(&record)?;
    fs::create_dir_all(root.join(DONE_DIR)).map_err(io_error)?;
    if path_exists(&destination)? {
        let (archived_raw, archived) = read_record(&destination, RecordLocation::Archived)?;
        if archived != record || archived_raw != source_raw {
            return Err(recovery_error(format!(
                "archived merge record '{merge_id}' does not match the open record"
            )));
        }
        fs::remove_file(&source).map_err(io_error)?;
    } else {
        rename_durable(&source, &destination, false).map_err(io_error)?;
    }
    sync_dir(&root.join(MERGE_DIR)).map_err(io_error)?;
    sync_dir(&root.join(DONE_DIR)).map_err(io_error)?;
    let (_, verified) = read_record(&destination, RecordLocation::Archived)?;
    if verified != record {
        return Err(recovery_error(format!(
            "archived merge record '{merge_id}' failed verification"
        )));
    }
    super::retention::enforce(root)
}
