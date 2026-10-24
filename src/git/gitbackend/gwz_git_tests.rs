//! Step 5.2: what core delegates to gwz-git, and that the old paths are gone.

use crate::git::{git_error, model_error};
use crate::model::ErrorCode;

#[test]
fn a_gwz_git_error_translates_like_the_native_error_it_carries() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing = dir.path().join("missing");
    let native = git2::Repository::open(&missing).err().unwrap();
    let library = gwz_git::Repository::open_exact(&missing).err().unwrap();
    let old = git_error(native);
    let new = model_error(library);
    assert_eq!(new.code, old.code);
    assert_eq!(new.code, ErrorCode::GitCommandFailed);
    assert_eq!(new.message, old.message);
    assert!(!new.message.is_empty());

    let plain = gwz_git::ObjectId::parse_hex(gwz_git::ObjectFormat::Sha1, "zz")
        .err()
        .unwrap();
    let translated = model_error(plain);
    assert_eq!(translated.code, ErrorCode::GitCommandFailed);
    assert!(!translated.message.is_empty());
}

#[test]
fn is_repository_asks_gwz_git_and_core_no_longer_opens_the_repository_for_it() {
    let source = include_str!("repository.rs");
    let body = source
        .split("fn is_repository")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("is_repository is in repository.rs");
    assert!(
        body.contains("gwz_git::Repository::is_repository"),
        "{body}"
    );
    assert!(!body.contains("git2::"), "{body}");
}
