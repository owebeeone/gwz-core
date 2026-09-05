//! Configured paths that would still name a location outside dest after copy.
//!
//! Design §4.0 refuses `core.worktree`, effective `core.hooksPath` (absolute
//! **or relative**), `include.path`, `includeIf` and `url.*.insteadOf` when
//! the effective path escapes the repository's copied boundary, and refuses an
//! effective configuration it cannot resolve rather than admitting it.
//!
//! **Only the repository's own configuration is read** — `<common dir>/config`
//! and, when present, `<git dir>/config.worktree`. The user's global and the
//! system configuration are deliberately not consulted: they are not copied,
//! so they cannot make the destination name a path outside itself, and
//! reading them would make one machine's `~/.gitconfig` decide whether another
//! machine's repository may be cloned.
//!
//! ## Hook working directories (design §4.0, checkpoint §6)
//!
//! "A relative hook path is relative to the hook's working directory, not
//! necessarily the config file: normally the worktree root, or the Git
//! directory for bare repositories and push-triggered hooks. Check applicable
//! bases and symlink resolution; keep valid internal relative paths."
//!
//! So one relative `core.hooksPath` has **two** effective locations in a
//! checkout — one for ordinary hooks (worktree root) and one for
//! push-triggered hooks such as `pre-receive` (the Git directory) — and one in
//! a bare repository. Every applicable base is resolved; an escape or an
//! unresolvable result under *any* of them refuses, because a hook that
//! escapes only when pushed still escapes.
//!
//! An absolute value is always an escape: after copy it names the same
//! absolute location, which by construction is not inside the destination.

use std::path::{Path, PathBuf};

use gwz_repo_contract::LayoutHazard;

use crate::layout::hazard_for;
use crate::paths::{self, Resolution};

/// Every configured-path hazard in this repository's own configuration.
pub(crate) fn hazards(
    boundary: &Path,
    git_dir: &Path,
    common_dir: &Path,
    work_dir: Option<&Path>,
) -> Vec<LayoutHazard> {
    let mut hazards = Vec::new();
    let local = common_dir.join("config");
    match std::fs::symlink_metadata(&local) {
        Ok(_) => scan_file(&mut hazards, boundary, git_dir, work_dir, &local),
        Err(error) => hazards.push(LayoutHazard::UnresolvableConfig {
            key: "config".to_owned(),
            detail: format!("{}: {error}", local.display()),
        }),
    }
    let worktree_config = git_dir.join("config.worktree");
    if std::fs::symlink_metadata(&worktree_config).is_ok() {
        scan_file(&mut hazards, boundary, git_dir, work_dir, &worktree_config);
    }
    hazards.dedup();
    hazards
}

fn scan_file(
    hazards: &mut Vec<LayoutHazard>,
    boundary: &Path,
    git_dir: &Path,
    work_dir: Option<&Path>,
    file: &Path,
) {
    let config = match git2::Config::open(file) {
        Ok(config) => config,
        Err(error) => {
            hazards.push(LayoutHazard::UnresolvableConfig {
                key: "config".to_owned(),
                detail: format!("{}: {}", file.display(), error.message()),
            });
            return;
        }
    };
    let entries = match config.entries(None) {
        Ok(entries) => entries,
        Err(error) => {
            hazards.push(LayoutHazard::UnresolvableConfig {
                key: "config".to_owned(),
                detail: format!("{}: {}", file.display(), error.message()),
            });
            return;
        }
    };
    let config_dir = file.parent().unwrap_or(git_dir).to_path_buf();
    let mut collected = Vec::new();
    let iterated = entries.for_each(|entry| {
        collected.push((
            String::from_utf8_lossy(entry.name_bytes()).into_owned(),
            entry.has_value().then(|| entry.value_bytes().to_vec()),
            entry.include_depth(),
        ));
    });
    if let Err(error) = iterated {
        hazards.push(LayoutHazard::UnresolvableConfig {
            key: "config".to_owned(),
            detail: format!("{}: {}", file.display(), error.message()),
        });
        return;
    }
    for (name, value, depth) in collected {
        hazards.extend(entry_hazards(
            boundary,
            git_dir,
            work_dir,
            &config_dir,
            &name,
            value,
            depth,
        ));
    }
}

fn entry_hazards(
    boundary: &Path,
    git_dir: &Path,
    work_dir: Option<&Path>,
    config_dir: &Path,
    name: &str,
    value: Option<Vec<u8>>,
    depth: u32,
) -> Vec<LayoutHazard> {
    let lower = name.to_ascii_lowercase();
    let Some(role) = classify(&lower) else {
        return Vec::new();
    };
    // libgit2 hands back the entry name with its section and variable
    // lower-cased; hazards carry the spelling a reader will find in the file.
    let key = canonical_key(name, &lower, role);
    // `url.<subsection>.insteadOf` puts the path in the *key*, not the value.
    if role == Role::InsteadOf {
        let Some(url) = insteadof_subsection(name, &lower) else {
            return vec![unresolvable(&key, "the url subsection could not be read")];
        };
        return match local_path_in_url(&url) {
            None => Vec::new(),
            Some(path) => check(
                &key,
                &path,
                boundary,
                &hook_bases(boundary, git_dir, work_dir),
            ),
        };
    }
    let Some(bytes) = value else {
        return vec![unresolvable(&key, "the key has no value")];
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return vec![unresolvable(&key, "the value is not valid UTF-8")];
    };
    if text.is_empty() {
        return vec![unresolvable(&key, "the value is empty")];
    }
    match role {
        Role::HooksPath => check(
            &key,
            &text,
            boundary,
            &hook_bases(boundary, git_dir, work_dir),
        ),
        // `core.worktree` is resolved relative to the Git directory.
        Role::Worktree => check(&key, &text, boundary, &[git_dir.to_path_buf()]),
        Role::Include if depth > 0 => vec![unresolvable(
            &key,
            "a relative include inside an included file has no knowable base",
        )],
        Role::Include => check(&key, &text, boundary, &[config_dir.to_path_buf()]),
        Role::InsteadOf => Vec::new(),
    }
}

/// The spelling Git's own documentation uses, with the subsection kept as the
/// file wrote it.
fn canonical_key(name: &str, lower: &str, role: Role) -> String {
    match role {
        Role::HooksPath => "core.hooksPath".to_owned(),
        Role::Worktree => "core.worktree".to_owned(),
        Role::Include if lower == "include.path" => "include.path".to_owned(),
        Role::Include => {
            let subsection = name
                .get("includeif.".len()..name.len() - ".path".len())
                .unwrap_or_default();
            format!("includeIf.{subsection}.path")
        }
        Role::InsteadOf => {
            let push = lower.ends_with(".pushinsteadof");
            let subsection = insteadof_subsection(name, lower).unwrap_or_default();
            let variable = if push { "pushInsteadOf" } else { "insteadOf" };
            format!("url.{subsection}.{variable}")
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    HooksPath,
    Worktree,
    Include,
    InsteadOf,
}

fn classify(lower: &str) -> Option<Role> {
    if lower == "core.hookspath" {
        return Some(Role::HooksPath);
    }
    if lower == "core.worktree" {
        return Some(Role::Worktree);
    }
    if lower == "include.path" || (lower.starts_with("includeif.") && lower.ends_with(".path")) {
        return Some(Role::Include);
    }
    if lower.starts_with("url.")
        && (lower.ends_with(".insteadof") || lower.ends_with(".pushinsteadof"))
    {
        return Some(Role::InsteadOf);
    }
    None
}

/// The subsection of `url.<subsection>.insteadOf`, with its original case.
fn insteadof_subsection(name: &str, lower: &str) -> Option<String> {
    let suffix = if lower.ends_with(".pushinsteadof") {
        ".pushinsteadof"
    } else {
        ".insteadof"
    };
    let end = name.len().checked_sub(suffix.len())?;
    name.get("url.".len()..end).map(str::to_owned)
}

/// The local filesystem path a remote URL names, if it names one at all. An
/// `https://`, `ssh://` or `user@host:path` URL names no local path and cannot
/// escape the destination.
fn local_path_in_url(url: &str) -> Option<String> {
    if let Some(rest) = url.strip_prefix("file://") {
        return Some(rest.to_owned());
    }
    if url.starts_with('~')
        || url.starts_with('/')
        || url.starts_with("./")
        || url.starts_with("../")
    {
        return Some(url.to_owned());
    }
    None
}

/// The working directories a hook may run in (design §4.0): the worktree root
/// for ordinary hooks, and the Git directory for push-triggered hooks and for
/// a bare repository.
///
/// A base that is itself outside the boundary is dropped — its own escape is
/// already reported by the `core.worktree` or common-directory check, and
/// resolving a relative path from outside the tree would say nothing about
/// what lands inside dest. If that leaves no base, the boundary itself is
/// used, so a relative value is still resolved rather than silently admitted.
fn hook_bases(boundary: &Path, git_dir: &Path, work_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    for base in work_dir.into_iter().chain(std::iter::once(git_dir)) {
        if paths::contains(boundary, base) && !bases.contains(&base.to_path_buf()) {
            bases.push(base.to_path_buf());
        }
    }
    if bases.is_empty() {
        bases.push(boundary.to_path_buf());
    }
    bases
}

fn check(key: &str, value: &str, boundary: &Path, bases: &[PathBuf]) -> Vec<LayoutHazard> {
    if value.starts_with('~') {
        // `~` and `~user` expand to a home directory, never inside dest.
        return vec![LayoutHazard::EscapingConfig {
            key: key.to_owned(),
            value: value.to_owned(),
        }];
    }
    let candidate = Path::new(value);
    let mut hazards = Vec::new();
    for base in bases {
        let resolution = paths::resolve_within(boundary, base, candidate);
        if resolution.is_inside() {
            continue;
        }
        if let Some(hazard) = hazard_for(key, value, &resolution) {
            hazards.push(hazard);
        }
        if matches!(resolution, Resolution::Unresolvable(_)) {
            // One unresolvable base is enough; do not also report the escape
            // a second base might imply from an unknown starting point.
            break;
        }
    }
    hazards.dedup();
    hazards
}

fn unresolvable(key: &str, detail: &str) -> LayoutHazard {
    LayoutHazard::UnresolvableConfig {
        key: key.to_owned(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_bases_are_the_worktree_and_the_git_directory_in_a_checkout() {
        let git_dir = PathBuf::from("/w/.git");
        let work_dir = PathBuf::from("/w");
        assert_eq!(
            hook_bases(&work_dir, &git_dir, Some(&work_dir)),
            vec![work_dir.clone(), git_dir.clone()]
        );
        // A bare repository: the Git directory is the only working directory,
        // and it is the boundary.
        assert_eq!(hook_bases(&git_dir, &git_dir, None), vec![git_dir.clone()]);
        // A worktree redirected outside the boundary is not a usable base.
        assert_eq!(
            hook_bases(&work_dir, &git_dir, Some(Path::new("/elsewhere"))),
            vec![git_dir]
        );
    }

    #[test]
    fn only_url_subsections_that_name_a_local_path_are_checked() {
        assert_eq!(
            local_path_in_url("file:///srv/mirror/"),
            Some("/srv/mirror/".to_owned())
        );
        assert_eq!(local_path_in_url("../peer"), Some("../peer".to_owned()));
        assert_eq!(
            local_path_in_url("~/mirrors/"),
            Some("~/mirrors/".to_owned())
        );
        assert_eq!(local_path_in_url("https://example.invalid/x"), None);
        assert_eq!(local_path_in_url("git@example.invalid:x.git"), None);
    }

    #[test]
    fn config_keys_are_classified_case_insensitively() {
        assert_eq!(classify("core.hookspath"), Some(Role::HooksPath));
        assert_eq!(classify("core.worktree"), Some(Role::Worktree));
        assert_eq!(classify("include.path"), Some(Role::Include));
        assert_eq!(classify("includeif.gitdir:/w/.path"), Some(Role::Include));
        assert_eq!(classify("url.../peer.insteadof"), Some(Role::InsteadOf));
        assert_eq!(classify("url.x.pushinsteadof"), Some(Role::InsteadOf));
        assert_eq!(classify("core.bare"), None);
        assert_eq!(
            insteadof_subsection("url.../peer.insteadOf", "url.../peer.insteadof"),
            Some("../peer".to_owned())
        );
    }

    #[test]
    fn a_home_relative_value_escapes_without_touching_the_filesystem() {
        let hazards = check(
            "core.hooksPath",
            "~/hooks",
            Path::new("/w"),
            &[PathBuf::from("/w")],
        );
        assert_eq!(
            hazards,
            vec![LayoutHazard::EscapingConfig {
                key: "core.hooksPath".to_owned(),
                value: "~/hooks".to_owned()
            }]
        );
    }
}
