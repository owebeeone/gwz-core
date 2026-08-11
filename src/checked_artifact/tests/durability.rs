use super::*;

#[test]
fn existing_replacement_recovers_after_detach_and_obtains_durability() {
    let root = TempRoot::new("replace-after-detach");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterDetach);
    assert!(checked.replace_exact(&expected, b"goal").is_err());
    assert!(!root.0.join("a/value").exists());
    assert_eq!(fs::read_dir(root.0.join("a")).unwrap().count(), 0);

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_replace(&expected, b"goal").unwrap(),
        CheckedArtifactTransition::Recoverable
    );
    resumed.replace_exact(&expected, b"goal").unwrap();
    assert_eq!(
        resumed.classify_replace(&expected, b"goal").unwrap(),
        CheckedArtifactTransition::After
    );
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"goal");
}

#[test]
fn visible_goal_after_barrier_failure_is_rebarriered_before_after() {
    let root = TempRoot::new("replace-before-durability");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    fail_next_checked_artifact_at(CheckedArtifactFault::BeforeDurability);
    assert!(checked.replace_exact(&expected, b"goal").is_err());
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"goal");

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_replace(&expected, b"goal").unwrap(),
        CheckedArtifactTransition::Recoverable
    );
    resumed.replace_exact(&expected, b"goal").unwrap();
    assert_eq!(
        resumed.classify_replace(&expected, b"goal").unwrap(),
        CheckedArtifactTransition::After
    );
}

#[test]
fn removal_recovers_without_a_managed_parent_tombstone() {
    let root = TempRoot::new("remove-after-detach");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterDetach);
    assert!(checked.remove_exact(&expected).is_err());
    assert!(!root.0.join("a/value").exists());
    assert_eq!(fs::read_dir(root.0.join("a")).unwrap().count(), 0);

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_remove(&expected).unwrap(),
        CheckedArtifactTransition::Recoverable
    );
    resumed.remove_exact(&expected).unwrap();
    assert_eq!(
        resumed.classify_remove(&expected).unwrap(),
        CheckedArtifactTransition::After
    );
}
