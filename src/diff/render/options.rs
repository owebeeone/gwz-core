//! Render options and the per-scope prefix policy.
//!
//! [`ScopeRender`] captures everything about *where* a repo lives in the unified
//! workspace that changes the rendered path text: its member prefix (empty for
//! the workspace root) and the `a/`,`b/` prefix policy (`--src-prefix` /
//! `--dst-prefix` / `--no-prefix`). [`RenderOptions`] captures the
//! *presentation* knobs that are orthogonal to the diff itself — `--line-prefix`
//! and `-z` NUL framing — plus a [`ScopeRender`].
//!
//! Both are plain data with no operation-layer dependency (per the module head):
//! D3 builds them from the wire request and threads them through the renderers.

use crate::diff::RepoDiffEntry;

/// How the virtual `a/`,`b/` diff prefixes are formed for one repo. This mirrors
/// `git diff`'s `--src-prefix` / `--dst-prefix` / `--no-prefix`, composed with
/// the workspace member path (plan §"Workspace path rendering").
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PrefixPolicy {
    /// Git defaults: `a/` and `b/` (root uses this; members use it composed with
    /// the member path — see [`ScopeRender::old_prefix`]).
    #[default]
    Default,
    /// `--src-prefix=<p>` / `--dst-prefix=<p>`: custom prefixes replace `a/`,`b/`.
    Custom { src: String, dst: String },
    /// `--no-prefix`: no `a/`,`b/` at all. For a member the member path is still
    /// kept so the unified output stays path-stable (plan §"Workspace path
    /// rendering"): a member renders as `<member>/…`, the root as bare `…`.
    None,
}

/// The per-repo rendering scope: the member prefix (workspace-relative directory
/// this repo is mounted at, e.g. `gwz-core`; **empty** for the workspace root)
/// and the [`PrefixPolicy`]. Resolve one per planned target and reuse it for
/// every entry and record in that repo.
///
/// The `old_prefix`/`new_prefix` values are what a D3 caller feeds to
/// `git2::DiffOptions::{old_prefix,new_prefix}`; [`workspace_path`] is what the
/// hand post-pass and the manifest-derived record builders prepend to a
/// repo-relative path.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ScopeRender {
    /// Workspace-relative mount directory of this repo. Empty for the root.
    /// Never has a trailing slash.
    member: String,
    policy: PrefixPolicy,
}

impl ScopeRender {
    /// A member scope mounted at `member_path` (e.g. `gwz-core`).
    pub fn member(member_path: impl Into<String>, policy: PrefixPolicy) -> Self {
        Self {
            member: trim_slashes(member_path.into()),
            policy,
        }
    }

    /// The workspace-root scope: no member prefix.
    pub fn root(policy: PrefixPolicy) -> Self {
        Self {
            member: String::new(),
            policy,
        }
    }

    /// The `old_prefix` string for `git2::DiffOptions::old_prefix`. Composes the
    /// prefix policy with the member path so the `diff --git`/`---` positions
    /// come out workspace-relative straight from libgit2 (spike Q1a).
    pub fn old_prefix(&self) -> String {
        self.compose_prefix(
            || "a".to_owned(),
            |p: &PrefixPolicy| match p {
                PrefixPolicy::Custom { src, .. } => Some(src.clone()),
                _ => None,
            },
        )
    }

    /// The `new_prefix` string for `git2::DiffOptions::new_prefix`.
    pub fn new_prefix(&self) -> String {
        self.compose_prefix(
            || "b".to_owned(),
            |p: &PrefixPolicy| match p {
                PrefixPolicy::Custom { dst, .. } => Some(dst.clone()),
                _ => None,
            },
        )
    }

    /// Compose `<virtual-prefix><member>` where the virtual prefix depends on the
    /// policy. `default_virtual` supplies the `a`/`b` letter; `custom` extracts
    /// the src/dst side of a custom prefix.
    ///
    /// libgit2 joins `old_prefix` to the repo-relative path with a `/`, so a
    /// member yields `a/<member>/<path>`; a custom prefix such as `x/` yields
    /// `x/<member>/<path>`; `--no-prefix` yields `<member>/<path>` (and bare
    /// `<path>` for the root, matching git).
    fn compose_prefix(
        &self,
        default_virtual: impl Fn() -> String,
        custom: impl Fn(&PrefixPolicy) -> Option<String>,
    ) -> String {
        let virtual_prefix = match &self.policy {
            PrefixPolicy::Default => default_virtual(),
            PrefixPolicy::Custom { .. } => {
                // A custom prefix such as `x/` already carries its own trailing
                // slash; libgit2 appends `/` before the path, so drop a trailing
                // slash here to avoid `x//<member>`.
                trim_slashes(custom(&self.policy).unwrap_or_default())
            }
            PrefixPolicy::None => String::new(),
        };
        join_nonempty(&[virtual_prefix.as_str(), self.member.as_str()])
    }

    /// Prepend the member prefix to a repo-relative path, yielding the
    /// workspace-relative path used in `rename from`/`rename to` headers and in
    /// every manifest-derived name/stat record. Root (empty member) returns the
    /// path unchanged.
    pub fn workspace_path(&self, repo_relative: &str) -> String {
        join_nonempty(&[self.member.as_str(), repo_relative])
    }

    /// This repo's member prefix (empty for the root).
    pub fn member_prefix(&self) -> &str {
        &self.member
    }
}

/// The full render options for one call: the per-repo [`ScopeRender`] plus the
/// presentation knobs that are independent of the diff engine.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RenderOptions {
    /// Per-repo prefix scope (member path + `a/`,`b/` policy).
    pub scope: ScopeRender,
    /// `--line-prefix=<p>`: prepended to every physical output line. `None` =
    /// no line prefix. `git2 0.21` has no setter, so the renderer applies it by
    /// hand (spike Q4a).
    pub line_prefix: Option<String>,
    /// `-z`: NUL-terminate name/status records instead of newline. Only affects
    /// the name-record builders; patch bytes are unaffected.
    pub null_terminated: bool,
    /// Emit `GIT binary patch` literals (`--binary`) instead of the
    /// `Binary files … differ` placeholder. Forwarded to
    /// `git2::DiffOptions::show_binary` by [`render_entry`](super::render_entry).
    pub show_binary: bool,
}

impl RenderOptions {
    /// Options for a member scope with git-default prefixes and no line prefix.
    pub fn member(member_path: impl Into<String>) -> Self {
        Self {
            scope: ScopeRender::member(member_path, PrefixPolicy::Default),
            ..Self::default()
        }
    }

    /// Options for the workspace-root scope with git-default prefixes.
    pub fn root() -> Self {
        Self {
            scope: ScopeRender::root(PrefixPolicy::Default),
            ..Self::default()
        }
    }
}

/// One manifest entry paired with its rendering scope, for the manifest-derived
/// record/summary builders. The builders take a slice of these so a caller can
/// interleave root and member entries in final display order and get one
/// coherent name/stat block with per-entry workspace paths.
#[derive(Clone, Debug)]
pub struct RenderEntry<'a> {
    pub entry: &'a RepoDiffEntry,
    pub scope: &'a ScopeRender,
}

impl<'a> RenderEntry<'a> {
    pub fn new(entry: &'a RepoDiffEntry, scope: &'a ScopeRender) -> Self {
        Self { entry, scope }
    }

    /// Workspace-relative new-side path (the side `--name-status`/`--stat` key
    /// on), falling back to the old side for a deletion.
    pub fn primary_ws_path(&self) -> Option<String> {
        self.entry
            .primary_path()
            .map(|p| self.scope.workspace_path(p))
    }

    /// Workspace-relative old path, if the entry has one.
    pub fn old_ws_path(&self) -> Option<String> {
        self.entry
            .old_path
            .as_deref()
            .map(|p| self.scope.workspace_path(p))
    }

    /// Workspace-relative new path, if the entry has one.
    pub fn new_ws_path(&self) -> Option<String> {
        self.entry
            .new_path
            .as_deref()
            .map(|p| self.scope.workspace_path(p))
    }
}

/// Join path-like segments with a single `/`, skipping empty segments so a root
/// scope (`""`) never introduces a leading slash and a `--no-prefix` member
/// renders as `<member>/<path>` without a stray separator.
fn join_nonempty(parts: &[&str]) -> String {
    let mut out = String::new();
    for part in parts {
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(part);
    }
    out
}

/// Strip leading/trailing slashes so composition never double-separates.
fn trim_slashes(s: String) -> String {
    s.trim_matches('/').to_owned()
}
