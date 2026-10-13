//! Shared helpers: hermetic child processes, twin repositories and snapshots.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use crate::git::gitbackend::native_commit::Route;
use crate::git::gitbackend::native_commit::RoutePolicy;
use crate::git::gitbackend::*;
use crate::model::ErrorCode;

pub(super) const AUTHOR_DATE: &str = "2001-02-03T04:05:06 +0530";
pub(super) const COMMITTER_DATE: &str = "2002-03-04T05:06:07 -0700";

/// First line of a test that needs the hermetic Git environment of
/// [`run_child`]: runs the test again in a child and returns from the parent.
macro_rules! hermetic {
    () => {
        if $crate::git::gitbackend::native_commit::tests::support::hermetic_here(
            std::any::type_name_of_val(&|| {}),
        ) {
            return;
        }
    };
}

const CHILD: &str = "GWZ_NATIVE_COMMIT_CHILD";

/// Whether this process is a hermetic child started by [`run_child`].
pub(super) fn is_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

/// Re-run the test `name` (module path below the crate) in a child process
/// whose environment is cleared down to a hermetic Git setup: a fresh HOME
/// and global configuration, no system configuration, fixed author and
/// committer dates, and the given extra variables on top.
pub(super) fn run_child(name: &str, extra: &[(&str, &str)]) {
    let home = TempDir::new().unwrap();
    let global = home.path().join("global.gitconfig");
    fs::write(&global, b"").unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.env_clear();
    for keep in ["PATH", "SystemRoot", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"] {
        if let Some(value) = std::env::var_os(keep) {
            command.env(keep, value);
        }
    }
    cfg_if::cfg_if! {
        if #[cfg(windows)] {
            // libgit2 reads %PROGRAMDATA%/Git/config whatever GIT_CONFIG_NOSYSTEM says, so the child
            // gets an empty machine-wide directory of its own.
            let program_data = TempDir::new().unwrap();
            command.env("PROGRAMDATA", program_data.path());
        }
    }
    command
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GIT_CONFIG_GLOBAL", &global)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_DATE", AUTHOR_DATE)
        .env("GIT_COMMITTER_DATE", COMMITTER_DATE)
        .env(CHILD, "1")
        .args(["--exact", name, "--nocapture", "--test-threads", "1"]);
    for (key, value) in extra {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.matches(&format!("test {name} ... ok")).count() == 1,
        "child {name} with {extra:?} failed:\n{stdout}\n{stderr}"
    );
}

/// For a test whose first line is `hermetic!();`: when this is not yet a
/// hermetic child, run the test in one and tell the caller to return.
pub(super) fn hermetic_here(closure_type: &str) -> bool {
    if is_child() {
        return false;
    }
    let name = closure_type
        .strip_suffix("::{{closure}}")
        .and_then(|name| name.split_once("::"))
        .map(|(_, rest)| rest)
        .expect("test name from closure type");
    run_child(name, &[]);
    true
}

/// Run `git -C path args` and return its trimmed standard output.
pub(super) fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

pub(super) fn write(path: &Path, relative: &str, text: &str) {
    let file = path.join(relative);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, text).unwrap();
}

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// The path as Git spells a repository directory when it matches a `gitdir:` pattern: the long name of every
        /// component (a hosted runner's temporary directory is `C:\Users\RUNNER~1\...`, and Git compares the
        /// pattern with the real path), without the `\\?\` prefix a canonical Windows path carries.
        pub(super) fn real(path: &Path) -> PathBuf {
            let canonical = path.canonicalize().unwrap();
            match canonical.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
                Some(plain) if !plain.starts_with("UNC\\") => PathBuf::from(plain),
                _ => canonical,
            }
        }

        #[test]
        fn a_path_given_by_its_short_names_is_spelled_by_its_long_ones() {
            use std::os::windows::ffi::{OsStrExt, OsStringExt};
            use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
            let dir = TempDir::new().unwrap();
            let long = dir.path().join("a-directory-with-a-long-name");
            fs::create_dir(&long).unwrap();
            let wide: Vec<u16> = long.as_os_str().encode_wide().chain([0]).collect();
            let mut buffer = vec![0u16; 1024];
            // SAFETY: `wide` is NUL-terminated and `buffer` is owned, with its length passed.
            let length =
                unsafe { GetShortPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
            assert!(length > 0 && length < buffer.len(), "GetShortPathNameW failed");
            let short = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
            // A volume without short names answers with the long path, and the check below is then the plain one.
            assert_eq!(real(&short), real(&long));
            assert!(real(&short).ends_with("a-directory-with-a-long-name"), "{short:?}");
            assert!(!real(&short).to_string_lossy().starts_with(r"\\?\"));
        }
    } else {
        /// The path with symlinks resolved, as Git sees the repository
        /// directory when it matches a `gitdir:` pattern (macOS `/var` is a link).
        pub(super) fn real(path: &Path) -> PathBuf {
            path.canonicalize().unwrap()
        }
    }
}

/// A twin pair of repositories built the same way: `native` is committed to
/// through the route rule, `cli` through the Git CLI only.
pub(super) struct Twin {
    _dir: TempDir,
    pub(super) native: PathBuf,
    pub(super) cli: PathBuf,
    pub(super) auto: Git2Backend,
    pub(super) cli_only: Git2Backend,
    /// The kept native `commit -a`, which the default route never reaches.
    pub(super) native_all: Git2Backend,
}

/// A repository born the way gwz births one, with a local identity and one
/// committed file, whose history is identical in every twin because the
/// child's author and committer dates are fixed.
pub(super) fn seeded(path: &Path) {
    let backend = Git2Backend::new();
    backend.create_repo(path).unwrap();
    git(path, &["config", "user.name", "Test User"]);
    git(path, &["config", "user.email", "test@example.invalid"]);
    write(path, "tracked.txt", "base\n");
    write(path, "other.txt", "other\n");
    git(path, &["add", "tracked.txt", "other.txt"]);
    git(
        path,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit",
            "-q",
            "--no-verify",
            "-m",
            "base",
        ],
    );
}

/// A gwz-born repository with an identity and nothing committed.
pub(super) fn unborn(path: &Path) {
    let backend = Git2Backend::new();
    backend.create_repo(path).unwrap();
    git(path, &["config", "user.name", "Test User"]);
    git(path, &["config", "user.email", "test@example.invalid"]);
}

/// Build two repositories with `setup` and compare them from then on.
pub(super) fn twin(setup: impl Fn(&Path)) -> Twin {
    let dir = TempDir::new().unwrap();
    let native = real(dir.path()).join("native");
    let cli = real(dir.path()).join("cli");
    for path in [&native, &cli] {
        setup(path);
    }
    let auto = Git2Backend::new();
    let cli_only = auto.with_route_policy(RoutePolicy::CLI_ONLY);
    let native_all = auto.with_route_policy(RoutePolicy::NATIVE_ALL);
    let twin = Twin {
        _dir: dir,
        native,
        cli,
        auto,
        cli_only,
        native_all,
    };
    twin.assert_same("after setup");
    twin
}

/// Everything about a repository that a commit or tag can change.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    refs: BTreeMap<String, String>,
    reflogs: BTreeMap<String, String>,
    objects: BTreeSet<String>,
    index: Vec<(String, u32, String)>,
    status: String,
}

fn reflogs(git_dir: &Path, common_dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, prefix: &str, out: &mut BTreeMap<String, String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, prefix, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(
                    format!("{prefix}{relative}"),
                    fs::read_to_string(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    let own = git_dir.join("logs");
    walk(&own, &own, "git-dir:", &mut out);
    if common_dir != git_dir {
        let shared = common_dir.join("logs");
        walk(&shared, &shared, "common:", &mut out);
    }
    out
}

pub(super) fn snapshot(path: &Path) -> Snapshot {
    let repo = git2::Repository::open(path).unwrap();
    let mut refs = BTreeMap::new();
    let head = repo.find_reference("HEAD").unwrap();
    refs.insert(
        "HEAD".to_owned(),
        match head.symbolic_target().unwrap() {
            Some(target) => format!("ref: {target}"),
            None => head.target().unwrap().to_string(),
        },
    );
    for reference in repo.references().unwrap().flatten() {
        let value = reference
            .target()
            .map(|oid| oid.to_string())
            .or_else(|| reference.symbolic_target().unwrap().map(str::to_owned))
            .unwrap();
        refs.insert(reference.name().unwrap().to_owned(), value);
    }
    let mut objects = BTreeSet::new();
    repo.odb()
        .unwrap()
        .foreach(|oid| {
            objects.insert(oid.to_string());
            true
        })
        .unwrap();
    let index = repo
        .index()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                String::from_utf8_lossy(&entry.path).into_owned(),
                entry.mode,
                entry.id.to_string(),
            )
        })
        .collect();
    Snapshot {
        refs,
        reflogs: reflogs(repo.path(), repo.commondir()),
        objects,
        index,
        status: git(path, &["status", "--porcelain=v1", "-uall"]),
    }
}

/// What an operation returned, reduced to what is compared.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Done(String),
    Failed(ErrorCode),
}

impl Twin {
    pub(super) fn assert_same(&self, when: &str) {
        assert_eq!(
            snapshot(&self.native),
            snapshot(&self.cli),
            "native and CLI repositories differ {when}"
        );
    }

    /// Commit on both and require the native one to be on the native route.
    /// A `commit -a` is Git's on the default route; this requires that, and
    /// then runs the kept native `-a` code against Git so it does not rot.
    pub(super) fn commit(&self, message: &str, all: bool) -> Outcome {
        let native_backend = if all {
            assert!(
                matches!(
                    self.auto.commit_route(&self.native, message, all),
                    Route::Cli(
                        crate::git::gitbackend::native_commit::route::CliReason::AllCommit(_)
                    )
                ),
                "commit -a {message:?} should be Git's"
            );
            &self.native_all
        } else {
            &self.auto
        };
        assert_eq!(
            native_backend.commit_route(&self.native, message, all),
            Route::Native,
            "commit {message:?} (all={all}) should be native"
        );
        let native = native_backend.commit(&self.native, message, all);
        let cli = self.cli_only.commit(&self.cli, message, all);
        let outcome = |result: crate::model::ModelResult<GitCommitResult>| match result {
            Ok(done) => Outcome::Done(done.commit),
            Err(error) => Outcome::Failed(error.code),
        };
        let (native, cli) = (outcome(native), outcome(cli));
        assert_eq!(native, cli, "commit {message:?} (all={all}) outcome");
        self.assert_same(&format!("after commit {message:?} (all={all})"));
        native
    }

    pub(super) fn tag_create(&self, name: &str, message: Option<&str>) -> Outcome {
        assert_eq!(
            self.auto
                .tag_create_route(&self.native, name, message, false),
            Route::Native,
            "tag {name} should be native"
        );
        let native = self.auto.tag_create(&self.native, name, message, false);
        let cli = self.cli_only.tag_create(&self.cli, name, message, false);
        let outcome = |result: crate::model::ModelResult<GitTagResult>| match result {
            Ok(done) => Outcome::Done(done.commit),
            Err(error) => Outcome::Failed(error.code),
        };
        let (native, cli) = (outcome(native), outcome(cli));
        assert_eq!(native, cli, "tag {name} {message:?} outcome");
        self.assert_same(&format!("after tag {name} {message:?}"));
        native
    }

    pub(super) fn tag_delete(&self, name: &str) -> Outcome {
        assert_eq!(
            self.auto.tag_delete_route(&self.native, name),
            Route::Native,
            "tag delete {name} should be native"
        );
        let native = self.auto.tag_delete(&self.native, name);
        let cli = self.cli_only.tag_delete(&self.cli, name);
        let outcome = |result: crate::model::ModelResult<()>| match result {
            Ok(()) => Outcome::Done(String::new()),
            Err(error) => Outcome::Failed(error.code),
        };
        let (native, cli) = (outcome(native), outcome(cli));
        assert_eq!(native, cli, "tag delete {name} outcome");
        self.assert_same(&format!("after tag delete {name}"));
        native
    }

    /// Apply the same edit to both worktrees.
    pub(super) fn both(&self, edit: impl Fn(&Path)) {
        edit(&self.native);
        edit(&self.cli);
    }
}

/// One repository for the route tests: seeded, with the Auto backend.
pub(super) struct Lab {
    _dir: TempDir,
    pub(super) path: PathBuf,
    pub(super) backend: Git2Backend,
}

pub(super) fn lab() -> Lab {
    let dir = TempDir::new().unwrap();
    let path = real(dir.path()).join("repo");
    seeded(&path);
    Lab {
        _dir: dir,
        path,
        backend: Git2Backend::new(),
    }
}

impl Lab {
    /// Change a tracked file and stage it, so a commit has something to do.
    pub(super) fn stage_change(&self) {
        write(&self.path, "tracked.txt", "changed\n");
        git(&self.path, &["add", "tracked.txt"]);
    }

    /// Change a tracked file without staging it, for `commit -a`.
    pub(super) fn dirty_change(&self) {
        write(&self.path, "tracked.txt", "dirty\n");
    }

    pub(super) fn config(&self, key: &str, value: &str) {
        git(&self.path, &["config", key, value]);
    }

    pub(super) fn commit_route(&self, all: bool) -> Route {
        self.backend.commit_route(&self.path, "message", all)
    }

    /// The route of `commit -a` under the policy that reaches the kept native code.
    pub(super) fn native_all_route(&self) -> Route {
        self.backend
            .with_route_policy(RoutePolicy::NATIVE_ALL)
            .commit_route(&self.path, "message", true)
    }

    pub(super) fn head(&self) -> String {
        git(&self.path, &["rev-parse", "HEAD"])
    }

    /// Install a hook that appends a line to the returned marker file.
    pub(super) fn hook(&self, name: &str) -> PathBuf {
        install_hook(&self.path.join(".git").join("hooks"), name, &self.path)
    }
}

/// Write `<directory>/<name>` as a shell script that appends a line to a
/// marker file beside `marker_root`'s `.git`, executable where that matters.
pub(super) fn install_hook(directory: &Path, name: &str, marker_root: &Path) -> PathBuf {
    let marker = marker_root.join(format!("{name}.marker"));
    fs::create_dir_all(directory).unwrap();
    let script = format!(
        "#!/bin/sh\ncat >/dev/null 2>&1\necho ran >> '{}'\nexit 0\n",
        marker.to_string_lossy().replace('\\', "/")
    );
    let hook = directory.join(name);
    fs::write(&hook, script).unwrap();
    make_executable(&hook);
    marker
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        pub(super) fn make_executable(path: &Path) {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    } else {
        pub(super) fn make_executable(_path: &Path) {}
    }
}
