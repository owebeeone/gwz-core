use super::repository_support::{branch_ref_name, ensure_no_integration_in_progress};
use super::*;

pub(super) fn classify_merge(
    repo: &git2::Repository,
    target_commit: git2::Oid,
    source_commit: git2::Oid,
) -> ModelResult<GitMergeAnalysisKind> {
    if target_commit == source_commit
        || repo
            .graph_descendant_of(target_commit, source_commit)
            .map_err(git_error)?
    {
        return Ok(GitMergeAnalysisKind::UpToDate);
    }
    if repo
        .graph_descendant_of(source_commit, target_commit)
        .map_err(git_error)?
    {
        return Ok(GitMergeAnalysisKind::FastForward);
    }
    match repo.merge_base(target_commit, source_commit) {
        Ok(_) => Ok(GitMergeAnalysisKind::TrueMerge),
        Err(err) if err.code() == git2::ErrorCode::NotFound => Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "target and source do not share a merge base",
        )),
        Err(err) => Err(git_error(err)),
    }
}

pub(super) fn validate_checked_merge_head(
    repo: &git2::Repository,
    branch: &str,
    expected: git2::Oid,
) -> ModelResult<()> {
    let target = repo
        .find_reference(&branch_ref_name(branch))
        .and_then(|reference| reference.peel_to_commit())
        .map_err(git_error)?
        .id();
    let observed = repo_head(repo)?;
    if target != expected
        || observed.branch.as_deref() != Some(branch)
        || observed.commit.as_deref() != Some(expected.to_string().as_str())
    {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!(
                "target branch '{branch}' changed before merge preparation; expected {expected}"
            ),
        ));
    }
    Ok(())
}

pub(super) fn in_memory_merge_index(
    repo: &git2::Repository,
    target: git2::Oid,
    source: git2::Oid,
) -> ModelResult<git2::Index> {
    let base = repo.merge_base(target, source).map_err(git_error)?;
    let base_tree = repo
        .find_commit(base)
        .and_then(|commit| commit.tree())
        .map_err(git_error)?;
    let target_tree = repo
        .find_commit(target)
        .and_then(|commit| commit.tree())
        .map_err(git_error)?;
    let source_tree = repo
        .find_commit(source)
        .and_then(|commit| commit.tree())
        .map_err(git_error)?;
    repo.merge_trees(&base_tree, &target_tree, &source_tree, None)
        .map_err(git_error)
}

pub(super) fn validate_prepared_merge_upstream_in_repo(
    backend: &impl GitBackend,
    path: &Path,
    repo: &git2::Repository,
    branch: &str,
    expected: git2::Oid,
    source: git2::Oid,
    prepared: &GitPreparedMerge,
) -> ModelResult<GitMergeAnalysisKind> {
    ensure_no_integration_in_progress(repo)?;
    validate_checked_merge_head(repo, branch, expected)?;
    if backend.status(path)?.is_dirty {
        return Err(ModelError::new(
            ErrorCode::DirtyMember,
            "merge requires a clean index and worktree",
        ));
    }
    repo.find_commit(source).map_err(git_error)?;
    let kind = classify_merge(repo, expected, source)?;
    match (kind, prepared) {
        (GitMergeAnalysisKind::UpToDate, GitPreparedMerge::Unchanged) => {}
        (GitMergeAnalysisKind::FastForward, GitPreparedMerge::FastForward) => {}
        (GitMergeAnalysisKind::FastForward, GitPreparedMerge::Commit(prepared_commit)) => {
            validate_prepared_commit(prepared_commit)?;
            let source_tree = repo
                .find_commit(source)
                .and_then(|commit| commit.tree())
                .map_err(git_error)?;
            if source_tree.id().to_string() != prepared_commit.tree_oid {
                return Err(prepared_merge_mismatch(
                    "forced merge-commit tree does not match the source tree",
                ));
            }
        }
        (GitMergeAnalysisKind::TrueMerge, GitPreparedMerge::ExpectedConflict) => {
            if !in_memory_merge_index(repo, expected, source)?.has_conflicts() {
                return Err(prepared_merge_mismatch(
                    "prepared conflict merge is now clean",
                ));
            }
        }
        (GitMergeAnalysisKind::TrueMerge, GitPreparedMerge::Commit(prepared_commit)) => {
            validate_prepared_commit(prepared_commit)?;
            let tree_oid = git2::Oid::from_str(&prepared_commit.tree_oid)
                .map_err(|_| prepared_merge_mismatch("recorded tree object id is malformed"))?;
            let tree = repo
                .find_tree(tree_oid)
                .map_err(|_| prepared_merge_mismatch("recorded tree object is unavailable"))?;
            let merge_index = in_memory_merge_index(repo, expected, source)?;
            if merge_index.has_conflicts() {
                return Err(prepared_merge_mismatch(
                    "prepared clean merge now has conflicts",
                ));
            }
            let diff = repo
                .diff_tree_to_index(Some(&tree), Some(&merge_index), None)
                .map_err(git_error)?;
            if diff.deltas().len() != 0 {
                return Err(prepared_merge_mismatch(
                    "clean merge tree changed after intent persistence",
                ));
            }
        }
        (GitMergeAnalysisKind::UpToDate, _) => {
            return Err(prepared_merge_mismatch("up-to-date result class changed"));
        }
        (GitMergeAnalysisKind::FastForward, _) => {
            return Err(prepared_merge_mismatch("fast-forward result class changed"));
        }
        (GitMergeAnalysisKind::TrueMerge, _) => {
            return Err(prepared_merge_mismatch("true-merge result class changed"));
        }
    }
    Ok(kind)
}

fn validate_prepared_commit(prepared: &GitPreparedCommit) -> ModelResult<()> {
    signature_from_prepared(&prepared.author)?;
    signature_from_prepared(&prepared.committer)?;
    Ok(())
}

pub(super) fn prepared_merge_mismatch(detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("prepared merge intent no longer matches the repository: {detail}"),
    )
}

/// Render one Git path without assuming that Git's byte string is UTF-8.
///
/// Ordinary printable UTF-8 keeps its familiar spelling. Paths that need
/// escaping are quoted so diagnostics remain lossless and unambiguous on every
/// supported host platform.
pub(crate) fn render_git_path(path: &[u8]) -> String {
    if let Ok(path) = std::str::from_utf8(path)
        && path
            .chars()
            .all(|character| !character.is_control() && character != '"' && character != '\\')
    {
        return path.to_owned();
    }

    fn push_hex_byte(output: &mut String, byte: u8) {
        use std::fmt::Write as _;

        write!(output, "\\x{byte:02X}").expect("writing to a String cannot fail");
    }

    fn push_valid(output: &mut String, text: &str) {
        for character in text.chars() {
            match character {
                '"' => output.push_str("\\\""),
                '\\' => output.push_str("\\\\"),
                '\0' => output.push_str("\\0"),
                '\u{0007}' => output.push_str("\\a"),
                '\u{0008}' => output.push_str("\\b"),
                '\t' => output.push_str("\\t"),
                '\n' => output.push_str("\\n"),
                '\u{000B}' => output.push_str("\\v"),
                '\u{000C}' => output.push_str("\\f"),
                '\r' => output.push_str("\\r"),
                character if character.is_control() => {
                    let mut encoded = [0; 4];
                    for byte in character.encode_utf8(&mut encoded).as_bytes() {
                        push_hex_byte(output, *byte);
                    }
                }
                character => output.push(character),
            }
        }
    }

    let mut output = String::from("\"");
    let mut remaining = path;
    while !remaining.is_empty() {
        match std::str::from_utf8(remaining) {
            Ok(valid) => {
                push_valid(&mut output, valid);
                break;
            }
            Err(error) => {
                let valid_length = error.valid_up_to();
                if valid_length != 0 {
                    // `valid_up_to` is guaranteed to end on a UTF-8 boundary.
                    let valid = std::str::from_utf8(&remaining[..valid_length])
                        .expect("validated UTF-8 prefix");
                    push_valid(&mut output, valid);
                }
                let invalid_length = error.error_len().unwrap_or(remaining.len() - valid_length);
                for byte in &remaining[valid_length..valid_length + invalid_length] {
                    push_hex_byte(&mut output, *byte);
                }
                remaining = &remaining[valid_length + invalid_length..];
            }
        }
    }
    output.push('"');
    output
}

/// Conflicted paths in `index`, sorted and de-duplicated. Git may record a
/// distinct path at each ancestor/our/their stage, so every present stage is
/// projected independently.
pub(crate) fn conflict_paths(index: &git2::Index) -> ModelResult<Vec<String>> {
    if !index.has_conflicts() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for conflict in index.conflicts().map_err(git_error)? {
        let conflict = conflict.map_err(git_error)?;
        for entry in [conflict.ancestor, conflict.our, conflict.their]
            .into_iter()
            .flatten()
        {
            paths.push(render_git_path(&entry.path));
        }
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "Git index reports conflicts but exposes no conflict paths",
        ));
    }
    Ok(paths)
}

/// Author/committer for gwz-created merge commits: the repo's configured identity
/// when present, else a stable gwz fallback so an unconfigured repo can still merge.
pub(crate) fn merge_signature(repo: &git2::Repository) -> ModelResult<git2::Signature<'static>> {
    if let Ok(signature) = repo.signature() {
        return Ok(signature);
    }
    git2::Signature::now("gwz", "gwz@localhost").map_err(git_error)
}

pub(super) fn merge_signatures(
    repo: &git2::Repository,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<(git2::Signature<'static>, git2::Signature<'static>)> {
    if let Some(attribution) = attribution {
        attribution.validate()?;
    }
    let author = match attribution.and_then(|value| value.git_author.as_ref()) {
        Some(identity) => signature_from_identity(identity)?,
        None => merge_signature(repo)?,
    };
    let committer = match attribution.and_then(|value| value.git_committer.as_ref()) {
        Some(identity) => signature_from_identity(identity)?,
        None => merge_signature(repo)?,
    };
    Ok((author, committer))
}

pub(super) fn prepared_signature(
    signature: &git2::Signature<'_>,
) -> ModelResult<GitPreparedSignature> {
    let name = std::str::from_utf8(signature.name_bytes()).map_err(|_| {
        ModelError::new(
            ErrorCode::GitCommandFailed,
            "git signature name is not valid UTF-8",
        )
    })?;
    let email = std::str::from_utf8(signature.email_bytes()).map_err(|_| {
        ModelError::new(
            ErrorCode::GitCommandFailed,
            "git signature email is not valid UTF-8",
        )
    })?;
    Ok(GitPreparedSignature {
        name: name.to_owned(),
        email: email.to_owned(),
        time_seconds: signature.when().seconds(),
        timezone_offset_minutes: signature.when().offset_minutes(),
    })
}

pub(super) fn signature_from_prepared(
    signature: &GitPreparedSignature,
) -> ModelResult<git2::Signature<'static>> {
    let identity = crate::model::GitObjectIdentity {
        name: signature.name.clone(),
        email: signature.email.clone(),
        time_ms: None,
        timezone_offset_minutes: Some(i64::from(signature.timezone_offset_minutes)),
    };
    identity.validate().map_err(|error| {
        prepared_merge_mismatch(&format!(
            "frozen Git signature is invalid: {}",
            error.message
        ))
    })?;
    git2::Signature::new(
        &signature.name,
        &signature.email,
        &git2::Time::new(signature.time_seconds, signature.timezone_offset_minutes),
    )
    .map_err(|error| {
        prepared_merge_mismatch(&format!(
            "frozen Git signature is not representable: {error}"
        ))
    })
}

pub(super) fn signature_matches_prepared(
    actual: &git2::Signature<'_>,
    expected: &GitPreparedSignature,
) -> bool {
    actual.name_bytes() == expected.name.as_bytes()
        && actual.email_bytes() == expected.email.as_bytes()
        && actual.when().seconds() == expected.time_seconds
        && actual.when().offset_minutes() == expected.timezone_offset_minutes
}

pub(super) fn signature_from_identity(
    identity: &crate::model::GitObjectIdentity,
) -> ModelResult<git2::Signature<'static>> {
    identity.validate()?;
    if identity.time_ms.is_none() && identity.timezone_offset_minutes.is_none() {
        return git2::Signature::now(&identity.name, &identity.email).map_err(git_error);
    }
    let seconds = match identity.time_ms {
        Some(value) => value.0.div_euclid(1_000),
        None => {
            let elapsed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| ModelError::new(ErrorCode::InternalError, error.to_string()))?;
            i64::try_from(elapsed.as_secs()).map_err(|_| {
                ModelError::new(ErrorCode::InternalError, "system time is out of Git range")
            })?
        }
    };
    let offset = i32::try_from(identity.timezone_offset_minutes.unwrap_or(0)).map_err(|_| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            "git identity timezone offset is out of range",
        )
    })?;
    git2::Signature::new(
        &identity.name,
        &identity.email,
        &git2::Time::new(seconds, offset),
    )
    .map_err(git_error)
}

pub(super) fn same_signature(left: &git2::Signature<'_>, right: &git2::Signature<'_>) -> bool {
    left.name_bytes() == right.name_bytes()
        && left.email_bytes() == right.email_bytes()
        && left.when().seconds() == right.when().seconds()
        && left.when().offset_minutes() == right.when().offset_minutes()
}
