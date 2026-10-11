//! The native commit and tag runs, over libgit2 and a decided plan.

use crate::git::git_error;
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::ident::Ident;
use super::message::reflog_message;
use super::route::{CommitPlan, HeadState, TagPlan};
use super::staging::Staging;

fn failed(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::GitCommandFailed, message)
}

fn signature(ident: &Ident) -> ModelResult<git2::Signature<'static>> {
    match ident.seconds {
        Some(seconds) => git2::Signature::new(
            &ident.name,
            &ident.email,
            &git2::Time::new(seconds, ident.offset_minutes),
        ),
        None => git2::Signature::now(&ident.name, &ident.email),
    }
    .map_err(git_error)
}

/// Lock `reference`, check it still holds `expected`, and move it to `target`
/// with a reflog line written under `who`.
fn move_reference(
    repo: &git2::Repository,
    reference: &str,
    expected: Option<git2::Oid>,
    target: git2::Oid,
    who: &git2::Signature<'_>,
    log: &str,
) -> ModelResult<()> {
    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref(reference).map_err(git_error)?;
    let observed = match repo.find_reference(reference) {
        Ok(found) => found.target(),
        Err(error) if error.code() == git2::ErrorCode::NotFound => None,
        Err(error) => return Err(git_error(error)),
    };
    if observed != expected {
        return Err(failed(format!(
            "'{reference}' changed while the commit was made"
        )));
    }
    transaction
        .set_target(reference, target, Some(who), log)
        .map_err(git_error)?;
    transaction.commit().map_err(git_error)
}

/// Make the commit `plan` describes. Returns the new commit id.
pub(super) fn commit(repo: &git2::Repository, plan: CommitPlan) -> ModelResult<git2::Oid> {
    let mut index = repo.index().map_err(git_error)?;
    let lock = index.path().map(|path| {
        let mut name = path.as_os_str().to_owned();
        name.push(".lock");
        std::path::PathBuf::from(name)
    });
    if let Some(lock) = lock.filter(|path| path.exists()) {
        return Err(failed(format!(
            "Unable to create '{}': File exists.",
            lock.display()
        )));
    }
    for step in &plan.staging {
        match step {
            Staging::Add(path) => index.add_path(path).map_err(git_error)?,
            Staging::Remove(path) => index.remove_path(path).map_err(git_error)?,
        }
    }
    let tree_id = index.write_tree().map_err(git_error)?;
    let tree = repo.find_tree(tree_id).map_err(git_error)?;

    let (reference, tip) = match &plan.head {
        HeadState::Branch { reference, tip } => (reference.as_str(), *tip),
        HeadState::Detached(oid) => ("HEAD", Some(*oid)),
    };
    let parent = tip
        .map(|oid| repo.find_commit(oid))
        .transpose()
        .map_err(git_error)?;
    let unchanged = match &parent {
        Some(parent) => parent.tree_id() == tree_id,
        None => tree.is_empty(),
    };
    if unchanged {
        return Err(failed("nothing to commit"));
    }

    let author = signature(&plan.author)?;
    let committer = signature(&plan.committer)?;
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    let oid = repo
        .commit(None, &author, &committer, &plan.message, &tree, &parents)
        .map_err(git_error)?;
    let kind = if parent.is_none() {
        "commit (initial)"
    } else {
        "commit"
    };
    let log = reflog_message(kind, &plan.message);
    move_reference(repo, reference, tip, oid, &committer, &log)?;
    if !plan.staging.is_empty() {
        index.write().map_err(git_error)?;
    }
    Ok(oid)
}

fn reference_name(name: &str) -> String {
    format!("refs/tags/{name}")
}

/// Create the tag `plan` describes.
pub(super) fn tag_create(repo: &git2::Repository, name: &str, plan: TagPlan) -> ModelResult<()> {
    if repo.find_reference(&reference_name(name)).is_ok() {
        return Err(failed(format!("tag '{name}' already exists")));
    }
    let target = repo.find_object(plan.target, None).map_err(git_error)?;
    match &plan.annotation {
        None => repo.tag_lightweight(name, &target, false),
        Some((text, tagger)) => {
            let tagger = signature(tagger)?;
            repo.tag(name, &target, &tagger, text, false)
        }
    }
    .map(|_| ())
    .map_err(git_error)
}

/// Delete the tag `name`.
pub(super) fn tag_delete(repo: &git2::Repository, name: &str) -> ModelResult<()> {
    if repo.find_reference(&reference_name(name)).is_err() {
        return Err(failed(format!("tag '{name}' not found")));
    }
    repo.tag_delete(name).map_err(git_error)
}
