



pub(crate) fn index_status_char(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::INDEX_NEW) {
        "A"
    } else if status.contains(git2::Status::INDEX_MODIFIED) {
        "M"
    } else if status.contains(git2::Status::INDEX_DELETED) {
        "D"
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        "R"
    } else if status.contains(git2::Status::INDEX_TYPECHANGE) {
        "T"
    } else {
        " "
    }
}

