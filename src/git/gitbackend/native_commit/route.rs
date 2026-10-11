//! The route rule: native libgit2, or the Git CLI.
//!
//! Fail safe: native runs only when nothing it cannot reproduce exactly is in
//! play. Every refusal names its reason so a test (and an operator) can see
//! why the CLI was chosen. The checks read; none of them changes the
//! repository.

use std::ffi::OsStr;
use std::path::Path;

use super::config_view::{ConfigView, Flag};
use super::environment::GitEnvironment;
use super::hooks::{self, COMMIT_HOOKS, TAG_HOOKS};
use super::ident::{self, Ident, IdentRefusal, Role};
use super::message::{self, Cleanup};
use super::staging::{Staging, index_check, staging_plan};
use super::system_config;

/// Why the Git CLI is used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CliReason {
    /// The backend was told to use the CLI only.
    Forced,
    /// An environment variable native does not honour.
    Environment(String),
    /// The repository did not open natively.
    CannotOpen,
    Bare,
    ObjectFormat,
    /// Configuration native cannot read or reproduce.
    Config(&'static str),
    /// A system configuration file native does not read, or cannot read.
    SystemConfig(&'static str),
    /// A hook that would fire, by name.
    Hook(String),
    Signing(&'static str),
    /// A merge, cherry-pick, revert, rebase or bisect state file.
    InProgress(&'static str),
    IndexState(&'static str),
    Identity(IdentRefusal),
    Message(&'static str),
    /// `-a` with something native does not stage the way Git does.
    AllCommit(&'static str),
    TagName,
    HeadShape,
}

/// What HEAD is, read once for a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum HeadState {
    /// On a branch; `tip` is `None` while the branch is unborn.
    Branch {
        reference: String,
        tip: Option<git2::Oid>,
    },
    Detached(git2::Oid),
}

/// Everything a native commit needs, decided before anything is written.
pub(super) struct CommitPlan {
    pub(super) message: String,
    pub(super) author: Ident,
    pub(super) committer: Ident,
    pub(super) head: HeadState,
    pub(super) staging: Vec<Staging>,
}

/// Everything a native tag creation needs.
pub(super) struct TagPlan {
    pub(super) target: git2::Oid,
    pub(super) annotation: Option<(String, Ident)>,
}

/// Open the repository the way native uses it: no parent search, and the
/// configuration locations Git's own variables name.
pub(super) fn open(path: &Path) -> Result<git2::Repository, CliReason> {
    git2::Repository::open_ext(
        path,
        git2::RepositoryOpenFlags::NO_SEARCH | git2::RepositoryOpenFlags::FROM_ENV,
        &[] as &[&OsStr],
    )
    .map_err(|_| CliReason::CannotOpen)
}

fn exists(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// libgit2 and Git for Windows can read drive letters and separators in a
        /// `gitdir:` pattern differently, so a Windows includeIf is Git's to read.
        const INCLUDE_IF_NATIVE: bool = false;
    } else {
        const INCLUDE_IF_NATIVE: bool = true;
    }
}

/// Checks every operation shares; returns the configuration view.
fn common(env: &GitEnvironment, repo: &git2::Repository) -> Result<ConfigView, CliReason> {
    if let Some(name) = env.unsupported() {
        return Err(CliReason::Environment(name.to_owned()));
    }
    if env.contains("GIT_CONFIG_GLOBAL") && git2::Config::find_xdg().is_ok() {
        return Err(CliReason::Environment(
            "GIT_CONFIG_GLOBAL beside an XDG configuration".to_owned(),
        ));
    }
    system_config::check(
        env,
        &system_config::candidates(env),
        &system_config::covered(env),
    )?;
    if repo.is_bare() || repo.workdir().is_none() {
        return Err(CliReason::Bare);
    }
    if repo.object_format() != git2::ObjectFormat::Sha1 {
        return Err(CliReason::ObjectFormat);
    }
    let config = ConfigView::new(repo).map_err(|_| CliReason::Config("unreadable"))?;
    for (key, what) in [
        ("extensions.worktreeConfig", "per-worktree configuration"),
        ("core.splitIndex", "split index"),
        ("index.sparse", "sparse index"),
        ("core.sparseCheckout", "sparse checkout"),
    ] {
        if config.flag(key).may_be_true() {
            return Err(CliReason::Config(what));
        }
    }
    if config.is_set("core.sharedRepository") {
        return Err(CliReason::Config("core.sharedRepository"));
    }
    if matches!(
        config.flag("core.logAllRefUpdates"),
        Flag::False | Flag::Unknown
    ) {
        return Err(CliReason::Config(
            "core.logAllRefUpdates other than default",
        ));
    }
    let conditions = config
        .names_matching("^includeif\\.")
        .map_err(|_| CliReason::Config("includeIf unreadable"))?;
    if !conditions.is_empty() && !INCLUDE_IF_NATIVE {
        return Err(CliReason::Config("includeIf on this platform"));
    }
    for name in conditions {
        let condition = name.trim_start_matches("includeif.");
        if !(condition.starts_with("gitdir:") || condition.starts_with("gitdir/i:")) {
            return Err(CliReason::Config(
                "includeIf condition native does not read",
            ));
        }
    }
    Ok(config)
}

fn hook_check(
    repo: &git2::Repository,
    config: &ConfigView,
    names: &[&str],
) -> Result<(), CliReason> {
    let worktree = repo.workdir().ok_or(CliReason::Bare)?;
    let configured = config
        .text("core.hooksPath")
        .map_err(|_| CliReason::Config("core.hooksPath unreadable"))?;
    let directories = hooks::hook_directories(
        configured.as_deref(),
        config.path("core.hooksPath"),
        worktree,
        repo.path(),
        repo.commondir(),
    )
    .map_err(|_| CliReason::Config("core.hooksPath form"))?;
    match hooks::firing_hook(&directories, names) {
        Some(name) => Err(CliReason::Hook(name)),
        None => Ok(()),
    }
}

fn head_state(repo: &git2::Repository) -> Result<HeadState, CliReason> {
    let head = repo
        .find_reference("HEAD")
        .map_err(|_| CliReason::HeadShape)?;
    if let Some(target) = head.symbolic_target().map_err(|_| CliReason::HeadShape)? {
        if !target.starts_with("refs/heads/") {
            return Err(CliReason::HeadShape);
        }
        let tip = match repo.find_reference(target) {
            Ok(reference) => {
                let oid = reference.target().ok_or(CliReason::HeadShape)?;
                repo.find_commit(oid).map_err(|_| CliReason::HeadShape)?;
                Some(oid)
            }
            Err(error) if error.code() == git2::ErrorCode::NotFound => None,
            Err(_) => return Err(CliReason::HeadShape),
        };
        return Ok(HeadState::Branch {
            reference: target.to_owned(),
            tip,
        });
    }
    let oid = head.target().ok_or(CliReason::HeadShape)?;
    repo.find_commit(oid).map_err(|_| CliReason::HeadShape)?;
    Ok(HeadState::Detached(oid))
}

fn in_progress(repo: &git2::Repository) -> Result<(), CliReason> {
    if repo.state() != git2::RepositoryState::Clean {
        return Err(CliReason::InProgress("repository state"));
    }
    let git_dir = repo.path();
    for name in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
        "BISECT_LOG",
        "MERGE_MSG",
        "SQUASH_MSG",
    ] {
        if exists(&git_dir.join(name)) {
            return Err(CliReason::InProgress(name));
        }
    }
    if exists(&git_dir.join("info").join("sparse-checkout"))
        || exists(&repo.commondir().join("info").join("sparse-checkout"))
    {
        return Err(CliReason::Config("info/sparse-checkout"));
    }
    for directory in [git_dir, repo.commondir()] {
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("sharedindex.")
                {
                    return Err(CliReason::Config("split index files"));
                }
            }
        }
    }
    Ok(())
}

fn identity(env: &GitEnvironment, config: &ConfigView, role: Role) -> Result<Ident, CliReason> {
    ident::resolve(env, config, role).map_err(CliReason::Identity)
}

fn comment_style_check(config: &ConfigView) -> Result<(), CliReason> {
    if config.is_set("core.commentChar") || config.is_set("core.commentString") {
        return Err(CliReason::Message("custom comment character"));
    }
    Ok(())
}

/// Decide a commit. `Ok` carries everything the native run needs.
pub(super) fn plan_commit(
    env: &GitEnvironment,
    repo: &git2::Repository,
    raw_message: &str,
    all: bool,
) -> Result<CommitPlan, CliReason> {
    let config = common(env, repo)?;
    if config.flag("commit.gpgSign").may_be_true() {
        return Err(CliReason::Signing("commit.gpgSign"));
    }
    hook_check(repo, &config, COMMIT_HOOKS)?;
    in_progress(repo)?;
    index_check(repo, all)?;
    if exists(&repo.path().join("index.lock")) {
        // Git refuses with the lock error; native refuses the same way.
        return Err(CliReason::IndexState("index.lock"));
    }
    if raw_message.contains('\0') {
        return Err(CliReason::Message("NUL byte"));
    }
    if let Some(encoding) = config
        .text("i18n.commitEncoding")
        .map_err(|_| CliReason::Config("i18n.commitEncoding"))?
        && !matches!(encoding.to_ascii_lowercase().as_str(), "utf-8" | "utf8")
    {
        return Err(CliReason::Config("i18n.commitEncoding"));
    }
    let cleanup = Cleanup::from_commit_config(
        config
            .text("commit.cleanup")
            .map_err(|_| CliReason::Config("commit.cleanup"))?
            .as_deref(),
    )
    .ok_or(CliReason::Config("commit.cleanup value"))?;
    if cleanup.uses_comment_character() {
        comment_style_check(&config)?;
    }
    let message = message::clean_commit_message(raw_message, cleanup);
    if message.trim().is_empty() {
        return Err(CliReason::Message("empty after cleanup"));
    }
    let author = identity(env, &config, Role::Author)?;
    let committer = identity(env, &config, Role::Committer)?;
    let head = head_state(repo)?;
    let staging = if all {
        staging_plan(repo, &config)?
    } else {
        Vec::new()
    };
    Ok(CommitPlan {
        message,
        author,
        committer,
        head,
        staging,
    })
}

/// A tag name made of characters and shapes that Git and libgit2 judge alike.
pub(super) fn plain_tag_name(name: &str) -> bool {
    !name.is_empty()
        && name != "HEAD"
        && !name.starts_with(['-', '/', '.'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && !name.ends_with(".lock")
        && !name.contains("/.")
        && !name.contains(".lock/")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/' | b'+'))
}

/// Decide a tag creation.
pub(super) fn plan_tag_create(
    env: &GitEnvironment,
    repo: &git2::Repository,
    name: &str,
    raw_message: Option<&str>,
    signed: bool,
) -> Result<TagPlan, CliReason> {
    if !plain_tag_name(name) {
        return Err(CliReason::TagName);
    }
    if signed {
        return Err(CliReason::Signing("signing requested"));
    }
    let config = common(env, repo)?;
    if config.flag("tag.gpgSign").may_be_true() {
        return Err(CliReason::Signing("tag.gpgSign"));
    }
    if raw_message.is_some() && config.flag("tag.forceSignAnnotated").may_be_true() {
        return Err(CliReason::Signing("tag.forceSignAnnotated"));
    }
    hook_check(repo, &config, TAG_HOOKS)?;
    let head = head_state(repo)?;
    let target = match head {
        HeadState::Branch { tip: Some(oid), .. } | HeadState::Detached(oid) => oid,
        HeadState::Branch { tip: None, .. } => return Err(CliReason::HeadShape),
    };
    let annotation = match raw_message {
        None => None,
        Some(raw) => {
            if raw.contains('\0') {
                return Err(CliReason::Message("NUL byte"));
            }
            comment_style_check(&config)?;
            let text = message::clean_tag_message(raw);
            if text.trim().is_empty() {
                return Err(CliReason::Message("empty after cleanup"));
            }
            Some((text, identity(env, &config, Role::Committer)?))
        }
    };
    Ok(TagPlan { target, annotation })
}

/// Decide a tag deletion.
pub(super) fn plan_tag_delete(
    env: &GitEnvironment,
    repo: &git2::Repository,
    name: &str,
) -> Result<(), CliReason> {
    if !plain_tag_name(name) {
        return Err(CliReason::TagName);
    }
    let config = common(env, repo)?;
    hook_check(repo, &config, TAG_HOOKS)
}
