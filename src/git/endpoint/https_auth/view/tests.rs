use super::super::lookup::lookup_owned;
use super::*;
use std::{fs, os::unix::ffi::OsStrExt};

fn config(home: &Path, global: &Path) -> Config {
    Config {
        executable: "/usr/bin/git".into(),
        environment: vec![
            ("HOME".into(), home.as_os_str().into()),
            ("XDG_CONFIG_HOME".into(), home.as_os_str().into()),
            ("GIT_CONFIG_GLOBAL".into(), global.as_os_str().into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ],
    }
}
async fn lookup(config: &Config) -> Result<Secret, AuthError> {
    lookup_owned(
        &AuthOwner::new(HelperSlots::new()),
        config,
        &Destination::parse("https://example.test/repo").unwrap(),
        Instant::now() + Duration::from_secs(5),
        &CancellationToken::new(),
    )
    .await
}
fn helper(home: &Path) -> PathBuf {
    let helper = home.join("helper");
    crate::git::endpoint::helper_script::write_helper_script(
        &helper,
        "printf started > \"$HOME/started\"\nprintf 'username=fixture\\npassword=fixture-answer\\n'",
    );
    helper
}

#[tokio::test]
async fn native_view_keeps_null_empty_escapes_bytes_and_repeated_scope_occurrences() {
    let home = tempfile::tempdir().unwrap();
    let global = home.path().join("global");
    let include = home.path().join("include");
    let helper_path = helper(home.path());
    fs::write(&include, b"[spike]\n null\n empty =\n quote = \"a\\nb\\t'apostrophe\\\\tail\"\n binary = \xff\n[SpIkE \"Mixed.Case\"]\n NaMe = Yes\n").unwrap();
    fs::write(
        &global,
        format!(
            "[include]\n path = {}\n[credential]\n helper =\n helper = {}\n",
            include.display(),
            helper_path.display()
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
    let owner = AuthOwner::new(HelperSlots::new());
    let executable = Path::new("/usr/bin/git");
    let cancelled = CancellationToken::new();
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
    let runner = super::super::runner::Runner {
        owner: &owner,
        config: &config,
        executable,
        permits,
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(5),
        setup: None,
    };
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
        2
    );
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.name.0 == b"credential.helper" && e.value.as_ref().is_some_and(|v| v.0.is_empty() || v.0 == helper_path.as_os_str().as_bytes()))
            .count(),
        4
    );
    assert!(entries.iter().any(|e| e.name.0 == b"spike.Mixed.Case.name"));
    assert!(entries.iter().any(|e| e.name.0 == b"spike.overlay"));
    assert!(entries.iter().any(|e| e.name.0 == b"spike.count"));
    let secret = lookup(&config).await.unwrap();
    assert!(secret.password == b"fixture-answer");
}

#[tokio::test]
async fn unsupported_home_refuses_only_when_tilde_is_needed_and_can_be_repaired() {
    let home = tempfile::tempdir().unwrap();
    let global = home.path().join("global");
    let include = home.path().join("included");
    fs::write(
        &include,
        format!(
            "[credential]\n helper = {}\n",
            helper(home.path()).display()
        ),
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
    assert!(include_path(&original, b"/source", b"nested").unwrap().0 == b"/source/nested");
    assert!(
        include_path(&original, b"/", b"command-relative")
            .unwrap()
            .0
            == b"/command-relative"
    );
}

#[tokio::test]
async fn overlarge_environment_is_authentication_refusal_before_fill_and_not_missing_git() {
    let home = tempfile::tempdir().unwrap();
    let global = home.path().join("global");
    let helper = helper(home.path());
    let header = format!(
        "[credential]\n helper = {}\n[spike]\n large = ",
        helper.display()
    );
    let mut source = header.into_bytes();
    source.resize(source.len() + 525_000, b'\'');
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
    fs::write(
        &global,
        format!("[credential]\n helper = {}\n", helper.display()),
    )
    .unwrap();
    let secret = lookup(&config).await.unwrap();
    assert!(secret.password == b"fixture-answer");
}

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
        &owner, &config(home.path(), &global),
        &Destination::parse("https://example.test/repo").unwrap(),
        &mut allocation, Duration::from_millis(150), &CancellationToken::new(),
        super::super::lookup::LookupAdmission {
            until,
            endpoint_slot: Some(endpoints.clone().acquire_owned().await.unwrap()),
        },
    ).await.err();
    assert_eq!(error, Some(AuthError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(owner.pending_cleanup_count(), 0);
    assert_eq!(owner.inner.active.load(Ordering::Acquire), 0);
    assert_eq!(owner.inner.helper_slots.0.available_permits(), HELPER_SLOTS);
    assert_eq!(endpoints.available_permits(), 1);
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn controlled_stdin_parse_does_not_preread_fifo_and_core_refuses_it() {
    let home = tempfile::tempdir().unwrap();
    let global = home.path().join("global");
    let fifo = home.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: the test owns this absent fixture path and passes a valid C string.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let source = format!("[include]\n path = {}\n[spike]\n null\n empty =\n", fifo.display());
    fs::write(&global, &source).unwrap();
    let config = config(home.path(), &global);
    let owner = AuthOwner::new(HelperSlots::new());
    let permits = Arc::new(super::super::owner::AdmissionPermits {
        _helper_slot: owner.inner.helper_slots.0.clone().acquire_owned().await.unwrap(),
        _endpoint_slot: None,
    });
    let cancelled = CancellationToken::new();
    let runner = super::super::runner::Runner {
        owner: &owner, config: &config, executable: Path::new("/usr/bin/git"), permits,
        cancelled: &cancelled, deadline: Instant::now() + Duration::from_secs(2),
        setup: None,
    };
    let raw = runner.run(
        &["config", "--no-includes", "--null", "--file", "-", "--list"],
        source.as_bytes(), Some(&[]), PREPARATION_LIMIT, true,
    ).await.unwrap();
    let entries = entries(&raw.0).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries[1].value.is_none());
    assert!(entries[2].value.as_ref().unwrap().0.is_empty());
    assert_eq!(super::super::file_worker::read(
        &runner, SecretBuffer(fifo.as_os_str().as_bytes().to_vec()),
    ).await.err(), Some(AuthError::ConfigurationRefused));
    drop(runner);
    assert_eq!(owner.pending_cleanup_count(), 0);
    assert_eq!(owner.inner.helper_slots.0.available_permits(), HELPER_SLOTS);
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 2);
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
