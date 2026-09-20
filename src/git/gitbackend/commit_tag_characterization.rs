//! Physical-backend characterization for the commit and tag subprocess boundary.
//!
//! These tests deliberately exercise the existing porcelain calls.  They are
//! evidence for a later native replacement; they do not select one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use super::*;

fn repository(label: &str) -> (TempDir, PathBuf, Git2Backend) {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join(label);
    let backend = Git2Backend::new();
    backend.create_repo(&path).unwrap();
    let repo = git2::Repository::open(&path).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "GWZ Characterization").unwrap();
    config
        .set_str("user.email", "gwz-characterization@example.invalid")
        .unwrap();
    let hooks = path.join(".git").join("gwz-hooks");
    fs::create_dir_all(&hooks).unwrap();
    config
        .set_str("core.hooksPath", hooks.to_str().unwrap())
        .unwrap();
    config.set_str("core.editor", "/usr/bin/false").unwrap();
    config.set_bool("commit.gpgSign", false).unwrap();
    config.set_bool("tag.gpgSign", false).unwrap();
    config.set_str("gpg.format", "openpgp").unwrap();
    config.set_str("gc.auto", "0").unwrap();
    (temp, path, backend)
}

fn seed(path: &Path, backend: &Git2Backend) -> String {
    fs::write(path.join("tracked.txt"), "base\n").unwrap();
    backend.stage_paths(path, &["tracked.txt"]).unwrap();
    backend.commit(path, "base", false).unwrap().commit
}

fn commit_message(path: &Path, oid: &str) -> String {
    let repo = git2::Repository::open(path).unwrap();
    repo.find_commit(git2::Oid::from_str(oid).unwrap())
        .unwrap()
        .message()
        .unwrap()
        .to_owned()
}

fn tree_file(path: &Path, oid: &str, name: &str) -> Option<Vec<u8>> {
    let repo = git2::Repository::open(path).unwrap();
    let commit = repo.find_commit(git2::Oid::from_str(oid).unwrap()).unwrap();
    let entry = commit.tree().unwrap().get_path(Path::new(name)).ok()?;
    let blob = entry.to_object(&repo).unwrap().into_blob().ok()?;
    Some(blob.content().to_vec())
}

fn run_in_clean_child(test_name: &str) -> bool {
    const MARKER: &str = "GWZ_COMMIT_TAG_CHARACTERIZATION_CHILD";
    if std::env::var_os(MARKER).is_some() {
        return false;
    }
    let path = std::env::var_os("PATH").expect("PATH is required to find git");
    let status = Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .env("PATH", path)
        .env(MARKER, "1")
        .args(["--exact", test_name, "--nocapture", "--test-threads", "1"])
        .status()
        .expect("spawn clean characterization child");
    assert!(status.success(), "clean characterization child failed");
    true
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn shell_path(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
    }

    fn hook(repo: &Path, name: &str, body: &str) -> PathBuf {
        let hooks = repo.join(".git").join("gwz-hooks");
        fs::create_dir_all(&hooks).unwrap();
        let path = hooks.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        git2::Repository::open(repo)
            .unwrap()
            .config()
            .unwrap()
            .set_str("core.hooksPath", hooks.to_str().unwrap())
            .unwrap();
        path
    }

    #[test]
    fn commit_uses_index_by_default_and_all_only_collects_tracked_work() {
        if run_in_clean_child(
            "git::gitbackend::commit_tag_characterization::unix::commit_uses_index_by_default_and_all_only_collects_tracked_work",
        ) {
            return;
        }
        let (_temp, path, backend) = repository("commit-index-all");
        let base = seed(&path, &backend);

        fs::write(path.join("tracked.txt"), "staged\n").unwrap();
        backend.stage_paths(&path, &["tracked.txt"]).unwrap();
        fs::write(path.join("tracked.txt"), "unstaged\n").unwrap();
        let indexed = backend.commit(&path, "indexed", false).unwrap().commit;
        assert_ne!(indexed, base);
        assert_eq!(
            tree_file(&path, &indexed, "tracked.txt").unwrap(),
            b"staged\n"
        );
        assert_eq!(fs::read(path.join("tracked.txt")).unwrap(), b"unstaged\n");

        fs::write(path.join("tracked.txt"), "all\n").unwrap();
        fs::write(path.join("untracked.txt"), "loose\n").unwrap();
        let all = backend.commit(&path, "all", true).unwrap().commit;
        assert_eq!(tree_file(&path, &all, "tracked.txt").unwrap(), b"all\n");
        assert_eq!(tree_file(&path, &all, "untracked.txt"), None);
        assert_eq!(backend.status(&path).unwrap().untracked, 1);
    }

    #[test]
    fn commit_message_hook_can_rewrite_message_and_rejection_preserves_head() {
        if run_in_clean_child(
            "git::gitbackend::commit_tag_characterization::unix::commit_message_hook_can_rewrite_message_and_rejection_preserves_head",
        ) {
            return;
        }
        let (_temp, path, backend) = repository("commit-hooks");
        let first = seed(&path, &backend);
        let log = path.join(".git").join("message-hook.log");
        let _ = hook(
            &path,
            "prepare-commit-msg",
            &format!(
                "printf 'rewritten\\n\\nbody\\n' > \"$1\"\necho ran > {}",
                shell_path(&log)
            ),
        );
        fs::write(path.join("tracked.txt"), "prepared\n").unwrap();
        backend.stage_paths(&path, &["tracked.txt"]).unwrap();
        let rewritten = backend
            .commit(&path, "caller message", false)
            .unwrap()
            .commit;
        assert_eq!(commit_message(&path, &rewritten), "rewritten\n\nbody\n");
        assert_eq!(fs::read_to_string(&log).unwrap().trim(), "ran");

        let _ = hook(&path, "pre-commit", "echo rejected >&2\nexit 1");
        fs::write(path.join("tracked.txt"), "rejected\n").unwrap();
        backend.stage_paths(&path, &["tracked.txt"]).unwrap();
        let rejected = backend.commit(&path, "must fail", false);
        assert!(rejected.is_err());
        assert_eq!(
            backend.head(&path).unwrap().commit.as_deref(),
            Some(rewritten.as_str())
        );
        assert_eq!(tree_file(&path, &first, "tracked.txt").unwrap(), b"base\n");
    }

    #[test]
    fn post_commit_failure_is_reported_by_git_without_rolling_back_commit() {
        if run_in_clean_child(
            "git::gitbackend::commit_tag_characterization::unix::post_commit_failure_is_reported_by_git_without_rolling_back_commit",
        ) {
            return;
        }
        let (_temp, path, backend) = repository("post-commit");
        let _ = seed(&path, &backend);
        let _ = hook(&path, "post-commit", "echo post-hook-ran >&2\nexit 1");
        fs::write(path.join("tracked.txt"), "post\n").unwrap();
        backend.stage_paths(&path, &["tracked.txt"]).unwrap();
        let before = backend.head(&path).unwrap().commit;
        let result = backend.commit(&path, "post hook", false);
        assert!(result.is_ok(), "post-commit is advisory to git commit");
        assert_ne!(backend.head(&path).unwrap().commit, before);
    }

    #[test]
    fn tag_forms_and_reference_transaction_hook_are_observable() {
        if run_in_clean_child(
            "git::gitbackend::commit_tag_characterization::unix::tag_forms_and_reference_transaction_hook_are_observable",
        ) {
            return;
        }
        let (_temp, path, backend) = repository("tag-forms");
        let head = seed(&path, &backend);
        let log = path.join(".git").join("reference-transaction.log");
        let _ = hook(
            &path,
            "reference-transaction",
            &format!("cat >> {}", shell_path(&log)),
        );

        let light = backend.tag_create(&path, "light", None, false).unwrap();
        assert_eq!(light.commit, head);
        let repo = git2::Repository::open(&path).unwrap();
        assert_eq!(
            repo.find_reference("refs/tags/light")
                .unwrap()
                .target()
                .unwrap()
                .to_string(),
            head
        );

        let annotated = backend
            .tag_create(&path, "annotated", Some("release"), false)
            .unwrap();
        assert_eq!(annotated.commit, head);
        let tag_oid = repo
            .find_reference("refs/tags/annotated")
            .unwrap()
            .target()
            .unwrap();
        assert_eq!(
            repo.find_object(tag_oid, None).unwrap().kind(),
            Some(git2::ObjectType::Tag)
        );

        let hook_text = fs::read_to_string(&log).unwrap();
        assert!(hook_text.contains("refs/tags/light"));
        assert!(hook_text.contains("refs/tags/annotated"));
        backend.tag_delete(&path, "annotated").unwrap();
        assert!(
            !backend
                .tag_list(&path)
                .unwrap()
                .iter()
                .any(|name| name == "annotated")
        );
        let after_delete = fs::read_to_string(&log).unwrap();
        assert!(
            after_delete.matches("refs/tags/annotated").count()
                > hook_text.matches("refs/tags/annotated").count()
        );
    }

    #[test]
    fn configured_tag_signing_does_not_bypass_no_message_and_fails_with_mock_openpgp_signer() {
        if run_in_clean_child(
            "git::gitbackend::commit_tag_characterization::unix::configured_tag_signing_does_not_bypass_no_message_and_fails_with_mock_openpgp_signer",
        ) {
            return;
        }
        let (_temp, path, backend) = repository("tag-signing-config");
        seed(&path, &backend);
        let signer_log = path.join(".git").join("signer.log");
        let signer = hook(
            &path,
            "fake-gpg",
            &format!("echo invoked >> {}\nexit 1", shell_path(&signer_log)),
        );
        let mut config = git2::Repository::open(&path).unwrap().config().unwrap();
        config.set_bool("tag.gpgSign", true).unwrap();
        config
            .set_str("gpg.program", signer.to_str().unwrap())
            .unwrap();

        let no_message = backend.tag_create(&path, "config-no-message", None, false);
        assert!(
            no_message.is_err(),
            "tag.gpgSign turns a no-message tag into an annotated-tag request"
        );
        assert!(
            !backend
                .tag_list(&path)
                .unwrap()
                .iter()
                .any(|name| name == "config-no-message")
        );
        assert!(
            !signer_log.exists(),
            "the editor requirement precedes signer invocation"
        );

        let signed = backend.tag_create(&path, "config-annotated", Some("release"), false);
        assert!(
            signed.is_err(),
            "configured signer failure must fail annotated tag creation"
        );
        assert!(
            !backend
                .tag_list(&path)
                .unwrap()
                .iter()
                .any(|name| name == "config-annotated")
        );
        assert_eq!(fs::read_to_string(&signer_log).unwrap().trim(), "invoked");
    }
}
