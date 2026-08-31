//! S2.1 requirement-row tests for default HEAD cursors.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Commit, ObjectType, Oid, Repository, Signature, Time};

use crate::artifact::{
    ArtifactSourceKind, CreatedByArtifact, LOCK_SCHEMA, LockArtifact, ManifestArtifact,
    ManifestMember, RemoteArtifact, ResolvedMemberArtifact, SNAPSHOT_SCHEMA, SnapshotArtifact,
    WORKSPACE_SCHEMA, WorkspaceHeader,
};

use super::merge::{CommitLogMergedEvent, stream_request_histories};
use super::*;

#[test]
fn l_sel_2_default_selection_includes_root_and_all_active_members() {
    let fixture = Fixture::new("default-selection");
    Repository::init(fixture.path().join("app")).unwrap();
    Repository::init(fixture.path().join("lib")).unwrap();
    Repository::init(fixture.path().join("old")).unwrap();
    fixture.write_manifest(&[
        member("mem_app", "app", true),
        member("mem_lib", "lib", true),
        member("mem_old", "old", false),
    ]);

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(
        histories
            .iter()
            .map(|history| history.target().member_id.as_str())
            .collect::<Vec<_>>(),
        ["@root", "mem_app", "mem_lib"]
    );
    assert_eq!(
        histories
            .iter()
            .map(|history| history.target().member_path.as_str())
            .collect::<Vec<_>>(),
        [".", "app", "lib"]
    );
}

#[test]
fn l_rng_2_no_operand_histories_start_at_each_repository_head() {
    let fixture = Fixture::new("head-history");
    let root_first = commit(&fixture.root, "root first", 100, &[]);
    let root_head = commit(&fixture.root, "root head", 200, &[root_first]);
    let member_repo = Repository::init(fixture.path().join("app")).unwrap();
    let member_first = commit(&member_repo, "member first", 300, &[]);
    let member_head = commit(&member_repo, "member head", 400, &[member_first]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let later_root = commit(&fixture.root, "after cursor open", 500, &[root_head]);

    assert_eq!(entry_ids(&histories[0]), [root_head, root_first]);
    assert_eq!(entry_ids(&histories[1]), [member_head, member_first]);
    assert_eq!(fixture.root.head().unwrap().target(), Some(later_root));
    let Some(CommitLogEvent::Entry(entry)) = histories[0].messages().next() else {
        panic!("root cursor did not emit a structured entry");
    };
    assert_eq!(entry.target.member_id, "@root");
    assert_eq!(entry.parent_ids, [root_first.to_string()]);
    assert_eq!(entry.message, b"root head");
    assert_eq!(entry.author.name, b"Test Author");
    assert_eq!(entry.committer.time.seconds, 200);
}

#[test]
fn entry_preserves_leading_newlines_from_the_raw_commit_message() {
    let fixture = Fixture::new("raw-leading-newline");
    let expected = b"\nleading newline is data\n";
    raw_commit(&fixture.root, expected, None);
    fixture.write_manifest(&[]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let Some(CommitLogEvent::Entry(entry)) = histories[0].messages().next() else {
        panic!("root cursor did not emit the raw commit");
    };

    assert_eq!(entry.message, expected);
}

#[test]
fn entry_preserves_non_utf8_encoding_header_bytes() {
    let fixture = Fixture::new("raw-non-utf8-encoding");
    let expected = b"ISO-8859-\xff";
    raw_commit(&fixture.root, b"encoded message\n", Some(expected));
    fixture.write_manifest(&[]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let Some(CommitLogEvent::Entry(entry)) = histories[0].messages().next() else {
        panic!("root cursor did not emit the raw commit");
    };
    assert_eq!(entry.message_encoding.as_deref(), Some(expected.as_slice()));
}

#[test]
fn l_rng_5_local_read_is_network_free_and_does_not_take_the_mutation_lock() {
    let fixture = Fixture::new("local-read");
    let head = commit(&fixture.root, "local", 100, &[]);
    fixture
        .root
        .remote("origin", "file:///definitely/absent/must-not-connect")
        .unwrap();
    fixture.write_manifest(&[]);
    let git_head_path = fixture.root.path().join("HEAD");
    let git_config_path = fixture.root.path().join("config");
    let before_head = fs::read(&git_head_path).unwrap();
    let before_config = fs::read(&git_config_path).unwrap();

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[0]), [head]);
    assert_eq!(fs::read(git_head_path).unwrap(), before_head);
    assert_eq!(fs::read(git_config_path).unwrap(), before_config);
    assert!(!fixture.root.path().join("FETCH_HEAD").exists());
    assert!(!fixture.path().join(".gwz").exists());
}

#[cfg(unix)]
#[test]
fn f1_path_history_is_offline_and_read_only_for_a_promisor_clone() {
    use std::os::unix::fs::PermissionsExt;

    let origin = Fixture::new("promisor-origin");
    commit_file(&origin.root, "p", b"remote tree\n", "origin", 100, &[]);
    git_ok(origin.path(), &["config", "uploadpack.allowFilter", "true"]);
    git_ok(
        origin.path(),
        &["config", "uploadpack.allowAnySHA1InWant", "true"],
    );

    let partial = origin.path().join("partial");
    let origin_url = format!("file://{}", origin.path().display());
    let status = Command::new("git")
        .args([
            "-c",
            "protocol.file.allow=always",
            "clone",
            "--quiet",
            "--no-local",
            "--filter=tree:0",
            "--no-checkout",
            &origin_url,
            partial.to_str().expect("UTF-8 temp path"),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "partial clone failed with {status}");
    write_manifest_at(&partial, &[]);

    let unavailable = Command::new("git")
        .arg("--git-dir")
        .arg(partial.join(".git"))
        .args(["cat-file", "-e", "HEAD^{tree}"])
        .env("GIT_NO_LAZY_FETCH", "1")
        .status()
        .unwrap();
    assert!(!unavailable.success(), "tree unexpectedly exists locally");

    // A lazy fetch would invoke this configured transport-side helper before
    // the real upload-pack. Its marker makes transport execution observable.
    let upload_pack = partial.join("observe-upload-pack");
    let upload_pack_marker = partial.join("observe-upload-pack.called");
    fs::write(
        &upload_pack,
        "#!/bin/sh\ntouch \"$0.called\"\nexec git-upload-pack \"$@\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&upload_pack).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&upload_pack, permissions).unwrap();
    git_ok(
        &partial,
        &[
            "config",
            "remote.origin.uploadpack",
            upload_pack.to_str().expect("UTF-8 temp path"),
        ],
    );

    let before = repository_bytes(&partial.join(".git"));
    let opened = open_request_histories(&partial, &log_request(&[], &["p"], false)).unwrap();
    let actual = events(&opened.histories()[0]);

    assert!(matches!(
        actual.as_slice(),
        [CommitLogEvent::Degradation(CommitLogDegradation {
            kind: CommitLogDegradationKind::HistoryUnreadable,
            ..
        })]
    ));
    assert!(
        !upload_pack_marker.exists(),
        "path history invoked a transport helper"
    );
    assert_eq!(repository_bytes(&partial.join(".git")), before);
}

#[test]
fn l_ord_1_cursor_matches_git_log_default_order() {
    let fixture = Fixture::new("git-default-order");
    let base = commit(&fixture.root, "base", 50, &[]);
    let left_one = commit(&fixture.root, "left one", 100, &[base]);
    let left_two = commit(&fixture.root, "left two", 400, &[left_one]);
    let right = detached_commit(&fixture.root, "right", 300, &[base]);
    let merge = commit(&fixture.root, "merge", 500, &[left_two, right]);
    fixture.write_manifest(&[]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let actual = entry_ids(&histories[0]);
    let output = Command::new("git")
        .args(["-C", fixture.path().to_str().unwrap(), "log", "--format=%H"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let expected = String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(|line| Oid::from_str(line).unwrap())
        .collect::<Vec<_>>();
    let topo_output = Command::new("git")
        .args([
            "-C",
            fixture.path().to_str().unwrap(),
            "log",
            "--topo-order",
            "--format=%H",
        ])
        .output()
        .unwrap();

    assert_eq!(actual, expected);
    assert_eq!(actual.first(), Some(&merge));
    assert_ne!(output.stdout, topo_output.stdout);
}

#[test]
fn l_tol_1_unreadable_member_degrades_without_stopping_other_histories() {
    let fixture = Fixture::new("degrade-one");
    let root_head = commit(&fixture.root, "root", 100, &[]);
    let good_repo = Repository::init(fixture.path().join("good")).unwrap();
    let good_head = commit(&good_repo, "good", 200, &[]);
    fs::create_dir(fixture.path().join("bad")).unwrap();
    let mut generated = member("mem_generated", "generated", true);
    generated.source_kind = ArtifactSourceKind::Generated;
    fixture.write_manifest(&[
        member("mem_good", "good", true),
        member("mem_bad", "bad", true),
        generated,
    ]);

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[0]), [root_head]);
    assert_eq!(entry_ids(&histories[1]), [good_head]);
    assert_eq!(
        degradations(&histories[2])
            .iter()
            .map(|record| record.kind)
            .collect::<Vec<_>>(),
        [CommitLogDegradationKind::RepositoryUnreadable]
    );
    assert_eq!(
        degradations(&histories[3])[0].kind,
        CommitLogDegradationKind::UnsupportedSourceKind
    );
}

#[test]
fn l_tol_3_unborn_repository_contributes_no_entries_and_a_degradation() {
    let fixture = Fixture::new("unborn");
    commit(&fixture.root, "root", 100, &[]);
    Repository::init(fixture.path().join("empty")).unwrap();
    fixture.write_manifest(&[member("mem_empty", "empty", true)]);

    let histories = open_default_head_histories(fixture.path()).unwrap();
    let events = histories[1].messages().collect::<Vec<_>>();

    assert!(
        events
            .iter()
            .all(|event| !matches!(event, CommitLogEvent::Entry(_)))
    );
    assert!(matches!(
        events.as_slice(),
        [CommitLogEvent::Degradation(CommitLogDegradation {
            kind: CommitLogDegradationKind::UnbornHead,
            ..
        })]
    ));
}

#[test]
fn l_tol_4_detached_member_logs_normally_from_the_detached_commit() {
    let fixture = Fixture::new("detached");
    commit(&fixture.root, "root", 100, &[]);
    let member_repo = Repository::init(fixture.path().join("app")).unwrap();
    let detached = commit(&member_repo, "detached target", 200, &[]);
    commit(&member_repo, "branch head", 300, &[detached]);
    member_repo.set_head_detached(detached).unwrap();
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[1]), [detached]);
}

#[test]
fn l_tol_5_shallow_member_contributes_every_locally_available_commit() {
    let fixture = Fixture::new("shallow");
    commit(&fixture.root, "root", 100, &[]);
    let member_repo = Repository::init(fixture.path().join("app")).unwrap();
    let first = commit(&member_repo, "first", 200, &[]);
    let head = commit(&member_repo, "head", 300, &[first]);
    fs::write(member_repo.path().join("shallow"), format!("{head}\n")).unwrap();
    drop(member_repo);
    assert!(
        Repository::open(fixture.path().join("app"))
            .unwrap()
            .is_shallow()
    );
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[1]), [head]);
}

#[test]
fn l_tol_6_conf_integrity_mismatch_does_not_gate_history_reads() {
    let fixture = Fixture::new("no-conf-gate");
    let head = commit(&fixture.root, "root", 100, &[]);
    fixture.write_manifest(&[]);
    crate::artifact::refresh_conf_integrity_marker(fixture.path()).unwrap();
    let manifest_path = fixture.path().join(crate::workspace::WORKSPACE_MANIFEST);
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    fs::write(&manifest_path, format!("{manifest}\n")).unwrap();
    assert!(crate::artifact::inspect_conf_integrity(fixture.path()).refuses());

    let histories = open_default_head_histories(fixture.path()).unwrap();

    assert_eq!(entry_ids(&histories[0]), [head]);
}

#[test]
fn l_rng_1_zero_or_more_revision_operands_use_the_diff_grammar() {
    let fixture = Fixture::new("multiple-revisions");
    let base = commit(&fixture.root, "base", 100, &[]);
    let left = detached_commit(&fixture.root, "left", 200, &[base]);
    let middle = detached_commit(&fixture.root, "middle", 300, &[base]);
    let right = detached_commit(&fixture.root, "right", 400, &[base]);
    fixture
        .root
        .reference("refs/heads/left", left, true, "test ref")
        .unwrap();
    fixture
        .root
        .reference("refs/heads/middle", middle, true, "test ref")
        .unwrap();
    fixture
        .root
        .reference("refs/heads/right", right, true, "test ref")
        .unwrap();
    fixture.write_manifest(&[]);

    let opened = open_request_histories(
        fixture.path(),
        &log_request(&["left", "middle", "right"], &[], false),
    )
    .unwrap();
    let mut actual = entry_ids(&opened.histories()[0]);
    actual.sort();
    let mut expected = vec![base, left, middle, right];
    expected.sort();

    assert_eq!(actual, expected);
}

#[test]
fn l_rng_1_three_dot_range_uses_symmetric_history() {
    let fixture = Fixture::new("symmetric-range");
    let base = commit(&fixture.root, "base", 100, &[]);
    let left = detached_commit(&fixture.root, "left", 200, &[base]);
    let right = detached_commit(&fixture.root, "right", 300, &[base]);
    fixture
        .root
        .reference("refs/heads/left", left, true, "test ref")
        .unwrap();
    fixture
        .root
        .reference("refs/heads/right", right, true, "test ref")
        .unwrap();
    fixture.write_manifest(&[]);

    let opened =
        open_request_histories(fixture.path(), &log_request(&["left...right"], &[], false))
            .unwrap();
    let mut actual = entry_ids(&opened.histories()[0]);
    actual.sort();
    let mut expected = vec![left, right];
    expected.sort();

    assert_eq!(actual, expected);
}

#[test]
fn f7_three_dot_hides_all_best_criss_cross_merge_bases() {
    let fixture = Fixture::new("criss-cross-range");
    let base = commit(&fixture.root, "base", 100, &[]);
    let left_base = detached_commit(&fixture.root, "left base", 200, &[base]);
    let right_base = detached_commit(&fixture.root, "right base", 300, &[base]);
    let left = detached_commit(&fixture.root, "left merge", 400, &[left_base, right_base]);
    let right = detached_commit(&fixture.root, "right merge", 500, &[right_base, left_base]);
    fixture
        .root
        .reference("refs/heads/left", left, true, "left")
        .unwrap();
    fixture
        .root
        .reference("refs/heads/right", right, true, "right")
        .unwrap();
    fixture.write_manifest(&[]);
    let bases = git_oid_lines(Command::new("git").arg("-C").arg(fixture.path()).args([
        "merge-base",
        "--all",
        "left",
        "right",
    ]));
    assert_eq!(bases.len(), 2, "fixture must have two best merge bases");

    let opened =
        open_request_histories(fixture.path(), &log_request(&["left...right"], &[], false))
            .unwrap();

    let mut actual = entry_ids(&opened.histories()[0]);
    let mut expected = native_log_ids(fixture.path(), &["left...right"], &[]);
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

#[test]
fn l_rng_1_post_dash_leading_plus_is_a_literal_pathspec() {
    let fixture = Fixture::new("post-dash-plus-path");
    let plus_commit = commit_file(&fixture.root, "+notes", b"one\n", "plus path", 100, &[]);
    commit_file(
        &fixture.root,
        "other.txt",
        b"two\n",
        "other path",
        200,
        &[plus_commit],
    );
    fixture.write_manifest(&[]);

    // No snapshot named `notes` exists. Success proves the post-`--` token never
    // entered the leading-`+` operand grammar.
    let opened =
        open_request_histories(fixture.path(), &log_request(&[], &["+notes"], false)).unwrap();

    assert_eq!(opened.histories()[0].pathspecs(), ["+notes"]);
    assert_eq!(entry_ids(&opened.histories()[0]), [plus_commit]);
}

#[test]
fn f7_bare_pre_dash_path_uses_the_shared_classifier() {
    let fixture = Fixture::new("bare-path");
    let path_commit = commit_file(&fixture.root, "notes", b"one\n", "notes", 100, &[]);
    commit_file(
        &fixture.root,
        "other",
        b"two\n",
        "other",
        200,
        &[path_commit],
    );
    fixture.write_manifest(&[]);

    let opened =
        open_request_histories(fixture.path(), &log_request(&["notes"], &[], false)).unwrap();

    assert_eq!(entry_ids(&opened.histories()[0]), [path_commit]);
}

#[test]
fn f8_log_classifier_diagnostics_name_log_for_ambiguous_and_unknown_operands() {
    let fixture = Fixture::new("log-classifier-diagnostic");
    let head = commit_file(&fixture.root, "topic", b"path\n", "topic", 100, &[]);
    fixture
        .root
        .reference("refs/heads/topic", head, true, "topic")
        .unwrap();
    fixture.write_manifest(&[]);

    for (operand, reason) in [
        ("topic", "both a revision and a path exist"),
        (
            "unknown",
            "unknown revision or path not in the working tree",
        ),
    ] {
        let error = open_request_histories(fixture.path(), &log_request(&[operand], &[], false))
            .err()
            .expect("classifier must reject the operand");
        assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
        assert!(error.message.contains(reason), "{}", error.message);
        assert!(
            error
                .message
                .contains("'gwz log [<revision>...] -- [<file>...]'"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("gwz diff"), "{}", error.message);
    }
}

#[test]
fn l_rng_1_pathspec_history_matches_git_merge_simplification() {
    let fixture = Fixture::new("path-merge-simplification");
    git_ok(fixture.path(), &["config", "user.name", "Test Author"]);
    git_ok(
        fixture.path(),
        &["config", "user.email", "test@example.com"],
    );
    fs::write(fixture.path().join("p"), "base\n").unwrap();
    git_ok(fixture.path(), &["add", "p"]);
    git_ok(fixture.path(), &["commit", "--quiet", "-m", "base"]);
    git_ok(fixture.path(), &["branch", "feature"]);
    fs::write(fixture.path().join("q"), "main\n").unwrap();
    git_ok(fixture.path(), &["add", "q"]);
    git_ok(fixture.path(), &["commit", "--quiet", "-m", "main"]);
    git_ok(fixture.path(), &["checkout", "--quiet", "feature"]);
    fs::write(fixture.path().join("p"), "feature\n").unwrap();
    git_ok(fixture.path(), &["commit", "--quiet", "-am", "feature"]);
    git_ok(fixture.path(), &["checkout", "--quiet", "main"]);
    git_ok(
        fixture.path(),
        &["merge", "--quiet", "--no-ff", "feature", "-m", "merge"],
    );
    fixture.write_manifest(&[]);

    let opened = open_request_histories(fixture.path(), &log_request(&[], &["p"], false)).unwrap();
    let actual = entry_ids(&opened.histories()[0]);
    let output = Command::new("git")
        .args([
            "-C",
            fixture.path().to_str().unwrap(),
            "log",
            "--format=%H",
            "--",
            "p",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let expected = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| Oid::from_str(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(actual, expected);
}

#[test]
fn f5_root_dot_and_companion_exclusion_match_native_git_history() {
    let fixture = Fixture::new("root-dot-history");
    build_native_path_history(fixture.path());
    fixture.write_manifest(&[]);

    for pathspecs in [vec!["."], vec![".", ":(exclude)p"]] {
        let opened =
            open_request_histories(fixture.path(), &log_request(&[], &pathspecs, false)).unwrap();
        assert_eq!(
            entry_ids(&opened.histories()[0]),
            native_log_ids(fixture.path(), &[], &pathspecs),
            "pathspecs {pathspecs:?}"
        );
    }
}

#[test]
fn f5_member_root_dot_matches_native_empty_commit_and_merge_simplification() {
    let fixture = Fixture::new("member-dot-history");
    commit(&fixture.root, "root", 50, &[]);
    let member_path = fixture.path().join("app");
    Repository::init(&member_path).unwrap();
    build_native_path_history(&member_path);
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    for pathspecs in [vec!["."], vec![".", ":(exclude)p"]] {
        let opened =
            open_request_histories(&member_path, &log_request(&[], &pathspecs, false)).unwrap();
        assert_eq!(target_ids(&opened), ["mem_app"]);
        assert_eq!(
            entry_ids(&opened.histories()[0]),
            native_log_ids(&member_path, &[], &pathspecs),
            "pathspecs {pathspecs:?}"
        );
    }
}

#[test]
fn f5_magic_pathspecs_match_native_rev_list_from_root_and_member_subdirectories() {
    let fixture = Fixture::new("magic-subdirectory-history");
    build_magic_path_history(fixture.path());
    let member_path = fixture.path().join("app");
    Repository::init(&member_path).unwrap();
    build_magic_path_history(&member_path);
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let cases = [
        vec![".", ":(exclude)artifact"],
        vec![".", ":!artifact"],
        vec![".", ":^artifact"],
        vec![":(top)src", ":(top,exclude)src/artifact"],
        vec![":/src", ":(top,exclude)src/artifact"],
    ];
    for (start, repo, target) in [
        (fixture.path().join("src"), fixture.path(), "@root"),
        (member_path.join("src"), member_path.as_path(), "mem_app"),
    ] {
        for pathspecs in &cases {
            let opened =
                open_request_histories(&start, &log_request(&[], pathspecs.as_slice(), false))
                    .unwrap();
            assert_eq!(target_ids(&opened), [target], "pathspecs {pathspecs:?}");
            assert_eq!(
                entry_ids(&opened.histories()[0]),
                native_rev_list_ids(&start, &[], pathspecs),
                "repo {} pathspecs {pathspecs:?}",
                repo.display()
            );
        }
    }
}

#[test]
fn f5_workspace_root_fanout_preserves_long_and_short_member_exclusions() {
    let fixture = Fixture::new("magic-workspace-fanout");
    build_magic_path_history(fixture.path());
    let member_path = fixture.path().join("app");
    Repository::init(&member_path).unwrap();
    build_magic_path_history(&member_path);
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    for pathspecs in [
        vec![".", ":(exclude)artifact"],
        vec![".", ":!artifact"],
        vec![".", ":^artifact"],
    ] {
        let opened =
            open_request_histories(fixture.path(), &log_request(&[], &pathspecs, false)).unwrap();
        assert_eq!(target_ids(&opened), ["@root", "mem_app"]);
        assert_eq!(
            entry_ids(&opened.histories()[0]),
            native_rev_list_ids(fixture.path(), &[], &pathspecs),
            "root pathspecs {pathspecs:?}"
        );
        assert_eq!(
            entry_ids(&opened.histories()[1]),
            native_rev_list_ids(&member_path, &[], &pathspecs),
            "member pathspecs {pathspecs:?}"
        );
    }
}

#[test]
fn l_rng_3_snapshot_resolves_independently_for_each_member() {
    let fixture = Fixture::new("snapshot-per-member");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let lib = Repository::init(fixture.path().join("lib")).unwrap();
    let app_snapshot = commit(&app, "app snapshot", 100, &[]);
    commit(&app, "app later", 200, &[app_snapshot]);
    let lib_snapshot = commit(&lib, "lib snapshot", 300, &[]);
    commit(&lib, "lib later", 400, &[lib_snapshot]);
    fixture.write_manifest(&[
        member("mem_app", "app", true),
        member("mem_lib", "lib", true),
    ]);
    fixture.write_snapshot(
        "release",
        &[
            ("mem_app", "app", Some(app_snapshot)),
            ("mem_lib", "lib", Some(lib_snapshot)),
        ],
    );

    let opened =
        open_request_histories(fixture.path(), &log_request(&["+release"], &[], false)).unwrap();

    assert_eq!(entry_ids(&opened.histories()[1])[0], app_snapshot);
    assert_eq!(entry_ids(&opened.histories()[2])[0], lib_snapshot);
}

#[test]
fn l_rng_3_snapshot_to_snapshot_range_resolves_both_endpoints() {
    let fixture = Fixture::new("snapshot-range");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let base = commit(&app, "base", 100, &[]);
    let tip = commit(&app, "tip", 200, &[base]);
    commit(&app, "head", 300, &[tip]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);
    fixture.write_snapshot("base", &[("mem_app", "app", Some(base))]);
    fixture.write_snapshot("tip", &[("mem_app", "app", Some(tip))]);

    let opened =
        open_request_histories(fixture.path(), &log_request(&["+base..+tip"], &[], false)).unwrap();

    assert_eq!(entry_ids(&opened.histories()[1]), [tip]);
}

#[test]
fn l_rng_6_log_internal_dotted_snapshot_ids_work_on_both_range_sides() {
    let fixture = Fixture::new("dotted-snapshot-ranges");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let base = commit(&app, "base", 100, &[]);
    let middle = commit(&app, "middle", 200, &[base]);
    let tip = commit(&app, "tip", 300, &[middle]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);
    fixture.write_snapshot("base.one", &[("mem_app", "app", Some(base))]);
    fixture.write_snapshot("tip.one", &[("mem_app", "app", Some(tip))]);

    for delimiter in ["..", "..."] {
        let operand = format!("+base.one{delimiter}+tip.one");
        let opened = open_request_histories(
            fixture.path(),
            &log_request(&[operand.as_str()], &[], false),
        )
        .unwrap();
        assert_eq!(
            entry_ids(&opened.histories()[1]),
            native_rev_list_ids(
                app.path().parent().unwrap(),
                &[&format!("{base}{delimiter}{tip}")],
                &[]
            ),
            "operand {operand:?}"
        );
    }
}

#[test]
fn l_rng_6_log_standalone_legacy_dotted_snapshot_ids_remain_accessible() {
    let fixture = Fixture::new("legacy-snapshot-standalone-log");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let head = commit(&app, "head", 100, &[]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);
    for id in ["adjacent..dots", ".leading", "trailing."] {
        fixture.write_legacy_snapshot(id, &[("mem_app", "app", Some(head))]);
    }

    for id in ["adjacent..dots", ".leading", "trailing."] {
        let operand = format!("+{id}");
        let opened = open_request_histories(
            fixture.path(),
            &log_request(&[operand.as_str()], &[], false),
        )
        .unwrap();
        assert_eq!(entry_ids(&opened.histories()[1]), [head], "id {id:?}");
    }
}

#[test]
fn l_rng_6_log_teaches_for_ambiguous_legacy_snapshot_range_endpoints() {
    let fixture = Fixture::new("legacy-snapshot-range-log");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let head = commit(&app, "head", 100, &[]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);
    fixture.write_snapshot("safe.one", &[("mem_app", "app", Some(head))]);
    for id in ["adjacent..dots", ".leading", "trailing."] {
        fixture.write_legacy_snapshot(id, &[("mem_app", "app", Some(head))]);
        for delimiter in ["..", "..."] {
            let operand = format!("+{id}{delimiter}+safe.one");
            let error = open_request_histories(
                fixture.path(),
                &log_request(&[operand.as_str()], &[], false),
            )
            .err()
            .expect("ambiguous legacy range endpoint must be refused");
            assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
            assert!(error.message.contains(id), "{}", error.message);
            assert!(error.message.contains("standalone"), "{}", error.message);
        }
    }
}

#[test]
fn l_rng_6_log_teaches_for_open_legacy_range_endpoints_before_shorter_matches() {
    let fixture = Fixture::new("legacy-snapshot-open-range-log");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let head = commit(&app, "head", 100, &[]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);
    for id in ["trailing", "adjacent"] {
        fixture.write_snapshot(id, &[("mem_app", "app", Some(head))]);
    }
    for id in [".leading", "trailing.", "adjacent..dots"] {
        fixture.write_legacy_snapshot(id, &[("mem_app", "app", Some(head))]);
    }
    let cases = [
        ("+.leading..", ".leading"),
        ("..+.leading", ".leading"),
        ("+.leading...", ".leading"),
        ("...+.leading", ".leading"),
        ("+trailing...", "trailing."),
        ("..+trailing.", "trailing."),
        ("+trailing....", "trailing."),
        ("...+trailing.", "trailing."),
        ("+adjacent..dots..", "adjacent..dots"),
        ("..+adjacent..dots", "adjacent..dots"),
        ("+adjacent..dots...", "adjacent..dots"),
        ("...+adjacent..dots", "adjacent..dots"),
    ];

    for (operand, id) in cases {
        let error = open_request_histories(fixture.path(), &log_request(&[operand], &[], false))
            .err()
            .expect("open legacy range endpoint must be refused");
        assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
        assert_eq!(
            error.message,
            format!(
                "snapshot id '{id}' is ambiguous as a revision-range endpoint; use '+{id}' standalone or create a range-safe snapshot id without adjacent, leading, or trailing dots"
            ),
            "operand {operand:?}"
        );
    }
}

#[test]
fn l_rng_3_snapshot_to_head_range_resolves_mixed_endpoints() {
    let fixture = Fixture::new("snapshot-head-range");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let base = commit(&app, "base", 100, &[]);
    let middle = commit(&app, "middle", 200, &[base]);
    let head = commit(&app, "head", 300, &[middle]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);
    fixture.write_snapshot("base", &[("mem_app", "app", Some(base))]);

    let opened =
        open_request_histories(fixture.path(), &log_request(&["+base..HEAD"], &[], false)).unwrap();

    assert_eq!(entry_ids(&opened.histories()[1]), [head, middle]);
}

#[test]
fn l_rng_3_missing_snapshot_member_and_root_both_degrade_with_records() {
    let fixture = Fixture::new("snapshot-degradations");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let missing = Repository::init(fixture.path().join("missing")).unwrap();
    let app_snapshot = commit(&app, "app snapshot", 100, &[]);
    commit(&missing, "missing head", 200, &[]);
    fixture.write_manifest(&[
        member("mem_app", "app", true),
        member("mem_missing", "missing", true),
    ]);
    fixture.write_snapshot("partial", &[("mem_app", "app", Some(app_snapshot))]);

    let opened =
        open_request_histories(fixture.path(), &log_request(&["+partial"], &[], false)).unwrap();

    let root = degradations(&opened.histories()[0]);
    let missing = degradations(&opened.histories()[2]);
    assert_eq!(root[0].kind, CommitLogDegradationKind::SnapshotEntryMissing);
    assert_eq!(root[0].operand.as_deref(), Some("+partial"));
    assert_eq!(
        missing[0].kind,
        CommitLogDegradationKind::SnapshotEntryMissing
    );
    assert_eq!(missing[0].operand.as_deref(), Some("+partial"));
    assert_eq!(entry_ids(&opened.histories()[1])[0], app_snapshot);
}

#[test]
fn l_rng_4_lock_resolves_each_member_to_pin_dot_dot_head_and_degrades_root() {
    let fixture = Fixture::new("lock-per-member");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let lib = Repository::init(fixture.path().join("lib")).unwrap();
    let app_pin = commit_file(&app, "app.txt", b"pin\n", "app pin", 100, &[]);
    let app_middle = commit_file(&app, "app.txt", b"middle\n", "app middle", 200, &[app_pin]);
    let app_head = commit_file(
        &app,
        "app.txt",
        b"head\n",
        "app detached head",
        300,
        &[app_middle],
    );
    app.set_head_detached(app_head).unwrap();
    let lib_pin = commit(&lib, "lib pin", 400, &[]);
    let lib_head = commit(&lib, "lib head", 500, &[lib_pin]);
    fixture.write_manifest(&[
        member("mem_app", "app", true),
        member("mem_lib", "lib", true),
    ]);
    fixture.write_lock(&[
        ("mem_app", "app", Some(app_pin)),
        ("mem_lib", "lib", Some(lib_pin)),
    ]);
    fixture.write_snapshot("lock", &[("mem_app", "app", Some(app_middle))]);

    for operand in ["+lock..HEAD", "+lock.."] {
        let opened =
            open_request_histories(fixture.path(), &log_request(&[operand], &[], false)).unwrap();

        let root = degradations(&opened.histories()[0]);
        assert_eq!(root[0].kind, CommitLogDegradationKind::LockEntryMissing);
        assert_eq!(root[0].operand.as_deref(), Some("+lock"));
        assert_eq!(entry_ids(&opened.histories()[1]), [app_head, app_middle]);
        assert_eq!(entry_ids(&opened.histories()[2]), [lib_head]);
    }

    let routed = open_request_histories(
        fixture.path(),
        &log_request(&["+lock..HEAD"], &["app"], false),
    )
    .unwrap();
    assert_eq!(target_ids(&routed), ["@root", "mem_app"]);
    assert_eq!(
        degradations(&routed.histories()[0])[0].kind,
        CommitLogDegradationKind::LockEntryMissing
    );
    assert_eq!(entry_ids(&routed.histories()[1]), [app_head, app_middle]);

    // A pre-existing snapshot named `lock` remains exact-standalone access;
    // only a range-position `+lock` is the lock pseudo-endpoint.
    let snapshot =
        open_request_histories(fixture.path(), &log_request(&["+lock"], &[], false)).unwrap();
    let ordinary =
        open_request_histories(fixture.path(), &log_request(&["HEAD"], &[], false)).unwrap();
    assert_eq!(entry_ids(&snapshot.histories()[1])[0], app_middle);
    assert_eq!(entry_ids(&ordinary.histories()[1])[0], app_head);

    let mut foreign = crate::artifact::read_lock(fixture.path()).unwrap();
    foreign.workspace_id = "ws_other".to_owned();
    crate::artifact::write_lock(fixture.path(), &foreign).unwrap();
    let error = open_request_histories(fixture.path(), &log_request(&["+lock..HEAD"], &[], false))
        .err()
        .expect("foreign workspace lock must be rejected");
    assert_eq!(error.code, crate::model::ErrorCode::SourceIdentityMismatch);
    assert_eq!(
        error.message,
        "workspace manifest and lock identify different workspaces"
    );
}

#[test]
fn l_rng_4_missing_and_unborn_lock_rows_degrade_benignly_and_strictly() {
    let fixture = Fixture::new("lock-degradations");
    commit(&fixture.root, "root", 50, &[]);
    let missing = Repository::init(fixture.path().join("missing")).unwrap();
    commit(&missing, "missing head", 100, &[]);
    Repository::init(fixture.path().join("unborn")).unwrap();
    fixture.write_manifest(&[
        member("mem_missing", "missing", true),
        member("mem_unborn", "unborn", true),
    ]);
    fixture.write_lock(&[("mem_unborn", "unborn", None)]);
    let request = log_request(&["+lock..HEAD"], &[], false);

    let benign = open_request_histories(fixture.path(), &request).unwrap();
    let missing = degradations(&benign.histories()[1]);
    let unborn = degradations(&benign.histories()[2]);
    assert_eq!(missing[0].kind, CommitLogDegradationKind::LockEntryMissing);
    assert_eq!(missing[0].operand.as_deref(), Some("+lock"));
    assert_eq!(unborn[0].kind, CommitLogDegradationKind::LockEntryMissing);
    assert_eq!(unborn[0].operand.as_deref(), Some("+lock"));
    assert_eq!(
        stream_request_histories(benign, &request, |_| {})
            .aggregate()
            .status(),
        crate::AggregateStatus::Ok
    );

    let mut strict_request = request.clone();
    strict_request.options = Some(crate::LogOptions {
        strict: Some(true),
        ..crate::LogOptions::default()
    });
    let strict = open_request_histories(fixture.path(), &strict_request).unwrap();
    assert_eq!(
        stream_request_histories(strict, &strict_request, |_| {})
            .aggregate()
            .status(),
        crate::AggregateStatus::Failed
    );
}

#[test]
fn f2_mismatched_snapshot_identity_cannot_panic_log_planning() {
    let fixture = Fixture::new("snapshot-escape-log");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let app_head = commit(&app, "app", 100, &[]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);
    let artifact =
        fixture.snapshot_artifact("embedded", "ws_test", &[("mem_app", "app", Some(app_head))]);
    fs::create_dir_all(fixture.path().join(crate::artifact::SNAPSHOT_DIR)).unwrap();
    fs::write(
        crate::artifact::snapshot_path(fixture.path(), "alias").unwrap(),
        artifact.to_yaml().unwrap(),
    )
    .unwrap();

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open_request_histories(fixture.path(), &log_request(&["+alias"], &["app"], false))
    }));
    let error = outcome
        .expect("malformed snapshot operand must not panic")
        .err()
        .expect("malformed snapshot operand must be rejected");

    assert_eq!(error.code, crate::model::ErrorCode::SnapshotNotFound);
    assert!(error.message.contains("alias"), "{}", error.message);
    assert!(error.message.contains("embedded"), "{}", error.message);
}

#[test]
fn f3_foreign_workspace_snapshot_is_rejected_before_member_resolution() {
    let fixture = Fixture::new("foreign-snapshot");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let locally_present = commit(&app, "local", 100, &[]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);
    fixture.write_snapshot_for_workspace(
        "release",
        "ws_foreign",
        &[("mem_app", "app", Some(locally_present))],
    );

    let error = open_request_histories(fixture.path(), &log_request(&["+release"], &[], false))
        .err()
        .expect("foreign snapshot must be rejected");

    assert_eq!(error.code, crate::model::ErrorCode::SnapshotNotFound);
    assert!(error.message.contains("ws_foreign"), "{}", error.message);
    assert!(error.message.contains("ws_test"), "{}", error.message);
}

#[test]
fn f4_snapshot_degradations_survive_member_path_routing() {
    let fixture = Fixture::new("snapshot-path-degradations");
    commit(&fixture.root, "root", 50, &[]);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let missing = Repository::init(fixture.path().join("missing")).unwrap();
    let app_snapshot = commit_file(&app, "file", b"one\n", "app", 100, &[]);
    commit(&missing, "missing", 150, &[]);
    fixture.write_manifest(&[
        member("mem_app", "app", true),
        member("mem_missing", "missing", true),
    ]);
    fixture.write_snapshot("partial", &[("mem_app", "app", Some(app_snapshot))]);

    let opened = open_request_histories(
        fixture.path(),
        &log_request(&["+partial"], &["app/file"], false),
    )
    .unwrap();

    assert_eq!(target_ids(&opened), ["@root", "mem_app", "mem_missing"]);
    assert!(matches!(
        events(&opened.histories()[0]).as_slice(),
        [CommitLogEvent::Degradation(CommitLogDegradation {
            kind: CommitLogDegradationKind::SnapshotEntryMissing,
            ..
        })]
    ));
    assert_eq!(entry_ids(&opened.histories()[1]), [app_snapshot]);
    assert!(matches!(
        events(&opened.histories()[2]).as_slice(),
        [CommitLogEvent::Degradation(CommitLogDegradation {
            kind: CommitLogDegradationKind::SnapshotEntryMissing,
            ..
        })]
    ));
}

#[test]
fn l_sel_3_tagged_narrows_to_repositories_containing_every_exact_local_tag() {
    let fixture = Fixture::new("tagged-narrowing");
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let old = Repository::init(fixture.path().join("old")).unwrap();
    let branch_only = Repository::init(fixture.path().join("branch-only")).unwrap();
    let root_v1 = commit(&fixture.root, "root v1", 100, &[]);
    let root_v2 = commit(&fixture.root, "root v2", 200, &[root_v1]);
    let app_v1 = commit(&app, "app v1", 300, &[]);
    let app_v2 = commit(&app, "app v2", 400, &[app_v1]);
    let old_v1 = commit(&old, "old v1", 500, &[]);
    let branch_v1 = commit(&branch_only, "branch v1", 600, &[]);
    let branch_v2 = commit(&branch_only, "branch v2", 700, &[branch_v1]);
    for (repo, v1, v2) in [
        (&fixture.root, root_v1, Some(root_v2)),
        (&app, app_v1, Some(app_v2)),
        (&old, old_v1, None),
        (&branch_only, branch_v1, None),
    ] {
        tag(repo, "v1", v1);
        if let Some(v2) = v2 {
            tag(repo, "v2", v2);
        }
    }
    branch_only
        .reference("refs/heads/v2", branch_v2, true, "same-name branch")
        .unwrap();
    fixture.write_manifest(&[
        member("mem_app", "app", true),
        member("mem_old", "old", true),
        member("mem_branch", "branch-only", true),
    ]);

    let opened =
        open_request_histories(fixture.path(), &log_request(&["v1..v2"], &[], true)).unwrap();

    assert_eq!(target_ids(&opened), ["@root", "mem_app"]);
    assert_eq!(entry_ids(&opened.histories()[0]), [root_v2]);
    assert_eq!(entry_ids(&opened.histories()[1]), [app_v2]);
}

#[test]
fn l_rng_1_tagged_supports_three_arguments_and_a_nonfirst_range() {
    let fixture = Fixture::new("tagged-many-revisions");
    let base = commit(&fixture.root, "base", 100, &[]);
    let first = detached_commit(&fixture.root, "first", 200, &[base]);
    let range_base = detached_commit(&fixture.root, "range base", 300, &[base]);
    let range_tip = detached_commit(&fixture.root, "range tip", 400, &[range_base]);
    let last = detached_commit(&fixture.root, "last", 500, &[base]);
    for (name, oid) in [
        ("first", first),
        ("range-base", range_base),
        ("range-tip", range_tip),
        ("last", last),
    ] {
        tag(&fixture.root, name, oid);
    }
    fixture.write_manifest(&[]);

    let opened = open_request_histories(
        fixture.path(),
        &log_request(&["first", "range-base..range-tip", "last"], &[], true),
    )
    .unwrap();
    let mut actual = entry_ids(&opened.histories()[0]);
    actual.sort();
    let mut expected = vec![first, range_tip, last];
    expected.sort();

    assert_eq!(actual, expected);
}

#[test]
fn l_sel_3_tagged_refusal_names_snapshot_and_lock_operands_accurately() {
    let fixture = Fixture::new("tagged-plus-refusal");
    fixture.write_manifest(&[]);

    for operand in ["+snapshot", "+lock..HEAD"] {
        let error = open_request_histories(fixture.path(), &log_request(&[operand], &[], true))
            .err()
            .expect("tagged GWZ '+' operand must be refused");

        assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
        assert_eq!(
            error.message,
            "--tagged does not accept GWZ '+'-prefixed operands"
        );
    }
}

#[test]
fn f7_tagged_rejects_a_tag_missing_from_every_selected_repository() {
    let fixture = Fixture::new("tagged-missing-everywhere");
    commit(&fixture.root, "root", 100, &[]);
    fixture.write_manifest(&[]);

    let error = open_request_histories(fixture.path(), &log_request(&["absent"], &[], true))
        .err()
        .expect("missing tag must be rejected");

    assert_eq!(error.code, crate::model::ErrorCode::TagNotFound);
    assert_eq!(
        error.message,
        "local tag 'absent' was not found in any selected log target"
    );
}

#[test]
fn f7_tagged_rejects_distributed_tags_without_an_all_tag_intersection() {
    let fixture = Fixture::new("tagged-empty-intersection");
    let root = commit(&fixture.root, "root", 100, &[]);
    tag(&fixture.root, "left", root);
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let app_head = commit(&app, "app", 200, &[]);
    tag(&app, "right", app_head);
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let error = open_request_histories(fixture.path(), &log_request(&["left", "right"], &[], true))
        .err()
        .expect("empty all-tag intersection must be rejected");

    assert_eq!(error.code, crate::model::ErrorCode::TagNotFound);
    assert_eq!(
        error.message,
        "no selected log target contains all requested local tags 'left', 'right'"
    );
}

#[test]
fn l_tol_2_mixed_resolvable_and_unresolvable_members_degrade_independently() {
    let fixture = Fixture::new("mixed-resolution");
    let root_head = commit(&fixture.root, "root shared", 100, &[]);
    fixture
        .root
        .reference("refs/heads/shared", root_head, true, "shared")
        .unwrap();
    let good = Repository::init(fixture.path().join("good")).unwrap();
    let good_head = commit(&good, "good shared", 200, &[]);
    good.reference("refs/heads/shared", good_head, true, "shared")
        .unwrap();
    let missing = Repository::init(fixture.path().join("missing")).unwrap();
    commit(&missing, "missing head", 300, &[]);
    fixture.write_manifest(&[
        member("mem_good", "good", true),
        member("mem_missing", "missing", true),
    ]);

    let opened =
        open_request_histories(fixture.path(), &log_request(&["shared"], &[], false)).unwrap();

    assert_eq!(entry_ids(&opened.histories()[0])[0], root_head);
    assert_eq!(entry_ids(&opened.histories()[1])[0], good_head);
    let missing = degradations(&opened.histories()[2]);
    assert_eq!(
        missing[0].kind,
        CommitLogDegradationKind::RevisionUnresolved
    );
    assert_eq!(missing[0].operand.as_deref(), Some("shared"));
}

#[test]
fn f7_two_dot_degrades_only_the_member_missing_one_ordinary_endpoint() {
    let fixture = Fixture::new("two-dot-member-miss");
    let start = commit(&fixture.root, "start", 100, &[]);
    let tip = commit(&fixture.root, "tip", 200, &[start]);
    fixture
        .root
        .reference("refs/heads/start", start, true, "start")
        .unwrap();
    fixture
        .root
        .reference("refs/heads/tip", tip, true, "tip")
        .unwrap();
    let app = Repository::init(fixture.path().join("app")).unwrap();
    let app_tip = commit(&app, "app tip", 300, &[]);
    app.reference("refs/heads/tip", app_tip, true, "tip")
        .unwrap();
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let opened =
        open_request_histories(fixture.path(), &log_request(&["start..tip"], &[], false)).unwrap();

    assert_eq!(entry_ids(&opened.histories()[0]), [tip]);
    assert!(matches!(
        events(&opened.histories()[1]).as_slice(),
        [CommitLogEvent::Degradation(CommitLogDegradation {
            kind: CommitLogDegradationKind::RevisionUnresolved,
            operand: Some(operand),
            ..
        })] if operand == "start"
    ));
}

#[test]
fn l_tol_2_default_degradation_is_benign_and_strict_escalates_aggregate() {
    let fixture = Fixture::new("strict-escalation");
    let root_head = commit(&fixture.root, "root only", 100, &[]);
    fixture
        .root
        .reference("refs/heads/root-only", root_head, true, "root only")
        .unwrap();
    let missing = Repository::init(fixture.path().join("missing")).unwrap();
    commit(&missing, "missing head", 200, &[]);
    fixture.write_manifest(&[member("mem_missing", "missing", true)]);
    let request = log_request(&["root-only"], &[], false);

    let benign = open_request_histories(fixture.path(), &request).unwrap();
    let mut clean_request = request.clone();
    clean_request.meta.selection = Some(crate::Selection {
        targets: vec!["@root".to_owned()],
        ..crate::Selection::default()
    });
    let clean_benign = open_request_histories(fixture.path(), &clean_request).unwrap();
    let mut strict_request = request.clone();
    strict_request.options = Some(crate::LogOptions {
        strict: Some(true),
        ..crate::LogOptions::default()
    });
    let strict = open_request_histories(fixture.path(), &strict_request).unwrap();
    clean_request.options = strict_request.options.clone();
    let clean_strict = open_request_histories(fixture.path(), &clean_request).unwrap();

    assert_eq!(
        stream_request_histories(benign, &request, |_| {})
            .aggregate()
            .status(),
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        stream_request_histories(clean_benign, &clean_request, |_| {})
            .aggregate()
            .status(),
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        stream_request_histories(strict, &strict_request, |_| {})
            .aggregate()
            .status(),
        crate::AggregateStatus::Failed
    );
    assert_eq!(
        stream_request_histories(clean_strict, &clean_request, |_| {})
            .aggregate()
            .status(),
        crate::AggregateStatus::Ok
    );
}

#[test]
fn s2_5_production_merge_consumes_range_histories_and_degradations() {
    let fixture = Fixture::new("s2-5-production-merge");
    let root_first = commit(&fixture.root, "root first", 100, &[]);
    let root_head = commit(&fixture.root, "root head", 200, &[root_first]);
    let member_repo = Repository::init(fixture.path().join("member")).unwrap();
    commit(&member_repo, "unrelated member", 300, &[]);
    fixture.write_manifest(&[member("mem_member", "member", true)]);
    let request = log_request(&[&format!("{root_first}..{root_head}")], &[], false);

    let histories = open_request_histories(fixture.path(), &request).unwrap();
    assert!(histories.has_explicit_range());
    let mut events = Vec::new();
    let stats = stream_request_histories(histories, &request, |event| events.push(event));

    assert_eq!(stats.groups_emitted(), 1);
    assert!(events.iter().any(|event| matches!(
        event,
        CommitLogMergedEvent::Group(group)
            if group.entries()[0].commit_id == root_head.to_string()
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        CommitLogMergedEvent::Degradation(record)
            if record.target.member_id == "mem_member"
                && record.kind == CommitLogDegradationKind::RevisionUnresolved
    )));
}

#[test]
fn l_fil_1_and_l_env_5_regexes_filter_raw_message_and_combined_author() {
    let fixture = Fixture::new("s2-6-regex-filters");
    let raw = raw_commit(
        &fixture.root,
        b"Subject without token\nbody has needle and byte \xff\n",
        None,
    );
    fixture.write_manifest(&[]);

    let mut request = log_request(&[], &[], false);
    request.options = Some(crate::LogOptions {
        author: Some(r"Test Author <test@example\.com>".to_owned()),
        grep: Some(r"needle.*(?-u:\xFF)".to_owned()),
        ..crate::LogOptions::default()
    });
    assert_eq!(
        entry_ids(
            &open_request_histories(fixture.path(), &request)
                .unwrap()
                .histories()[0]
        ),
        [raw]
    );

    request.options.as_mut().unwrap().grep = Some("Needle".to_owned());
    assert!(
        entry_ids(
            &open_request_histories(fixture.path(), &request)
                .unwrap()
                .histories()[0]
        )
        .is_empty(),
        "matching is case-sensitive"
    );
}

#[test]
fn l_env_5_6_filters_distinct_raw_author_and_committer_surfaces() {
    let fixture = Fixture::new("s2-6-distinct-author-committer");
    let raw = raw_identity_commit(
        &fixture.root,
        b"raw identity fixture\n",
        b"Auth\xffor",
        b"author-\xfe@example.com",
        10,
        b"Comm\xfdtter",
        b"committer-\xfc@example.com",
        100,
    );
    fixture.write_manifest(&[]);

    let unfiltered = open_default_head_histories(fixture.path()).unwrap();
    let unfiltered_events = events(&unfiltered[0]);
    let [CommitLogEvent::Entry(entry)] = unfiltered_events.as_slice() else {
        panic!("raw identity fixture did not yield exactly one entry");
    };
    assert_eq!(entry.author.name, b"Auth\xffor");
    assert_eq!(entry.author.email, b"author-\xfe@example.com");
    assert_eq!(entry.author.time.seconds, 10);
    assert_eq!(entry.committer.name, b"Comm\xfdtter");
    assert_eq!(entry.committer.email, b"committer-\xfc@example.com");
    assert_eq!(entry.committer.time.seconds, 100);

    for options in [
        crate::LogOptions {
            author: Some(r"(?-u:Auth\xFFor <author-\xFE@example\.com>)".to_owned()),
            ..Default::default()
        },
        crate::LogOptions {
            since: Some("@100".to_owned()),
            until: Some("@100".to_owned()),
            ..Default::default()
        },
        crate::LogOptions {
            author: Some(r"(?-u:Auth\xFFor <author-\xFE@example\.com>)".to_owned()),
            since: Some("@100".to_owned()),
            until: Some("@100".to_owned()),
            ..Default::default()
        },
    ] {
        let mut request = log_request(&[], &[], false);
        request.options = Some(options);
        assert_eq!(
            entry_ids(
                &open_request_histories(fixture.path(), &request)
                    .unwrap()
                    .histories()[0]
            ),
            [raw]
        );
    }
}

#[test]
fn l_env_5_invalid_regex_refuses_before_workspace_access() {
    for (field, option) in [
        (
            "--grep",
            crate::LogOptions {
                grep: Some("[".to_owned()),
                ..Default::default()
            },
        ),
        (
            "--author",
            crate::LogOptions {
                author: Some("(".to_owned()),
                ..Default::default()
            },
        ),
    ] {
        let request = crate::LogRequest {
            options: Some(option),
            ..crate::LogRequest::default()
        };
        let error =
            open_request_histories(Path::new("/definitely/missing/gwz-workspace"), &request)
                .err()
                .expect("invalid Rust regex must be an invocation refusal");
        assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
        assert!(error.message.contains(field), "{}", error.message);
        assert!(error.message.contains("Rust regex"), "{}", error.message);
        assert!(error.message.contains("error"), "{}", error.message);
    }
}

#[test]
fn l_env_6_time_grammar_is_exact_local_and_inclusive() {
    use chrono::{Local, NaiveDate, TimeZone};

    assert_eq!(super::filter::parse_filter_time("@-1").unwrap(), -1);
    assert_eq!(
        super::filter::parse_filter_time("2026-08-31T02:03:04+10:00").unwrap(),
        1_788_105_784
    );
    assert_eq!(
        super::filter::parse_filter_time("20260831T020304+1000").unwrap(),
        1_788_105_784
    );
    let midnight = Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 8, 31)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .single()
        .unwrap()
        .timestamp();
    assert_eq!(
        super::filter::parse_filter_time("2026-08-31").unwrap(),
        midnight
    );
    let local_time = Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 8, 31)
                .unwrap()
                .and_hms_opt(2, 3, 4)
                .unwrap(),
        )
        .single()
        .unwrap()
        .timestamp();
    assert_eq!(
        super::filter::parse_filter_time("2026-08-31T02:03:04").unwrap(),
        local_time
    );
    assert_eq!(
        super::filter::parse_filter_time("2026-08-31T02:03").unwrap(),
        local_time - 4
    );

    for value in ["yesterday", "2026/08/31", "2026-02-30"] {
        let error = super::filter::parse_filter_time(value).unwrap_err();
        assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
        assert!(error.message.contains("RFC3339"), "{}", error.message);
        assert!(
            error.message.contains("@<epoch-seconds>"),
            "{}",
            error.message
        );
    }
    let request = crate::LogRequest {
        options: Some(crate::LogOptions {
            since: Some("yesterday".to_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let error = open_request_histories(Path::new("/definitely/missing/gwz-workspace"), &request)
        .err()
        .expect("approxidate must refuse before workspace access");
    assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);

    let fixture = Fixture::new("s2-6-inclusive-time");
    let before = commit(&fixture.root, "before", 99, &[]);
    let at = commit(&fixture.root, "at", 100, &[before]);
    commit(&fixture.root, "after", 101, &[at]);
    fixture.write_manifest(&[]);
    let mut request = log_request(&[], &[], false);
    request.options = Some(crate::LogOptions {
        since: Some("@100".to_owned()),
        until: Some("@100".to_owned()),
        ..Default::default()
    });
    assert_eq!(
        entry_ids(
            &open_request_histories(fixture.path(), &request)
                .unwrap()
                .histories()[0]
        ),
        [at]
    );
}

#[test]
fn l_env_5_6_locale_independence_and_local_timezone_are_process_pinned() {
    const CHILD_EPOCH: &str = "GWZ_S2_6_EXPECT_LOCAL_EPOCH";
    if let Ok(expected) = std::env::var(CHILD_EPOCH) {
        assert_eq!(
            super::filter::parse_filter_time("2026-08-31T02:03:04").unwrap(),
            expected.parse::<i64>().unwrap()
        );
        return;
    }

    for (timezone, expected) in [
        ("UTC", 1_788_141_784_i64),
        ("Australia/Sydney", 1_788_105_784_i64),
    ] {
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("operation::commit_log::tests::l_env_5_6_locale_independence_and_local_timezone_are_process_pinned")
            .arg("--nocapture")
            .env(CHILD_EPOCH, expected.to_string())
            .env("TZ", timezone)
            .env("LC_ALL", "tr_TR.UTF-8")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "TZ={timezone}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn l_env_6_epoch_extremes_and_dst_gap_overlap_fail_closed() {
    const DST_CHILD: &str = "GWZ_S2_6_DST_FAIL_CLOSED_CHILD";
    if std::env::var_os(DST_CHILD).is_some() {
        for local in ["2026-04-05T02:30:00", "2026-10-04T02:30:00"] {
            let error = super::filter::parse_filter_time(local).unwrap_err();
            assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
            assert!(error.message.contains(local), "{}", error.message);
        }
        return;
    }

    assert_eq!(
        super::filter::parse_filter_time(&format!("@{}", i64::MIN)).unwrap(),
        i64::MIN
    );
    assert_eq!(
        super::filter::parse_filter_time(&format!("@{}", i64::MAX)).unwrap(),
        i64::MAX
    );
    for overflow in ["@9223372036854775808", "@-9223372036854775809"] {
        let error = super::filter::parse_filter_time(overflow).unwrap_err();
        assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
        assert!(error.message.contains(overflow), "{}", error.message);
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("operation::commit_log::tests::l_env_6_epoch_extremes_and_dst_gap_overlap_fail_closed")
        .arg("--nocapture")
        .env(DST_CHILD, "1")
        .env("TZ", "Australia/Sydney")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Sydney DST child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn l_fil_1_first_parent_and_no_merges_match_native_git_premerge() {
    let fixture = Fixture::new("s2-6-ancestry-filters");
    build_native_path_history(fixture.path());
    fixture.write_manifest(&[]);

    for (options, native_flag) in [
        (
            crate::LogOptions {
                first_parent: Some(true),
                ..Default::default()
            },
            "--first-parent",
        ),
        (
            crate::LogOptions {
                no_merges: Some(true),
                ..Default::default()
            },
            "--no-merges",
        ),
    ] {
        let mut request = log_request(&[], &["."], false);
        request.options = Some(options);
        let actual = entry_ids(
            &open_request_histories(fixture.path(), &request)
                .unwrap()
                .histories()[0],
        );
        let expected = git_oid_lines(
            Command::new("git")
                .arg("-C")
                .arg(fixture.path())
                .arg("rev-list")
                .arg(native_flag)
                .arg("HEAD")
                .arg("--")
                .arg("."),
        );
        assert_eq!(actual, expected, "{native_flag}");
    }
}

#[test]
fn l_fil_1_ancestry_filters_match_native_merge_range_with_and_without_paths() {
    let fixture = Fixture::new("s2-6-ancestry-range");
    build_native_path_history(fixture.path());
    fixture.write_manifest(&[]);

    for (options, native_flag) in [
        (
            crate::LogOptions {
                first_parent: Some(true),
                ..Default::default()
            },
            "--first-parent",
        ),
        (
            crate::LogOptions {
                no_merges: Some(true),
                ..Default::default()
            },
            "--no-merges",
        ),
    ] {
        for pathspecs in [&[][..], &["."][..]] {
            let mut request = log_request(&["HEAD~2..HEAD"], pathspecs, false);
            request.options = Some(options.clone());
            let actual = entry_ids(
                &open_request_histories(fixture.path(), &request)
                    .unwrap()
                    .histories()[0],
            );
            let mut native = Command::new("git");
            native
                .arg("-C")
                .arg(fixture.path())
                .arg("rev-list")
                .arg(native_flag)
                .arg("HEAD~2..HEAD");
            if !pathspecs.is_empty() {
                native.arg("--").args(pathspecs);
            }
            let expected = git_oid_lines(&mut native);
            assert_eq!(expected.len(), 2, "{native_flag} {pathspecs:?}");
            assert_eq!(actual, expected, "{native_flag} {pathspecs:?}");
        }
    }
}

#[test]
fn l_coa_5_l_env_7_partial_filtering_keeps_survivors_and_empty_is_ok() {
    const MARKER: &str = "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91";
    let fixture = Fixture::new("s2-6-filter-survivors");
    let root_base = commit(&fixture.root, "root base", 100, &[]);
    let root_side = detached_commit(&fixture.root, "root side", 100, &[root_base]);
    commit(
        &fixture.root,
        &format!("shared\n\nGWZ-Commit-ID: {MARKER}\nGWZ-Workspace-ID: ws_test"),
        200,
        &[root_base, root_side],
    );
    let member_repo = Repository::init(fixture.path().join("member")).unwrap();
    let member_base = commit(&member_repo, "member base", 100, &[]);
    commit(
        &member_repo,
        &format!("shared\n\nGWZ-Commit-ID: {MARKER}\nGWZ-Workspace-ID: ws_test"),
        200,
        &[member_base],
    );
    fixture.write_manifest(&[member("mem_member", "member", true)]);

    let mut request = log_request(&[], &[], false);
    request.options = Some(crate::LogOptions {
        since: Some("@200".to_owned()),
        no_merges: Some(true),
        ..Default::default()
    });
    let mut events = Vec::new();
    let run = stream_request_histories(
        open_request_histories(fixture.path(), &request).unwrap(),
        &request,
        |event| events.push(event),
    );
    let groups = events
        .iter()
        .filter_map(|event| match event {
            CommitLogMergedEvent::Group(group) => Some(group),
            CommitLogMergedEvent::Degradation(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].entries()[0].target.member_id, "mem_member");
    assert_eq!(
        groups[0].provenance(),
        &super::coalesce::CommitLogProvenance::Marker(MARKER.to_owned())
    );
    assert_eq!(run.aggregate().status(), crate::AggregateStatus::Ok);

    request.options.as_mut().unwrap().grep = Some("does-not-exist".to_owned());
    let empty = stream_request_histories(
        open_request_histories(fixture.path(), &request).unwrap(),
        &request,
        |_| {},
    );
    assert_eq!(empty.groups_emitted(), 0);
    assert_eq!(empty.aggregate().status(), crate::AggregateStatus::Ok);
    assert!(
        empty.aggregate().outcomes().iter().all(|outcome| {
            outcome.kind() == super::aggregate::CommitLogMemberOutcomeKind::Empty
        })
    );
}

#[test]
fn l_env_7_filtering_precedes_cap_and_order_for_every_jobs_value() {
    fn add_history(repository: &Repository, survivor: &str, seconds: i64, prefix: &str) -> Oid {
        let survivor = commit(repository, survivor, seconds, &[]);
        let first = commit(
            repository,
            &format!("reject {prefix} newest"),
            seconds + 300,
            &[survivor],
        );
        commit(
            repository,
            &format!("reject {prefix} head"),
            seconds + 200,
            &[first],
        );
        survivor
    }

    let fixture = Fixture::new("s2-6-filter-before-cap");
    let root = add_history(&fixture.root, "KEEP root bytes", 300, "root");
    let member_a_repo = Repository::init(fixture.path().join("member-a")).unwrap();
    let member_a = add_history(&member_a_repo, "KEEP member-a", 200, "a");
    let member_b_repo = Repository::init(fixture.path().join("member-b")).unwrap();
    add_history(&member_b_repo, "KEEP member-b", 100, "b");
    fixture.write_manifest(&[
        member("mem_a", "member-a", true),
        member("mem_b", "member-b", true),
    ]);

    let run = |jobs| {
        let mut request = log_request(&[], &[], false);
        request.options = Some(crate::LogOptions {
            grep: Some("^KEEP".to_owned()),
            max_entries: Some(2),
            ..Default::default()
        });
        request.meta.policy = Some(crate::OperationPolicy {
            concurrency: Some(jobs),
            ..Default::default()
        });
        let mut events = Vec::new();
        let result = stream_request_histories(
            open_request_histories(fixture.path(), &request).unwrap(),
            &request,
            |event| events.push(event),
        );
        assert_eq!(result.groups_emitted(), 2);
        events
    };

    let one = run(1);
    let two = run(2);
    let eight = run(8);
    assert_eq!(one, two, "jobs=1 and jobs=2 must be byte-complete equal");
    assert_eq!(one, eight, "jobs=1 and jobs=8 must be byte-complete equal");
    assert_eq!(
        one.iter()
            .map(|event| match event {
                CommitLogMergedEvent::Group(group) => group.entries()[0].commit_id.clone(),
                CommitLogMergedEvent::Degradation(record) => {
                    panic!("unexpected degradation: {record:?}")
                }
            })
            .collect::<Vec<_>>(),
        [root.to_string(), member_a.to_string()]
    );
}

#[test]
fn l_exit_1_aggregate_distinguishes_benign_strict_and_read_failure() {
    let fixture = Fixture::new("s2-6-aggregate");
    let root = commit(&fixture.root, "root", 100, &[]);
    fixture
        .root
        .reference("refs/heads/root-only", root, true, "root-only")
        .unwrap();
    let member_repo = Repository::init(fixture.path().join("member")).unwrap();
    commit(&member_repo, "member", 100, &[]);
    fixture.write_manifest(&[member("mem_member", "member", true)]);

    let request = log_request(&["root-only"], &[], false);
    let benign = stream_request_histories(
        open_request_histories(fixture.path(), &request).unwrap(),
        &request,
        |_| {},
    );
    assert_eq!(benign.aggregate().status(), crate::AggregateStatus::Ok);
    assert_eq!(benign.aggregate().outcomes()[0].target().member_id, "@root");
    assert_eq!(
        benign.aggregate().outcomes()[1].kind(),
        super::aggregate::CommitLogMemberOutcomeKind::Degraded
    );

    let mut strict_request = request.clone();
    strict_request.options = Some(crate::LogOptions {
        strict: Some(true),
        ..Default::default()
    });
    let strict = stream_request_histories(
        open_request_histories(fixture.path(), &strict_request).unwrap(),
        &strict_request,
        |_| {},
    );
    assert_eq!(strict.aggregate().status(), crate::AggregateStatus::Failed);

    fixture.write_manifest(&[member("mem_missing", "missing", true)]);
    let failed_request = log_request(&[], &[], false);
    let failed = stream_request_histories(
        open_request_histories(fixture.path(), &failed_request).unwrap(),
        &failed_request,
        |_| {},
    );
    assert_eq!(failed.aggregate().status(), crate::AggregateStatus::Partial);
    assert_eq!(
        failed.aggregate().outcomes()[1].kind(),
        super::aggregate::CommitLogMemberOutcomeKind::Failed
    );

    let mut failed_only_request = failed_request;
    failed_only_request.meta.selection = Some(crate::Selection {
        targets: vec!["mem_missing".to_owned()],
        ..Default::default()
    });
    let failed_only = stream_request_histories(
        open_request_histories(fixture.path(), &failed_only_request).unwrap(),
        &failed_only_request,
        |_| {},
    );
    assert_eq!(
        failed_only.aggregate().status(),
        crate::AggregateStatus::Failed
    );
}

#[test]
fn l_exit_1_aggregate_tables_every_degradation_kind_and_truth_class() {
    use super::aggregate::{CommitLogAggregateCollector, CommitLogMemberOutcomeKind};
    use super::coalesce::assemble_commit_log_groups;

    let cases = [
        (
            CommitLogDegradationKind::UnbornHead,
            CommitLogMemberOutcomeKind::Degraded,
            crate::AggregateStatus::Ok,
        ),
        (
            CommitLogDegradationKind::RevisionUnresolved,
            CommitLogMemberOutcomeKind::Degraded,
            crate::AggregateStatus::Ok,
        ),
        (
            CommitLogDegradationKind::SnapshotEntryMissing,
            CommitLogMemberOutcomeKind::Degraded,
            crate::AggregateStatus::Ok,
        ),
        (
            CommitLogDegradationKind::LockEntryMissing,
            CommitLogMemberOutcomeKind::Degraded,
            crate::AggregateStatus::Ok,
        ),
        (
            CommitLogDegradationKind::UnsupportedSourceKind,
            CommitLogMemberOutcomeKind::Failed,
            crate::AggregateStatus::Failed,
        ),
        (
            CommitLogDegradationKind::RepositoryUnreadable,
            CommitLogMemberOutcomeKind::Failed,
            crate::AggregateStatus::Failed,
        ),
        (
            CommitLogDegradationKind::HistoryUnreadable,
            CommitLogMemberOutcomeKind::Failed,
            crate::AggregateStatus::Failed,
        ),
    ];

    let target = |member_id: &str| CommitLogTarget {
        member_id: member_id.to_owned(),
        member_path: member_id.to_owned(),
        source_kind: ArtifactSourceKind::Git,
    };
    let degradation = |target: &CommitLogTarget, kind| CommitLogDegradation {
        target: target.clone(),
        kind,
        operand: Some("HEAD".to_owned()),
        detail: "aggregate class fixture".to_owned(),
    };

    let empty_target = target("mem_empty");
    let empty = CommitLogAggregateCollector::new(vec![empty_target], false).finish();
    assert_eq!(
        empty.outcomes()[0].kind(),
        CommitLogMemberOutcomeKind::Empty
    );
    assert_eq!(empty.status(), crate::AggregateStatus::Ok);

    for (kind, expected_outcome, expected_status) in cases {
        let case_target = target("mem_case");
        let record = degradation(&case_target, kind);
        let mut default = CommitLogAggregateCollector::new(vec![case_target.clone()], false);
        default.observe_degradation(&record);
        let default = default.finish();
        assert_eq!(default.outcomes()[0].kind(), expected_outcome, "{kind:?}");
        assert_eq!(default.status(), expected_status, "{kind:?}");

        let mut strict = CommitLogAggregateCollector::new(vec![case_target], true);
        strict.observe_degradation(&record);
        assert_eq!(
            strict.finish().status(),
            crate::AggregateStatus::Failed,
            "{kind:?}"
        );
    }

    let contributed_target = target("mem_contributed");
    let failed_target = target("mem_failed");
    let mut collector = CommitLogAggregateCollector::new(
        vec![contributed_target.clone(), failed_target.clone()],
        false,
    );
    let contributed = CommitLogEntry {
        target: contributed_target,
        commit_id: "contributed".to_owned(),
        parent_ids: Vec::new(),
        author: CommitLogIdentity {
            name: b"Author".to_vec(),
            email: b"author@example.invalid".to_vec(),
            time: CommitLogTime {
                seconds: 100,
                offset_minutes: 0,
            },
        },
        committer: CommitLogIdentity {
            name: b"Committer".to_vec(),
            email: b"committer@example.invalid".to_vec(),
            time: CommitLogTime {
                seconds: 100,
                offset_minutes: 0,
            },
        },
        message: b"contributed".to_vec(),
        message_encoding: None,
    };
    let group = assemble_commit_log_groups([contributed], false).remove(0);
    collector.observe_group(&group);
    collector.observe_degradation(&degradation(
        &failed_target,
        CommitLogDegradationKind::HistoryUnreadable,
    ));
    let partial = collector.finish();
    assert_eq!(
        partial.outcomes()[0].kind(),
        CommitLogMemberOutcomeKind::Contributed
    );
    assert_eq!(
        partial.outcomes()[1].kind(),
        CommitLogMemberOutcomeKind::Failed
    );
    assert_eq!(partial.status(), crate::AggregateStatus::Partial);
}

#[test]
fn f4_jobs_one_bounds_real_path_reader_lifetimes() {
    let fixture = Fixture::new("s2-5-path-reader-jobs");
    commit_file(&fixture.root, "root.txt", b"root\n", "root", 100, &[]);
    let members = [
        ("mem_f4_jobs_a", "member-a", 200),
        ("mem_f4_jobs_b", "member-b", 300),
        ("mem_f4_jobs_c", "member-c", 400),
    ];
    for (member_id, path, seconds) in members {
        let repository = Repository::init(fixture.path().join(path)).unwrap();
        commit_file(
            &repository,
            "tracked.txt",
            member_id.as_bytes(),
            member_id,
            seconds,
            &[],
        );
    }
    fixture.write_manifest(
        &members
            .into_iter()
            .map(|(member_id, path, _)| member(member_id, path, true))
            .collect::<Vec<_>>(),
    );
    let mut request = log_request(&[], &["."], false);
    request.meta.policy = Some(crate::OperationPolicy {
        concurrency: Some(1),
        ..crate::OperationPolicy::default()
    });

    watch_path_readers("mem_f4_jobs_");
    let histories = open_request_histories(fixture.path(), &request).unwrap();
    let stats = stream_request_histories(histories, &request, |_| {});
    let max_path_readers = finish_watching_path_readers();

    assert_eq!(stats.groups_emitted(), 4);
    assert_eq!(stats.max_concurrent_reads(), 1);
    assert_eq!(max_path_readers, 1);
}

fn entry_ids(history: &RepositoryHistory) -> Vec<Oid> {
    events(history)
        .into_iter()
        .map(|event| match event {
            CommitLogEvent::Entry(entry) => Oid::from_str(&entry.commit_id).unwrap(),
            CommitLogEvent::Degradation(record) => {
                panic!("expected only entries, got degradation {record:?}")
            }
        })
        .collect()
}

fn degradations(history: &RepositoryHistory) -> Vec<CommitLogDegradation> {
    events(history)
        .into_iter()
        .map(|event| match event {
            CommitLogEvent::Entry(entry) => {
                panic!("expected only degradations, got entry {entry:?}")
            }
            CommitLogEvent::Degradation(record) => record,
        })
        .collect()
}

fn events(history: &RepositoryHistory) -> Vec<CommitLogEvent> {
    history.messages().collect()
}

fn target_ids(opened: &CommitLogHistories) -> Vec<&str> {
    opened
        .histories()
        .iter()
        .map(|history| history.target().member_id.as_str())
        .collect()
}

fn log_request(operands: &[&str], explicit_pathspecs: &[&str], tagged: bool) -> crate::LogRequest {
    crate::LogRequest {
        meta: crate::RequestMeta {
            request_id: "req-log-test".to_owned(),
            schema_version: "gwz.v0".to_owned(),
            ..crate::RequestMeta::default()
        },
        workspace_cwd: Some(String::new()),
        operands: operands
            .iter()
            .map(|operand| (*operand).to_owned())
            .collect(),
        explicit_pathspecs: explicit_pathspecs
            .iter()
            .map(|path| (*path).to_owned())
            .collect(),
        options: None,
        tagged: tagged.then_some(true),
    }
}

fn member(id: &str, path: &str, active: bool) -> ManifestMember {
    ManifestMember {
        id: id.to_owned(),
        path: path.to_owned(),
        source_kind: ArtifactSourceKind::Git,
        source_id: format!("src_{}", id.trim_start_matches("mem_")),
        active,
        desired: None,
        remotes: Vec::<RemoteArtifact>::new(),
    }
}

fn commit(repo: &Repository, message: &str, seconds: i64, parents: &[Oid]) -> Oid {
    let oid = detached_commit(repo, message, seconds, parents);
    match repo.head() {
        Ok(head) if head.is_branch() => {
            repo.reference(head.name().unwrap(), oid, true, message)
                .unwrap();
        }
        _ => {
            repo.reference("refs/heads/main", oid, true, message)
                .unwrap();
            repo.set_head("refs/heads/main").unwrap();
        }
    }
    oid
}

fn detached_commit(repo: &Repository, message: &str, seconds: i64, parents: &[Oid]) -> Oid {
    let signature =
        Signature::new("Test Author", "test@example.com", &Time::new(seconds, 0)).unwrap();
    let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parents = parents
        .iter()
        .map(|oid| repo.find_commit(*oid).unwrap())
        .collect::<Vec<Commit<'_>>>();
    let parent_refs = parents.iter().collect::<Vec<_>>();
    repo.commit(None, &signature, &signature, message, &tree, &parent_refs)
        .unwrap()
}

fn commit_file(
    repo: &Repository,
    path: &str,
    contents: &[u8],
    message: &str,
    seconds: i64,
    parents: &[Oid],
) -> Oid {
    let workdir = repo.workdir().unwrap();
    let file = workdir.join(path);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&file, contents).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new(path)).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature =
        Signature::new("Test Author", "test@example.com", &Time::new(seconds, 0)).unwrap();
    let parents = parents
        .iter()
        .map(|oid| repo.find_commit(*oid).unwrap())
        .collect::<Vec<_>>();
    let parent_refs = parents.iter().collect::<Vec<_>>();
    let oid = repo
        .commit(None, &signature, &signature, message, &tree, &parent_refs)
        .unwrap();
    match repo.head() {
        Ok(head) if head.is_branch() => {
            repo.reference(head.name().unwrap(), oid, true, message)
                .unwrap();
        }
        _ => {
            repo.reference("refs/heads/main", oid, true, message)
                .unwrap();
            repo.set_head("refs/heads/main").unwrap();
        }
    }
    oid
}

fn tag(repo: &Repository, name: &str, oid: Oid) {
    repo.reference(&format!("refs/tags/{name}"), oid, true, "test tag")
        .unwrap();
}

fn git_ok(path: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed with {status}");
}

fn build_native_path_history(path: &Path) {
    git_ok(path, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git_ok(path, &["config", "user.name", "Test Author"]);
    git_ok(path, &["config", "user.email", "test@example.com"]);
    fs::write(path.join("p"), "base\n").unwrap();
    git_ok(path, &["add", "p"]);
    git_ok(path, &["commit", "--quiet", "-m", "base"]);
    git_ok(path, &["commit", "--quiet", "--allow-empty", "-m", "empty"]);
    git_ok(path, &["branch", "feature"]);
    fs::write(path.join("q"), "main\n").unwrap();
    git_ok(path, &["add", "q"]);
    git_ok(path, &["commit", "--quiet", "-m", "main"]);
    git_ok(path, &["checkout", "--quiet", "feature"]);
    fs::write(path.join("p"), "feature\n").unwrap();
    git_ok(path, &["commit", "--quiet", "-am", "feature"]);
    git_ok(path, &["checkout", "--quiet", "main"]);
    git_ok(
        path,
        &["merge", "--quiet", "--no-ff", "feature", "-m", "merge"],
    );
}

fn build_magic_path_history(path: &Path) {
    git_ok(path, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git_ok(path, &["config", "user.name", "Test Author"]);
    git_ok(path, &["config", "user.email", "test@example.com"]);
    fs::create_dir_all(path.join("src")).unwrap();
    fs::write(path.join("artifact"), "root artifact\n").unwrap();
    fs::write(path.join("src/artifact"), "src artifact\n").unwrap();
    fs::write(path.join("src/keep"), "src keep\n").unwrap();
    git_ok(path, &["add", "artifact", "src/artifact", "src/keep"]);
    git_ok(path, &["commit", "--quiet", "-m", "base"]);
    fs::write(path.join("artifact"), "root artifact two\n").unwrap();
    git_ok(path, &["commit", "--quiet", "-am", "root artifact"]);
    fs::write(path.join("src/artifact"), "src artifact two\n").unwrap();
    git_ok(path, &["commit", "--quiet", "-am", "src artifact"]);
    fs::write(path.join("src/keep"), "src keep two\n").unwrap();
    git_ok(path, &["commit", "--quiet", "-am", "src keep"]);
    git_ok(path, &["commit", "--quiet", "--allow-empty", "-m", "empty"]);
}

fn native_log_ids(path: &Path, revisions: &[&str], pathspecs: &[&str]) -> Vec<Oid> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .arg("log")
        .arg("--format=%H")
        .args(revisions);
    if !pathspecs.is_empty() {
        command.arg("--").args(pathspecs);
    }
    git_oid_lines(&mut command)
}

fn native_rev_list_ids(path: &Path, revisions: &[&str], pathspecs: &[&str]) -> Vec<Oid> {
    let mut command = Command::new("git");
    command.arg("-C").arg(path).arg("rev-list");
    if revisions.is_empty() {
        command.arg("HEAD");
    } else {
        command.args(revisions);
    }
    if !pathspecs.is_empty() {
        command.arg("--").args(pathspecs);
    }
    git_oid_lines(&mut command)
}

fn git_oid_lines(command: &mut Command) -> Vec<Oid> {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| Oid::from_str(line).unwrap())
        .collect()
}

fn repository_bytes(git_dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                visit(root, &path, out);
            } else if file_type.is_file() {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut out = BTreeMap::new();
    visit(git_dir, git_dir, &mut out);
    out
}

fn write_manifest_at(path: &Path, members: &[ManifestMember]) {
    let manifest = ManifestArtifact {
        schema: WORKSPACE_SCHEMA.to_owned(),
        workspace: WorkspaceHeader {
            id: "ws_test".to_owned(),
        },
        members: members.to_vec(),
    };
    fs::create_dir_all(path.join(crate::workspace::WORKSPACE_DIR)).unwrap();
    fs::write(
        path.join(crate::workspace::WORKSPACE_MANIFEST),
        manifest.to_yaml().unwrap(),
    )
    .unwrap();
}

fn raw_commit(repo: &Repository, message: &[u8], encoding: Option<&[u8]>) -> Oid {
    let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
    let mut raw = format!(
        "tree {tree_id}\nauthor Test Author <test@example.com> 100 +0000\ncommitter Test Author <test@example.com> 100 +0000\n"
    )
    .into_bytes();
    if let Some(encoding) = encoding {
        raw.extend_from_slice(b"encoding ");
        raw.extend_from_slice(encoding);
        raw.push(b'\n');
    }
    raw.push(b'\n');
    raw.extend_from_slice(message);

    let oid = repo.odb().unwrap().write(ObjectType::Commit, &raw).unwrap();
    repo.reference("refs/heads/main", oid, true, "raw test commit")
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();
    oid
}

#[allow(clippy::too_many_arguments)]
fn raw_identity_commit(
    repo: &Repository,
    message: &[u8],
    author_name: &[u8],
    author_email: &[u8],
    author_seconds: i64,
    committer_name: &[u8],
    committer_email: &[u8],
    committer_seconds: i64,
) -> Oid {
    let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
    let mut raw = format!("tree {tree_id}\nauthor ").into_bytes();
    raw.extend_from_slice(author_name);
    raw.extend_from_slice(b" <");
    raw.extend_from_slice(author_email);
    raw.extend_from_slice(format!("> {author_seconds} +0000\ncommitter ").as_bytes());
    raw.extend_from_slice(committer_name);
    raw.extend_from_slice(b" <");
    raw.extend_from_slice(committer_email);
    raw.extend_from_slice(format!("> {committer_seconds} +0000\n\n").as_bytes());
    raw.extend_from_slice(message);

    let oid = repo.odb().unwrap().write(ObjectType::Commit, &raw).unwrap();
    repo.reference("refs/heads/main", oid, true, "raw identity test commit")
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();
    oid
}

#[test]
fn l_int_1_dispatch_spools_every_record_with_cursor_eof_and_release() {
    let fixture = Fixture::new("handler-spool");
    let root = commit(&fixture.root, "root subject\nroot body", 100, &[]);
    let member_repo = Repository::init(fixture.path().join("app")).unwrap();
    let app = commit(&member_repo, "app subject", 200, &[]);
    fixture.write_manifest(&[member("mem_app", "app", true)]);

    let registry = crate::operation::CommitLogOutputRegistry::new();
    let response = crate::operation::handle_log(
        fixture.path(),
        log_request(&[], &[], false),
        "op-log-spool",
        &registry,
    )
    .unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    let log_id = response.output.log_id;
    assert!(!log_id.is_empty());

    let second_response = crate::operation::handle_log(
        fixture.path(),
        log_request(&[], &[], false),
        "op-log-spool-second",
        &registry,
    )
    .unwrap();
    let second_log_id = second_response.output.log_id;
    assert!(!second_log_id.is_empty());
    assert_ne!(log_id, second_log_id);
    for id in [&log_id, &second_log_id] {
        let batch = registry
            .read(
                id,
                &crate::operation::CommitLogReadRequest {
                    cursor: None,
                    max_records: Some(1),
                },
            )
            .unwrap();
        assert_eq!(
            batch.records[0].entry.as_ref().unwrap().members[0].commit,
            app.to_string()
        );
    }

    for max_records in [0, u32::MAX] {
        let error = registry
            .read(
                &log_id,
                &crate::operation::CommitLogReadRequest {
                    cursor: None,
                    max_records: Some(max_records),
                },
            )
            .expect_err("unbounded batch spellings must be refused");
        assert_eq!(error.code, crate::model::ErrorCode::InvalidRequest);
    }

    let first = registry
        .read(
            &log_id,
            &crate::operation::CommitLogReadRequest {
                cursor: None,
                max_records: Some(1),
            },
        )
        .unwrap();
    assert_eq!(first.state, crate::operation::CommitLogReadState::Data);
    assert_eq!(first.records.len(), 1);
    assert_eq!(
        first.records[0].entry.as_ref().unwrap().members[0].commit,
        app.to_string()
    );

    let second = registry
        .read(
            &log_id,
            &crate::operation::CommitLogReadRequest {
                cursor: Some(first.next_cursor),
                max_records: Some(1),
            },
        )
        .unwrap();
    assert_eq!(second.state, crate::operation::CommitLogReadState::Data);
    assert_eq!(
        second.records[0].entry.as_ref().unwrap().members[0].commit,
        root.to_string()
    );

    let eof = registry
        .read(
            &log_id,
            &crate::operation::CommitLogReadRequest {
                cursor: Some(second.next_cursor),
                max_records: Some(1),
            },
        )
        .unwrap();
    assert_eq!(eof.state, crate::operation::CommitLogReadState::Eof);
    assert!(eof.records.is_empty());

    let invalid = registry
        .read(
            &log_id,
            &crate::operation::CommitLogReadRequest {
                cursor: Some(1),
                max_records: Some(1),
            },
        )
        .expect_err("a non-boundary cursor must be rejected");
    assert_eq!(invalid.code, crate::model::ErrorCode::InvalidRequest);

    registry.release(&log_id);
    registry.release(&log_id);
    let released = registry
        .read(&log_id, &crate::operation::CommitLogReadRequest::default())
        .expect_err("released ids must stop resolving");
    assert_eq!(released.code, crate::model::ErrorCode::InvalidRequest);
    assert_eq!(
        registry
            .read(
                &second_log_id,
                &crate::operation::CommitLogReadRequest {
                    cursor: None,
                    max_records: Some(1),
                },
            )
            .unwrap()
            .records[0]
            .entry
            .as_ref()
            .unwrap()
            .members[0]
            .commit,
        app.to_string()
    );
    registry.release(&second_log_id);
}

#[test]
fn missing_explicit_root_manifest_is_a_typed_pre_dispatch_rejection() {
    let fixture = Fixture::new("handler-missing-manifest");
    let mut request = log_request(&[], &[], false);
    request.meta.workspace = Some(crate::WorkspaceRef {
        root: Some(fixture.path().to_string_lossy().into_owned()),
        workspace_id: None,
    });
    let registry = crate::operation::CommitLogOutputRegistry::new();

    let error = crate::operation::handle_log(
        fixture.path(),
        request,
        "op-log-missing-manifest",
        &registry,
    )
    .expect_err("an explicit existing directory without a manifest must refuse");

    assert_eq!(error.code, crate::model::ErrorCode::ManifestNotFound);
    assert!(
        registry
            .read(
                "commitlog_000000000001",
                &crate::operation::CommitLogReadRequest::default(),
            )
            .is_err(),
        "pre-dispatch rejection must not create an output spool"
    );
}

#[test]
fn l_env_1_l_env_12_projection_preserves_seconds_and_marks_lossy_bytes() {
    let marker =
        b"\n\nGWZ-Commit-ID: 01987b0c-2f75-7c4a-9a32-8fd22f7d7c91\nGWZ-Workspace-ID: ws_test\n";
    let mut a_message = b"least\xff\nbody".to_vec();
    a_message.extend_from_slice(marker);
    let mut z_message = b"other sibling".to_vec();
    z_message.extend_from_slice(marker);
    let entries = vec![
        CommitLogEntry {
            target: CommitLogTarget {
                member_id: "mem_z".to_owned(),
                member_path: "z".to_owned(),
                source_kind: ArtifactSourceKind::Git,
            },
            commit_id: "b".repeat(40),
            parent_ids: Vec::new(),
            author: CommitLogIdentity {
                name: b"Zed".to_vec(),
                email: b"z@example.invalid".to_vec(),
                time: CommitLogTime {
                    seconds: 7,
                    offset_minutes: 0,
                },
            },
            committer: CommitLogIdentity {
                name: b"Zed".to_vec(),
                email: b"z@example.invalid".to_vec(),
                time: CommitLogTime {
                    seconds: i64::MAX,
                    offset_minutes: 0,
                },
            },
            message: z_message,
            message_encoding: None,
        },
        CommitLogEntry {
            target: CommitLogTarget {
                member_id: "mem_a".to_owned(),
                member_path: "a".to_owned(),
                source_kind: ArtifactSourceKind::Git,
            },
            commit_id: "a".repeat(40),
            parent_ids: vec!["0".repeat(40)],
            author: CommitLogIdentity {
                name: b"A\xff".to_vec(),
                email: b"a@example.invalid".to_vec(),
                time: CommitLogTime {
                    seconds: i64::MAX,
                    offset_minutes: 600,
                },
            },
            committer: CommitLogIdentity {
                name: b"A".to_vec(),
                email: b"a@example.invalid".to_vec(),
                time: CommitLogTime {
                    seconds: i64::MIN,
                    offset_minutes: -600,
                },
            },
            message: a_message,
            message_encoding: Some(b"ISO-8859-1".to_vec()),
        },
    ];
    let group = super::coalesce::assemble_commit_log_groups(entries, true)
        .into_iter()
        .next()
        .unwrap();
    let record =
        super::handler::project_merged_event(CommitLogMergedEvent::Group(group), true).unwrap();
    let entry = record.entry.unwrap();

    assert_eq!(
        entry
            .members
            .iter()
            .map(|member| member.member_id.as_str())
            .collect::<Vec<_>>(),
        ["mem_a", "mem_z"]
    );
    assert_eq!(entry.subject, "least�");
    assert!(entry.body.as_deref().unwrap().contains("body"));
    assert_eq!(entry.author.name, "A�");
    assert_eq!(entry.author_timestamp_seconds, i64::MAX);
    assert_eq!(entry.committer_timestamp_seconds, i64::MIN);
    assert_eq!(entry.ordering_timestamp_seconds, i64::MAX);
    assert_eq!(entry.author.time_ms, None);
    assert_eq!(entry.committer.time_ms, None);
    assert_eq!(entry.ordering_timestamp_ms, None);
    assert_eq!(entry.lossy, Some(true));
    assert_eq!(entry.provenance.kind, crate::LogMergeKind::Marker);
}

#[test]
fn l_int_1_spool_creation_failure_leaves_no_resolvable_log_id() {
    let fixture = Fixture::new("handler-spool-failure");
    commit(&fixture.root, "root", 100, &[]);
    fixture.write_manifest(&[]);
    let registry = crate::operation::CommitLogOutputRegistry::new();
    registry.fail_next_spool_for_test();

    let error = crate::operation::handle_log(
        fixture.path(),
        log_request(&[], &[], false),
        "op-log-spool-failure",
        &registry,
    )
    .expect_err("spool failure must fail the unary request");
    assert_eq!(error.code, crate::model::ErrorCode::IoError);
    assert_eq!(registry.registered_log_count_for_test(), 0);
}

#[test]
fn l_int_1_post_registration_spool_failure_removes_the_output_authority() {
    let fixture = Fixture::new("handler-spool-append-failure");
    commit(&fixture.root, "root", 100, &[]);
    fixture.write_manifest(&[]);
    let registry = crate::operation::CommitLogOutputRegistry::new();
    registry.fail_next_append_for_test();

    let error = crate::operation::handle_log(
        fixture.path(),
        log_request(&[], &[], false),
        "op-log-spool-append-failure",
        &registry,
    )
    .expect_err("append failure must fail the unary request");
    assert_eq!(error.code, crate::model::ErrorCode::IoError);
    assert_eq!(registry.registered_log_count_for_test(), 0);
}

#[test]
fn l_int_1_seal_failure_removes_the_output_authority() {
    let fixture = Fixture::new("handler-spool-seal-failure");
    commit(&fixture.root, "root", 100, &[]);
    fixture.write_manifest(&[]);
    let registry = crate::operation::CommitLogOutputRegistry::new();
    registry.fail_next_seal_for_test();

    let error = crate::operation::handle_log(
        fixture.path(),
        log_request(&[], &[], false),
        "op-log-spool-seal-failure",
        &registry,
    )
    .expect_err("seal failure must fail the unary request");
    assert_eq!(error.code, crate::model::ErrorCode::IoError);
    assert_eq!(registry.registered_log_count_for_test(), 0);
}

#[test]
fn l_int_1_public_handler_preserves_partial_aggregate() {
    let fixture = Fixture::new("handler-partial");
    commit(&fixture.root, "root", 100, &[]);
    fixture.write_manifest(&[member("mem_missing", "missing", true)]);
    let registry = crate::operation::CommitLogOutputRegistry::new();

    let response = crate::operation::handle_log(
        fixture.path(),
        log_request(&[], &[], false),
        "op-log-partial",
        &registry,
    )
    .unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Partial
    );
    let batch = registry
        .read(
            &response.output.log_id,
            &crate::operation::CommitLogReadRequest::default(),
        )
        .unwrap();
    assert_eq!(batch.records.len(), 2);
    assert_eq!(
        batch.records[0].degradation.as_ref().unwrap().reason,
        crate::LogDegradationReason::RepositoryUnreadable
    );
    assert_eq!(batch.records[1].kind, crate::LogOutputRecordKind::Entry);
}

#[test]
fn l_int_1_strict_degradation_sets_final_aggregate_and_streams_once() {
    let fixture = Fixture::new("handler-strict");
    commit(&fixture.root, "root", 100, &[]);
    Repository::init(fixture.path().join("empty")).unwrap();
    fixture.write_manifest(&[member("mem_empty", "empty", true)]);
    let registry = crate::operation::CommitLogOutputRegistry::new();
    let mut request = log_request(&[], &[], false);
    request.options = Some(crate::LogOptions {
        strict: Some(true),
        ..crate::LogOptions::default()
    });

    let response =
        crate::operation::handle_log(fixture.path(), request, "op-log-strict", &registry).unwrap();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Failed
    );
    let batch = registry
        .read(
            &response.output.log_id,
            &crate::operation::CommitLogReadRequest::default(),
        )
        .unwrap();
    assert_eq!(batch.records.len(), 2);
    assert_eq!(
        batch.records[0].kind,
        crate::LogOutputRecordKind::Degradation
    );
    assert_eq!(
        batch.records[0].degradation.as_ref().unwrap().reason,
        crate::LogDegradationReason::Unborn
    );
    assert_eq!(batch.records[1].kind, crate::LogOutputRecordKind::Entry);
}

#[test]
fn l_coa_6_invalid_marker_projects_the_frozen_additive_token() {
    let entry = CommitLogEntry {
        target: CommitLogTarget {
            member_id: "mem_bad".to_owned(),
            member_path: "bad".to_owned(),
            source_kind: ArtifactSourceKind::Git,
        },
        commit_id: "d".repeat(40),
        parent_ids: Vec::new(),
        author: CommitLogIdentity {
            name: b"Author".to_vec(),
            email: b"author@example.invalid".to_vec(),
            time: CommitLogTime {
                seconds: 1,
                offset_minutes: 0,
            },
        },
        committer: CommitLogIdentity {
            name: b"Author".to_vec(),
            email: b"author@example.invalid".to_vec(),
            time: CommitLogTime {
                seconds: 1,
                offset_minutes: 0,
            },
        },
        message: b"invalid\n\nGWZ-Commit-ID=not-a-v7\nGWZ-Workspace-ID: ws_test\n".to_vec(),
        message_encoding: None,
    };
    let group = super::coalesce::assemble_commit_log_groups([entry], true)
        .into_iter()
        .next()
        .unwrap();
    let record =
        super::handler::project_merged_event(CommitLogMergedEvent::Group(group), false).unwrap();
    let provenance = record.entry.unwrap().provenance;
    assert_eq!(provenance.kind, crate::LogMergeKind::None);
    assert_eq!(provenance.gwz_commit_id.as_deref(), Some("marker-invalid"));
}

struct Fixture {
    path: PathBuf,
    root: Repository,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gwz-core-commit-log-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        let root = Repository::init(&path).unwrap();
        root.set_head("refs/heads/main").unwrap();
        Self { path, root }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_manifest(&self, members: &[ManifestMember]) {
        write_manifest_at(&self.path, members);
    }

    fn write_snapshot(&self, id: &str, members: &[(&str, &str, Option<Oid>)]) {
        self.write_snapshot_for_workspace(id, "ws_test", members);
    }

    fn write_lock(&self, members: &[(&str, &str, Option<Oid>)]) {
        let members = self
            .snapshot_artifact("lock-fixture", "ws_test", members)
            .members;
        crate::artifact::write_lock(
            self.path(),
            &LockArtifact {
                schema: LOCK_SCHEMA.to_owned(),
                workspace_id: "ws_test".to_owned(),
                manifest_schema: WORKSPACE_SCHEMA.to_owned(),
                members,
            },
        )
        .unwrap();
    }

    fn write_snapshot_for_workspace(
        &self,
        id: &str,
        workspace_id: &str,
        members: &[(&str, &str, Option<Oid>)],
    ) {
        crate::artifact::write_snapshot(
            self.path(),
            &self.snapshot_artifact(id, workspace_id, members),
        )
        .unwrap();
    }

    fn write_legacy_snapshot(&self, id: &str, members: &[(&str, &str, Option<Oid>)]) {
        let artifact = self.snapshot_artifact("legacy_placeholder", "ws_test", members);
        let yaml = artifact.to_yaml().unwrap().replace(
            "snapshot_id: legacy_placeholder",
            &format!("snapshot_id: {id}"),
        );
        fs::create_dir_all(self.path().join(crate::artifact::SNAPSHOT_DIR)).unwrap();
        fs::write(
            self.path()
                .join(crate::artifact::SNAPSHOT_DIR)
                .join(format!("{id}.yaml")),
            yaml,
        )
        .unwrap();
    }

    fn snapshot_artifact(
        &self,
        id: &str,
        workspace_id: &str,
        members: &[(&str, &str, Option<Oid>)],
    ) -> SnapshotArtifact {
        let members: std::collections::BTreeMap<String, ResolvedMemberArtifact> = members
            .iter()
            .map(|(member_id, member_path, commit)| {
                (
                    (*member_id).to_owned(),
                    ResolvedMemberArtifact {
                        path: (*member_path).to_owned(),
                        source_id: Some(format!("src_{}", member_id.trim_start_matches("mem_"))),
                        source_kind: ArtifactSourceKind::Git,
                        commit: commit.map(|oid| oid.to_string()),
                        branch: Some("main".to_owned()),
                        detached: Some(false),
                        upstream: None,
                        dirty: Some(false),
                        materialized: Some(true),
                    },
                )
            })
            .collect();
        SnapshotArtifact {
            schema: SNAPSHOT_SCHEMA.to_owned(),
            workspace_id: workspace_id.to_owned(),
            snapshot_id: id.to_owned(),
            created_at: "2026-08-30T00:00:00Z".to_owned(),
            created_by: CreatedByArtifact {
                actor_id: "agent_test".to_owned(),
            },
            selected_members: members.keys().cloned().collect(),
            members,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
