use super::super::lookup::lookup_owned;
use super::*;
use crate::git::endpoint::https_destination::Destination;
use std::fs;

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;

        /// The Git these tests run.
        pub(super) fn git() -> PathBuf {
            PathBuf::from("/usr/bin/git")
        }

        /// What a Git child needs besides what a test adds: nothing on Unix, where the tests name Git by path.
        pub(super) fn platform_environment() -> Vec<(OsString, OsString)> {
            Vec::new()
        }

        /// The `credential.helper` value for the script at `path`, as it is written in a configuration file and
        /// as Git parses it back. A path is a helper on Unix as it stands.
        pub(super) fn helper_value(path: &Path) -> (String, String) {
            (path.display().to_string(), path.display().to_string())
        }

        /// A name Git reads as a file that is not a regular one, and does not wait for.
        pub(super) const DEVICE: &str = "/dev/null";
    } else if #[cfg(windows)] {
        /// The Git these tests run: the first `git.exe` on the test process's `PATH`, as production finds it.
        pub(super) fn git() -> PathBuf {
            let config = Config {
                executable: PathBuf::from("git"),
                environment: platform_environment(),
            };
            super::super::executable::resolve(&config).expect("the Windows tests need Git for Windows on PATH")
        }

        /// What a Git child needs besides what a test adds: the test process's `PATH`, under the spelling Windows
        /// programs usually give it, so that every test also checks that the key is found whatever its case; and
        /// the system root.
        pub(super) fn platform_environment() -> Vec<(OsString, OsString)> {
            vec![
                ("Path".into(), std::env::var_os("PATH").expect("the test process has a PATH")),
                ("SystemRoot".into(), std::env::var_os("SystemRoot").expect("Windows sets SystemRoot")),
            ]
        }

        /// The `credential.helper` value for the script at `path`: Git runs a path through its shell as it stands,
        /// so the value is the shell form that quotes it, with forward slashes. The first string is how it is
        /// written in a configuration file (quotes escaped), the second what Git parses back.
        pub(super) fn helper_value(path: &Path) -> (String, String) {
            let forward = path.display().to_string().replace('\\', "/");
            (format!("!\\\"{forward}\\\""), format!("!\"{forward}\""))
        }

        /// A name Git reads as a file that is not a regular one, and does not wait for.
        pub(super) const DEVICE: &str = "//./NUL";
    }
}

pub(super) fn config(home: &Path, global: &Path) -> Config {
    let mut environment: Vec<(OsString, OsString)> = vec![
        ("HOME".into(), home.as_os_str().into()),
        ("XDG_CONFIG_HOME".into(), home.as_os_str().into()),
        ("GIT_CONFIG_GLOBAL".into(), global.as_os_str().into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    environment.extend(platform_environment());
    Config {
        executable: git(),
        environment,
    }
}
pub(super) async fn lookup(config: &Config) -> Result<Secret, AuthError> {
    lookup_owned(
        &AuthOwner::new(HelperSlots::new()),
        config,
        &Destination::parse("https://example.test/repo").unwrap(),
        Instant::now() + Duration::from_secs(5),
        &CancellationToken::new(),
    )
    .await
}
/// Writes a helper script that records that it started and answers; returns its `credential.helper` value as it is
/// written in a configuration file and as Git parses it back.
fn helper(home: &Path) -> (String, String) {
    let helper = home.join("helper");
    crate::git::endpoint::helper_script::write_helper_script(
        &helper,
        "printf started > \"$HOME/started\"\nprintf 'username=fixture\\npassword=fixture-answer\\n'",
    );
    helper_value(&helper)
}
/// What one test needs to run Git's configuration reads and a file read as the lookup does.
pub(super) struct Parts {
    pub(super) owner: AuthOwner,
    permits: Arc<super::super::owner::AdmissionPermits>,
    cancelled: CancellationToken,
}
impl Parts {
    pub(super) async fn new() -> Self {
        let owner = AuthOwner::new(HelperSlots::new());
        let permits = Arc::new(super::super::owner::AdmissionPermits {
            _helper_slot: owner
                .inner
                .helper_slots
                .0
                .clone()
                .acquire_owned()
                .await
                .unwrap(),
            _endpoint_slot: None,
        });
        Self {
            owner,
            permits,
            cancelled: CancellationToken::new(),
        }
    }
    pub(super) fn runner<'a>(
        &'a self,
        config: &'a Config,
        executable: &'a Path,
        seconds: u64,
    ) -> super::super::runner::Runner<'a> {
        super::super::runner::Runner {
            owner: &self.owner,
            config,
            executable,
            permits: self.permits.clone(),
            cancelled: &self.cancelled,
            deadline: Instant::now() + Duration::from_secs(seconds),
            setup: None,
        }
    }
}

// The binary-valued line of the first test, which only Unix can carry: a Windows environment value is UTF-16,
// so a configuration value that is not UTF-8 is refused there (see `a_view_that_is_not_utf8_is_refused`).
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        const BINARY_LINE: &[u8] = b" binary = \xff\n";
        const BINARY_ENTRIES: usize = 2;
    } else if #[cfg(windows)] {
        const BINARY_LINE: &[u8] = b"";
        const BINARY_ENTRIES: usize = 0;
    }
}

#[tokio::test]
async fn native_view_keeps_null_empty_escapes_bytes_and_repeated_scope_occurrences() {
    let home = tempfile::tempdir().unwrap();
    let global = home.path().join("global");
    let include = home.path().join("include");
    let (helper_text, helper_value) = helper(home.path());
    let mut included =
        b"[spike]\n null\n empty =\n quote = \"a\\nb\\t'apostrophe\\\\tail\"\n".to_vec();
    included.extend_from_slice(BINARY_LINE);
    included.extend_from_slice(b"[SpIkE \"Mixed.Case\"]\n NaMe = Yes\n");
    fs::write(&include, included).unwrap();
    fs::write(
        &global,
        format!(
            "[include]\n path = {}\n[credential]\n helper =\n helper = {helper_text}\n",
            include.display().to_string().replace('\\', "/"),
        ),
    )
    .unwrap();
    let mut config = config(home.path(), &global);
    config
        .environment
        .retain(|(key, _)| key != "GIT_CONFIG_NOSYSTEM");
    config.environment.extend([
        ("GIT_CONFIG_SYSTEM".into(), global.as_os_str().into()),
        (
            "GIT_CONFIG_PARAMETERS".into(),
            "'spike.overlay'='parameter'".into(),
        ),
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        ("GIT_CONFIG_KEY_0".into(), "spike.count".into()),
        ("GIT_CONFIG_VALUE_0".into(), "count".into()),
    ]);
    let parts = Parts::new().await;
    let executable = git();
    let runner = parts.runner(&config, &executable, 5);
    let params = prepare(&runner).await.unwrap();
    let raw = runner
        .run(DISCOVERY, &[], Some(&params.0), PREPARATION_LIMIT, true)
        .await
        .unwrap();
    let entries: Vec<_> = discovery(&raw.0)
        .unwrap()
        .into_iter()
        .map(|root| match root {
            Root::Command(e) => e,
            Root::File(_) => panic!("file origin re-entered view"),
        })
        .collect();
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.name.0 == b"spike.null" && e.value.is_none())
            .count(),
        2
    );
    assert_eq!(
        entries
            .iter()
            .filter(
                |e| e.name.0 == b"spike.empty" && e.value.as_ref().is_some_and(|v| v.0.is_empty())
            )
            .count(),
        2
    );
    assert_eq!(
        entries
            .iter()
            .filter(
                |e| e.name.0 == b"spike.binary" && e.value.as_ref().is_some_and(|v| v.0 == b"\xff")
            )
            .count(),
        BINARY_ENTRIES
    );
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.name.0 == b"credential.helper"
                && e.value
                    .as_ref()
                    .is_some_and(|v| v.0.is_empty() || v.0 == helper_value.as_bytes()))
            .count(),
        4
    );
    assert!(entries.iter().any(|e| e.name.0 == b"spike.Mixed.Case.name"));
    assert!(entries.iter().any(|e| e.name.0 == b"spike.overlay"));
    assert!(entries.iter().any(|e| e.name.0 == b"spike.count"));
    let secret = lookup(&config).await.unwrap();
    assert!(secret.password == b"fixture-answer");
}

// The paths of the include-path assertions: Unix names a directory by its root; Windows by a drive.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        const SOURCE: &[u8] = b"/source";
        const SOURCE_NESTED: &[u8] = b"/source/nested";
    } else if #[cfg(windows)] {
        const SOURCE: &[u8] = b"C:/source";
        const SOURCE_NESTED: &[u8] = b"C:/source/nested";
    }
}

#[tokio::test]
async fn unsupported_home_refuses_only_when_tilde_is_needed_and_can_be_repaired() {
    let home = tempfile::tempdir().unwrap();
    let global = home.path().join("global");
    let include = home.path().join("included");
    fs::write(
        &include,
        format!("[credential]\n helper = {}\n", helper(home.path()).0),
    )
    .unwrap();
    fs::write(&global, "[include]\n path = ~/included\n").unwrap();
    let original = config(home.path(), &global);
    for value in [None, Some(""), Some("relative-home")] {
        let mut config = original.clone();
        config.environment.retain(|(key, _)| key != "HOME");
        if let Some(value) = value {
            config.environment.push(("HOME".into(), value.into()));
        }
        assert_eq!(
            lookup(&config).await.err(),
            Some(AuthError::ConfigurationRefused)
        );
        assert!(!home.path().join("started").exists());
    }
    let secret = lookup(&original).await.unwrap();
    assert!(secret.password == b"fixture-answer");
    assert!(include_path(&original, SOURCE, b"nested").unwrap().0 == SOURCE_NESTED);
}

/// A relative include from the command line is anchored at the helper's working directory, `/`, on Unix. Windows
/// has no such anchor (its neutral directory is the Windows directory, and Git itself refuses a relative include
/// that does not come from a file), so the include is refused.
#[test]
fn a_relative_include_from_the_command_line_is_anchored_at_the_root_on_unix_and_refused_on_windows()
{
    let original = Config {
        executable: git(),
        environment: platform_environment(),
    };
    let anchored = include_path(&original, paths::ROOT, b"command-relative");
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            assert!(anchored.unwrap().0 == b"/command-relative");
        } else if #[cfg(windows)] {
            assert_eq!(anchored.err(), Some(AuthError::ConfigurationRefused));
        }
    }
}

#[tokio::test]
async fn overlarge_environment_is_authentication_refusal_before_fill_and_not_missing_git() {
    let home = tempfile::tempdir().unwrap();
    let global = home.path().join("global");
    let helper = helper(home.path()).0;
    let header = format!("[credential]\n helper = {helper}\n[spike]\n large = ");
    let mut source = header.into_bytes();
    source.resize(source.len() + OVERLARGE_QUOTES, b'\'');
    source.push(b'\n');
    fs::write(&global, &source).unwrap();
    let config = config(home.path(), &global);
    let error = lookup(&config).await.err().unwrap();
    assert_eq!(error, AuthError::ConfigurationRefused);
    assert_eq!(
        error.code(),
        gwz_transport::protocol::ErrorCode::Authentication
    );
    assert!(!home.path().join("started").exists());
    fs::write(&global, format!("[credential]\n helper = {helper}\n")).unwrap();
    let secret = lookup(&config).await.unwrap();
    assert!(secret.password == b"fixture-answer");
}

// How many apostrophes make a configuration value no environment can carry: on Unix the system's argument and
// environment limit refuses a value that quotes to about two megabytes; Windows has no spawn-time bound (WH2
// contract, OQ-G), so there the view's own bound on everything one preparation reads and builds is what refuses,
// and the value must be large enough to cross it (1 MiB of source, quoted fourfold).
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        const OVERLARGE_QUOTES: usize = 525_000;
    } else if #[cfg(windows)] {
        const OVERLARGE_QUOTES: usize = 1_000_000;
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[tokio::test]
        async fn initial_discovery_fifo_is_deadline_bounded_and_releases_both_admissions() {
            let home = tempfile::tempdir().unwrap();
            let global = home.path().join("global");
            let fifo = home.path().join("fifo");
            let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
            // SAFETY: the test owns this absent fixture path and passes a valid C string.
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            fs::write(&global, format!("[include]\n path = {}\n", fifo.display())).unwrap();
            let owner = AuthOwner::new(HelperSlots::new());
            let endpoints = Arc::new(Semaphore::new(1));
            let started = Instant::now();
            let mut allocation = Duration::from_secs(1);
            let until = started + allocation;
            let error = super::super::lookup::lookup_until(
                &owner,
                &config(home.path(), &global),
                &Destination::parse("https://example.test/repo").unwrap(),
                &mut allocation,
                Duration::from_millis(150),
                &CancellationToken::new(),
                super::super::lookup::LookupAdmission {
                    until,
                    endpoint_slot: Some(endpoints.clone().acquire_owned().await.unwrap()),
                },
            )
            .await
            .err();
            assert_eq!(error, Some(AuthError::Timeout));
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(owner.pending_cleanup_count(), 0);
            assert_eq!(owner.inner.active.load(Ordering::Acquire), 0);
            assert_eq!(owner.inner.helper_slots.0.available_permits(), HELPER_SLOTS);
            assert_eq!(endpoints.available_permits(), 1);
            assert_eq!(fs::read_dir(home.path()).unwrap().count(), 2);
        }
    } else if #[cfg(windows)] {
        /// The Windows twin of the FIFO row: an include that names a device is refused where it is read, at once,
        /// not waited for, and the lookup returns both admissions.
        #[tokio::test]
        async fn an_include_that_names_a_device_is_refused_and_releases_both_admissions() {
            let home = tempfile::tempdir().unwrap();
            let global = home.path().join("global");
            fs::write(&global, format!("[include]\n path = {DEVICE}\n")).unwrap();
            let owner = AuthOwner::new(HelperSlots::new());
            let endpoints = Arc::new(Semaphore::new(1));
            let started = Instant::now();
            let mut allocation = Duration::from_secs(5);
            let until = started + allocation;
            let error = super::super::lookup::lookup_until(
                &owner,
                &config(home.path(), &global),
                &Destination::parse("https://example.test/repo").unwrap(),
                &mut allocation,
                Duration::from_secs(5),
                &CancellationToken::new(),
                super::super::lookup::LookupAdmission {
                    until,
                    endpoint_slot: Some(endpoints.clone().acquire_owned().await.unwrap()),
                },
            )
            .await
            .err();
            assert_eq!(error, Some(AuthError::ConfigurationRefused));
            assert_eq!(owner.pending_cleanup_count(), 0);
            assert_eq!(owner.inner.active.load(Ordering::Acquire), 0);
            assert_eq!(owner.inner.helper_slots.0.available_permits(), HELPER_SLOTS);
            assert_eq!(endpoints.available_permits(), 1);
            assert_eq!(fs::read_dir(home.path()).unwrap().count(), 1);
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// A file the view must refuse without waiting for it: a FIFO, which blocks a reader until a writer comes.
        fn special_file(directory: &Path) -> PathBuf {
            let fifo = directory.join("fifo");
            let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
            // SAFETY: the test owns this absent fixture path and passes a valid C string.
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            fifo
        }

        /// What the home holds afterwards: the configuration file and the FIFO.
        const SPECIAL_ENTRIES: usize = 2;
    } else if #[cfg(windows)] {
        /// A file the view must refuse without waiting for it: a device, which a reader can wait on. Windows has
        /// no FIFO to make; the name is the thing (see `regular_file`).
        fn special_file(_: &Path) -> PathBuf {
            PathBuf::from(DEVICE)
        }

        /// What the home holds afterwards: the configuration file.
        const SPECIAL_ENTRIES: usize = 1;
    }
}

/// Git parses a source that the view hands it on its standard input without reading what the source includes,
/// and the view's own reader refuses the special file where it reads it.
#[tokio::test]
async fn controlled_stdin_parse_does_not_preread_a_special_file_and_core_refuses_it() {
    let home = tempfile::tempdir().unwrap();
    let global = home.path().join("global");
    let special = special_file(home.path());
    let source = format!(
        "[include]\n path = {}\n[spike]\n null\n empty =\n",
        special.display()
    );
    fs::write(&global, &source).unwrap();
    let config = config(home.path(), &global);
    let parts = Parts::new().await;
    let executable = git();
    let runner = parts.runner(&config, &executable, 2);
    let raw = runner
        .run(
            &["config", "--no-includes", "--null", "--file", "-", "--list"],
            source.as_bytes(),
            Some(&[]),
            PREPARATION_LIMIT,
            true,
        )
        .await
        .unwrap();
    let entries = entries(&raw.0).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries[1].value.is_none());
    assert!(entries[2].value.as_ref().unwrap().0.is_empty());
    assert_eq!(
        super::super::file_worker::read(
            &runner,
            SecretBuffer(paths::bytes(&special).unwrap().to_vec()),
        )
        .await
        .err(),
        Some(AuthError::ConfigurationRefused)
    );
    drop(runner);
    drop(parts.permits);
    assert_eq!(parts.owner.pending_cleanup_count(), 0);
    assert_eq!(
        parts.owner.inner.helper_slots.0.available_permits(),
        HELPER_SLOTS
    );
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), SPECIAL_ENTRIES);
}

#[test]
fn discovery_framing_and_unrecognized_scope_fail_without_payload_diagnostics() {
    for bytes in [
        b"global\0file:/one\0key=value".as_slice(),
        b"local\0file:/repo\0spike.private\nsentinel\0",
        b"command\0file:/one\0spike.x\ny\0",
    ] {
        assert!(discovery(bytes).is_err());
    }
}
