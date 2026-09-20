//! The same shared-object fixture characterizes stock C and qualifies the fix.
use git2::{AutotagOption, ErrorCode, FetchOptions, ObjectType, Repository, Signature, Time};

#[test]
fn receiver_noncommit_hints_do_not_block_a_new_commit() {
    let fixed = match std::env::var("GWZ_NATIVE_FIX").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => panic!("run through prove.py to select the qualified native source"),
    };
    let version = git2::Version::get();
    assert_eq!(version.libgit2_version(), (1, 9, 7));
    assert!(version.vendored());
    for (kind, annotated) in [
        (ObjectType::Tree, false),
        (ObjectType::Blob, false),
        (ObjectType::Tree, true),
        (ObjectType::Blob, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = Repository::init_bare(temp.path().join("source")).unwrap();
        let receiver = Repository::init_bare(temp.path().join("receiver")).unwrap();
        let signature =
            Signature::new("Fixture", "fixture@example.invalid", &Time::new(1000, 0)).unwrap();
        let tree_id = source.treebuilder(None).unwrap().write().unwrap();
        let tree = source.find_tree(tree_id).unwrap();
        let base = source
            .commit(
                Some("refs/heads/main"),
                &signature,
                &signature,
                "base",
                &tree,
                &[],
            )
            .unwrap();
        let url = format!("file://{}", source.path().canonicalize().unwrap().display());
        let mut remote = receiver.remote_anonymous(&url).unwrap();
        let mut options = FetchOptions::new();
        options
            .download_tags(AutotagOption::None)
            .update_fetchhead(false);
        remote
            .fetch(
                &["refs/heads/main:refs/heads/main"],
                Some(&mut options),
                None,
            )
            .unwrap();
        let mut hint = tree_id;
        for repo in [&source, &receiver] {
            let id = if kind == ObjectType::Blob {
                repo.blob(b"shared hint").unwrap()
            } else {
                tree_id
            };
            hint = if annotated {
                repo.tag_annotation_create(
                    "hint",
                    &repo.find_object(id, None).unwrap(),
                    &signature,
                    "hint",
                )
                .unwrap()
            } else {
                id
            };
        }
        receiver
            .reference("refs/checkpoints/hint", hint, false, "hint")
            .unwrap();
        let parent = source.find_commit(base).unwrap();
        let wanted = source
            .commit(
                Some("refs/heads/main"),
                &signature,
                &signature,
                "advance",
                &tree,
                &[&parent],
            )
            .unwrap();
        assert_eq!(
            receiver.find_commit(wanted).err().unwrap().code(),
            ErrorCode::NotFound
        );
        let result = remote.fetch(
            &["refs/heads/main:refs/import/wanted"],
            Some(&mut options),
            None,
        );
        if fixed {
            result.unwrap();
            assert_eq!(
                receiver.refname_to_id("refs/import/wanted").unwrap(),
                wanted
            );
            assert_eq!(receiver.find_commit(wanted).unwrap().tree_id(), tree_id);
        } else {
            let error = result.err().unwrap();
            assert_eq!(
                error.code(),
                if annotated {
                    ErrorCode::Peel
                } else {
                    ErrorCode::InvalidSpec
                }
            );
            assert_eq!(
                receiver
                    .find_reference("refs/import/wanted")
                    .err()
                    .unwrap()
                    .code(),
                ErrorCode::NotFound
            );
        }
        assert_eq!(
            receiver.refname_to_id("refs/checkpoints/hint").unwrap(),
            hint
        );
    }
}

#[test]
fn native_error_classes_are_lossless() {
    let temp = if cfg!(windows) {
        std::fs::create_dir_all("D:/gwz-tests").unwrap();
        tempfile::Builder::new()
            .prefix("native-error-")
            .tempdir_in("D:/gwz-tests")
            .unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let path = temp.path().join("repo");
    let repo = Repository::init_bare(&path).unwrap();
    let message = std::ffi::CString::new("raw class fixture").unwrap();
    for class in [0, 1, 34, 35, 36, 12345] {
        unsafe {
            assert_eq!(libgit2_sys::git_error_set_str(class, message.as_ptr()), 0);
        }
        let error = git2::Error::last_error(-1);
        assert_eq!(error.raw_code(), -1);
        assert_eq!(error.raw_class() as i32, class);
        assert_eq!(error.message(), "raw class fixture");
    }
    let metadata = repo.path().join("info/grafts");
    std::fs::create_dir_all(metadata.parent().unwrap()).unwrap();
    drop(repo);
    std::fs::write(metadata, b"invalid\n").unwrap();
    let error = Repository::open(&path)
        .err()
        .expect("malformed graft must fail");
    // Pinned libgit2 1.9.7 errors.h, independent of Rust's enum/getter.
    assert_eq!(error.raw_class() as i32, 36);
    assert_eq!(error.raw_code(), -1);
}
