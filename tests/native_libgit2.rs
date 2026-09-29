//! gwz-core links the git2-rs fork's vendored libgit2, never a system one.
//!
//! With the `git fetch` fallback gone, a family import runs on libgit2 alone,
//! so the linked libgit2 must be the fork's vendored tree: it carries the
//! local-fetch correction. Without the `vendored-libgit2` feature the fork's
//! build script links a system `libgit2-experimental` (a SHA-256 build, since
//! `unstable-sha256` is on) in [1.9.7, 1.10.0) whenever pkg-config finds one,
//! and upstream 1.9.7 reports the same version numbers as the fork
//! (dev-docs/GwzCrossLaneCleanup-ReviewSafety.md S-1, in the gwz-dev root).
//! `LIBGIT2_NO_VENDOR=1` stays the fork's explicit opt-out.

#[test]
fn production_links_the_forks_vendored_libgit2() {
    let version = git2::Version::get();
    assert!(
        version.vendored(),
        "gwz-core linked a system libgit2, which lacks the fork's local-fetch correction"
    );
    assert_eq!(version.libgit2_version(), (1, 9, 7));
}

#[test]
fn the_manifest_requests_the_forks_vendored_libgit2_on_every_host() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let manifest = std::fs::read_to_string(path).expect("gwz-core's manifest");
    let git2 = manifest
        .lines()
        .find(|line| line.starts_with("git2 = "))
        .expect("gwz-core's git2 dependency");
    assert!(git2.contains("package = \"gwz-git2\""), "{git2}");
    assert!(git2.contains("\"vendored-libgit2\""), "{git2}");
}
