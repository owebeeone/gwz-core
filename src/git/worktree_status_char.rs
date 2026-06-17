



pub(crate) fn worktree_status_char(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::WT_NEW) {
        "?"
    } else if status.contains(git2::Status::WT_MODIFIED) {
        "M"
    } else if status.contains(git2::Status::WT_DELETED) {
        "D"
    } else if status.contains(git2::Status::WT_RENAMED) {
        "R"
    } else if status.contains(git2::Status::WT_TYPECHANGE) {
        "T"
    } else {
        " "
    }
}

