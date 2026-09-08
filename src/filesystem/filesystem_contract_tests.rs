use super::*;
use std::io::ErrorKind;

#[cfg(windows)]
#[test]
fn publication_moves_the_retained_object_after_its_name_is_replaced() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    let file = fs.create_file_at(&root, "source".as_ref()).unwrap();
    fs.write_all(&file, b"captured").unwrap();
    drop(file);
    let retained = fs
        .open_publication_source(&root, "source".as_ref())
        .unwrap();
    fs.publish_source(
        FsPublicationSource {
            file: &retained,
            parent: &root,
            name: "source".as_ref(),
        },
        &root,
        "published".as_ref(),
        RenameMode::NoReplace,
        &|| {
            fs.rename_at(
                &root,
                "source".as_ref(),
                &root,
                "displaced".as_ref(),
                RenameMode::NoReplace,
            )?;
            let replacement = fs.create_file_at(&root, "source".as_ref())?;
            fs.write_all(&replacement, b"foreign")
        },
    )
    .unwrap();
    assert_eq!(
        fs.read(&workspace.path().join("published")).unwrap(),
        b"captured"
    );
    assert_eq!(
        fs.read(&workspace.path().join("source")).unwrap(),
        b"foreign"
    );
    assert_eq!(
        fs.kind(&workspace.path().join("displaced"))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn persistent_facts_follow_retained_objects_across_name_replacement() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    let original = fs.create_file_at(&root, "original".as_ref()).unwrap();
    let before = fs.persistent_file_identity(&original).unwrap();
    let directory_before = fs.persistent_directory_identity(&root).unwrap();
    let domain = fs.rename_domain(&root).unwrap();
    fs.rename_at(
        &root,
        "original".as_ref(),
        &root,
        "moved".as_ref(),
        RenameMode::NoReplace,
    )
    .unwrap();
    let replacement = fs.create_file_at(&root, "original".as_ref()).unwrap();
    let reopened = fs.open_file_at(&root, "moved".as_ref()).unwrap();
    assert_eq!(before, fs.persistent_file_identity(&original).unwrap());
    assert_eq!(before, fs.persistent_file_identity(&reopened).unwrap());
    assert_ne!(before, fs.persistent_file_identity(&replacement).unwrap());
    assert_eq!(
        directory_before,
        fs.persistent_directory_identity(&fs.clone_directory(&root).unwrap())
            .unwrap()
    );
    assert_eq!(domain, fs.rename_domain(&root).unwrap());
    assert!(matches!(
        fs.lookup_mode(&root).unwrap(),
        FsLookupMode::Sensitive | FsLookupMode::AsciiCaseFold
    ));
    let volume = fs.describe_volume(&root).unwrap();
    assert!(!volume.remote && !volume.volatile);
}

#[test]
fn retained_metadata_observes_kind_without_following_a_symlink() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    let file = fs.create_file_at(&root, "file".as_ref()).unwrap();
    fs.write_all(&file, b"bytes").unwrap();
    fs.create_directory_at(&root, "directory".as_ref()).unwrap();
    fs.test_create_symlink_at(&root, "link".as_ref(), Path::new("file"))
        .unwrap();

    assert_eq!(
        fs.metadata_at(&root, "file".as_ref()).unwrap().kind,
        FsKind::File
    );
    assert_eq!(
        fs.metadata_at(&root, "directory".as_ref()).unwrap().kind,
        FsKind::Directory
    );
    assert_eq!(
        fs.metadata_at(&root, "link".as_ref()).unwrap().kind,
        FsKind::Symlink
    );
}

#[test]
fn cloned_directory_retains_the_same_namespace() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    let retained = fs.clone_directory(&root).unwrap();
    let file = fs.create_file_at(&root, "file".as_ref()).unwrap();
    fs.write_all(&file, b"bytes").unwrap();

    let reopened = fs.open_file_at(&retained, "file".as_ref()).unwrap();
    assert_eq!(fs.read_all(&reopened).unwrap(), b"bytes");
    assert_eq!(
        fs.directory_identity(&retained).unwrap(),
        fs.directory_identity(&root).unwrap()
    );
}

#[test]
fn replacement_preserves_retained_file_and_reopening_observes_new_bytes() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    let old = fs.create_file_at(&root, "record".as_ref()).unwrap();
    fs.write_all(&old, b"old").unwrap();
    fs.sync_file(&old).unwrap();
    let new = fs.create_file_at(&root, "temporary".as_ref()).unwrap();
    fs.write_all(&new, b"new").unwrap();
    fs.sync_file(&new).unwrap();
    fs.rename_at(
        &root,
        "temporary".as_ref(),
        &root,
        "record".as_ref(),
        RenameMode::Replace,
    )
    .unwrap();
    let reopened_context = world.context();
    let reopened = reopened_context
        .filesystem()
        .open_file_at(&root, "record".as_ref())
        .unwrap();
    assert_eq!(fs.read_all(&old).unwrap(), b"old");
    assert_eq!(fs.read_all(&reopened).unwrap(), b"new");
}

#[test]
fn create_new_refuses_existing_name_without_changing_contents() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    let file = fs.create_file_at(&root, "file".as_ref()).unwrap();
    fs.write_all(&file, b"preserve me").unwrap();
    assert_eq!(
        fs.create_file_at(&root, "file".as_ref())
            .err()
            .unwrap()
            .kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(fs.read_all(&file).unwrap(), b"preserve me");
}

#[test]
fn retained_directory_survives_rename_and_components_cannot_escape() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    fs.create_directory_at(&root, "before".as_ref()).unwrap();
    let retained = fs.open_directory_at(&root, "before".as_ref()).unwrap();
    fs.rename_at(
        &root,
        "before".as_ref(),
        &root,
        "after".as_ref(),
        RenameMode::Replace,
    )
    .unwrap();
    let file = fs.create_file_at(&retained, "child".as_ref()).unwrap();
    fs.write_all(&file, b"retained").unwrap();
    let after = fs.open_directory_at(&root, "after".as_ref()).unwrap();
    assert_eq!(
        fs.read_all(&fs.open_file_at(&after, "child".as_ref()).unwrap())
            .unwrap(),
        b"retained"
    );
    assert_eq!(
        fs.create_file_at(&retained, "../escape".as_ref())
            .err()
            .unwrap()
            .kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn path_helpers_share_the_retained_namespace() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let nested = workspace.path().join("one/two");
    fs.create_directories(&nested).unwrap();
    let path = nested.join("record");
    let file = fs.create_file(&path).unwrap();
    fs.write_all(&file, b"journal").unwrap();
    assert_eq!(fs.kind(&path).unwrap(), FsKind::File);
    assert_eq!(fs.read(&path).unwrap(), b"journal");
    assert_eq!(
        fs.canonical_path(&path).unwrap(),
        fs.canonical_path(workspace.path())
            .unwrap()
            .join("one/two/record")
    );
    fs.remove_file(&path).unwrap();
    assert_eq!(fs.kind(&path).unwrap_err().kind(), ErrorKind::NotFound);
}

#[test]
fn no_replace_rename_refuses_an_existing_destination_without_changing_either_file() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let source = workspace.path().join("source");
    let destination = workspace.path().join("destination");
    let source_file = fs.create_file(&source).unwrap();
    fs.write_all(&source_file, b"source").unwrap();
    let destination_file = fs.create_file(&destination).unwrap();
    fs.write_all(&destination_file, b"destination").unwrap();

    assert_eq!(
        fs.rename(&source, &destination, RenameMode::NoReplace)
            .unwrap_err()
            .kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(fs.read(&source).unwrap(), b"source");
    assert_eq!(fs.read(&destination).unwrap(), b"destination");
}

#[test]
fn path_rename_replaces_the_destination_and_directory_sync_accepts_both_parents() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let source_parent = workspace.path().join("source-parent");
    let destination_parent = workspace.path().join("destination-parent");
    fs.create_directories(&source_parent).unwrap();
    fs.create_directories(&destination_parent).unwrap();
    let source = source_parent.join("record");
    let destination = destination_parent.join("record");
    let source_file = fs.create_file(&source).unwrap();
    fs.write_all(&source_file, b"new").unwrap();
    fs.sync_file(&source_file).unwrap();
    let destination_file = fs.create_file(&destination).unwrap();
    fs.write_all(&destination_file, b"old").unwrap();

    fs.rename(&source, &destination, RenameMode::Replace)
        .unwrap();
    fs.sync_directory(&source_parent).unwrap();
    fs.sync_directory(&destination_parent).unwrap();

    assert_eq!(fs.kind(&source).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(fs.read(&destination).unwrap(), b"new");
}

#[test]
fn directory_listing_reports_names_and_kinds_from_the_selected_namespace() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    fs.create_directories(&workspace.path().join("directory"))
        .unwrap();
    fs.create_file(&workspace.path().join("file")).unwrap();

    let entries = fs.read_directory(workspace.path()).unwrap();

    assert_eq!(
        entries,
        vec![
            FsDirectoryEntry {
                name: "directory".into(),
                kind: FsKind::Directory,
            },
            FsDirectoryEntry {
                name: "file".into(),
                kind: FsKind::File,
            },
        ]
    );
}

#[test]
fn empty_directory_can_be_removed_without_affecting_its_parent() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let directory = workspace.path().join("empty");
    fs.create_directories(&directory).unwrap();

    fs.remove_directory(&directory).unwrap();

    assert_eq!(fs.kind(&directory).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(fs.kind(workspace.path()).unwrap(), FsKind::Directory);
}

#[test]
fn retained_directory_listing_and_sync_use_the_opened_namespace() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    fs.create_directory_at(&root, "directory".as_ref()).unwrap();
    fs.create_file_at(&root, "file".as_ref()).unwrap();

    fs.sync_directory_at(&root).unwrap();
    assert_eq!(
        fs.read_directory_at(&root).unwrap(),
        vec![
            FsDirectoryEntry {
                name: "directory".into(),
                kind: FsKind::Directory,
            },
            FsDirectoryEntry {
                name: "file".into(),
                kind: FsKind::File,
            },
        ]
    );
}

#[test]
fn retained_identity_detects_a_same_name_replacement() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    fs.create_directory_at(&root, "current".as_ref()).unwrap();
    let retained = fs.open_directory_at(&root, "current".as_ref()).unwrap();

    assert!(
        fs.directory_entry_matches(&root, "current".as_ref(), &retained)
            .unwrap()
    );
    fs.rename_at(
        &root,
        "current".as_ref(),
        &root,
        "retired".as_ref(),
        RenameMode::NoReplace,
    )
    .unwrap();
    fs.create_directory_at(&root, "current".as_ref()).unwrap();
    let replacement = fs.open_directory_at(&root, "current".as_ref()).unwrap();

    assert!(
        !fs.directory_entry_matches(&root, "current".as_ref(), &retained)
            .unwrap()
    );
    assert!(
        fs.directory_entry_matches(&root, "current".as_ref(), &replacement)
            .unwrap()
    );
}

#[test]
fn retained_file_identity_survives_replacement_of_its_name() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    let retained = fs.create_file_at(&root, "record".as_ref()).unwrap();
    let retained_identity = fs.file_identity(&retained).unwrap();
    assert!(
        fs.file_entry_matches(&root, "record".as_ref(), &retained)
            .unwrap()
    );
    fs.create_file_at(&root, "replacement".as_ref()).unwrap();

    fs.rename_at(
        &root,
        "replacement".as_ref(),
        &root,
        "record".as_ref(),
        RenameMode::Replace,
    )
    .unwrap();
    let reopened = fs.open_file_at(&root, "record".as_ref()).unwrap();

    assert_eq!(fs.file_identity(&retained).unwrap(), retained_identity);
    assert_ne!(fs.file_identity(&reopened).unwrap(), retained_identity);
    assert!(
        !fs.file_entry_matches(&root, "record".as_ref(), &retained)
            .unwrap()
    );
    assert!(
        fs.file_entry_matches(&root, "record".as_ref(), &reopened)
            .unwrap()
    );
}

#[test]
fn retained_no_replace_rename_preserves_both_directories_on_collision() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    fs.create_directory_at(&root, "source".as_ref()).unwrap();
    fs.create_directory_at(&root, "destination".as_ref())
        .unwrap();

    assert_eq!(
        fs.rename_at(
            &root,
            "source".as_ref(),
            &root,
            "destination".as_ref(),
            RenameMode::NoReplace,
        )
        .unwrap_err()
        .kind(),
        ErrorKind::AlreadyExists
    );
    assert!(fs.open_directory_at(&root, "source".as_ref()).is_ok());
    assert!(fs.open_directory_at(&root, "destination".as_ref()).is_ok());
}

#[test]
fn advisory_file_lock_contends_and_drop_releases_it() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    let file = fs.create_file_at(&root, "lease".as_ref()).unwrap();
    let same_file = fs.open_file_at(&root, "lease".as_ref()).unwrap();

    let held = fs
        .try_lock_file(&file)
        .unwrap()
        .expect("first lock acquired");
    assert!(fs.try_lock_file(&same_file).unwrap().is_none());
    drop(held);
    assert!(fs.try_lock_file(&same_file).unwrap().is_some());
}

#[test]
fn symlink_observation_never_opens_the_target_as_a_regular_leaf() {
    let world = crate::operation_context::TestWorld::selected();
    let context = world.context();
    let fs = context.filesystem();
    let workspace = fs.test_workspace().unwrap();
    let root = fs.open_directory(workspace.path()).unwrap();
    fs.test_create_symlink_at(&root, "link".as_ref(), Path::new("target"))
        .unwrap();

    let link = workspace.path().join("link");
    assert_eq!(fs.kind(&link).unwrap(), FsKind::Symlink);
    assert_eq!(fs.link_target(&link).unwrap(), Path::new("target"));
    assert!(fs.open_file_at(&root, "link".as_ref()).is_err());
}
