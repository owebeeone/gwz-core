//! C1 identity, date, message, and empty-message characterization.
//!
//! This child exercises the existing porcelain route as evidence only. It does
//! not select a native replacement or widen the backend contract.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use super::*;

const CHILD: &str = "GWZ_C1_IDENTITY_CHARACTERIZATION_CHILD";

fn temp_dir(prefix: &str) -> TempDir {
    if cfg!(windows) {
        let root = Path::new("D:/gwz-tests");
        fs::create_dir_all(root).unwrap();
        tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(root)
            .unwrap()
    } else {
        TempDir::new().unwrap()
    }
}

fn run_in_clean_child(test_name: &str, variables: &[(&str, &str)]) -> bool {
    if env::var_os(CHILD).is_some() {
        return false;
    }
    let environment = temp_dir("c1-environment");
    let global = environment.path().join("global.gitconfig");
    fs::write(
        &global,
        b"[user]\n\tname = Global User\n\temail = global@example.invalid\n",
    )
    .unwrap();
    let system = environment.path().join("system.gitconfig");
    fs::write(&system, b"").unwrap();
    let path = env::var_os("PATH").expect("parent PATH is required for git");
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .env_clear()
        .env("PATH", path)
        .env("HOME", environment.path())
        .env("USERPROFILE", environment.path())
        .env("GIT_CONFIG_GLOBAL", &global)
        .env("GIT_CONFIG_SYSTEM", &system)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env(CHILD, "1")
        .args(["--exact", test_name, "--nocapture", "--test-threads", "1"]);
    for (key, value) in variables {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let selected = format!("test {test_name} ... ok");
    assert!(
        output.status.success(),
        "C1 child failed: stdout={stdout} stderr={stderr}"
    );
    assert_eq!(
        stdout.matches(selected.as_str()).count(),
        1,
        "C1 child did not execute exactly one selected test: {stdout}"
    );
    true
}

fn repository(
    label: &str,
    local_identity: Option<(&str, &str)>,
) -> (TempDir, PathBuf, Git2Backend) {
    let temp = temp_dir(label);
    let path = temp.path().join(label);
    let backend = Git2Backend::new();
    backend.create_repo(&path).unwrap();
    let repo = git2::Repository::open(&path).unwrap();
    let mut config = repo.config().unwrap();
    if let Some((name, email)) = local_identity {
        config.set_str("user.name", name).unwrap();
        config.set_str("user.email", email).unwrap();
    }
    let hooks = temp.path().join(format!("{label}-hooks"));
    fs::create_dir_all(&hooks).unwrap();
    config
        .set_str("core.hooksPath", hooks.to_str().unwrap())
        .unwrap();
    config.set_str("core.editor", "/usr/bin/false").unwrap();
    config.set_bool("commit.gpgSign", false).unwrap();
    config.set_bool("tag.gpgSign", false).unwrap();
    config.set_str("gc.auto", "0").unwrap();
    (temp, path, backend)
}

fn stage(path: &Path, backend: &Git2Backend, text: &[u8]) {
    fs::write(path.join("tracked.txt"), text).unwrap();
    backend.stage_paths(path, &["tracked.txt"]).unwrap();
}

fn signature_fields(signature: &git2::Signature<'_>) -> (Vec<u8>, Vec<u8>, i64, i32) {
    (
        signature.name_bytes().to_vec(),
        signature.email_bytes().to_vec(),
        signature.when().seconds(),
        signature.when().offset_minutes(),
    )
}

fn commit_signatures(
    path: &Path,
    oid: &str,
) -> ((Vec<u8>, Vec<u8>, i64, i32), (Vec<u8>, Vec<u8>, i64, i32)) {
    let repo = git2::Repository::open(path).unwrap();
    let commit = repo.find_commit(git2::Oid::from_str(oid).unwrap()).unwrap();
    (
        signature_fields(&commit.author()),
        signature_fields(&commit.committer()),
    )
}

fn commit_message(path: &Path, oid: &str) -> Vec<u8> {
    let repo = git2::Repository::open(path).unwrap();
    let odb = repo.odb().unwrap();
    let object = odb.read(git2::Oid::from_str(oid).unwrap()).unwrap();
    let data = object.data();
    let start = data.windows(2).position(|bytes| bytes == b"\n\n").unwrap() + 2;
    data[start..].to_vec()
}

fn staged_blob(path: &Path) -> Vec<u8> {
    let repo = git2::Repository::open(path).unwrap();
    let index = repo.index().unwrap();
    let entry = index.get_path(Path::new("tracked.txt"), 0).unwrap();
    repo.find_blob(entry.id).unwrap().content().to_vec()
}

fn index_entries(path: &Path) -> Vec<(Vec<u8>, u32, git2::Oid)> {
    let repo = git2::Repository::open(path).unwrap();
    repo.index()
        .unwrap()
        .iter()
        .map(|e| (e.path, e.mode, e.id))
        .collect()
}

fn tagger_fields(path: &Path, name: &str) -> (Vec<u8>, Vec<u8>, i64, i32) {
    let repo = git2::Repository::open(path).unwrap();
    let oid = repo
        .find_reference(&format!("refs/tags/{name}"))
        .unwrap()
        .target()
        .unwrap();
    signature_fields(&repo.find_tag(oid).unwrap().tagger().unwrap())
}

#[test]
fn commit_identity_repository_precedes_global() {
    if run_in_clean_child(
        "git::gitbackend::commit_tag_identity_characterization::commit_identity_repository_precedes_global",
        &[],
    ) {
        return;
    }
    let (_temp, path, backend) = repository(
        "c1-id-local",
        Some(("Repository User", "repository@example.invalid")),
    );
    stage(&path, &backend, b"identity\n");
    let oid = backend.commit(&path, "identity", false).unwrap().commit;
    let (author, committer) = commit_signatures(&path, &oid);
    assert_eq!(author.0, b"Repository User");
    assert_eq!(author.1, b"repository@example.invalid");
    assert_eq!(committer.0, b"Repository User");
    assert_eq!(committer.1, b"repository@example.invalid");
}

#[test]
fn commit_author_environment_precedes_local_identity() {
    if run_in_clean_child(
        "git::gitbackend::commit_tag_identity_characterization::commit_author_environment_precedes_local_identity",
        &[
            ("GIT_AUTHOR_NAME", "Environment Author"),
            ("GIT_AUTHOR_EMAIL", "environment-author@example.invalid"),
        ],
    ) {
        return;
    }
    let (_temp, path, backend) = repository(
        "c1-id-author-env",
        Some(("Repository User", "repository@example.invalid")),
    );
    stage(&path, &backend, b"identity\n");
    let oid = backend.commit(&path, "identity", false).unwrap().commit;
    let (author, committer) = commit_signatures(&path, &oid);
    assert_eq!(author.0, b"Environment Author");
    assert_eq!(author.1, b"environment-author@example.invalid");
    assert_eq!(committer.0, b"Repository User");
    assert_eq!(committer.1, b"repository@example.invalid");
    backend
        .tag_create(&path, "author-env-tag", Some("tag"), false)
        .unwrap();
    let tagger = tagger_fields(&path, "author-env-tag");
    assert_eq!(tagger.0, b"Repository User");
    assert_eq!(tagger.1, b"repository@example.invalid");
}

#[test]
fn commit_committer_environment_precedes_local_identity() {
    if run_in_clean_child(
        "git::gitbackend::commit_tag_identity_characterization::commit_committer_environment_precedes_local_identity",
        &[
            ("GIT_COMMITTER_NAME", "Environment Committer"),
            (
                "GIT_COMMITTER_EMAIL",
                "environment-committer@example.invalid",
            ),
        ],
    ) {
        return;
    }
    let (_temp, path, backend) = repository(
        "c1-id-committer-env",
        Some(("Repository User", "repository@example.invalid")),
    );
    stage(&path, &backend, b"identity\n");
    let commit_oid = backend.commit(&path, "identity", false).unwrap().commit;
    let (author, committer) = commit_signatures(&path, &commit_oid);
    assert_eq!(author.0, b"Repository User");
    assert_eq!(author.1, b"repository@example.invalid");
    assert_eq!(committer.0, b"Environment Committer");
    assert_eq!(committer.1, b"environment-committer@example.invalid");
    backend
        .tag_create(&path, "committer-env-tag", Some("tag"), false)
        .unwrap();
    let tagger = tagger_fields(&path, "committer-env-tag");
    assert_eq!(tagger.0, b"Environment Committer");
    assert_eq!(tagger.1, b"environment-committer@example.invalid");
}

#[test]
fn commit_and_tag_identity_use_hermetic_global_config_without_local_values() {
    if run_in_clean_child(
        "git::gitbackend::commit_tag_identity_characterization::commit_and_tag_identity_use_hermetic_global_config_without_local_values",
        &[],
    ) {
        return;
    }
    let (_temp, path, backend) = repository("c1-id-global", None);
    stage(&path, &backend, b"global\n");
    let commit_oid = backend.commit(&path, "global", false).unwrap().commit;
    let (author, committer) = commit_signatures(&path, &commit_oid);
    assert_eq!(author.0, b"Global User");
    assert_eq!(author.1, b"global@example.invalid");
    assert_eq!(committer.0, b"Global User");
    assert_eq!(committer.1, b"global@example.invalid");

    let tag_result = backend
        .tag_create(&path, "global-tag", Some("tag message"), false)
        .unwrap();
    let repo = git2::Repository::open(&path).unwrap();
    let tag_oid = repo
        .find_reference("refs/tags/global-tag")
        .unwrap()
        .target()
        .unwrap();
    let tag = repo.find_tag(tag_oid).unwrap();
    assert_eq!(tag.target_id().to_string(), tag_result.commit);
    assert_eq!(tag.message_bytes().unwrap(), b"tag message\n");
    let tagger = tag.tagger().unwrap();
    assert_eq!(tagger.name_bytes(), b"Global User");
    assert_eq!(tagger.email_bytes(), b"global@example.invalid");
}

#[test]
fn commit_dates_preserve_author_and_committer_environment_values() {
    if run_in_clean_child(
        "git::gitbackend::commit_tag_identity_characterization::commit_dates_preserve_author_and_committer_environment_values",
        &[
            ("GIT_AUTHOR_DATE", "2001-02-03T04:05:06 +0530"),
            ("GIT_COMMITTER_DATE", "2002-03-04T05:06:07 -0700"),
        ],
    ) {
        return;
    }
    let (_temp, path, backend) = repository("c1-date", None);
    stage(&path, &backend, b"date\n");
    let oid = backend.commit(&path, "date", false).unwrap().commit;
    let (author, committer) = commit_signatures(&path, &oid);
    assert_eq!(author.2, 981_153_306);
    assert_eq!(author.3, 330);
    assert_eq!(committer.2, 1_015_243_567);
    assert_eq!(committer.3, -420);
    backend
        .tag_create(&path, "date-tag", Some("date"), false)
        .unwrap();
    let tagger = tagger_fields(&path, "date-tag");
    assert_eq!(tagger.2, 1_015_243_567);
    assert_eq!(tagger.3, -420);
    let light = backend
        .tag_create(&path, "date-light", None, false)
        .unwrap();
    let repo = git2::Repository::open(&path).unwrap();
    let light_oid = repo
        .find_reference("refs/tags/date-light")
        .unwrap()
        .target()
        .unwrap();
    assert_eq!(light_oid.to_string(), light.commit);
    assert_eq!(
        repo.find_object(light_oid, None).unwrap().kind(),
        Some(git2::ObjectType::Commit)
    );
}

#[test]
fn commit_cleanup_default_strip_and_verbatim_are_stored_by_git() {
    if run_in_clean_child(
        "git::gitbackend::commit_tag_identity_characterization::commit_cleanup_default_strip_and_verbatim_are_stored_by_git",
        &[],
    ) {
        return;
    }
    let message = "\n  body  \n# comment\nnext  \n\n";
    let cases = [
        ("default", None, b"  body\n# comment\nnext\n".as_slice()),
        ("strip", Some("strip"), b"  body\nnext\n".as_slice()),
        ("verbatim", Some("verbatim"), message.as_bytes()),
    ];
    for (label, cleanup, expected) in cases {
        let (_temp, path, backend) = repository(
            &format!("c1-cleanup-{label}"),
            Some(("Cleanup User", "cleanup@example.invalid")),
        );
        if let Some(cleanup) = cleanup {
            git2::Repository::open(&path)
                .unwrap()
                .config()
                .unwrap()
                .set_str("commit.cleanup", cleanup)
                .unwrap();
        }
        stage(&path, &backend, b"cleanup\n");
        let oid = backend.commit(&path, message, false).unwrap().commit;
        assert_eq!(commit_message(&path, &oid), expected, "cleanup={label}");
    }
}

#[test]
fn empty_messages_preserve_refs_and_staged_content_but_refresh_index() {
    if run_in_clean_child(
        "git::gitbackend::commit_tag_identity_characterization::empty_messages_preserve_refs_and_staged_content_but_refresh_index",
        &[],
    ) {
        return;
    }
    let (_temp, path, backend) =
        repository("c1-empty", Some(("Empty User", "empty@example.invalid")));
    stage(&path, &backend, b"base\n");
    let base = backend.commit(&path, "base", false).unwrap().commit;
    stage(&path, &backend, b"pending\n");
    let index = path.join(".git").join("index");
    let head_ref = path.join(".git").join("refs/heads/main");
    let before_index = fs::read(&index).unwrap();
    let before_entries = index_entries(&path);
    let head_log = path.join(".git/logs/HEAD");
    let before_log = fs::read(&head_log).unwrap();
    let before_ref = fs::read(&head_ref).unwrap();
    let before_staged_blob = staged_blob(&path);
    let before_worktree = fs::read(path.join("tracked.txt")).unwrap();
    for (attempt, message) in ["", "   "].into_iter().enumerate() {
        let error = backend.commit(&path, message, false).unwrap_err();
        assert_eq!(error.code, ErrorCode::GitCommandFailed);
        assert_eq!(
            backend.head(&path).unwrap().commit.as_deref(),
            Some(base.as_str())
        );
        // First rejection changes raw index bytes without publishing a commit.
        if attempt == 0 {
            assert_ne!(fs::read(&index).unwrap(), before_index);
        }
        assert_eq!(index_entries(&path), before_entries);
        assert_eq!(fs::read(&head_log).unwrap(), before_log);
        assert_eq!(fs::read(path.join(".git/COMMIT_EDITMSG")).unwrap(), b"");
        assert_eq!(fs::read(&head_ref).unwrap(), before_ref);
        assert_eq!(staged_blob(&path), before_staged_blob);
        assert_eq!(fs::read(path.join("tracked.txt")).unwrap(), before_worktree);
    }
}
