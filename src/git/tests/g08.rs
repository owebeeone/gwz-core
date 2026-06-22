use super::*;

// The CLI-backed commit primitive (WS6) and SSH fail-fast behavior.

#[test]
fn commit_creates_a_commit_and_self_verifies_head_advanced() {
    let temp = TempDir::new("commit-prim");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    // The CLI commit needs a resolvable identity (no ambient global identity assumed).
    run_git(&repo, &["config", "user.name", "GWZ"]);
    run_git(&repo, &["config", "user.email", "gwz@example.invalid"]);
    std::fs::write(repo.join("f.txt"), "x\n").unwrap();
    backend.stage_paths(&repo, &["f.txt"]).unwrap();

    let before = backend.head(&repo).unwrap().commit;
    let result = backend.commit(&repo, "first", false).unwrap();
    let after = backend.head(&repo).unwrap().commit;
    assert_eq!(after.as_deref(), Some(result.commit.as_str()));
    assert_ne!(before, after);
    assert!(status_porcelain(&repo).trim().is_empty(), "clean tree at the new commit");
}

#[test]
fn commit_all_stages_tracked_modifications_like_git_commit_dash_a() {
    let temp = TempDir::new("commit-all");
    let backend = Git2Backend::new();
    let repo = temp.path().join("repo");
    backend.create_repo(&repo).unwrap();
    run_git(&repo, &["config", "user.name", "GWZ"]);
    run_git(&repo, &["config", "user.email", "gwz@example.invalid"]);
    let base = commit_file(&repo, "f.txt", "x\n", "base", &[]).unwrap();
    // Modify a tracked file without staging; `-a` must pick it up.
    std::fs::write(repo.join("f.txt"), "y\n").unwrap();
    assert!(backend.status(&repo).unwrap().unstaged > 0);

    let result = backend.commit(&repo, "second", true).unwrap();
    assert_ne!(result.commit, base);
    assert!(status_porcelain(&repo).trim().is_empty(), "clean after commit -a");
}

// SSH robustness: fail fast instead of hanging on auth/handshake stalls.

#[test]
fn ssh_credential_callback_gives_up_after_one_agent_attempt() {
    // libgit2 re-invokes the credentials callback after each auth rejection. We must
    // offer the agent once, then return an error — never re-offer it (which would loop).
    let mut attempts = 0u32;
    let first = remote_credential(
        "ssh://git@example.invalid/x.git",
        Some("git"),
        git2::CredentialType::SSH_KEY,
        CredentialHelperPolicy::Disabled,
        &mut attempts,
    );
    assert!(first.is_ok(), "first ssh-key request offers the agent");
    let second = remote_credential(
        "ssh://git@example.invalid/x.git",
        Some("git"),
        git2::CredentialType::SSH_KEY,
        CredentialHelperPolicy::Disabled,
        &mut attempts,
    );
    assert!(second.is_err(), "second ssh-key request must give up, not loop");
    // The username-negotiation phase must not consume an ssh attempt.
    let mut username_attempts = 0u32;
    let _ = remote_credential(
        "ssh://git@example.invalid/x.git",
        None,
        git2::CredentialType::USERNAME,
        CredentialHelperPolicy::Disabled,
        &mut username_attempts,
    );
    assert_eq!(username_attempts, 0);
}

#[test]
fn ssh_clone_times_out_instead_of_hanging() {
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    // A server that accepts the TCP connection but never speaks SSH — libssh2 would block
    // forever reading the banner. With a server timeout set, it must fail fast instead.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 64];
            let _ = stream.read(&mut buf); // read the client banner, never reply
            std::thread::sleep(Duration::from_secs(30));
        }
    });

    set_server_timeout_ms(500);
    let temp = TempDir::new("ssh-timeout");
    let url = format!("ssh://git@127.0.0.1:{port}/x.git");
    let start = Instant::now();
    let result = Git2Backend::new().clone_repo(&url, &temp.path().join("clone"));
    let elapsed = start.elapsed();
    assert!(result.is_err(), "clone of a silent SSH endpoint must fail, not hang");
    assert!(
        elapsed < Duration::from_secs(10),
        "must terminate quickly via the timeout (took {elapsed:?})"
    );
}
