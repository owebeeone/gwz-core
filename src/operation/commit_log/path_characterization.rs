//! Focused native-oracle characterization for the no-fallback history design.
//!
//! This child module intentionally reuses the private fixture, request, and
//! native-oracle helpers from `tests.rs`.  It records behavior that a future
//! path-sensitive walk must preserve without changing the current engine.

use super::*;

#[test]
fn l4_attr_magic_current_cursor_exposes_worktree_context_gap() {
    let fixture = Fixture::new("attr-magic");
    let (head, base) = build_attr_path_history(fixture.path());
    fixture.write_manifest(&[]);

    for (pathspecs, expected_native, expected_current) in [
        (vec![":(attr:gwz-path)src"], vec![head, base], Vec::new()),
        (vec![":(attr:-gwz-path)src"], vec![base], Vec::new()),
        (vec![":(attr:gwz-path=blue)src"], vec![base], Vec::new()),
        (vec![":(attr:gwz-path=green)src"], Vec::new(), Vec::new()),
        (vec![":(attr:!gwz-path)src"], Vec::new(), vec![head, base]),
    ] {
        let request = log_request(&[], &pathspecs, false);
        let opened = open_request_histories(fixture.path(), &request).unwrap();
        let actual = entry_ids(&opened.histories()[0]);
        let expected = native_rev_list_ids(fixture.path(), &[], &pathspecs);
        assert_eq!(expected, expected_native, "native {pathspecs:?}");
        assert_eq!(actual, expected_current, "current {pathspecs:?}");
    }
}

#[test]
fn l4_attr_magic_preserves_long_envelope_and_records_worktree_gap() {
    let fixture = Fixture::new("attr-magic-routing");
    let (head, base) = build_attr_path_history(fixture.path());
    fixture.write_manifest(&[]);

    let pathspecs = [":(top,attr:gwz-path)src"];
    let request = log_request(&[], &pathspecs, false);
    let opened = open_request_histories(fixture.path(), &request).unwrap();
    assert_eq!(target_ids(&opened), ["@root"]);
    assert_eq!(opened.histories()[0].pathspecs(), pathspecs);
    assert_ne!(
        entry_ids(&opened.histories()[0]),
        native_rev_list_ids(fixture.path(), &[], &pathspecs)
    );
    assert_eq!(
        entry_ids(&opened.histories()[0]),
        Vec::<Oid>::new(),
        "current --git-dir cursor has no worktree attributes"
    );
    assert_eq!(
        native_rev_list_ids(fixture.path(), &[], &pathspecs),
        vec![head, base]
    );
}

#[test]
fn l4_ordered_merge_range_and_first_parent_match_native() {
    let fixture = Fixture::new("ordered-merge-range-first-parent");
    build_native_path_history(fixture.path());
    fixture.write_manifest(&[]);

    for (operands, pathspecs, options, native_flags) in [
        (&[][..], &["p"][..], crate::LogOptions::default(), &[][..]),
        (
            &["HEAD~3..HEAD"][..],
            &["p"][..],
            crate::LogOptions::default(),
            &[][..],
        ),
        (
            &["HEAD~3..HEAD"][..],
            &["p"][..],
            crate::LogOptions {
                first_parent: Some(true),
                ..Default::default()
            },
            &["--first-parent"][..],
        ),
        (
            &["HEAD~3..HEAD"][..],
            &["p"][..],
            crate::LogOptions {
                no_merges: Some(true),
                ..Default::default()
            },
            &["--no-merges"][..],
        ),
    ] {
        let mut request = log_request(operands, pathspecs, false);
        request.options = Some(options);
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
            .args(native_flags)
            .args(if operands.is_empty() {
                &["HEAD"][..]
            } else {
                operands
            });
        native.arg("--").args(pathspecs);
        let expected = git_oid_lines(&mut native);
        assert_eq!(
            actual, expected,
            "{operands:?} {pathspecs:?} {native_flags:?}"
        );
    }
}

fn build_attr_path_history(path: &Path) -> (Oid, Oid) {
    git_ok(path, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git_ok(path, &["config", "user.name", "Test Author"]);
    git_ok(path, &["config", "user.email", "test@example.com"]);
    fs::create_dir_all(path.join("src")).unwrap();
    fs::write(path.join("src/set"), "set\n").unwrap();
    fs::write(path.join("src/unset"), "unset\n").unwrap();
    fs::write(path.join("src/value"), "value\n").unwrap();
    fs::write(
        path.join(".gitattributes"),
        "src/set gwz-path\nsrc/unset -gwz-path\nsrc/value gwz-path=blue\n",
    )
    .unwrap();
    git_ok(path, &["add", "."]);
    git_ok(path, &["commit", "--quiet", "-m", "attribute base"]);
    let base = Repository::open(path)
        .unwrap()
        .head()
        .unwrap()
        .target()
        .unwrap();
    fs::write(path.join("src/set"), "set two\n").unwrap();
    git_ok(path, &["commit", "--quiet", "-am", "attribute set"]);
    let head = Repository::open(path)
        .unwrap()
        .head()
        .unwrap()
        .target()
        .unwrap();
    (head, base)
}
