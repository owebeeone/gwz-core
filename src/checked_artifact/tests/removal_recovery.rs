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
fn every_removal_fault_restarts_to_exact_absence() {
    let boundaries = [
        CheckedArtifactFault::BeforeAuthorityScratchCreate,
        CheckedArtifactFault::AfterAuthorityScratchCreate,
        CheckedArtifactFault::AfterAuthorityScratchWrite,
        CheckedArtifactFault::AfterAuthorityScratchFlush,
        // R2-D Phase 4 Step 4.1, split per edge at Step 4.2 ([P3-4]). A removal
        // stages no goal and publishes none, so it crosses exactly these two of
        // the four sealed leaf edges.
        CheckedArtifactFault::BeforeAuthorityPublication,
        CheckedArtifactFault::BeforeDetachPublication,
        CheckedArtifactFault::AfterAuthorityPublication,
        CheckedArtifactFault::AfterAuthorityParentBarrier,
        CheckedArtifactFault::BeforeDestinationDurability,
        CheckedArtifactFault::AfterDestinationDurability,
        CheckedArtifactFault::BeforeSourceRetirement,
        CheckedArtifactFault::AfterSourceRetirement,
        CheckedArtifactFault::AfterDetach,
        CheckedArtifactFault::AfterMutation,
        CheckedArtifactFault::BeforeDurability,
        CheckedArtifactFault::AfterDurability,
        CheckedArtifactFault::BeforeSourceCleanup,
        CheckedArtifactFault::AfterSourceCleanup,
        CheckedArtifactFault::BeforeAuthorityCleanup,
        CheckedArtifactFault::AfterAuthorityCleanup,
        CheckedArtifactFault::BeforeFinalCheck,
        CheckedArtifactFault::AfterFinalProof,
    ];
    for boundary in boundaries {
        let root = TempRoot::new(&format!("remove-restart-{boundary:?}"));
        fs::create_dir_all(root.0.join("a")).unwrap();
        let path = root.0.join("a/value");
        fs::write(&path, b"source").unwrap();
        let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
        let checked = artifact(&root.0, "a/value");
        let family = checked.family_key();

        fail_next_checked_artifact_at(boundary);
        assert!(checked.remove_exact(&expected).is_err(), "{boundary:?}");
        artifact(&root.0, "a/value")
            .remove_exact(&expected)
            .unwrap();

        assert!(!path.exists(), "{boundary:?}");
        assert!(family_entries(&root, &family).is_empty(), "{boundary:?}");
    }
}

// Windows denies renaming a directory retained without DELETE sharing; the race is unproducible.
#[cfg(not(windows))]
#[test]
fn removal_parent_replacement_retains_the_quarantined_source() {
    let root = TempRoot::new("remove-parent-replacement");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterDetach);
    assert!(checked.remove_exact(&expected).is_err());
    fs::rename(root.0.join("a"), root.0.join("old-a")).unwrap();
    fs::create_dir(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"foreign").unwrap();

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_remove(&expected).unwrap(),
        CheckedArtifactTransition::Ambiguous
    );
    assert!(resumed.remove_exact(&expected).is_err());
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"foreign");
    let retained = family_entries(&root, &family)
        .into_iter()
        .find(|path| path.extension().is_some_and(|value| value == "source"))
        .unwrap();
    assert_eq!(fs::read(retained).unwrap(), b"source");
}

#[test]
fn removal_same_byte_source_substitution_is_ambiguous_and_retained() {
    let root = TempRoot::new("remove-source-substitution");
    fs::create_dir_all(root.0.join("a")).unwrap();
    fs::write(root.0.join("a/value"), b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterDetach);
    assert!(checked.remove_exact(&expected).is_err());
    let source = family_entries(&root, &family)
        .into_iter()
        .find(|path| path.extension().is_some_and(|value| value == "source"))
        .unwrap();
    fs::remove_file(&source).unwrap();
    fs::write(&source, b"source").unwrap();

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_remove(&expected).unwrap(),
        CheckedArtifactTransition::Ambiguous
    );
    assert!(resumed.remove_exact(&expected).is_err());
    assert_eq!(fs::read(source).unwrap(), b"source");
}

#[cfg(target_os = "linux")]
#[test]
fn removal_recovers_a_same_identity_source_alias() {
    let root = TempRoot::new("remove-source-alias");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let managed = root.0.join("a/value");
    fs::write(&managed, b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterDetach);
    assert!(checked.remove_exact(&expected).is_err());
    let source = family_entries(&root, &family)
        .into_iter()
        .find(|path| path.extension().is_some_and(|value| value == "source"))
        .unwrap();
    fs::hard_link(&source, &managed).unwrap();

    let resumed = artifact(&root.0, "a/value");
    assert_eq!(
        resumed.classify_remove(&expected).unwrap(),
        CheckedArtifactTransition::Recoverable
    );
    resumed.remove_exact(&expected).unwrap();
    assert!(!managed.exists());
    assert!(family_entries(&root, &family).is_empty());
}
