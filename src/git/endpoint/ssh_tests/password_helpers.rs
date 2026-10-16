//! Actual trusted native password-only setup, through the endpoint attachment.
//!
//! The first six rows use a fake helper. On Unix it is a script that stands in for `git` itself; on Windows a
//! program is an `.exe`, so the helper is a credential helper that Git for Windows runs through its shell, from a
//! configuration file in the test's home (plan step 4.5). The X6 rows at the end use the real Git on both
//! platforms, as 1.0.17 does.
use super::attachment;
use crate::git::endpoint::{
    helper_script, https_auth, shared_reservation::Authority, ssh_channel::GitService, ssh_local,
    ssh_password_fixture::PasswordSshd, ssh_password_helpers::Helpers,
    ssh_worker::EndpointOpenFailure,
};
use gwz_transport::{
    pool::{Config, Key},
    protocol::{AuthMethod, ErrorCode, SetupFailureCause},
};
use std::{
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum Fault {
    None,
    EndpointSlots,
    HostSlots,
    Cancel,
    Refused,
}

/// How long the fake helper waits before it answers.
#[derive(Clone, Copy)]
enum Delay {
    None,
    /// Long enough to outlast the original connect clock, short enough to finish.
    Short,
    /// Never within a test: the helper is ended, or it ends the test.
    Forever,
}

// The clocks of the rows. The Unix fake helper is a script that answers at once; on Windows the helper is the real
// Git, which runs three configuration reads and the fill (each a process start that costs tens of milliseconds
// there, more on a loaded host) before the helper's script starts, so every clock that bounds helper work is wider
// there and the "short" helper outlasts the connect clock by the same margin in proportion.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// The pool's connect clock, which a short helper outlasts.
        const CONNECT_MS: u64 = 250;
        /// The interaction clock of the rows whose helper must finish.
        const INTERACTION_MS: u64 = 1500;
        /// How long a row waits for the fake helper to have started, or for a cleanup to be complete.
        const WAIT: Duration = Duration::from_secs(3);
    } else if #[cfg(windows)] {
        const CONNECT_MS: u64 = 1000;
        const INTERACTION_MS: u64 = 15_000;
        const WAIT: Duration = Duration::from_secs(15);
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// The fake helper: a script standing in for `git`, which records its process id in `marker` before it
        /// waits and answers, and forwards the `config` commands of the view to the real Git.
        fn fake_helper(directory: &Path, marker: &Path, delay: Delay, password: &str) -> https_auth::Config {
            let executable = directory.join("git");
            let delay = match delay {
                Delay::None => "",
                Delay::Short => "/bin/sleep 0.35",
                Delay::Forever => "/bin/sleep 30",
            };
            helper_script::write_git_fixture(
                &executable,
                &format!(
                    "printf '%s' $$ > '{}'\n{delay}\nprintf 'username=git\\npassword={password}\\n'\n",
                    marker.display()
                ),
            );
            https_auth::Config {
                executable,
                environment: Vec::new(),
            }
        }

        /// Whether the process the fake helper recorded has gone.
        fn has_ended(pid: &str) -> bool {
            let pid: i32 = pid.trim().parse().unwrap();
            // SAFETY: positive PID captured from this fixture's owned helper child.
            unsafe { libc::kill(pid, 0) == -1 }
        }
    } else if #[cfg(windows)] {
        /// The fake helper: the real Git, with a credential helper in the test's home that runs under Git's shell.
        /// It records its Windows process id in `marker` before it waits and answers.
        fn fake_helper(directory: &Path, marker: &Path, delay: Delay, password: &str) -> https_auth::Config {
            let delay = match delay {
                Delay::None => "",
                Delay::Short => "sleep 1.2;",
                Delay::Forever => "sleep 30;",
            };
            let home = directory.join("home");
            fs::create_dir_all(&home).unwrap();
            fs::write(
                home.join(".gitconfig"),
                format!(
                    "[credential]\n\thelper = \"!f() {{ cat /proc/$$/winpid > '{}'; {delay} echo username=git; echo password={password}; }}; f\"\n",
                    marker.display().to_string().replace('\\', "/")
                ),
            )
            .unwrap();
            real_git(&home, None)
        }

        /// Whether the process the fake helper recorded has gone.
        fn has_ended(pid: &str) -> bool {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, STILL_ACTIVE},
                System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
            };
            let pid: u32 = pid.trim().parse().unwrap();
            // SAFETY: the handle is checked, queried and closed here; the id is the helper's own.
            unsafe {
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if process.is_null() {
                    return true;
                }
                let mut code = 0;
                let known = GetExitCodeProcess(process, &mut code);
                CloseHandle(process);
                known != 0 && code != STILL_ACTIVE as u32
            }
        }
    }
}

// The spelling of the names a helper's environment is given: Windows programs get them in whichever case their
// parent chose, so the Windows rows use the mixed one, and the helper's lookup must find them all the same.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        const HOME: &str = "HOME";
        const PATH: &str = "PATH";
        const NO_SYSTEM: &str = "GIT_CONFIG_NOSYSTEM";
    } else if #[cfg(windows)] {
        const HOME: &str = "Home";
        const PATH: &str = "Path";
        const NO_SYSTEM: &str = "Git_Config_NoSystem";
    }
}

/// The real Git, found on the `PATH` this process has (with `first` ahead of it), with `home` as the user's home
/// and no system configuration: the captured environment of a helper, as production derives it from a snapshot.
fn real_git(home: &Path, first: Option<&Path>) -> https_auth::Config {
    let inherited = std::env::var_os("PATH").expect("the test process has a PATH");
    let path = match first {
        Some(first) => {
            let mut entries = vec![first.to_path_buf()];
            entries.extend(std::env::split_paths(&inherited));
            std::env::join_paths(entries).unwrap()
        }
        None => inherited,
    };
    let mut environment: Vec<(OsString, OsString)> = vec![
        (HOME.into(), home.as_os_str().into()),
        (PATH.into(), path),
        (NO_SYSTEM.into(), "1".into()),
    ];
    if let Some(root) = std::env::var_os("SystemRoot") {
        environment.push(("SystemRoot".into(), root));
    }
    https_auth::Config {
        executable: PathBuf::from("git"),
        environment,
    }
}

fn run(
    methods: &[&str],
    enabled: bool,
    delay: Delay,
    interaction: u64,
) -> (
    Result<gwz_transport::protocol::Opened, gwz_transport::protocol::Failure>,
    Vec<String>,
    bool,
) {
    run_fault(methods, enabled, delay, interaction, Fault::None)
}

fn run_fault(
    methods: &[&str],
    enabled: bool,
    delay: Delay,
    interaction: u64,
    fault: Fault,
) -> (
    Result<gwz_transport::protocol::Opened, gwz_transport::protocol::Failure>,
    Vec<String>,
    bool,
) {
    let temp = tempfile::tempdir().unwrap();
    let server = PasswordSshd::start(
        &temp.path().join("server"),
        "fixture-password",
        methods,
        &[],
    );
    let known = temp.path().join("known_hosts");
    fs::write(&known, server.known_host("127.0.0.1")).unwrap();
    let repo = temp.path().join("repository with space.git");
    git2::Repository::init_bare(&repo).unwrap();
    let marker = temp.path().join("helper-ran");
    let password = if matches!(fault, Fault::Refused) {
        "wrong-fixture-password"
    } else {
        "fixture-password"
    };
    let config = fake_helper(temp.path(), &marker, delay, password);
    let slots = https_auth::HelperSlots::new();
    let helpers = enabled.then(|| Arc::new(Helpers::new(config, slots.clone())));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let held = match fault {
        Fault::HostSlots => Some(runtime.block_on(slots.hold_all())),
        Fault::EndpointSlots => Some(runtime.block_on(helpers.as_ref().unwrap().hold_endpoint())),
        _ => None,
    };
    let config = Config {
        allocation_timeout_ms: 100,
        connect_timeout_ms: CONNECT_MS,
        interaction_timeout_ms: interaction,
        ..Config::default()
    };
    let endpoint = ssh_local::connect_with_helpers(
        config.clone(),
        known,
        crate::git::endpoint::agent_source::AgentSource::None,
        2000,
        Authority::new(config.total, config.per_host),
        Default::default(),
        helpers.clone(),
    )
    .unwrap();
    let key = Key::ssh("git", "127.0.0.1", server.port);
    let result = if matches!(fault, Fault::Cancel) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let open = endpoint
            .start_endpoint_open(
                key,
                None,
                GitService::UploadPack,
                repo.to_str().unwrap(),
                attachment::context(attachment::deadlines(&config, 2000)),
                cancelled.clone(),
            )
            .unwrap();
        let until = Instant::now() + WAIT;
        // The helper's shell creates the marker before it writes its PID into it, in
        // one write; a cancel that kills it between the two leaves an empty marker.
        while !fs::read_to_string(&marker).is_ok_and(|pid| !pid.trim().is_empty()) {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        cancelled.store(true, Ordering::Release);
        attachment::finish(&open).map(|_| panic!("cancelled setup offered a password"))
    } else {
        attachment::open(
            &endpoint,
            key,
            None,
            GitService::UploadPack,
            repo.to_str().unwrap(),
            attachment::deadlines(&config, 2000),
        )
    };
    let result = match result {
        Ok((mut stream, opened)) => {
            stream.write_all(b"0000").unwrap();
            stream.end_write().unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
            stream.close().unwrap();
            Ok(opened)
        }
        Err(error) => Err(error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<EndpointOpenFailure>())
            .expect("typed failure")
            .failure
            .clone()),
    };
    endpoint.shutdown();
    let until = Instant::now() + WAIT;
    while !endpoint.shutdown_status().cleanup_complete {
        assert!(Instant::now() < until, "setup cleanup remains owned");
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(held);
    assert_eq!(slots.available(), 8);
    if let Some(helpers) = &helpers {
        assert_eq!(helpers.available(), 8);
    }
    // The helper's shell creates the marker before it writes its PID into it: a
    // helper killed between the two (a 75 ms interaction budget under load)
    // leaves an empty marker and no PID to look for. Its reaping is then the
    // cleanup completion asserted above.
    let pid = fs::read_to_string(&marker).unwrap_or_default();
    if !pid.trim().is_empty() {
        assert!(has_ended(&pid), "helper was not reaped");
    }
    (result, server.attempts(), marker.exists())
}

#[test]
fn password_only_helper_outlasts_original_connect_clock() {
    let (result, attempts, ran) = run(&["password"], true, Delay::Short, INTERACTION_MS);
    let opened = result.unwrap();
    assert!(ran);
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt == "password:accepted")
    );
    assert_eq!(opened.facts.method, AuthMethod::Gh);
}

#[test]
fn disabled_publickey_and_combined_offers_never_run_helpers() {
    for (methods, enabled) in [
        (&["password"][..], false),
        (&["publickey"][..], true),
        (&["password", "publickey"][..], true),
    ] {
        let (_, attempts, ran) = run(methods, enabled, Delay::None, INTERACTION_MS);
        assert!(!ran);
        assert!(
            !attempts
                .iter()
                .any(|attempt| attempt.starts_with("password:"))
        );
    }
}

#[test]
fn helper_expiry_has_exact_interaction_and_offers_no_password() {
    let (result, attempts, _ran) = run(&["password"], true, Delay::Forever, 75);
    let failed = result.unwrap_err();
    assert_eq!(failed.code, ErrorCode::Timeout);
    assert_eq!(failed.setup_cause, Some(SetupFailureCause::Interaction));
    assert_eq!(failed.detail.unwrap().helper_budget_ms, Some(75));
    assert!(
        !attempts
            .iter()
            .any(|attempt| attempt.starts_with("password:"))
    );
}

#[test]
fn both_admission_limits_timeout_without_a_helper_or_password() {
    for fault in [Fault::EndpointSlots, Fault::HostSlots] {
        let (result, attempts, ran) =
            run_fault(&["password"], true, Delay::None, INTERACTION_MS, fault);
        let failed = result.unwrap_err();
        assert_eq!(failed.code, ErrorCode::Timeout, "{failed:?}");
        assert_eq!(failed.setup_cause, Some(SetupFailureCause::Allocation));
        let captured = failed.detail.unwrap().helper_budget_ms.unwrap();
        assert!((1..=100).contains(&captured));
        assert!(!ran);
        assert!(
            !attempts
                .iter()
                .any(|attempt| attempt.starts_with("password:"))
        );
    }
}

#[test]
fn cancellation_kills_reaps_and_never_offers_the_helper_answer() {
    let (result, attempts, ran) = run_fault(
        &["password"],
        true,
        Delay::Forever,
        INTERACTION_MS,
        Fault::Cancel,
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    assert!(ran);
    assert!(
        !attempts
            .iter()
            .any(|attempt| attempt.starts_with("password:"))
    );
}

#[test]
fn helper_password_rejection_is_authentication_and_has_no_timeout_detail() {
    let (result, attempts, ran) = run_fault(
        &["password"],
        true,
        Delay::None,
        INTERACTION_MS,
        Fault::Refused,
    );
    let failed = result.unwrap_err();
    assert_eq!(failed.code, ErrorCode::Authentication);
    assert!(failed.detail.is_none());
    assert!(ran);
    assert!(attempts.iter().any(|attempt| attempt == "password:refused"));
}

/// What a password-only server and the real Git make of the helper forms 1.0.17 runs (plan step 2.2, row X6; its
/// evidence is `2026-10-10-tr18-windows-ssh-rows`). The helper is the product's own lookup of `git`, on the
/// `PATH` the snapshot holds, which gains the directory of `named_script` (as `git-credential-tr18`); with
/// `git_on_path` false the snapshot's `PATH` holds that directory alone, and so no Git.
fn x6(
    gitconfig: &str,
    named_script: Option<&str>,
    git_on_path: bool,
) -> (
    Result<gwz_transport::protocol::Opened, gwz_transport::protocol::Failure>,
    Vec<String>,
) {
    let temp = tempfile::tempdir().unwrap();
    let server = PasswordSshd::start(
        &temp.path().join("server"),
        "fixture-password",
        &["password"],
        &[],
    );
    let known = temp.path().join("known_hosts");
    fs::write(&known, server.known_host("127.0.0.1")).unwrap();
    let repo = temp.path().join("repository with space.git");
    git2::Repository::init_bare(&repo).unwrap();
    let home = temp.path().join("home dir");
    let bin = home.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(home.join(".gitconfig"), gitconfig).unwrap();
    if let Some(body) = named_script {
        helper_script::write_helper_script(&bin.join("git-credential-tr18"), body);
    }
    let mut config = real_git(&home, Some(&bin));
    if !git_on_path {
        // A PATH that holds the script directory and nothing else has no Git.
        config
            .environment
            .retain(|(name, _)| !name.to_string_lossy().eq_ignore_ascii_case("PATH"));
        config
            .environment
            .push((PATH.into(), bin.as_os_str().into()));
    }
    let slots = https_auth::HelperSlots::new();
    let helpers = Arc::new(Helpers::new(config, slots.clone()));
    let pool = Config {
        allocation_timeout_ms: 2000,
        connect_timeout_ms: 2000,
        interaction_timeout_ms: 10_000,
        ..Config::default()
    };
    let endpoint = ssh_local::connect_with_helpers(
        pool.clone(),
        known,
        crate::git::endpoint::agent_source::AgentSource::None,
        10_000,
        Authority::new(pool.total, pool.per_host),
        Default::default(),
        Some(helpers),
    )
    .unwrap();
    let result = attachment::open(
        &endpoint,
        Key::ssh("git", "127.0.0.1", server.port),
        None,
        GitService::UploadPack,
        repo.to_str().unwrap(),
        attachment::deadlines(&pool, 10_000),
    );
    let result = match result {
        Ok((mut stream, opened)) => {
            stream.write_all(b"0000").unwrap();
            stream.end_write().unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
            stream.close().unwrap();
            Ok(opened)
        }
        Err(error) => Err(error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<EndpointOpenFailure>())
            .expect("typed failure")
            .failure
            .clone()),
    };
    endpoint.shutdown();
    let until = Instant::now() + Duration::from_secs(30);
    while !endpoint.shutdown_status().cleanup_complete {
        assert!(Instant::now() < until, "setup cleanup remains owned");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(slots.available(), 8);
    (result, server.attempts())
}

/// What a helper does: answers a `get` with the fixture's user and password.
const ANSWER: &str = "test \"$1\" = get && echo username=git && echo password=fixture-password";

/// `ANSWER` as a shell-form helper in a configuration file (quotes escaped as Git reads them).
const SHELL_FORM: &str = "[credential]\n\thelper = \"!f() { test \\\"$1\\\" = get && echo username=git && echo password=fixture-password; }; f\"\n";

/// X6: a shell-form helper (`!f() { ...; }; f`) runs under the real Git and authenticates.
#[test]
fn x6_a_shell_form_helper_authenticates_a_password_only_server() {
    let (result, attempts) = x6(SHELL_FORM, None, true);
    assert_eq!(result.unwrap().facts.method, AuthMethod::Gh);
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt == "password:accepted")
    );
}

/// X6: a named helper (`credential.helper = tr18`) is the script `git-credential-tr18` that Git finds on the
/// captured `PATH`, and it authenticates.
#[test]
fn x6_a_named_helper_script_on_path_authenticates_a_password_only_server() {
    let (result, attempts) = x6("[credential]\n\thelper = tr18\n", Some(ANSWER), true);
    assert_eq!(result.unwrap().facts.method, AuthMethod::Gh);
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt == "password:accepted")
    );
}

/// X6: with no Git on the captured `PATH` the helper is not run, no password is offered and the attempt ends,
/// reporting that no Git was found.
#[test]
fn x6_a_password_only_server_with_no_git_on_the_path_runs_no_helper_and_is_refused() {
    let (result, attempts) = x6("[credential]\n\thelper = tr18\n", Some(ANSWER), false);
    let failed = result.unwrap_err();
    assert_eq!(failed.code, ErrorCode::Unavailable);
    assert_eq!(failed.setup_cause, Some(SetupFailureCause::NotFound));
    assert!(
        !attempts
            .iter()
            .any(|attempt| attempt.starts_with("password:"))
    );
}
