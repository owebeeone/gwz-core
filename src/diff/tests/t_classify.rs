//! Unit tests for bare-operand disambiguation (`super::super::classify`).
//!
//! These drive [`classify_operands`] directly with an injected revision resolver
//! (a name set) and a real temp directory for the existing-path stat, so every
//! cell of the rule table is exercised without a full Git workspace. The
//! end-to-end handler behaviour (the user's `gwz diff <file>` repro) is covered by
//! the `t_handle` integration tests.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact::{ManifestArtifact, WorkspaceHeader};
use crate::diff::{ClassifiedOperands, RevContext, classify_operands};
use crate::model::{ErrorCode, ModelResult};

/// A throwaway temp dir that is both the workspace root and the cwd; drops on
/// scope exit.
struct Sandbox {
    root: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "gwz-classify-{tag}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Sandbox { root }
    }

    fn touch(&self, rel: &str) {
        let full = self.root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, b"").unwrap();
    }
}

fn manifest() -> ManifestArtifact {
    ManifestArtifact {
        schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: "ws".to_owned(),
        },
        members: Vec::new(),
    }
}

/// Classify `operands` where the revisions in `revs` resolve, statting paths
/// against `sandbox` (root == cwd).
fn run(sandbox: &Sandbox, operands: &[&str], revs: &[&str]) -> ModelResult<ClassifiedOperands> {
    let rev_set: BTreeSet<String> = revs.iter().map(|s| (*s).to_owned()).collect();
    let resolve =
        move |_repos: &[PathBuf], token: &str| -> ModelResult<bool> { Ok(rev_set.contains(token)) };
    let ctx = RevContext {
        repos: vec![sandbox.root.clone()],
        cwd: sandbox.root.clone(),
        workspace_root: sandbox.root.clone(),
        resolve: &resolve,
    };
    let ops: Vec<String> = operands.iter().map(|s| (*s).to_owned()).collect();
    classify_operands(&ops, &manifest(), &ctx)
}

fn revs(c: &ClassifiedOperands) -> Vec<&str> {
    c.revisions.iter().map(String::as_str).collect()
}

fn paths(c: &ClassifiedOperands) -> Vec<&str> {
    c.pathspecs.iter().map(String::as_str).collect()
}

#[test]
fn bare_existing_file_is_a_pathspec() {
    // The user's repro shape: `gwz diff <existing file>` (no `--`) → pathspec.
    let sb = Sandbox::new("path");
    sb.touch("src/mod.rs");
    let c = run(&sb, &["src/mod.rs"], &[]).unwrap();
    assert_eq!(revs(&c), Vec::<&str>::new());
    assert_eq!(paths(&c), vec!["src/mod.rs"]);
}

#[test]
fn bare_subdir_operand_is_a_pathspec() {
    let sb = Sandbox::new("subdir");
    sb.touch("gwz-core/src/lib.rs");
    let c = run(&sb, &["gwz-core"], &[]).unwrap();
    assert_eq!(paths(&c), vec!["gwz-core"]);
    assert!(c.revisions.is_empty());
}

#[test]
fn bare_revision_that_is_not_a_path_is_a_revision() {
    let sb = Sandbox::new("rev");
    let c = run(&sb, &["HEAD"], &["HEAD"]).unwrap();
    assert_eq!(revs(&c), vec!["HEAD"]);
    assert!(c.pathspecs.is_empty());
}

#[test]
fn mixed_revision_then_file_splits() {
    // `gwz diff HEAD file.txt` — rev then path, no `--`.
    let sb = Sandbox::new("mixed");
    sb.touch("file.txt");
    let c = run(&sb, &["HEAD", "file.txt"], &["HEAD"]).unwrap();
    assert_eq!(revs(&c), vec!["HEAD"]);
    assert_eq!(paths(&c), vec!["file.txt"]);
}

#[test]
fn ambiguous_name_that_is_both_errors_suggesting_dashdash() {
    // A branch named exactly like an existing file: BOTH → typed error.
    let sb = Sandbox::new("ambig");
    sb.touch("main");
    let err = run(&sb, &["main"], &["main"]).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
    assert!(err.message.contains("ambiguous"), "{}", err.message);
    assert!(err.message.contains("--"), "{}", err.message);
}

#[test]
fn nonexistent_operand_errors_with_improved_message() {
    // NEITHER a rev nor a path → git's "unknown revision or path" message.
    let sb = Sandbox::new("none");
    let err = run(&sb, &["nope"], &[]).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
    assert!(
        err.message.contains("unknown revision or path"),
        "{}",
        err.message
    );
    assert!(err.message.contains("--"), "{}", err.message);
}

#[test]
fn range_operand_is_never_a_path_even_if_a_file_matches() {
    // A file literally named `a..b` must not shadow the range operand.
    let sb = Sandbox::new("range");
    sb.touch("a..b");
    let c = run(&sb, &["a..b"], &[]).unwrap();
    assert_eq!(revs(&c), vec!["a..b"]);
    assert!(c.pathspecs.is_empty());
}

#[test]
fn snapshot_operand_is_never_a_path() {
    let sb = Sandbox::new("snap");
    sb.touch("+snap1");
    let c = run(&sb, &["+snap1"], &[]).unwrap();
    assert_eq!(revs(&c), vec!["+snap1"]);
    assert!(c.pathspecs.is_empty());
}

#[test]
fn once_a_pathspec_later_tokens_are_pathspecs() {
    // After the first path, a following existing file is a pathspec even though
    // no `--` was given.
    let sb = Sandbox::new("zone");
    sb.touch("a.txt");
    sb.touch("b.txt");
    let c = run(&sb, &["a.txt", "b.txt"], &[]).unwrap();
    assert_eq!(paths(&c), vec!["a.txt", "b.txt"]);
    assert!(c.revisions.is_empty());
}

#[test]
fn revision_after_a_pathspec_errors() {
    // Git's verify_filename: a rev-looking, non-path token after a path is the
    // ambiguous error again.
    let sb = Sandbox::new("revafter");
    sb.touch("a.txt");
    let err = run(&sb, &["a.txt", "HEAD"], &["HEAD"]).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
    assert!(err.message.contains("--"), "{}", err.message);
}

#[test]
fn workspace_escape_is_not_a_path() {
    // An absolute operand outside the workspace can never be a pathspec (the same
    // escape rule route_pathspec applies); with no rev match it falls through to
    // the unknown-revision-or-path error. (A `..`-prefixed token is rev-range
    // syntax and handled by `range_operand_is_never_a_path`, not here.)
    let sb = Sandbox::new("escape");
    let outside = std::env::temp_dir().join("gwz-classify-escape-outside-marker");
    fs::write(&outside, b"").unwrap();
    let err = run(&sb, &[&outside.to_string_lossy()], &[]).unwrap_err();
    let _ = fs::remove_file(&outside);
    assert_eq!(err.code, ErrorCode::InvalidRequest);
    assert!(
        err.message.contains("unknown revision or path"),
        "{}",
        err.message
    );
}
