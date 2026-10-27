//! The per-user directory (remediation round 1: C-P2-4): the frozen locations of `GwzSplitCommandChannel.md` §1 and
//! §4.2, resolved from values passed in, and the parent walk.
use super::super::local::{
    DirFlavor, DirectoryFault, LocalRefusal, RuntimeDir, RuntimeEnv, resolve_directory,
};
use std::{
    ffi::OsString,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

fn env(xdg: Option<&str>, tmp: Option<&str>) -> RuntimeEnv {
    RuntimeEnv {
        xdg_runtime_dir: xdg.map(OsString::from),
        tmpdir: tmp.map(OsString::from),
    }
}

#[test]
fn macos_uses_the_system_per_user_directory_and_reads_no_environment() {
    let darwin = Path::new("/var/folders/ab/cdef/0");
    // The environment is not read on macOS, whatever it holds.
    for env in [
        env(None, None),
        env(Some("/run/user/501"), Some("/somewhere/else")),
    ] {
        assert_eq!(
            resolve_directory(DirFlavor::Darwin, &env, 501, Some(darwin)).unwrap(),
            PathBuf::from("/var/folders/ab/cdef/0/gwz-501")
        );
    }
    // A failing `confstr` refuses; there is no `/tmp` fallback.
    assert_eq!(
        resolve_directory(DirFlavor::Darwin, &env(None, Some("/tmp")), 501, None),
        Err(LocalRefusal::Directory(DirectoryFault::NoUserDirectory))
    );
}

#[test]
fn linux_keeps_the_frozen_order_of_xdg_runtime_dir_tmpdir_and_tmp() {
    let resolve = |env: &RuntimeEnv| resolve_directory(DirFlavor::Linux, env, 1000, None);
    assert_eq!(
        resolve(&env(Some("/run/user/1000"), Some("/var/tmp"))).unwrap(),
        PathBuf::from("/run/user/1000/gwz")
    );
    assert_eq!(
        resolve(&env(None, Some("/var/tmp"))).unwrap(),
        PathBuf::from("/var/tmp/gwz-1000")
    );
    assert_eq!(
        resolve(&env(None, None)).unwrap(),
        PathBuf::from("/tmp/gwz-1000")
    );
    // An unset or empty variable moves to the next choice; a set value that is not absolute is refused, not skipped.
    assert_eq!(
        resolve(&env(Some(""), Some("/var/tmp"))).unwrap(),
        PathBuf::from("/var/tmp/gwz-1000")
    );
    assert_eq!(
        resolve(&env(Some(""), Some(""))).unwrap(),
        PathBuf::from("/tmp/gwz-1000")
    );
    for bad in [env(Some("relative/dir"), None), env(None, Some("rel"))] {
        assert_eq!(
            resolve(&bad),
            Err(LocalRefusal::Directory(DirectoryFault::NotAbsolute))
        );
    }
}

cfg_if::cfg_if! {
    if #[cfg(target_vendor = "apple")] {
    #[test]
    fn on_macos_the_users_directory_lies_under_confstr_and_ends_in_gwz_uid() {
        // SAFETY: the buffer is writable for its stated length; `confstr` NUL-terminates what it writes.
        let mut buffer = vec![0u8; libc::PATH_MAX as usize];
        let written = unsafe {
            libc::confstr(libc::_CS_DARWIN_USER_DIR, buffer.as_mut_ptr().cast(), buffer.len())
        };
        assert!(written > 0, "the system has a per-user directory");
        let expected = std::ffi::CStr::from_bytes_until_nul(&buffer).unwrap().to_str().unwrap().to_owned();
        // Even with an environment that points elsewhere.
        let runtime = RuntimeDir::for_current_user(&env(Some("/tmp/elsewhere"), Some("/tmp/elsewhere")))
            .expect("the user's directory");
        let uid = unsafe { libc::geteuid() };
        assert_eq!(runtime.path().file_name().unwrap().to_str().unwrap(), format!("gwz-{uid}"));
        assert_eq!(
            std::fs::canonicalize(runtime.path().parent().unwrap()).unwrap(),
            std::fs::canonicalize(expected).unwrap()
        );
        // Not the per-user temporary directory.
        assert!(!runtime.path().starts_with(std::env::temp_dir()));
    }

    }
}

fn private_base(mode: u32) -> (tempfile::TempDir, PathBuf) {
    // Under `/tmp`, whose own chain is the sticky-parent case the walk accepts, not under `$TMPDIR`: a test runner may
    // point that at a volume whose root is group-writable, which the walk rightly refuses.
    let root = tempfile::Builder::new()
        .prefix("gwz-dir")
        .tempdir_in("/tmp")
        .unwrap();
    let base = root.path().join("base");
    std::fs::create_dir(&base).unwrap();
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(mode)).unwrap();
    (root, base)
}

#[test]
fn a_parent_writable_by_group_or_other_is_refused_unless_it_is_sticky() {
    let (_root, base) = private_base(0o775);
    assert_eq!(
        RuntimeDir::in_resolved(DirFlavor::Darwin, &env(None, None), Some(&base)).err(),
        Some(LocalRefusal::Directory(DirectoryFault::ParentWritable))
    );
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o757)).unwrap();
    assert_eq!(
        RuntimeDir::in_resolved(DirFlavor::Darwin, &env(None, None), Some(&base)).err(),
        Some(LocalRefusal::Directory(DirectoryFault::ParentWritable))
    );
    // A sticky world-writable parent (as `/tmp` is) is the frozen fallback's own parent and passes.
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o1777)).unwrap();
    let ok = RuntimeDir::in_resolved(DirFlavor::Darwin, &env(None, None), Some(&base)).unwrap();
    assert_eq!(ok.path().parent().unwrap(), base);
    // And a plain 0755 parent, as the system makes the macOS directory, passes.
    let (_root, base) = private_base(0o755);
    assert!(RuntimeDir::in_resolved(DirFlavor::Darwin, &env(None, None), Some(&base)).is_ok());
}

#[test]
fn the_directory_gwz_makes_is_private_and_a_looser_one_is_refused_on_the_next_use() {
    let (_root, base) = private_base(0o755);
    let first = RuntimeDir::in_resolved(DirFlavor::Darwin, &env(None, None), Some(&base)).unwrap();
    let mode = std::fs::metadata(first.path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
    std::fs::set_permissions(first.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(
        RuntimeDir::in_resolved(DirFlavor::Darwin, &env(None, None), Some(&base)).err(),
        Some(LocalRefusal::Directory(DirectoryFault::GroupOrOtherAccess))
    );
}
