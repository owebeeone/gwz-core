//! Faults specific to the configured Git helper interaction.
use super::*;
use crate::git::endpoint::helper_script::write_helper_script;

fn fake_git(script: &str) -> (tempfile::TempDir, Config) {
    let home = tempfile::tempdir().unwrap();
    let executable = home.path().join("git");
    crate::git::endpoint::helper_script::write_git_fixture(&executable, script);
    (
        home,
        Config {
            executable,
            environment: Vec::new(),
        },
    )
}

#[test]
fn remediation_ssh_username_never_grows_a_populated_allocation() {
    let mut secret = parse_secret(b"username=alice\npassword=token\n").unwrap();
    let before = secret.username.as_ptr();
    let capacity = secret.username.capacity();
    assert!(capacity > secret.username.len(), "terminator space must precede the secret copy");
    assert_eq!(secret.ssh_parts().0.to_bytes(), b"alice");
    assert_eq!(secret.username.as_ptr(), before);
    assert_eq!(secret.username.capacity(), capacity);
    assert_eq!(secret.ssh_parts().0.to_bytes(), b"alice");
    assert_eq!(secret.header(), "Basic YWxpY2U6dG9rZW4=");
}

#[test]
fn configured_output_accepts_empty_fields_and_one_trailing_cr() {
    for output in [
        b"username=\npassword=\n".as_slice(),
        b"username=\r\npassword=\r\n\r\n".as_slice(),
    ] {
        assert_eq!(parse_secret(output).unwrap().header(), "Basic Og==");
    }
    assert_eq!(
        parse_secret(b"username=alice\r\npassword=token\r\n")
            .unwrap()
            .header(),
        "Basic YWxpY2U6dG9rZW4="
    );
}

#[test]
fn unrecognized_lines_do_not_become_credential_fields() {
    let secret =
        parse_secret(b"ignored=\x01\nusername=alice\npassword=token\n\nignored-again\n").unwrap();
    assert_eq!(secret.header(), "Basic YWxpY2U6dG9rZW4=");
    for output in [
        b"username=alice\nusername=other\npassword=token\n".as_slice(),
        b"username=alice\npassword=token\npassword=other\n".as_slice(),
        b"username\npassword=token\n".as_slice(),
    ] {
        assert!(matches!(
            parse_secret(output),
            Err(AuthError::MalformedOutput)
        ));
    }
}

#[tokio::test]
async fn git_receives_only_url_input_and_filtered_snapshot() {
    let (home, mut config) = fake_git("\
if [ \"$*\" != '-c core.askPass= credential fill' ]; then exit 6; fi
if [ \"${GIT_ASKPASS+x}${SSH_ASKPASS+x}${GIT_DIR+x}${GIT_COMMON_DIR+x}${GIT_WORK_TREE+x}${GH_PROMPT_DISABLED+x}\" != '' ]; then exit 7; fi
if [ \"$GIT_TERMINAL_PROMPT\" != 0 ] || [ \"$KEPT\" != snapshot ]; then exit 8; fi
if [ \"$(/bin/pwd)\" != / ]; then exit 9; fi
input=$(/bin/cat)
if [ \"$input\" != 'url=https://example.com/repo%20name' ]; then exit 10; fi
printf 'username=alice\\npassword=token\\n'");
    config.environment = vec![("KEPT".into(), "snapshot".into())];
    for key in [
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
    ] {
        config
            .environment
            .push((key.into(), "must-be-removed".into()));
    }
    let secret = lookup_owned(
        &AuthOwner::new(HelperSlots::new()),
        &config,
        &Destination::parse("https://example.com/repo%20name").unwrap(),
        Instant::now() + Duration::from_secs(3),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(secret.header(), "Basic YWxpY2U6dG9rZW4=");
    drop(home);
}

#[tokio::test]
async fn completed_git_does_not_wait_for_a_background_stderr_writer() {
    let (home, config) = fake_git(
        "\
/bin/sleep 1 >&2 &
printf 'username=alice\\npassword=token\\n'
exit 0",
    );
    let owner = AuthOwner::new(HelperSlots::new());
    let result = tokio::time::timeout(
        Duration::from_millis(400),
        lookup_owned(
            &owner,
            &config,
            &Destination::parse("https://example.com/repo").unwrap(),
            Instant::now() + Duration::from_secs(3),
            &CancellationToken::new(),
        ),
    )
    .await;
    assert!(
        result.is_ok(),
        "git exited and stdout ended; only background stderr remained"
    );
    assert_eq!(result.unwrap().unwrap().header(), "Basic YWxpY2U6dG9rZW4=");
    assert_eq!(owner.pending_cleanup_count(), 0);
    drop(home);
}

#[tokio::test]
async fn occupied_host_slot_waits_for_release_instead_of_failing_capacity() {
    let (_home, config) = fake_git("printf 'username=alice\\npassword=token\\n'");
    let host = HelperSlots::new();
    let held = host
        .0
        .clone()
        .acquire_many_owned(HELPER_SLOTS as u32)
        .await
        .unwrap();
    let owner = AuthOwner::new(host);
    let destination = Destination::parse("https://example.com/repo").unwrap();
    let release = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(held);
    };
    let started = Instant::now();
    let cancelled = CancellationToken::new();
    let (result, ()) = tokio::join!(
        lookup_owned(
            &owner,
            &config,
            &destination,
            started + Duration::from_secs(1),
            &cancelled
        ),
        release,
    );
    assert!(
        result.is_ok(),
        "a slot released within the allocation allowance must admit"
    );
    assert!(started.elapsed() >= Duration::from_millis(30));
}

#[tokio::test]
async fn globally_matching_conditional_include_does_not_replace_unconditional_helper() {
    let home = tempfile::tempdir().unwrap();
    let a = home.path().join("helper-a");
    let b = home.path().join("helper-b");
    write_helper_script(&a, "printf 'username=fixture\\npassword=unconditional\\n'");
    write_helper_script(&b, "printf 'username=fixture\\npassword=conditional\\n'");
    let unconditional = home.path().join("unconditional-config");
    let conditional = home.path().join("conditional-config");
    std::fs::write(
        &unconditional,
        format!("[credential]\n helper = {}\n", a.display()),
    )
    .unwrap();
    std::fs::write(
        &conditional,
        format!("[credential]\n helper =\n helper = {}\n", b.display()),
    )
    .unwrap();
    let global = home.path().join("global-config");
    std::fs::write(&global, format!(
        "[include]\n path = {}\n[remote \"fixture\"]\n url = https://example.com/repo\n[includeIf \"hasconfig:remote.*.url:https://example.com/**\"]\n path = {}\n",
        unconditional.display(), conditional.display(),
    )).unwrap();
    let config = Config {
        executable: PathBuf::from("/usr/bin/git"),
        environment: vec![
            ("HOME".into(), home.path().as_os_str().to_owned()),
            ("XDG_CONFIG_HOME".into(), home.path().as_os_str().to_owned()),
            ("GIT_CONFIG_GLOBAL".into(), global.as_os_str().to_owned()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ],
    };
    let result = lookup_owned(
        &AuthOwner::new(HelperSlots::new()),
        &config,
        &Destination::parse("https://example.com/repo").unwrap(),
        Instant::now() + Duration::from_secs(3),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    // Compare synthetic bytes without formatting or printing the secret on failure.
    assert!(
        result.password == b"unconditional",
        "a conditional include selected the wrong fixture helper"
    );
}

#[tokio::test]
async fn host_wait_charges_allocation_but_started_lookup_keeps_its_interaction() {
    let (_home, config) = fake_git("/bin/sleep 0.3\nprintf 'username=alice\\npassword=token\\n'");
    let host = HelperSlots::new();
    let held = host
        .0
        .clone()
        .acquire_many_owned(HELPER_SLOTS as u32)
        .await
        .unwrap();
    let owner = AuthOwner::new(host);
    let destination = Destination::parse("https://example.com/repo").unwrap();
    let cancelled = CancellationToken::new();
    let mut allocation = Duration::from_millis(700);
    let release = async {
        tokio::time::sleep(Duration::from_millis(400)).await;
        drop(held);
    };
    let (result, ()) = tokio::join!(
        lookup_with_budget(
            &owner,
            &config,
            &destination,
            &mut allocation,
            Duration::from_millis(500),
            &cancelled
        ),
        release,
    );
    assert!(
        result.is_ok(),
        "admission must not consume the interaction allowance"
    );
    assert!(allocation <= Duration::from_millis(350));
    assert!(
        allocation > Duration::from_millis(200),
        "helper execution must not consume allocation"
    );
}

#[tokio::test]
async fn zero_allocation_with_free_slots_starts_no_helper() {
    let (home, mut config) =
        fake_git("printf started > \"$STARTED\"\nprintf 'username=alice\\npassword=token\\n'");
    let started = home.path().join("started");
    config
        .environment
        .push(("STARTED".into(), started.as_os_str().to_owned()));
    let owner = AuthOwner::new(HelperSlots::new());
    for allowance in [Duration::ZERO, Duration::from_nanos(999_999)] {
        let result = lookup_with_budget(
            &owner,
            &config,
            &Destination::parse("https://example.com/repo").unwrap(),
            &mut allowance.clone(),
            Duration::from_secs(1),
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(result, Err(AuthError::AllocationTimeout)));
        assert!(!started.exists());
        assert_eq!(owner.active_count(), 0);
        assert_eq!(owner.pending_cleanup_count(), 0);
    }
}

#[tokio::test]
async fn timeout_kills_descendants_even_after_git_exited_with_stdout_open() {
    let (home, mut config) = fake_git(
        "\
/bin/sh -c 'while :; do printf x >> \"$HEARTBEAT\"; /bin/sleep 0.02; done' &
exit 0",
    );
    let heartbeat = home.path().join("heartbeat");
    config
        .environment
        .push(("HEARTBEAT".into(), heartbeat.as_os_str().to_owned()));
    let owner = AuthOwner::new(HelperSlots::new());
    let result = lookup_owned(
        &owner,
        &config,
        &Destination::parse("https://example.com/repo").unwrap(),
        Instant::now() + Duration::from_millis(150),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(AuthError::Timeout)));
    assert!(
        std::fs::metadata(&heartbeat).unwrap().len() > 0,
        "the descendant must have started"
    );
    // SIGKILL delivery to a descendant is asynchronous even when its direct
    // parent has already exited. Bound that physical retirement by the same
    // cleanup grace before checking for any further externally visible writes.
    tokio::time::sleep(CLEANUP_GRACE).await;
    let length = std::fs::metadata(&heartbeat).unwrap().len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        std::fs::metadata(&heartbeat).unwrap().len(),
        length,
        "the timed-out process group's descendant must stop"
    );
    assert_eq!(
        owner
            .reap_pending(Instant::now() + Duration::from_secs(1))
            .await,
        0
    );
}

#[tokio::test]
async fn stderr_over_limit_is_discarded_without_rejecting_credential() {
    let (_home, config) = fake_git(
        "\
/usr/bin/head -c 40000 /dev/zero >&2
printf 'username=alice\\npassword=token\\n'",
    );
    let result = lookup_owned(
        &AuthOwner::new(HelperSlots::new()),
        &config,
        &Destination::parse("https://example.com/repo").unwrap(),
        Instant::now() + Duration::from_secs(3),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        result.is_ok(),
        "only stdout has the credential-answer bound"
    );
}

#[test]
fn executable_discovery_never_uses_relative_or_ambient_path() {
    let (_home, mut config) = fake_git("exit 0");
    let absolute = config.executable.clone();
    let directory = absolute.parent().unwrap();
    config.executable = PathBuf::from("git");
    assert!(matches!(
        super::executable::resolve(&config),
        Err(AuthError::MissingExecutable)
    ));
    config
        .environment
        .push(("PATH".into(), ".:relative".into()));
    assert!(matches!(
        super::executable::resolve(&config),
        Err(AuthError::MissingExecutable)
    ));
    config.environment[0].1 =
        std::env::join_paths([std::path::Path::new("relative"), directory]).unwrap();
    assert_eq!(super::executable::resolve(&config).unwrap(), absolute);
}
