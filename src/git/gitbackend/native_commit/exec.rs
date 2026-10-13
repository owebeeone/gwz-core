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

/// What time it is "now", for an identity given no date. Git reads the clock
/// once per process and uses that date for the author and the committer, so a
/// run takes it from here at most once.
pub(super) type Clock<'a> = &'a mut dyn FnMut() -> ModelResult<git2::Time>;

/// The system clock, with the local offset libgit2 gives a signature.
pub(super) fn system_clock() -> ModelResult<git2::Time> {
    git2::Signature::now("gwz", "gwz@localhost")
        .map(|signature| signature.when())
        .map_err(git_error)
}

/// The signatures for `idents`, all dated by the one `now` the clock gives
/// when any of them has no date of its own.
fn signatures<const N: usize>(
    idents: [&Ident; N],
    clock: Clock<'_>,
) -> ModelResult<[git2::Signature<'static>; N]> {
    let mut now = None;
    let mut made = Vec::with_capacity(N);
    for ident in idents {
        let when = match ident.seconds {
            Some(seconds) => git2::Time::new(seconds, ident.offset_minutes),
            None => match now {
                Some(time) => time,
                None => *now.insert(clock()?),
            },
        };
        made.push(git2::Signature::new(&ident.name, &ident.email, &when).map_err(git_error)?);
    }
    made.try_into()
        .map_err(|_| failed("signatures do not match identities"))
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
    commit_at(repo, plan, &mut system_clock)
}

/// `commit`, with the clock the dates of an identity without one come from.
pub(super) fn commit_at(
    repo: &git2::Repository,
    plan: CommitPlan,
    clock: Clock<'_>,
) -> ModelResult<git2::Oid> {
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

    let [author, committer] = signatures([&plan.author, &plan.committer], clock)?;
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

/// The steps of a tag creation a test may act between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TagStage {
    /// Native has seen there is no such tag, and has taken no lock.
    Checked,
    /// The tag's reference is locked and no tag has been seen under the lock.
    Locked,
}

/// Create the tag `plan` describes.
pub(super) fn tag_create(repo: &git2::Repository, name: &str, plan: TagPlan) -> ModelResult<()> {
    tag_create_observed(repo, name, plan, &mut |_| {})
}

/// `tag_create`, telling `observe` where it is. The reference is created in a
/// transaction that locks it, checks under the lock that it does not exist,
/// then writes it; a tag made by another process after the early check is
/// refused rather than overwritten.
pub(super) fn tag_create_observed(
    repo: &git2::Repository,
    name: &str,
    plan: TagPlan,
    observe: &mut dyn FnMut(TagStage),
) -> ModelResult<()> {
    let reference = reference_name(name);
    if repo.find_reference(&reference).is_ok() {
        return Err(failed(format!("tag '{name}' already exists")));
    }
    observe(TagStage::Checked);
    let target = repo.find_object(plan.target, None).map_err(git_error)?;
    let id = match &plan.annotation {
        None => target.id(),
        Some((text, tagger)) => {
            let [tagger] = signatures([tagger], &mut system_clock)?;
            repo.tag_annotation_create(name, &target, &tagger, text)
                .map_err(git_error)?
        }
    };
    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref(&reference).map_err(git_error)?;
    observe(TagStage::Locked);
    match repo.find_reference(&reference) {
        Ok(_) => return Err(failed(format!("tag '{name}' already exists"))),
        Err(error) if error.code() == git2::ErrorCode::NotFound => {}
        Err(error) => return Err(git_error(error)),
    }
    transaction
        .set_target(&reference, id, None, "")
        .map_err(git_error)?;
    transaction.commit().map_err(git_error)
}

/// Delete the tag `name`.
pub(super) fn tag_delete(repo: &git2::Repository, name: &str) -> ModelResult<()> {
    if repo.find_reference(&reference_name(name)).is_err() {
        return Err(failed(format!("tag '{name}' not found")));
    }
    repo.tag_delete(name).map_err(git_error)
}
