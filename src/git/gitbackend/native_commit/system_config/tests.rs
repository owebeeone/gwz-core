use super::*;

fn file(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, "[core]\n\thooksPath = /elsewhere\n").unwrap();
    path
}

fn view(reads: &[&PathBuf], may_read: &[&PathBuf], covered: &[&PathBuf]) -> SystemView {
    let own = |files: &[&PathBuf]| files.iter().map(|file| (*file).clone()).collect();
    SystemView {
        git: Some(GitFiles {
            reads: own(reads),
            may_read: own(may_read),
        }),
        git_missing: "",
        covered: own(covered),
        always: Vec::new(),
    }
}

fn is_file_reason(result: &Result<(), CliReason>, why: &str, path: &Path) -> bool {
    matches!(result, Err(CliReason::SystemFile { why: found, path: file }) if *found == why && file == path)
}

#[test]
fn a_file_the_git_may_read_that_native_does_not_picks_the_cli() {
    let dir = tempfile::TempDir::new().unwrap();
    let read = file(&dir, "read");
    let other = file(&dir, "other");
    let env = GitEnvironment::none();
    assert_eq!(check(&env, &view(&[&read], &[&read], &[&read])), Ok(()));
    assert!(is_file_reason(
        &check(&env, &view(&[&read], &[&read, &other], &[&read])),
        GIT_ONLY,
        &other
    ));
    assert!(is_file_reason(
        &check(&env, &view(&[], &[&other], &[])),
        GIT_ONLY,
        &other
    ));
    let missing = dir.path().join("missing");
    assert_eq!(
        check(&env, &view(&[&read], &[&missing, &read], &[&read])),
        Ok(())
    );
}

#[test]
fn a_file_native_reads_that_the_git_does_not_read_picks_the_cli() {
    let dir = tempfile::TempDir::new().unwrap();
    let read = file(&dir, "read");
    let stranger = file(&dir, "stranger");
    let env = GitEnvironment::none();
    // libgit2 reads `/etc/gitconfig` where a Homebrew `git` reads its own prefix's file.
    assert!(is_file_reason(
        &check(&env, &view(&[&read], &[&read], &[&read, &stranger])),
        NATIVE_ONLY,
        &stranger
    ));
    let missing = dir.path().join("missing");
    assert_eq!(
        check(&env, &view(&[&read], &[&read], &[&read, &missing])),
        Ok(())
    );
}

#[test]
fn files_both_sides_read_leave_the_route_native() {
    let dir = tempfile::TempDir::new().unwrap();
    // A Homebrew-like prefix whose file libgit2 is also given.
    let prefix = dir.path().join("brew");
    std::fs::create_dir_all(prefix.join("etc")).unwrap();
    let shared = prefix.join("etc").join("gitconfig");
    std::fs::write(&shared, "").unwrap();
    let git = derive(&prefix.join("bin").join("git"), None, Host::Unix, None).unwrap();
    assert!(git.reads.contains(&shared));
    let env = GitEnvironment::none();
    let both = SystemView {
        git: Some(git),
        git_missing: "",
        covered: vec![shared.clone()],
        always: Vec::new(),
    };
    assert_eq!(check(&env, &both), Ok(()));
}

#[test]
fn no_git_or_no_prefix_picks_the_cli() {
    let env = GitEnvironment::none();
    let none = SystemView {
        git: None,
        git_missing: NO_GIT,
        ..SystemView::default()
    };
    assert_eq!(check(&env, &none), Err(CliReason::SystemConfig(NO_GIT)));
    assert_eq!(derive(Path::new("/git"), None, Host::Unix, None), None);
    assert_eq!(derive(Path::new("git"), None, Host::Windows, None), None);
}

#[test]
fn no_system_leaves_only_the_files_native_always_reads() {
    let dir = tempfile::TempDir::new().unwrap();
    let other = file(&dir, "other");
    let always = file(&dir, "always");
    let missing = dir.path().join("missing");
    for value in ["1", "true", "YES", "on"] {
        let env = GitEnvironment::from_vars([("GIT_CONFIG_NOSYSTEM", value)]);
        let mut subject = view(&[], &[&other], &[&other]);
        assert_eq!(check(&env, &subject), Ok(()), "{value}");
        subject.always = vec![missing.clone()];
        assert_eq!(check(&env, &subject), Ok(()), "{value}: absent");
        subject.always = vec![always.clone()];
        assert!(
            is_file_reason(&check(&env, &subject), ALWAYS, &always),
            "{value}: libgit2 reads it whatever NOSYSTEM says"
        );
    }
    // Without NOSYSTEM that file is just another covered file.
    let env = GitEnvironment::none();
    let mut subject = view(&[&always], &[&always], &[&always]);
    subject.always = vec![always];
    assert_eq!(check(&env, &subject), Ok(()));
}

#[test]
fn the_named_system_file_and_odd_nosystem_values_are_honoured() {
    let dir = tempfile::TempDir::new().unwrap();
    let other = file(&dir, "other");
    let named = GitEnvironment::from_vars([("GIT_CONFIG_SYSTEM", other.to_str().unwrap())]);
    assert_eq!(check(&named, &view(&[], &[&other], &[])), Ok(()));
    let odd = GitEnvironment::from_vars([("GIT_CONFIG_NOSYSTEM", "perhaps")]);
    assert!(matches!(
        check(&odd, &SystemView::default()),
        Err(CliReason::SystemConfig(_))
    ));
    let off = GitEnvironment::from_vars([("GIT_CONFIG_NOSYSTEM", "0")]);
    assert!(is_file_reason(
        &check(&off, &view(&[], &[&other], &[])),
        GIT_ONLY,
        &other
    ));
}

#[test]
fn the_system_file_follows_the_prefix_of_the_git() {
    let etc = |files: &GitFiles| files.reads.clone();
    for (program, expected) in [
        ("/opt/homebrew/bin/git", "/opt/homebrew/etc/gitconfig"),
        ("/usr/local/bin/git", "/usr/local/etc/gitconfig"),
        (
            "/home/linuxbrew/.linuxbrew/bin/git",
            "/home/linuxbrew/.linuxbrew/etc/gitconfig",
        ),
        ("/opt/local/bin/git", "/opt/local/etc/gitconfig"),
        // A build for /usr keeps its file in /etc, as does a /bin/git.
        ("/usr/bin/git", "/etc/gitconfig"),
        ("/bin/git", "/etc/gitconfig"),
    ] {
        let files = derive(Path::new(program), None, Host::Unix, None).unwrap();
        assert_eq!(etc(&files), [PathBuf::from(expected)], "{program}");
        assert!(
            files.may_read.contains(&PathBuf::from(expected)),
            "{program}"
        );
    }
    let files = derive(Path::new("/opt/homebrew/bin/git"), None, Host::Unix, None).unwrap();
    assert!(
        files
            .may_read
            .contains(&PathBuf::from("/opt/homebrew/share/git-core/gitconfig"))
    );
    // A link into a Cellar adds the Cellar's own prefix to what the git may read.
    let linked = derive(
        Path::new("/opt/homebrew/bin/git"),
        Some(Path::new("/opt/homebrew/Cellar/git/2.52.0/bin/git")),
        Host::Unix,
        None,
    )
    .unwrap();
    assert_eq!(linked.reads, [PathBuf::from("/opt/homebrew/etc/gitconfig")]);
    assert!(linked.may_read.contains(&PathBuf::from(
        "/opt/homebrew/Cellar/git/2.52.0/etc/gitconfig"
    )));
}

#[test]
fn apples_git_stub_reads_the_developer_directory_files() {
    let stub = derive(Path::new("/usr/bin/git"), None, Host::MacOs, None).unwrap();
    assert!(
        stub.reads
            .iter()
            .all(|file| file.to_string_lossy().contains("/Developer/"))
    );
    assert!(!stub.reads.contains(&PathBuf::from("/etc/gitconfig")));
    assert!(stub.may_read.contains(&PathBuf::from("/etc/gitconfig")));
    let elsewhere = derive(Path::new("/opt/homebrew/bin/git"), None, Host::MacOs, None).unwrap();
    assert_eq!(
        elsewhere.reads,
        [PathBuf::from("/opt/homebrew/etc/gitconfig")]
    );
}

#[test]
fn git_for_windows_keeps_its_file_at_the_root_of_the_install() {
    let data = Path::new("C:/ProgramData");
    for program in [
        "C:/Program Files/Git/cmd/git.exe",
        "C:/Program Files/Git/mingw64/bin/git.exe",
        "C:/Program Files/Git/bin/git.exe",
    ] {
        let files = derive(Path::new(program), None, Host::Windows, Some(data)).unwrap();
        assert_eq!(
            files.reads,
            [
                PathBuf::from("C:/Program Files/Git/etc/gitconfig"),
                PathBuf::from("C:/ProgramData/Git/config")
            ],
            "{program}"
        );
        assert!(
            files
                .may_read
                .contains(&PathBuf::from("C:/Program Files/Git/mingw64/etc/gitconfig"))
        );
    }
}

#[test]
fn the_first_git_on_path_is_the_one_found() {
    let dir = tempfile::TempDir::new().unwrap();
    let make = |name: &str, executable: bool| {
        let bin = dir.path().join(name).join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let program = bin.join(if cfg!(windows) { "git.exe" } else { "git" });
        std::fs::write(&program, "").unwrap();
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                let mode = if executable { 0o755 } else { 0o644 };
                std::fs::set_permissions(&program, std::fs::Permissions::from_mode(mode)).unwrap();
            } else {
                let _ = executable;
            }
        }
        (bin, program)
    };
    let (empty, _) = {
        let bin = dir.path().join("empty");
        std::fs::create_dir_all(&bin).unwrap();
        (bin, ())
    };
    let (first_bin, first) = make("first", true);
    let (second_bin, second) = make("second", true);
    let join = |entries: &[&Path]| std::env::join_paths(entries).unwrap();
    let host = Host::current();
    assert_eq!(
        find_git(Some(&join(&[&empty, &first_bin, &second_bin])), host),
        Some(first.clone())
    );
    assert_eq!(
        find_git(Some(&join(&[&second_bin, &first_bin])), host),
        Some(second)
    );
    assert_eq!(find_git(Some(&join(&[&empty])), host), None);
    assert_eq!(find_git(None, host), None);
    // A directory that depends on the working directory, before any git, leaves it open;
    // after the git it does not matter.
    assert_eq!(
        find_git(Some(&join(&[Path::new("relative"), &first_bin])), host),
        None
    );
    assert_eq!(
        find_git(Some(&join(&[&first_bin, Path::new("relative")])), host),
        Some(first)
    );
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            let (blocked_bin, _) = make("blocked", false);
            assert_eq!(find_git(Some(&join(&[&blocked_bin])), host), None, "not executable");
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[test]
        fn an_unreadable_file_picks_the_cli() {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::TempDir::new().unwrap();
            let path = file(&dir, "locked");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
            let env = GitEnvironment::none();
            let result = check(&env, &view(&[&path], &[&path], &[&path]));
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            // Running as root can read it anyway; then the file is covered.
            if std::fs::File::open(&path).is_ok() && result.is_ok() {
                return;
            }
            assert!(is_file_reason(&result, UNREADABLE, &path));
        }
    }
}
