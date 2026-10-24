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

fn source_file(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/git/gitbackend")
        .join(relative)
}

#[test]
fn native_commit_and_tag_live_in_gwz_git_and_core_keeps_no_copy() {
    // G1 moved to gwz-git: the module, its two characterization files and the
    // three Git CLI fallbacks are gone from core, not kept beside the new path.
    for gone in [
        "native_commit",
        "native_commit.rs",
        "commit_tag_characterization.rs",
        "commit_tag_identity_characterization.rs",
    ] {
        assert!(!source_file(gone).exists(), "{gone} is still in core");
    }
    for (file, forbidden) in [
        ("repository.rs", "Command::new(\"git\")"),
        ("refs.rs", "Command::new(\"git\")"),
        ("repository.rs", "fn commit_cli"),
        ("refs.rs", "fn tag_create_cli"),
        ("refs.rs", "fn tag_delete_cli"),
        ("refs.rs", "fn verify_tag_created"),
        ("refs.rs", "fn verify_tag_deleted"),
    ] {
        let text = std::fs::read_to_string(source_file(file)).unwrap();
        assert!(!text.contains(forbidden), "{file} still has {forbidden}");
    }
    let backend = std::fs::read_to_string(source_file("commit_tag.rs")).unwrap();
    for needed in [
        "use gwz_git::native_commit::{self, Services}",
        "native_commit::commit(",
        "native_commit::tag_create(",
        "native_commit::tag_delete(",
    ] {
        assert!(
            backend.contains(needed),
            "commit_tag.rs does not contain {needed}"
        );
    }
}
