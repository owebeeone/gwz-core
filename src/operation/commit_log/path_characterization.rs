//! Focused native-oracle characterization for the no-fallback history design.
//!
//! This child module intentionally reuses the private fixture, request, and
//! native-oracle helpers from `tests.rs`.  It records behavior that a future
//! path-sensitive walk must preserve without changing the current engine.

use super::*;

fn run_in_clean_child(test_name: &str) -> bool {
    const MARKER: &str = "GWZ_HISTORY_CHARACTERIZATION_CHILD";
    if std::env::var_os(MARKER).is_some() {
        return false;
    }

    let suffix = format!(
        "{}-{}-{}",
        std::process::id(),
        test_name.replace("::", "-"),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = if cfg!(windows) {
        PathBuf::from("D:/gwz-tests").join(suffix)
    } else {
        std::env::temp_dir().join(format!("gwz-history-tests-{suffix}"))
    };
    let home = root.join("home");
    let xdg = root.join("xdg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&xdg).unwrap();
    let path = std::env::var_os("PATH").expect("PATH is required to find git");
    let full_name = format!("operation::commit_log::tests::path_characterization::{test_name}");
    let output = Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .current_dir(&root)
        .env("PATH", path)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("APPDATA", &home)
        .env("LOCALAPPDATA", &home)
        .env("XDG_CONFIG_HOME", &xdg)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", root.join("missing-system-config"))
        .env("GIT_CONFIG_GLOBAL", root.join("missing-global-config"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Test Author")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test Author")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC")
        .env("TMPDIR", &root)
        .env("TMP", &root)
        .env("TEMP", &root)
        .env(MARKER, "1")
        .args([
            "--exact",
            full_name.as_str(),
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .output()
        .unwrap_or_else(|error| {
            let _ = fs::remove_dir_all(&root);
            panic!("spawn clean characterization child: {error}");
        });
    let status_success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = fs::remove_dir_all(&root);
    assert!(
        status_success,
        "clean characterization child failed:\n{}\n{}",
        stdout, stderr
    );
    let ran = stdout
        .lines()
        .filter(|line| line.starts_with("test ") && line.ends_with("... ok"))
        .count();
    assert_eq!(ran, 1, "clean child did not run exactly one test");
    assert!(
        stdout.contains(&format!("test {full_name} ... ok")),
        "clean child omitted the selected test"
    );
    true
}

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

#[test]
fn h_member_direct_attributes_match_native_and_current() {
    if run_in_clean_child("h_member_direct_attributes_match_native_and_current") {
        return;
    }
    let fixture = Fixture::new("h-member-attributes");
    let member_path = fixture.path().join("app");
    Repository::init(&member_path).unwrap();
    let (member_head, member_base) = build_attr_path_history(&member_path);
    fixture.write_manifest(&[member("mem_app", "app", true)]);
    let cases = [
        (
            ":(attr:gwz-path)app/src",
            ":(attr:gwz-path)src",
            vec![member_head, member_base],
            Vec::new(),
        ),
        (
            ":(attr:-gwz-path)app/src",
            ":(attr:-gwz-path)src",
            vec![member_base],
            Vec::new(),
        ),
        (
            ":(attr:gwz-path=blue)app/src",
            ":(attr:gwz-path=blue)src",
            vec![member_base],
            Vec::new(),
        ),
        (
            ":(attr:!gwz-path)app/src",
            ":(attr:!gwz-path)src",
            Vec::new(),
            vec![member_head, member_base],
        ),
    ];
    let before_git = repository_bytes(&member_path.join(".git"));
    let before_attributes = fs::read(member_path.join(".gitattributes")).unwrap();

    for (workspace_spec, repo_spec, native_expected, current_expected) in cases {
        let request = log_request(&[], &[workspace_spec], false);
        let opened = open_request_histories(fixture.path(), &request).unwrap();
        assert_eq!(target_ids(&opened), ["mem_app"], "{workspace_spec}");
        assert_eq!(
            opened.histories()[0].pathspecs(),
            [repo_spec],
            "{workspace_spec}"
        );
        assert_eq!(
            native_rev_list_ids(&member_path, &[], &[repo_spec]),
            native_expected,
            "native {repo_spec}"
        );
        assert_eq!(
            entry_ids(&opened.histories()[0]),
            current_expected,
            "current {repo_spec}"
        );
    }
    assert_eq!(repository_bytes(&member_path.join(".git")), before_git);
    assert_eq!(
        fs::read(member_path.join(".gitattributes")).unwrap(),
        before_attributes
    );
}

#[test]
fn h_bare_attributes_and_info_override_match_native() {
    if run_in_clean_child("h_bare_attributes_and_info_override_match_native") {
        return;
    }
    let fixture = Fixture::new("h-bare-attributes");
    let seed = Fixture::new("h-bare-attributes-seed");
    build_attr_path_history(seed.path());
    let bare_path = fixture.path().join("bare");
    let clone = Command::new("git")
        .arg("clone")
        .arg("--bare")
        .arg(seed.path())
        .arg(&bare_path)
        .status()
        .unwrap();
    assert!(clone.success(), "bare fixture clone failed: {clone}");
    fixture.write_manifest(&[member("mem_bare", "bare", true)]);

    let baseline_git = repository_bytes(&bare_path);
    let baseline_info = bare_path.join("info").join("attributes");
    let baseline_info_bytes = fs::read(&baseline_info).ok();
    let (baseline_native, baseline_current) = bare_attribute_rows(fixture.path(), &bare_path);
    let baseline_after_git = repository_bytes(&bare_path);
    let baseline_after_info = fs::read(&baseline_info).ok();

    fs::write(
        &baseline_info,
        "src/set -gwz-path\nsrc/unset gwz-path\nsrc/value gwz-path=green\n",
    )
    .unwrap();
    let override_git = repository_bytes(&bare_path);
    let override_info_bytes = fs::read(&baseline_info).unwrap();
    let (override_native, override_current) = bare_attribute_rows(fixture.path(), &bare_path);

    let seed_repo = Repository::open(seed.path()).unwrap();
    let seed_head = seed_repo.head().unwrap().target().unwrap();
    let seed_commit = seed_repo
        .find_commit(seed_head)
        .unwrap()
        .parent_id(0)
        .unwrap();
    assert_eq!(
        baseline_native,
        vec![
            (":(attr:gwz-path)src".to_owned(), Vec::new()),
            (
                ":(attr:!gwz-path)src".to_owned(),
                vec![seed_head, seed_commit]
            ),
        ]
    );
    assert_eq!(
        override_native,
        vec![
            (":(attr:gwz-path)src".to_owned(), vec![seed_commit]),
            (":(attr:!gwz-path)src".to_owned(), Vec::new()),
        ]
    );
    assert_bare_current_rows(&baseline_current, &baseline_native, "H-BARE");
    assert_bare_current_rows(&override_current, &override_native, "H-INFO");
    eprintln!("H-BARE baseline current: {baseline_current:?}");
    eprintln!("H-INFO override current: {override_current:?}");
    assert_eq!(baseline_git, baseline_after_git);
    assert_eq!(baseline_info_bytes, baseline_after_info);
    assert_eq!(repository_bytes(&bare_path), override_git);
    assert_eq!(fs::read(&baseline_info).unwrap(), override_info_bytes);
    assert_ne!(
        baseline_git, override_git,
        "the info file is part of the snapshot"
    );
    assert_eq!(baseline_info_bytes, None);
}

fn assert_bare_current_rows(
    current: &[(String, Result<Vec<Oid>, String>)],
    native: &[(String, Vec<Oid>)],
    label: &str,
) {
    assert_eq!(current.len(), native.len(), "{label} row count");
    for ((current_spec, current_ids), (native_spec, native_ids)) in current.iter().zip(native) {
        assert_eq!(current_spec, native_spec, "{label} pathspec");
        match current_ids {
            Ok(current_ids) => assert_eq!(current_ids, native_ids, "{label} {native_spec}"),
            Err(detail) => panic!("{label} unexpected current refusal: {detail}"),
        }
    }
}

fn bare_attribute_rows(
    workspace: &Path,
    bare: &Path,
) -> (
    Vec<(String, Vec<Oid>)>,
    Vec<(String, Result<Vec<Oid>, String>)>,
) {
    let mut native_rows = Vec::new();
    let mut current_rows = Vec::new();
    [":(attr:gwz-path)bare/src", ":(attr:!gwz-path)bare/src"]
        .into_iter()
        .for_each(|workspace_spec| {
            let repo_spec = workspace_spec.replacen("bare/", "", 1);
            let native = native_bare_rev_list_ids(bare, &repo_spec);
            let request = log_request(&[], &[workspace_spec], false);
            native_rows.push((repo_spec.to_owned(), native));
            current_rows.push((
                repo_spec.to_owned(),
                current_ids_or_refusal(workspace, &request, &repo_spec),
            ));
        });
    (native_rows, current_rows)
}

fn current_ids_or_refusal(
    workspace: &Path,
    request: &crate::LogRequest,
    repo_spec: &str,
) -> Result<Vec<Oid>, String> {
    let opened = open_request_histories(workspace, request)
        .map_err(|error| format!("request refused: {:?}: {}", error.code, error.message))?;
    if target_ids(&opened) != ["mem_bare"] {
        return Err(format!("selected targets: {:?}", target_ids(&opened)));
    }
    let history = opened
        .histories()
        .first()
        .ok_or_else(|| "no selected history".to_owned())?;
    if history.pathspecs() != [repo_spec] {
        return Err(format!("routed pathspecs: {:?}", history.pathspecs()));
    }
    let mut ids = Vec::new();
    for event in history.messages() {
        match event {
            CommitLogEvent::Entry(entry) => {
                ids.push(Oid::from_str(&entry.commit_id).unwrap());
            }
            CommitLogEvent::Degradation(record) => {
                return Err(format!("history refused: {record:?}"));
            }
        }
    }
    Ok(ids)
}

fn native_bare_rev_list_ids(path: &Path, pathspec: &str) -> Vec<Oid> {
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(path)
        .args(["rev-list", "HEAD", "--", pathspec]);
    git_oid_lines(&mut command)
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
    fixed_git_ok(
        path,
        &["commit", "--quiet", "-m", "attribute base"],
        "2020-01-01T00:00:00+0000",
    );
    let base = Repository::open(path)
        .unwrap()
        .head()
        .unwrap()
        .target()
        .unwrap();
    fs::write(path.join("src/set"), "set two\n").unwrap();
    fixed_git_ok(
        path,
        &["commit", "--quiet", "-am", "attribute set"],
        "2020-01-01T00:00:01+0000",
    );
    let head = Repository::open(path)
        .unwrap()
        .head()
        .unwrap()
        .target()
        .unwrap();
    (head, base)
}

fn fixed_git_ok(path: &Path, args: &[&str], date: &str) {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed with {status}");
}
