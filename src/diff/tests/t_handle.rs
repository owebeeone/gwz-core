//! D3 acceptance tests for `handle_diff` (plan → manifest → producer → log).
//!
//! Each test drives the real handler over an on-disk workspace and reads the
//! `diff.output` log back through the real [`DiffLogRegistry`], so the manifest,
//! the output ordering, the per-file records, and the workspace-relative bytes
//! are all checked against actual libgit2 output — not a mock.

use crate::diff::{DiffLogRegistry, LogReadRequest, LogReadState, decode_record, handle_diff};
use crate::git::{Git2Backend, GitBackend};
use crate::protocol::generated::{
    DiffManifestMode, DiffOptions, DiffOutputFormat, DiffOutputRecordKind, DiffStatus,
};
use std::process::Command;

use super::workspace_fixture::Workspace;

/// Drain the whole output log into its decoded records, following the cursor to
/// EOF. Reads in small batches so cursor resume is exercised on the happy path.
fn drain_all(registry: &DiffLogRegistry, log_id: &str) -> Vec<crate::DiffOutputRecord> {
    let mut records = Vec::new();
    let mut cursor = None;
    loop {
        let resp = registry
            .read(
                log_id,
                &LogReadRequest {
                    stream_id: "s1".to_owned(),
                    cursor,
                    max_records: Some(2),
                    max_bytes: None,
                    timeout_ms: Some(0),
                },
            )
            .unwrap();
        for r in &resp.records {
            records.push(decode_record(&r.payload));
        }
        cursor = Some(resp.next_cursor);
        match resp.state {
            LogReadState::Data => continue,
            LogReadState::Eof => break,
            other => panic!("unexpected drain state {other:?}"),
        }
    }
    records
}

fn patch_text(records: &[crate::DiffOutputRecord]) -> String {
    let mut bytes = Vec::new();
    for r in records {
        if matches!(r.kind, DiffOutputRecordKind::PatchBytes) {
            bytes.extend_from_slice(r.data.as_deref().unwrap_or(&[]));
        }
    }
    String::from_utf8(bytes).expect("patch utf8")
}

fn git_stdout(root: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout utf8")
}

fn git_tree_oid(root: &std::path::Path, rev: &str) -> String {
    git_stdout(root, &["rev-parse", &format!("{rev}^{{tree}}")])
        .trim()
        .to_owned()
}

#[test]
fn manifest_order_is_root_first_then_members() {
    let ws = Workspace::new("order");
    let member_a = ws.add_member("mem_a", "crate-a");
    let member_b = ws.add_member("mem_b", "crate-b");

    // Seed and commit all three repos, then dirty each in the worktree.
    Workspace::write(ws.root(), "root.txt", b"root v1\n");
    Workspace::write(&member_a, "a.txt", b"a v1\n");
    Workspace::write(&member_b, "b.txt", b"b v1\n");
    Workspace::commit(ws.root(), "root init");
    Workspace::commit(&member_a, "a init");
    Workspace::commit(&member_b, "b init");
    Workspace::write(ws.root(), "root.txt", b"root v2\n");
    Workspace::write(&member_a, "a.txt", b"a v2\n");
    Workspace::write(&member_b, "b.txt", b"b v2\n");

    let registry = DiffLogRegistry::new();
    let outcome = handle_diff(
        ws.root(),
        ws.request(DiffOptions::default()),
        "op_1",
        &registry,
    )
    .unwrap();

    // Manifest file order: root first, then members in manifest order.
    let scopes: Vec<Option<bool>> = outcome
        .response
        .files
        .iter()
        .map(|f| f.scope.root)
        .collect();
    assert_eq!(outcome.response.files.len(), 3);
    assert_eq!(scopes[0], Some(true), "root entry first");
    assert_eq!(
        outcome.response.files[1].scope.member_id.as_deref(),
        Some("mem_a")
    );
    assert_eq!(
        outcome.response.files[2].scope.member_id.as_deref(),
        Some("mem_b")
    );

    // The output log records file-scoped runs in the same root-first order.
    let log_id = outcome.response.output.as_ref().unwrap().log_id.clone();
    let records = drain_all(&registry, &log_id);
    let started: Vec<String> = records
        .iter()
        .filter(|r| matches!(r.kind, DiffOutputRecordKind::FileStarted))
        .map(|r| r.file_id.clone().unwrap())
        .collect();
    assert_eq!(started, vec!["@root#0", "mem_a#0", "mem_b#0"]);
}

#[test]
fn per_file_metadata_records_bracket_patch_bytes() {
    let ws = Workspace::new("perfile");
    Workspace::write(ws.root(), "a.txt", b"one\n");
    Workspace::commit(ws.root(), "init");
    Workspace::write(ws.root(), "a.txt", b"two\n");

    let registry = DiffLogRegistry::new();
    let outcome = handle_diff(
        ws.root(),
        ws.request(DiffOptions::default()),
        "op_1",
        &registry,
    )
    .unwrap();
    let log_id = outcome.response.output.as_ref().unwrap().log_id.clone();
    let records = drain_all(&registry, &log_id);

    let kinds: Vec<DiffOutputRecordKind> = records.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            DiffOutputRecordKind::FileStarted,
            DiffOutputRecordKind::PatchBytes,
            DiffOutputRecordKind::FileFinished,
        ]
    );
    // Every record correlates by scope + file_id without parsing the patch text.
    for r in &records {
        assert_eq!(r.file_id.as_deref(), Some("@root#0"));
        assert_eq!(r.scope.as_ref().unwrap().root, Some(true));
        assert!(r.entry.is_none(), "echo off by default");
    }
}

#[test]
fn member_patch_bytes_are_workspace_relative() {
    let ws = Workspace::new("wsrel");
    let member = ws.add_member("mem_a", "crate-a");
    Workspace::write(&member, "src/lib.rs", b"fn a() {}\n");
    Workspace::commit(&member, "init");
    Workspace::write(&member, "src/lib.rs", b"fn a() { 1 }\n");

    let registry = DiffLogRegistry::new();
    let outcome = handle_diff(
        ws.root(),
        ws.request(DiffOptions::default()),
        "op_1",
        &registry,
    )
    .unwrap();
    let log_id = outcome.response.output.as_ref().unwrap().log_id.clone();
    let text = patch_text(&drain_all(&registry, &log_id));

    assert!(
        text.contains("diff --git a/crate-a/src/lib.rs b/crate-a/src/lib.rs"),
        "member-prefixed diff header, got:\n{text}"
    );
    assert!(text.contains("--- a/crate-a/src/lib.rs"), "got:\n{text}");
    assert!(text.contains("+++ b/crate-a/src/lib.rs"), "got:\n{text}");

    // The manifest path is workspace-relative too.
    assert_eq!(
        outcome.response.files[0].new_path.as_deref(),
        Some("crate-a/src/lib.rs")
    );
}

#[test]
fn raw_bytes_are_raw_records_not_patch() {
    let ws = Workspace::new("raw-root");
    Workspace::write(ws.root(), "a.txt", b"one\n");
    Workspace::commit(ws.root(), "init");
    Workspace::write(ws.root(), "a.txt", b"two\n");

    let registry = DiffLogRegistry::new();
    let options = DiffOptions {
        output_format: Some(DiffOutputFormat::Raw),
        ..Default::default()
    };
    let outcome = handle_diff(ws.root(), ws.request(options), "op_1", &registry).unwrap();

    assert_eq!(
        outcome.response.output.as_ref().unwrap().format,
        DiffOutputFormat::Raw
    );
    let log_id = outcome.response.output.as_ref().unwrap().log_id.clone();
    let text = patch_text(&drain_all(&registry, &log_id));
    assert!(
        text.starts_with(":100644 100644 "),
        "expected raw record, got:\n{text}"
    );
    assert!(text.contains(" M\ta.txt"), "got:\n{text}");
    assert!(!text.contains("diff --git"), "got:\n{text}");
    assert!(!text.contains("@@"), "got:\n{text}");
}

#[test]
fn member_raw_bytes_are_workspace_relative() {
    let ws = Workspace::new("raw-member");
    let member = ws.add_member("mem_a", "crate-a");
    Workspace::write(&member, "src/lib.rs", b"fn a() {}\n");
    Workspace::commit(&member, "init");
    Workspace::write(&member, "src/lib.rs", b"fn a() { 1 }\n");

    let registry = DiffLogRegistry::new();
    let options = DiffOptions {
        output_format: Some(DiffOutputFormat::Raw),
        ..Default::default()
    };
    let outcome = handle_diff(ws.root(), ws.request(options), "op_1", &registry).unwrap();
    let log_id = outcome.response.output.as_ref().unwrap().log_id.clone();
    let text = patch_text(&drain_all(&registry, &log_id));

    assert!(
        text.contains(" M\tcrate-a/src/lib.rs"),
        "member raw path must be workspace-relative, got:\n{text}"
    );
    assert!(
        !text.contains(" M\tsrc/lib.rs"),
        "member raw path leaked repo-relative form:\n{text}"
    );
}

#[test]
fn rename_headers_survive_end_to_end() {
    let ws = Workspace::new("rename");
    let member = ws.add_member("mem_a", "crate-a");
    let body = "line1\nline2\nline3\nline4\nline5\n";
    Workspace::write(&member, "old_name.txt", body.as_bytes());
    Workspace::commit(&member, "init");
    // Rename and STAGE it so it is a tracked index-side rename that `--cached`
    // (index-vs-tree) detects; plain worktree-vs-index would treat the new name
    // as untracked and never pair it, matching git.
    Workspace::remove(&member, "old_name.txt");
    Workspace::write(&member, "new_name.txt", body.as_bytes());
    super::workspace_fixture::run_git(&member, &["add", "-A"]);

    let registry = DiffLogRegistry::new();
    let mut request = ws.request(DiffOptions {
        find_renames: Some(true),
        output_format: Some(DiffOutputFormat::Patch),
        ..Default::default()
    });
    request.cached = Some(true);
    let outcome = handle_diff(ws.root(), request, "op_1", &registry).unwrap();

    // Manifest keeps it a rename with both workspace-relative paths.
    let entry = &outcome.response.files[0];
    assert_eq!(entry.status, DiffStatus::Renamed);
    assert_eq!(entry.old_path.as_deref(), Some("crate-a/old_name.txt"));
    assert_eq!(entry.new_path.as_deref(), Some("crate-a/new_name.txt"));

    // The patch carries member-prefixed rename headers, not add/delete.
    let log_id = outcome.response.output.as_ref().unwrap().log_id.clone();
    let text = patch_text(&drain_all(&registry, &log_id));
    assert!(
        text.contains("rename from crate-a/old_name.txt"),
        "got:\n{text}"
    );
    assert!(
        text.contains("rename to crate-a/new_name.txt"),
        "got:\n{text}"
    );
    assert!(
        !text.contains("/dev/null"),
        "must not degrade to add/delete"
    );
}

#[test]
fn metadata_only_format_omits_output_log() {
    let ws = Workspace::new("nameonly");
    Workspace::write(ws.root(), "a.txt", b"one\n");
    Workspace::commit(ws.root(), "init");
    Workspace::write(ws.root(), "a.txt", b"two\n");

    let registry = DiffLogRegistry::new();
    let options = DiffOptions {
        output_format: Some(DiffOutputFormat::NameStatus),
        ..Default::default()
    };
    let outcome = handle_diff(ws.root(), ws.request(options), "op_1", &registry).unwrap();

    // Manifest is populated; no output log ref (client answers name-status from it).
    assert!(outcome.response.output.is_none());
    assert!(outcome.log_id.is_none());
    assert_eq!(outcome.response.files.len(), 1);
    assert_eq!(outcome.response.summary.as_ref().unwrap().files_changed, 1);
}

#[test]
fn quiet_any_difference_short_circuits_without_files_or_log() {
    let ws = Workspace::new("quiet");
    Workspace::write(ws.root(), "a.txt", b"one\n");
    Workspace::commit(ws.root(), "init");
    Workspace::write(ws.root(), "a.txt", b"two\n");

    let registry = DiffLogRegistry::new();
    let options = DiffOptions {
        manifest_mode: Some(DiffManifestMode::AnyDifference),
        ..Default::default()
    };
    let outcome = handle_diff(ws.root(), ws.request(options), "op_1", &registry).unwrap();

    assert!(outcome.response.output.is_none());
    assert!(
        outcome.response.files.is_empty(),
        "files omitted in --quiet"
    );
    let summary = outcome.response.summary.as_ref().unwrap();
    assert!(summary.has_differences, "exit-code signal is set");
}

#[test]
fn quiet_reports_no_difference_when_clean() {
    let ws = Workspace::new("quiet-clean");
    Workspace::write(ws.root(), "a.txt", b"one\n");
    Workspace::commit(ws.root(), "init");
    // No worktree edits: clean.

    let registry = DiffLogRegistry::new();
    let options = DiffOptions {
        manifest_mode: Some(DiffManifestMode::AnyDifference),
        ..Default::default()
    };
    let outcome = handle_diff(ws.root(), ws.request(options), "op_1", &registry).unwrap();
    assert!(!outcome.response.summary.as_ref().unwrap().has_differences);
}

#[test]
fn parsed_targets_include_tree_oids() {
    let ws = Workspace::new("target-oids");
    Workspace::write(ws.root(), "a.txt", b"a1\n");
    Workspace::commit(ws.root(), "c1");
    Workspace::write(ws.root(), "a.txt", b"a2\n");
    Workspace::commit(ws.root(), "c2");

    let registry = DiffLogRegistry::new();
    let mut request = ws.request_operands(&["HEAD~1", "HEAD"], &[], "");
    request.options = Some(DiffOptions {
        output_format: Some(DiffOutputFormat::NoPatch),
        ..Default::default()
    });
    let outcome = handle_diff(ws.root(), request, "op_1", &registry).unwrap();

    let target = outcome
        .response
        .targets
        .iter()
        .find(|t| t.target_id == "@root")
        .expect("root parsed target");
    assert_eq!(
        target.left_oid.as_deref(),
        Some(git_tree_oid(ws.root(), "HEAD~1").as_str())
    );
    assert_eq!(
        target.right_oid.as_deref(),
        Some(git_tree_oid(ws.root(), "HEAD").as_str())
    );
    assert_eq!(target.merge_base_oid, None);
}

#[test]
fn parsed_targets_preserve_merge_base_oid() {
    let ws = Workspace::new("target-merge-base");
    Workspace::write(ws.root(), "base.txt", b"base\n");
    Workspace::commit(ws.root(), "base");
    super::workspace_fixture::run_git(ws.root(), &["checkout", "-b", "topic"]);
    Workspace::write(ws.root(), "topic.txt", b"topic\n");
    Workspace::commit(ws.root(), "topic work");
    super::workspace_fixture::run_git(ws.root(), &["checkout", "main"]);
    Workspace::write(ws.root(), "main.txt", b"main\n");
    Workspace::commit(ws.root(), "main work");

    let registry = DiffLogRegistry::new();
    let mut request = ws.request_operands(&["main...topic"], &[], "");
    request.options = Some(DiffOptions {
        output_format: Some(DiffOutputFormat::NoPatch),
        ..Default::default()
    });
    let outcome = handle_diff(ws.root(), request, "op_1", &registry).unwrap();
    let base_commit = git_stdout(ws.root(), &["merge-base", "main", "topic"])
        .trim()
        .to_owned();

    let target = outcome
        .response
        .targets
        .iter()
        .find(|t| t.target_id == "@root")
        .expect("root parsed target");
    assert_eq!(
        target.left_oid.as_deref(),
        Some(git_tree_oid(ws.root(), "main").as_str())
    );
    assert_eq!(
        target.right_oid.as_deref(),
        Some(git_tree_oid(ws.root(), "topic").as_str())
    );
    assert_eq!(
        target.merge_base_oid.as_deref(),
        Some(git_tree_oid(ws.root(), &base_commit).as_str())
    );
}

#[test]
fn root_diff_excludes_member_directories() {
    let ws = Workspace::new("rootexcl");
    let member = ws.add_member("mem_a", "crate-a");
    // Commit the root without the member content, then materialize + dirty the
    // member. The member dir is untracked at the root; even so, exercise that a
    // staged member path would be filtered. Here we simply assert the root diff
    // never surfaces the member path.
    Workspace::write(ws.root(), "root.txt", b"r1\n");
    Workspace::commit(ws.root(), "root init");
    Workspace::write(&member, "a.txt", b"a1\n");
    Workspace::commit(&member, "a init");
    Workspace::write(ws.root(), "root.txt", b"r2\n");
    Workspace::write(&member, "a.txt", b"a2\n");

    let registry = DiffLogRegistry::new();
    let outcome = handle_diff(
        ws.root(),
        ws.request(DiffOptions::default()),
        "op_1",
        &registry,
    )
    .unwrap();

    // No root-scoped file entry names anything under crate-a/.
    for f in &outcome.response.files {
        if f.scope.root == Some(true) {
            let p = f.new_path.as_deref().unwrap_or("");
            assert!(
                !p.starts_with("crate-a/"),
                "root diff leaked member path {p}"
            );
        }
    }
}

#[test]
fn is_repository_backend_helper_available() {
    // Guard that the materialization oracle's building block exists and behaves.
    let ws = Workspace::new("isrepo");
    assert!(
        Git2Backend::new().is_repository(ws.root()).unwrap(),
        "workspace root is a git repo"
    );
}

// ── D5 bare-operand classification (git's rev/path split) ────────────────────

/// Unwrap the error from a `handle_diff` result (`DiffOutcome` is not `Debug`,
/// so `unwrap_err` cannot be used directly).
fn expect_err(
    result: crate::model::ModelResult<crate::diff::DiffOutcome>,
) -> crate::model::ModelError {
    match result {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(err) => err,
    }
}

/// The workspace-relative new_path of every file entry, sorted.
fn changed_paths(outcome: &crate::diff::DiffOutcome) -> Vec<String> {
    let mut paths: Vec<String> = outcome
        .response
        .files
        .iter()
        .filter_map(|f| f.new_path.clone().or_else(|| f.old_path.clone()))
        .collect();
    paths.sort();
    paths
}

#[test]
fn bare_file_operand_equals_dashdash_file_form() {
    // The user's exact repro: `gwz diff <file>` (no `--`) must behave like
    // `gwz diff -- <file>`, not fail as an unknown revspec.
    let ws = Workspace::new("bare-file");
    Workspace::write(ws.root(), "a.txt", b"a1\n");
    Workspace::write(ws.root(), "b.txt", b"b1\n");
    Workspace::commit(ws.root(), "init");
    Workspace::write(ws.root(), "a.txt", b"a2\n");
    Workspace::write(ws.root(), "b.txt", b"b2\n");

    let registry = DiffLogRegistry::new();
    let bare = handle_diff(
        ws.root(),
        ws.request_operands(&["a.txt"], &[], ""),
        "op_bare",
        &registry,
    )
    .unwrap();
    let dashdash = handle_diff(
        ws.root(),
        ws.request_operands(&[], &["a.txt"], ""),
        "op_dd",
        &registry,
    )
    .unwrap();

    assert_eq!(changed_paths(&bare), vec!["a.txt".to_owned()]);
    assert_eq!(changed_paths(&bare), changed_paths(&dashdash));
}

#[test]
fn bare_member_subdir_operand_routes_as_pathspec() {
    let ws = Workspace::new("bare-member");
    let member = ws.add_member("mem_a", "crate-a");
    Workspace::write(ws.root(), "root.txt", b"r1\n");
    Workspace::commit(ws.root(), "root init");
    Workspace::write(&member, "src/a.txt", b"a1\n");
    Workspace::commit(&member, "a init");
    // Dirty both root and the member subdir; the operand `crate-a/src` must scope
    // to the member and only surface files under `src/`.
    Workspace::write(ws.root(), "root.txt", b"r2\n");
    Workspace::write(&member, "src/a.txt", b"a2\n");

    let registry = DiffLogRegistry::new();
    let outcome = handle_diff(
        ws.root(),
        ws.request_operands(&["crate-a/src"], &[], ""),
        "op_1",
        &registry,
    )
    .unwrap();

    // Only the member file surfaces; root.txt is out of the pathspec's scope.
    assert_eq!(
        changed_paths(&outcome),
        vec!["crate-a/src/a.txt".to_owned()]
    );
}

#[test]
fn bare_operand_from_member_subdir_stats_against_physical_cwd() {
    // Regression: `gwz diff a.txt` invoked from *inside* a member subdir. The
    // physical `start` dir (not the workspace root) is the base a bare path
    // operand stats against — `handle_diff` derives the logical cwd from `start`
    // vs the resolved root, so `a.txt` resolves to `crate-a/src/a.txt` and
    // classifies as a pathspec. Before the cwd fix this stat'd against the root
    // (`<root>/a.txt`, absent) and misfired as "unknown revision or path".
    let ws = Workspace::new("subdir-cwd");
    let member = ws.add_member("mem_a", "crate-a");
    Workspace::write(ws.root(), "root.txt", b"r1\n");
    Workspace::commit(ws.root(), "root init");
    Workspace::write(&member, "src/a.txt", b"a1\n");
    Workspace::commit(&member, "a init");
    Workspace::write(&member, "src/a.txt", b"a2\n");

    let subdir = member.join("src");
    let registry = DiffLogRegistry::new();
    // `start` is the deep physical cwd; `workspace_cwd` is left empty on the
    // request to prove the handler recomputes it from `start`, not the client hint.
    let outcome = handle_diff(
        &subdir,
        ws.request_operands(&["a.txt"], &[], ""),
        "op_1",
        &registry,
    )
    .unwrap();

    assert_eq!(
        changed_paths(&outcome),
        vec!["crate-a/src/a.txt".to_owned()]
    );
}

#[test]
fn mixed_head_and_file_operand_works() {
    // `gwz diff HEAD a.txt` — a revision then a bare file, no `--`.
    let ws = Workspace::new("mixed");
    Workspace::write(ws.root(), "a.txt", b"a1\n");
    Workspace::write(ws.root(), "b.txt", b"b1\n");
    Workspace::commit(ws.root(), "init");
    Workspace::write(ws.root(), "a.txt", b"a2\n");
    Workspace::write(ws.root(), "b.txt", b"b2\n");

    let registry = DiffLogRegistry::new();
    let outcome = handle_diff(
        ws.root(),
        ws.request_operands(&["HEAD", "a.txt"], &[], ""),
        "op_1",
        &registry,
    )
    .unwrap();

    // HEAD is the old side; the pathspec limits the diff to a.txt.
    assert_eq!(changed_paths(&outcome), vec!["a.txt".to_owned()]);
}

#[test]
fn nonexistent_operand_reports_improved_message() {
    let ws = Workspace::new("nonexistent");
    Workspace::write(ws.root(), "a.txt", b"a1\n");
    Workspace::commit(ws.root(), "init");

    let registry = DiffLogRegistry::new();
    let err = expect_err(handle_diff(
        ws.root(),
        ws.request_operands(&["does-not-exist"], &[], ""),
        "op_1",
        &registry,
    ));
    assert!(
        err.message.contains("unknown revision or path"),
        "{}",
        err.message
    );
    assert!(err.message.contains("--"), "{}", err.message);
}

#[test]
fn ambiguous_operand_branch_named_like_file_errors() {
    // A branch named exactly like an existing file → git's ambiguous error.
    let ws = Workspace::new("ambiguous");
    Workspace::write(ws.root(), "a.txt", b"a1\n");
    Workspace::commit(ws.root(), "init");
    Workspace::write(ws.root(), "ambig", b"file\n");
    // Create a branch literally named `ambig` at HEAD; the worktree file `ambig`
    // now collides.
    super::workspace_fixture::run_git(ws.root(), &["branch", "ambig"]);

    let registry = DiffLogRegistry::new();
    let err = expect_err(handle_diff(
        ws.root(),
        ws.request_operands(&["ambig"], &[], ""),
        "op_1",
        &registry,
    ));
    assert!(err.message.contains("ambiguous"), "{}", err.message);
    assert!(err.message.contains("--"), "{}", err.message);
}

#[test]
fn dashdash_operands_before_are_revs_only() {
    // With `--` present, an operand before it is never stat-checked as a path:
    // `HEAD` before `--` is a revision, and the file after `--` is the pathspec.
    let ws = Workspace::new("dashdash");
    Workspace::write(ws.root(), "a.txt", b"a1\n");
    Workspace::commit(ws.root(), "init");
    Workspace::write(ws.root(), "a.txt", b"a2\n");

    let registry = DiffLogRegistry::new();
    let outcome = handle_diff(
        ws.root(),
        ws.request_operands(&["HEAD"], &["a.txt"], ""),
        "op_1",
        &registry,
    )
    .unwrap();
    assert_eq!(changed_paths(&outcome), vec!["a.txt".to_owned()]);
}
