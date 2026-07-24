use super::*;

pub(crate) fn stash_push_flags(options: GitStashPushOptions) -> git2::StashFlags {
    let mut flags = git2::StashFlags::empty();
    if options.preserve_index {
        flags |= git2::StashFlags::KEEP_INDEX;
    }
    if options.include_untracked {
        flags |= git2::StashFlags::INCLUDE_UNTRACKED;
    }
    if options.include_ignored {
        flags |= git2::StashFlags::INCLUDE_UNTRACKED;
        flags |= git2::StashFlags::INCLUDE_IGNORED;
    }
    flags
}

pub(crate) fn stash_restore_options(
    options: GitStashRestoreOptions,
) -> git2::StashApplyOptions<'static> {
    let mut apply_options = git2::StashApplyOptions::new();
    if options.preserve_index {
        apply_options.reinstantiate_index();
    }
    apply_options
}

pub(crate) fn stash_entries(repo: &mut git2::Repository) -> ModelResult<Vec<GitStashEntry>> {
    let mut entries = Vec::new();
    repo.stash_foreach(|index, message, oid| {
        entries.push(GitStashEntry {
            index,
            object_id: oid.to_string(),
            message: message.to_owned(),
        });
        true
    })
    .map_err(git_error)?;
    Ok(entries)
}

pub(crate) fn resolve_stash_index(
    repo: &mut git2::Repository,
    target: &GitStashTarget,
) -> ModelResult<usize> {
    let entries = stash_entries(repo)?;
    if let Some(object_id) = target.object_id.as_deref() {
        let oid = git2::Oid::from_str(object_id).map_err(git_error)?;
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.object_id == oid.to_string())
        {
            return Ok(entry.index);
        }
    }

    if let Some(prefix) = target.gwz_message_prefix.as_deref() {
        if !prefix.starts_with("gwz:") {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "stash message prefix fallback is restricted to gwz: prefixes",
            ));
        }
        if let Some(entry) = entries
            .iter()
            .find(|entry| stash_message_matches_gwz_prefix(&entry.message, prefix))
        {
            return Ok(entry.index);
        }
    }

    Err(ModelError::new(
        ErrorCode::GitCommandFailed,
        "stash entry not found",
    ))
}

pub(crate) fn stash_restore_error(error: git2::Error) -> ModelError {
    match error.code() {
        git2::ErrorCode::Conflict => ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("stash restore conflict: {}", error.message()),
        ),
        _ => git_error(error),
    }
}

pub(crate) fn stash_message_matches_gwz_prefix(message: &str, prefix: &str) -> bool {
    message.starts_with(prefix)
        || message
            .split_once(": ")
            .is_some_and(|(_, suffix)| suffix.starts_with(prefix))
}
