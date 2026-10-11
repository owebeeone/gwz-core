//! `commit`, `tag_create` and `tag_delete` over libgit2, with the Git CLI
//! kept for what libgit2 cannot reproduce.
//!
//! Operator decision 2026-10-12 (GwzNoFallbackPlan.md §6): these three use
//! libgit2 when nothing needs the Git CLI, and the CLI otherwise. [`route`]
//! holds the rule; this file picks the route and runs it. Removing the CLI
//! route altogether (hooks, signing, filters) is lane 3's later work.

use super::*;

mod config_view;
mod environment;
mod exec;
mod hooks;
mod ident;
mod message;
mod route;
mod staging;
mod system_config;

pub use environment::GitEnvironment;
use route::CliReason;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}

/// Which route the backend may take. The default is the route rule; tests
/// can force the Git CLI to compare it with the native route.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RoutePolicy {
    cli_only: bool,
}

impl Git2Backend {
    fn plan_commit(
        &self,
        path: &Path,
        message: &str,
        all: bool,
    ) -> Result<(git2::Repository, route::CommitPlan), CliReason> {
        self.require_auto()?;
        let repo = route::open(path)?;
        let plan = route::plan_commit(&self.environment, &repo, message, all)?;
        Ok((repo, plan))
    }

    fn plan_tag_create(
        &self,
        path: &Path,
        name: &str,
        message: Option<&str>,
        signed: bool,
    ) -> Result<(git2::Repository, route::TagPlan), CliReason> {
        self.require_auto()?;
        let repo = route::open(path)?;
        let plan = route::plan_tag_create(&self.environment, &repo, name, message, signed)?;
        Ok((repo, plan))
    }

    fn plan_tag_delete(&self, path: &Path, name: &str) -> Result<git2::Repository, CliReason> {
        self.require_auto()?;
        let repo = route::open(path)?;
        route::plan_tag_delete(&self.environment, &repo, name)?;
        Ok(repo)
    }

    fn require_auto(&self) -> Result<(), CliReason> {
        if self.route_policy.cli_only {
            Err(CliReason::Forced)
        } else {
            Ok(())
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl RoutePolicy {
            /// The route rule decides.
            pub(crate) const AUTO: Self = Self { cli_only: false };
            /// Always the Git CLI.
            pub(crate) const CLI_ONLY: Self = Self { cli_only: true };
        }

        /// The route one operation takes.
        #[derive(Clone, Debug, PartialEq, Eq)]
        enum Route {
            Native,
            Cli(CliReason),
        }

        impl Git2Backend {
        /// The route `commit` takes for this repository now.
        fn commit_route(&self, path: &Path, message: &str, all: bool) -> Route {
            match self.plan_commit(path, message, all) {
                Ok(_) => Route::Native,
                Err(reason) => Route::Cli(reason),
            }
        }

        /// The route `tag_create` takes for this repository now.
        fn tag_create_route(
            &self,
            path: &Path,
            name: &str,
            message: Option<&str>,
            signed: bool,
        ) -> Route {
            match self.plan_tag_create(path, name, message, signed) {
                Ok(_) => Route::Native,
                Err(reason) => Route::Cli(reason),
            }
        }

        /// The route `tag_delete` takes for this repository now.
        fn tag_delete_route(&self, path: &Path, name: &str) -> Route {
            match self.plan_tag_delete(path, name) {
                Ok(_) => Route::Native,
                Err(reason) => Route::Cli(reason),
            }
        }

            /// A copy of this backend that takes the given route policy.
            pub(crate) fn with_route_policy(&self, policy: RoutePolicy) -> Self {
                let mut backend = self.clone();
                backend.route_policy = policy;
                backend
            }

            /// A copy of this backend that sees the given environment.
            pub(crate) fn with_environment(&self, environment: GitEnvironment) -> Self {
                let mut backend = self.clone();
                backend.environment = environment;
                backend
            }
        }
    }
}

pub(super) fn commit(
    backend: &Git2Backend,
    path: &Path,
    message: &str,
    all: bool,
) -> ModelResult<GitCommitResult> {
    let Ok((repo, plan)) = backend.plan_commit(path, message, all) else {
        return repository::commit_cli(backend, path, message, all);
    };
    let oid = exec::commit(&repo, plan)?;
    // AD1 self-verify: HEAD is the new commit (read fresh).
    let after = backend.head(path)?.commit;
    if after.as_deref() != Some(oid.to_string().as_str()) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "HEAD is not the new commit after commit",
        ));
    }
    Ok(GitCommitResult {
        commit: oid.to_string(),
    })
}

pub(super) fn tag_create(
    backend: &Git2Backend,
    path: &Path,
    name: &str,
    message: Option<&str>,
    signed: bool,
) -> ModelResult<GitTagResult> {
    let Ok((repo, plan)) = backend.plan_tag_create(path, name, message, signed) else {
        return refs::tag_create_cli(backend, path, name, message, signed);
    };
    exec::tag_create(&repo, name, plan)?;
    refs::verify_tag_created(backend, path, name)
}

pub(super) fn tag_delete(backend: &Git2Backend, path: &Path, name: &str) -> ModelResult<()> {
    let Ok(repo) = backend.plan_tag_delete(path, name) else {
        return refs::tag_delete_cli(backend, path, name);
    };
    exec::tag_delete(&repo, name)?;
    refs::verify_tag_deleted(backend, path, name)
}
