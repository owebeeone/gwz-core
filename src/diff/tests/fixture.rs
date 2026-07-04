//! Temp-repo fixtures and `git diff` parity helpers, in the style of the
//! existing gitbackend tests (see `src/git/tests/g06.rs`): a self-cleaning
//! `TempDir` under the system temp dir plus thin wrappers over the `git` CLI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::diff::{ComparisonSpec, RepoDiffEntry, RepoDiffManifest, RepoDiffOptions};

pub(crate) use crate::git::{Git2Backend, GitBackend};

/// A unique temp directory that removes itself on drop.
pub(crate) struct TempDir {
    pub(crate) path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gwz-core-diff-{prefix}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Initialize a repo with a deterministic identity and config so `git diff`
/// output is stable across environments.
pub(crate) fn init_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    Git2Backend::new().create_repo(root).unwrap();
    // Deterministic diff output regardless of the ambient user config.
    run_git(root, &["config", "core.autocrlf", "false"]);
    run_git(root, &["config", "diff.renames", "true"]);
}

/// Run `git` in `root` with a fixed identity; assert success.
pub(crate) fn run_git(root: &Path, args: &[&str]) {
    let status = git_command(root, args).status().expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

/// Run `git` in `root` and return captured stdout as a UTF-8 string.
pub(crate) fn git_stdout(root: &Path, args: &[&str]) -> String {
    let output = git_command(root, args).output().expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout utf8")
}

fn git_command(root: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args([
        "-c",
        "user.name=GWZ",
        "-c",
        "user.email=gwz@example.invalid",
    ])
    .arg("-C")
    .arg(root)
    .args(args);
    cmd
}

pub(crate) fn write_file(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

pub(crate) fn write_bytes(root: &Path, rel: &str, contents: &[u8]) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

pub(crate) fn commit_all(root: &Path, message: &str) {
    run_git(root, &["add", "-A"]);
    run_git(root, &["commit", "-m", message]);
}

/// Convenience: worktree-vs-index diff (`git diff`) over the whole repo.
pub(crate) fn diff_worktree(root: &Path, options: RepoDiffOptions) -> RepoDiffManifest {
    let backend = Git2Backend::new();
    let comparison = backend
        .resolve_comparison(root, &ComparisonSpec::default())
        .expect("resolve worktree comparison");
    backend
        .diff_manifest(root, &comparison, &options)
        .expect("diff_manifest")
}

/// Convenience: run `diff_manifest` for an arbitrary resolved spec + options.
pub(crate) fn diff_spec(
    root: &Path,
    spec: &ComparisonSpec,
    options: RepoDiffOptions,
) -> RepoDiffManifest {
    let backend = Git2Backend::new();
    let comparison = backend
        .resolve_comparison(root, spec)
        .expect("resolve comparison");
    backend
        .diff_manifest(root, &comparison, &options)
        .expect("diff_manifest")
}

/// Render a manifest into `git diff --name-status -M`-style lines
/// (`R\told\tnew` for renames, `<X>\t<path>` otherwise), sorted for a
/// set-comparison against Git.
///
/// The rename *similarity percentage* is deliberately dropped from the line:
/// libgit2 and Git use slightly different similarity metrics, so the exact
/// number legitimately differs (libgit2 may report 97% where Git reports 95%
/// for the same move). These lines test rename *detection and pairing*; the
/// similarity value itself is asserted separately with a tolerance where it
/// matters.
pub(crate) fn name_status_lines(manifest: &RepoDiffManifest) -> Vec<String> {
    let mut lines: Vec<String> = manifest.entries.iter().map(name_status_line).collect();
    lines.sort();
    lines
}

fn name_status_line(entry: &RepoDiffEntry) -> String {
    match entry.status {
        crate::diff::RepoDiffStatus::Renamed => format!(
            "R\t{}\t{}",
            entry.old_path.as_deref().unwrap_or(""),
            entry.new_path.as_deref().unwrap_or(""),
        ),
        status => format!(
            "{}\t{}",
            status.status_char(),
            entry.primary_path().unwrap_or(""),
        ),
    }
}

/// Parse `git diff`'s `--name-status`-style stdout into sorted lines, dropping
/// the similarity fraction from rename lines' status column (`R097` -> `R`) so
/// the comparison is metric-agnostic (see [`name_status_lines`]).
pub(crate) fn git_name_status(root: &Path, extra_args: &[&str]) -> Vec<String> {
    let mut args = vec!["diff", "--name-status", "-M", "--no-color"];
    args.extend_from_slice(extra_args);
    let raw = git_stdout(root, &args);
    let mut lines: Vec<String> = raw.lines().map(normalize_git_name_status_line).collect();
    lines.sort();
    lines
}

fn normalize_git_name_status_line(line: &str) -> String {
    let mut parts = line.splitn(2, '\t');
    let status = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    if status.starts_with('R') {
        // Drop the similarity index: "R097" -> "R".
        format!("R\t{rest}")
    } else {
        format!("{status}\t{rest}")
    }
}

/// Extract Git's reported rename similarity percentage from the first `R<nn>`
/// line of `git diff --name-status -M`.
pub(crate) fn git_rename_similarity(root: &Path, extra_args: &[&str]) -> u16 {
    let mut args = vec!["diff", "--name-status", "-M", "--no-color"];
    args.extend_from_slice(extra_args);
    let raw = git_stdout(root, &args);
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix('R') {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(value) = digits.parse::<u16>() {
                return value;
            }
        }
    }
    panic!("no rename line in git output: {raw}");
}
