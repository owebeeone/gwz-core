//! R2-D Phase 4 Step 4.1 — the four legacy leaf edges publish through the
//! sealed source-associated family (`GwzM5-8R2D-Plan.md` §4 Step 4.1;
//! `GwzM5-8R2DInterfaceFreeze.md` §4.1 row P1 and §4.3 rows E18-E21).
//!
//! Each test drives one converted edge to the front of a real flow and
//! substitutes its source inside the window between the caller's exact proof
//! and the namespace edge. The property under test is the one P1 adds over the
//! raw relative rename these sites used before: the substituted object is
//! refused *before* the edge, so nothing is displaced.

use super::super::identity::ObjectIdentity;
use super::*;

fn private_root(workspace: &Path) -> PathBuf {
    workspace.join(".gwz/checked-artifacts")
}

fn entries_with_prefix(directory: &Path, prefix: &str) -> Vec<PathBuf> {
    if !directory.exists() {
        return Vec::new();
    }
    let mut entries = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(prefix)
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn family_entries(workspace: &Path, family: &str) -> Vec<PathBuf> {
    entries_with_prefix(&private_root(workspace), &format!("ca1-{family}-"))
}

fn family_entry(workspace: &Path, family: &str, extension: &str) -> PathBuf {
    let mut matched = family_entries(workspace, family)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|value| value == extension))
        .collect::<Vec<_>>();
    assert_eq!(matched.len(), 1, "exactly one .{extension} family entry");
    matched.pop().expect("one matching family entry")
}

/// Scratch names are `.ca1-scratch-<kind>-<hex>` (`authority::scratch_name`), so
/// a fault hook can address one without knowing its random half.
fn scratch_entry(workspace: &Path, kind: &str) -> PathBuf {
    let mut matched =
        entries_with_prefix(&private_root(workspace), &format!(".ca1-scratch-{kind}-"));
    assert_eq!(matched.len(), 1, "exactly one live {kind} scratch");
    matched.pop().expect("one matching scratch")
}

/// Same bytes, provably different object. The replacement is staged while the
/// original still exists so an allocator that recycles the freed inode number
/// cannot falsify the new-object precondition — the idiom and its rationale are
/// `exact_source::same_byte_new_inode_after_final_proof_is_not_accepted_as_the_source`'s.
/// The staging name is dotted and outside the `ca1-` family grammar, so it stays
/// invisible to `inspect_family` for the moment it exists.
fn substitute_same_bytes(path: &Path) {
    let bytes = fs::read(path).unwrap();
    let staged = path.with_file_name(".gwz-substitution-staging");
    fs::write(&staged, &bytes).unwrap();
    fs::rename(&staged, path).unwrap();
}

/// Edge E21 (`residue::publish_scratch`). The authority record's publication is
/// the first sealed leaf edge of any drive, so a fresh replacement reaches it
/// directly.
#[test]
fn substituted_authority_scratch_is_refused_before_the_sealed_authority_edge() {
    let root = TempRoot::new("sealed-authority-edge");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();

    let workspace = root.0.clone();
    run_next_checked_artifact_at(
        CheckedArtifactFault::BeforeAuthorityPublication,
        move || substitute_same_bytes(&scratch_entry(&workspace, "authority")),
    );
    let error = checked
        .replace_exact(&CheckedArtifactFact::Missing, b"goal")
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(
        family_entries(&root.0, &family).is_empty(),
        "a substituted authority record must never reach the family grammar"
    );
    assert!(!root.0.join("a/value").exists());
    // The refused scratch is dotted and therefore inert, so the artifact
    // converges on the next drive exactly as any other interrupted staging does.
    artifact(&root.0, "a/value")
        .replace_exact(&CheckedArtifactFact::Missing, b"goal")
        .unwrap();
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"goal");
    assert!(family_entries(&root.0, &family).is_empty());
}

/// Edge E20 (`residue::ensure_goal`). Resuming behind a published authority
/// makes the goal staging the drive's first sealed leaf edge.
#[test]
fn substituted_goal_scratch_is_refused_before_the_sealed_goal_edge() {
    let root = TempRoot::new("sealed-goal-edge");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterAuthorityParentBarrier);
    assert!(
        checked
            .replace_exact(&CheckedArtifactFact::Missing, b"goal")
            .is_err()
    );
    assert_eq!(
        family_entries(&root.0, &family).len(),
        1,
        "the authority record is resident"
    );

    let workspace = root.0.clone();
    run_next_checked_artifact_at(CheckedArtifactFault::BeforeGoalPublication, move || {
        substitute_same_bytes(&scratch_entry(&workspace, "goal"))
    });
    let error = artifact(&root.0, "a/value")
        .replace_exact(&CheckedArtifactFact::Missing, b"goal")
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(
        family_entries(&root.0, &family).len(),
        1,
        "no goal alias may be published from a substituted scratch"
    );
    assert!(!root.0.join("a/value").exists());
    artifact(&root.0, "a/value")
        .replace_exact(&CheckedArtifactFact::Missing, b"goal")
        .unwrap();
    assert_eq!(fs::read(root.0.join("a/value")).unwrap(), b"goal");
}

/// Edge E18 (`transition::detach_existing`). Resuming behind a staged goal makes
/// the managed source's detachment the drive's first sealed leaf edge.
#[test]
fn substituted_managed_source_is_refused_before_the_sealed_detach_edge() {
    let root = TempRoot::new("sealed-detach-edge");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let managed = root.0.join("a/value");
    fs::write(&managed, b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterGoalParentBarrier);
    assert!(checked.replace_exact(&expected, b"goal").is_err());
    assert_eq!(
        family_entries(&root.0, &family).len(),
        2,
        "the authority record and the staged goal are resident"
    );

    let substituted = managed.clone();
    run_next_checked_artifact_at(CheckedArtifactFault::BeforeDetachPublication, move || {
        substitute_same_bytes(&substituted);
    });
    let error = artifact(&root.0, "a/value")
        .replace_exact(&expected, b"goal")
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    // The raw rename this edge used before would have moved the substituted
    // object into the private area and rejected it only afterwards, leaving the
    // managed name empty. Source association refuses ahead of the edge instead.
    assert_eq!(fs::read(&managed).unwrap(), b"source");
    assert_eq!(
        family_entries(&root.0, &family).len(),
        2,
        "no source alias may be quarantined from a substituted managed leaf"
    );
}

/// Edge E19 (`transition::publish_goal`). Resuming behind a detached source
/// makes the managed publication the drive's first sealed leaf edge.
#[test]
fn substituted_staged_goal_is_refused_before_the_sealed_managed_edge() {
    let root = TempRoot::new("sealed-managed-edge");
    fs::create_dir_all(root.0.join("a")).unwrap();
    let managed = root.0.join("a/value");
    fs::write(&managed, b"source").unwrap();
    let expected = CheckedArtifactFact::Bytes(b"source".to_vec());
    let checked = artifact(&root.0, "a/value");
    let family = checked.family_key();
    fail_next_checked_artifact_at(CheckedArtifactFault::AfterDetach);
    assert!(checked.replace_exact(&expected, b"goal").is_err());
    assert!(
        !managed.exists(),
        "the source is detached and the goal is not yet published"
    );

    let staged = family_entry(&root.0, &family, "goal");
    run_next_checked_artifact_at(CheckedArtifactFault::BeforeManagedPublication, move || {
        substitute_same_bytes(&staged);
    });
    let error = artifact(&root.0, "a/value")
        .replace_exact(&expected, b"goal")
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    // The raw rename would have delivered the substituted object onto the
    // managed name and rejected it only on the post-publication reobservation.
    assert!(
        !managed.exists(),
        "a substituted staged goal must never reach the managed parent"
    );
    assert_eq!(family_entries(&root.0, &family).len(), 3);
}

/// The primitive itself: it moves the verified object, consumes the source name,
/// refuses to replace an occupied destination, and refuses both a substituted
/// object and changed bytes before the edge. This is the legacy twin of the
/// freeze's §4.2 spike over `publish_verified_no_replace`.
#[test]
fn the_sealed_leaf_publication_moves_only_the_verified_object() {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{Dir, OpenOptions};
    use std::ffi::OsStr;

    use super::super::platform::{LeafPublicationSourceV1, publish_verified_leaf_no_replace};

    let root = TempRoot::new("sealed-leaf-primitive");
    let directory = Dir::open_ambient_dir(&root.0, cap_std::ambient_authority()).unwrap();
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let identity_of = |name: &str| {
        let file = directory.open_with(OsStr::new(name), &options).unwrap();
        super::super::identity::file_identity(&file).unwrap()
    };
    let publish = |identity: &ObjectIdentity, bytes: &[u8], destination: &str| {
        publish_verified_leaf_no_replace(
            &directory,
            OsStr::new("source"),
            &directory,
            OsStr::new(destination),
            &LeafPublicationSourceV1 { identity, bytes },
            ErrorCode::MergeRecoveryRequired,
            "sealed leaf publication test",
        )
    };

    fs::write(root.0.join("source"), b"payload").unwrap();
    publish(&identity_of("source"), b"payload", "published").unwrap();
    assert!(!root.0.join("source").exists());
    assert_eq!(fs::read(root.0.join("published")).unwrap(), b"payload");

    fs::write(root.0.join("source"), b"payload").unwrap();
    let resident = identity_of("source");
    let error = publish(&resident, b"payload", "published").unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(fs::read(root.0.join("source")).unwrap(), b"payload");
    assert_eq!(fs::read(root.0.join("published")).unwrap(), b"payload");

    substitute_same_bytes(&root.0.join("source"));
    let error = publish(&resident, b"payload", "delivered").unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(!root.0.join("delivered").exists());
    assert_eq!(fs::read(root.0.join("source")).unwrap(), b"payload");

    let error = publish(&identity_of("source"), b"other", "delivered").unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(!root.0.join("delivered").exists());
    assert_eq!(fs::read(root.0.join("source")).unwrap(), b"payload");
}
