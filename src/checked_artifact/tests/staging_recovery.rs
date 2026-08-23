use super::*;

fn family_entries(root: &TempRoot, family: &str) -> Vec<PathBuf> {
    let private = root.0.join(".gwz/checked-artifacts");
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

#[test]
fn every_missing_source_fault_restarts_to_one_exact_goal() {
    let boundaries = [
        CheckedArtifactFault::BeforeAuthorityScratchCreate,
        CheckedArtifactFault::AfterAuthorityScratchCreate,
        CheckedArtifactFault::AfterAuthorityScratchWrite,
        CheckedArtifactFault::AfterAuthorityScratchFlush,
        // R2-D Phase 4 Step 4.1, split per edge at Step 4.2 ([P3-4]). A
        // missing-source replacement has no source to detach, so it crosses
        // exactly these three of the four sealed leaf edges.
        CheckedArtifactFault::BeforeAuthorityPublication,
        CheckedArtifactFault::BeforeGoalPublication,
        CheckedArtifactFault::BeforeManagedPublication,
        CheckedArtifactFault::AfterAuthorityPublication,
        CheckedArtifactFault::AfterAuthorityParentBarrier,
        CheckedArtifactFault::BeforeGoalScratchCreate,
        CheckedArtifactFault::AfterGoalScratchCreate,
        CheckedArtifactFault::AfterGoalScratchWrite,
        CheckedArtifactFault::AfterGoalScratchFlush,
        CheckedArtifactFault::AfterGoalPublication,
        CheckedArtifactFault::AfterGoalParentBarrier,
        CheckedArtifactFault::AfterMutation,
        CheckedArtifactFault::BeforeManagedDestinationDurability,
        CheckedArtifactFault::AfterManagedDestinationDurability,
        CheckedArtifactFault::BeforeQuarantineSourceRetirement,
        CheckedArtifactFault::AfterQuarantineSourceRetirement,
        CheckedArtifactFault::BeforeAuthorityCleanup,
        CheckedArtifactFault::AfterAuthorityCleanup,
        CheckedArtifactFault::BeforeFinalCheck,
        CheckedArtifactFault::AfterFinalProof,
    ];
    for boundary in boundaries {
        let root = TempRoot::new(&format!("missing-restart-{boundary:?}"));
        fs::create_dir_all(root.0.join("a")).unwrap();
        let path = root.0.join("a/value");
        let checked = artifact(&root.0, "a/value");
        let family = checked.family_key();

        fail_next_checked_artifact_at(boundary);
        assert!(
            checked
                .replace_exact(&CheckedArtifactFact::Missing, b"goal")
                .is_err(),
            "{boundary:?}"
        );
        artifact(&root.0, "a/value")
            .replace_exact(&CheckedArtifactFact::Missing, b"goal")
            .unwrap_or_else(|error| panic!("{boundary:?}: {error:?}"));

        assert_eq!(fs::read(&path).unwrap(), b"goal", "{boundary:?}");
        assert!(family_entries(&root, &family).is_empty(), "{boundary:?}");
    }
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn staged_missing_source_cannot_rebind_to_a_replacement_parent() {
    let root = TempRoot::new("missing-staged-parent-replacement");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterGoalParentBarrier);
    assert!(
        checked
            .replace_exact(&CheckedArtifactFact::Missing, b"goal")
            .is_err()
    );
    fs::rename(root.0.join("a"), root.0.join("old-a")).unwrap();
    fs::create_dir(root.0.join("a")).unwrap();

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed
            .classify_replace(&CheckedArtifactFact::Missing, b"goal")
            .unwrap(),
        CheckedArtifactTransition::Ambiguous
    );
    assert!(
        resumed
            .replace_exact(&CheckedArtifactFact::Missing, b"goal")
            .is_err()
    );
    assert!(!root.0.join("a/value").exists());
    assert!(family_entries(&root, &family).len() >= 2);
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn staged_existing_source_cannot_rebind_to_a_same_byte_foreign_parent() {
    let root = TempRoot::new("existing-staged-parent-replacement");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterGoalParentBarrier);
    assert!(checked.replace_exact(&expected, b"goal").is_err());
    fs::rename(root.0.join("a"), root.0.join("old-a")).unwrap();
    fs::create_dir(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_replace(&expected, b"goal").unwrap(),
        CheckedArtifactTransition::Ambiguous
    );
    assert!(resumed.replace_exact(&expected, b"goal").is_err());
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"source");
    assert!(family_entries(&root, &family).len() >= 2);
}

#[cfg(target_os = "linux")]
#[test]
fn replacement_recovers_a_same_identity_goal_alias() {
    let root = TempRoot::new("replacement-goal-alias");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let managed = root.0.join("a/value");
    fs::write(&managed, b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterSourceRetirement);
    assert!(checked.replace_exact(&expected, b"goal").is_err());
    let staged = family_entries(&root, &family)
        .into_iter()
        .find(|path| path.extension().is_some_and(|value| value == "goal"))
        .unwrap();
    fs::hard_link(&staged, &managed).unwrap();

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_replace(&expected, b"goal").unwrap(),
        CheckedArtifactTransition::Recoverable
    );
    resumed.replace_exact(&expected, b"goal").unwrap();
    assert_eq!(fs::read(&managed).unwrap(), b"goal");
    assert!(family_entries(&root, &family).is_empty());
}
