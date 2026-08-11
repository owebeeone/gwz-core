use super::*;

fn private(root: &TempRoot) -> PathBuf {
    root.0.join(".gwz/checked-artifacts")
}

fn family_entries(root: &TempRoot, family: &str) -> Vec<PathBuf> {
    let private = private(root);
    if !private.exists() {
        return Vec::new();
    }
    let prefix = format!("ca1-{family}-");
    let mut entries = fs::read_dir(private)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&prefix)
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

#[cfg(unix)]
fn inode(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;

    fs::metadata(path).unwrap().ino()
}

#[test]
fn source_equals_goal_is_proof_only_and_preserves_identity() {
    let root = TempRoot::new("proof-only");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let path = root.0.join("a/value");
    fs::write(&path, b"same").unwrap();
    let checked = artifact(&root.0, "a/value");
    #[cfg(unix)]
    let before = inode(&path);

    checked
        .replace_exact(&CheckedArtifactFact::Bytes(b"same".to_vec()), b"same")
        .unwrap();

    assert_eq!(fs::read(&path).unwrap(), b"same");
    assert!(!private(&root).exists());
    #[cfg(unix)]
    assert_eq!(inode(&path), before);
}

#[test]
fn source_equals_goal_rejects_and_retains_preexisting_family_state() {
    let root = TempRoot::new("proof-only-residue");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let path = root.0.join("a/value");
    fs::write(&path, b"same").unwrap();
    let checked = artifact(&root.0, "a/value");
    let private = private(&root);
    fs::create_dir_all(&private).unwrap();
    let residue = private.join(format!("ca1-{}-foreign", checked.family_key()));
    fs::write(&residue, b"retained").unwrap();

    assert_eq!(
        checked
            .classify_replace(&CheckedArtifactFact::Bytes(b"same".to_vec()), b"same")
            .unwrap(),
        CheckedArtifactTransition::Ambiguous
    );
    assert!(
        checked
            .replace_exact(&CheckedArtifactFact::Bytes(b"same".to_vec()), b"same")
            .is_err()
    );
    assert_eq!(fs::read(residue).unwrap(), b"retained");
    assert_eq!(fs::read(path).unwrap(), b"same");
}

#[test]
fn workspace_policy_never_redirects_private_state_into_dot_git() {
    let root = TempRoot::new("workspace-policy");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let checked = artifact(&root.0, "a/value");
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterAuthorityPublication);
    assert!(
        checked
            .replace_exact(&CheckedArtifactFact::Bytes(b"source".to_vec()), b"goal")
            .is_err()
    );
    assert!(private(&root).exists());
    assert!(!root.0.join(".git/gwz/checked-artifacts").exists());
}

#[test]
fn workspace_policy_proves_one_opened_atomic_rename_domain() {
    let root = TempRoot::new("workspace-rename-domain");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let checked = artifact(&root.0, "a/value");
    let private = checked.open_private(true).unwrap().unwrap();
    let ParentState::Open { dir: managed, .. } = &checked.parent else {
        panic!("managed parent is open");
    };

    assert_eq!(
        identity::rename_domain(managed).unwrap(),
        identity::rename_domain(&private).unwrap()
    );
}

#[test]
fn git_directory_policy_keeps_private_state_inside_the_git_directory() {
    let root = TempRoot::new("git-policy");
    let repo = git2::Repository::open(&root.0).unwrap();
    fs::create_dir_all(repo.path().join("checked-test")).unwrap();
    fs::write(repo.path().join("checked-test/value"), b"source").unwrap();
    let checked = CheckedArtifact::acquire(
        CheckedArtifactPolicy::git_directory(repo.path()),
        Path::new("checked-test/value"),
        ErrorCode::MergeRecoveryRequired,
        "git-directory artifact",
    )
    .unwrap();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterAuthorityPublication);
    assert!(
        checked
            .replace_exact(&CheckedArtifactFact::Bytes(b"source".to_vec()), b"goal")
            .is_err()
    );
    assert!(repo.path().join("gwz/checked-artifacts").exists());
    assert!(!private(&root).exists());
}

#[test]
fn separate_git_directory_does_not_redirect_workspace_recovery_state() {
    let root = TempRoot::new("separate-git-directory-policy");
    let worktree = root.0.join("linked-worktree");
    let git_directory = root.0.join("separate.git");
    let status = std::process::Command::new("git")
        .args(["init", "--separate-git-dir"])
        .arg(&git_directory)
        .arg(&worktree)
        .status()
        .unwrap();
    assert!(status.success());
    fs::create_dir_all(worktree.join("a")).unwrap();
    fs::write(worktree.join("a/value"), b"source").unwrap();
    let checked = artifact(&worktree, "a/value");
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterAuthorityPublication);
    assert!(
        checked
            .replace_exact(&CheckedArtifactFact::Bytes(b"source".to_vec()), b"goal")
            .is_err()
    );

    assert!(worktree.join(".gwz/checked-artifacts").exists());
    assert!(!git_directory.join("gwz/checked-artifacts").exists());
}

#[test]
fn every_non_cleanup_fault_restarts_to_one_exact_goal() {
    let boundaries = [
        CheckedArtifactFault::BeforeAuthorityScratchCreate,
        CheckedArtifactFault::AfterAuthorityScratchCreate,
        CheckedArtifactFault::AfterAuthorityScratchWrite,
        CheckedArtifactFault::AfterAuthorityScratchFlush,
        CheckedArtifactFault::AfterAuthorityPublication,
        CheckedArtifactFault::AfterAuthorityParentBarrier,
        CheckedArtifactFault::BeforeGoalScratchCreate,
        CheckedArtifactFault::AfterGoalScratchCreate,
        CheckedArtifactFault::AfterGoalScratchWrite,
        CheckedArtifactFault::AfterGoalScratchFlush,
        CheckedArtifactFault::AfterGoalPublication,
        CheckedArtifactFault::AfterGoalParentBarrier,
        CheckedArtifactFault::AfterDetach,
        CheckedArtifactFault::BeforeDestinationDurability,
        CheckedArtifactFault::AfterDestinationDurability,
        CheckedArtifactFault::BeforeSourceRetirement,
        CheckedArtifactFault::AfterSourceRetirement,
        CheckedArtifactFault::AfterMutation,
        CheckedArtifactFault::BeforeManagedDestinationDurability,
        CheckedArtifactFault::AfterManagedDestinationDurability,
        CheckedArtifactFault::BeforeQuarantineSourceRetirement,
        CheckedArtifactFault::AfterQuarantineSourceRetirement,
    ];
    for boundary in boundaries {
        let root = TempRoot::new(&format!("restart-{boundary:?}"));
        fs::create_dir_all(root.0.join("a")).unwrap();
        let path = root.0.join("a/value");
        fs::write(&path, b"source").unwrap();
        let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
        let checked = artifact(&root.0, "a/value");
        let family = checked.family_key();
        fail_next_checked_artifact_at(boundary);
        assert!(checked.replace_exact(&expected, b"goal").is_err());
        #[cfg(unix)]
        let published_inode =
            (path.exists() && fs::read(&path).unwrap() == b"goal").then(|| inode(&path));

        let resumed = artifact(&root.0, "a/value");
        resumed.replace_exact(&expected, b"goal").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"goal", "{boundary:?}");
        assert!(family_entries(&root, &family).is_empty(), "{boundary:?}");
        #[cfg(unix)]
        if let Some(published_inode) = published_inode {
            assert_eq!(inode(&path), published_inode, "{boundary:?}");
        }
    }
}

#[test]
fn cleanup_faults_restart_without_replacing_the_managed_goal() {
    let boundaries = [
        CheckedArtifactFault::BeforeSourceCleanup,
        CheckedArtifactFault::AfterSourceCleanup,
        CheckedArtifactFault::BeforeAuthorityCleanup,
        CheckedArtifactFault::AfterAuthorityCleanup,
    ];
    for boundary in boundaries {
        let root = TempRoot::new(&format!("cleanup-{boundary:?}"));
        fs::create_dir_all(root.0.join("a")).unwrap();
        let path = root.0.join("a/value");
        fs::write(&path, b"source").unwrap();
        let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
        let checked = artifact(&root.0, "a/value");
        let family = checked.family_key();
        fail_next_checked_artifact_at(boundary);
        assert!(checked.replace_exact(&expected, b"goal").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"goal");
        #[cfg(unix)]
        let goal_inode = inode(&path);

        artifact(&root.0, "a/value")
            .replace_exact(&expected, b"goal")
            .unwrap();

        #[cfg(unix)]
        assert_eq!(inode(&path), goal_inode, "{boundary:?}");
        assert!(family_entries(&root, &family).is_empty(), "{boundary:?}");
    }
}

#[test]
fn parent_replacement_after_detach_is_ambiguous_and_retains_family_state() {
    let root = TempRoot::new("parent-after-detach");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterDetach);
    assert!(checked.replace_exact(&expected, b"goal").is_err());
    fs::rename(root.0.join("a"), root.0.join("old-a")).unwrap();
    fs::create_dir(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"foreign").unwrap();

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_replace(&expected, b"goal").unwrap(),
        CheckedArtifactTransition::Ambiguous
    );
    assert!(family_entries(&root, &family).len() >= 2);
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"foreign");
}

#[test]
fn same_byte_family_substitution_is_ambiguous_and_never_cleaned() {
    let root = TempRoot::new("family-substitution");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterDetach);
    assert!(checked.replace_exact(&expected, b"goal").is_err());
    let source = family_entries(&root, &family)
        .into_iter()
        .find(|path| path.extension().is_some_and(|value| value == "source"))
        .unwrap();
    fs::remove_file(&source).unwrap();
    fs::write(&source, b"source").unwrap();

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_replace(&expected, b"goal").unwrap(),
        CheckedArtifactTransition::Ambiguous
    );
    assert!(resumed.replace_exact(&expected, b"goal").is_err());
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert!(!root.0.join("a/value").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_equivalent_path_spellings_reacquire_one_family() {
    let root = TempRoot::new("mac-path-equivalence");
    let composed = "Caf\u{e9}/Value";
    let alias = "cafe\u{301}/value";
    fs::create_dir_all(root.0.join("Caf\u{e9}")).unwrap();
    fs::write(root.0.join(composed), b"source").unwrap();
    if !root.0.join(alias).exists() {
        return;
    }
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let original = artifact(&root.0, composed);
    let family = original.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterDetach);
    assert!(original.replace_exact(&expected, b"goal").is_err());

    let reacquired = artifact(&root.0, alias);
    assert_eq!(reacquired.family_key(), family);
    assert_eq!(
        reacquired.classify_replace(&expected, b"goal").unwrap(),
        CheckedArtifactTransition::Recoverable
    );
    reacquired.replace_exact(&expected, b"goal").unwrap();
    assert_eq!(fs::read(root.0.join(alias)).unwrap(), b"goal");
}
