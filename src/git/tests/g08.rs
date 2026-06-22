use crate::model::ErrorCode;

use super::*;

// Contract tests for `sync_gitlinks` (GWZGitlinkPlan §3): reconcile the root index's
// `160000` entries to a desired set, matching porcelain `git update-index --cacheinfo`,
// removing stale gitlinks, idempotently, and never touching real tracked files.

#[test]
fn sync_gitlinks_matches_porcelain_cacheinfo() {
    let temp = TempDir::new("gitlink-parity");
    let backend = Git2Backend::new();
    let prim = temp.path().join("prim");
    let porc = temp.path().join("porc");
    backend.create_repo(&prim).unwrap();
    backend.create_repo(&porc).unwrap();
    // A real member repo nested in prim; its HEAD is the gitlink target.
    backend.create_repo(&prim.join("member")).unwrap();
    let oid = commit_file(&prim.join("member"), "m.txt", "x\n", "member init", &[]).unwrap();

    let result = backend
        .sync_gitlinks(&prim, &[("member", &oid)])
        .expect("sync gitlinks");
    assert_eq!((result.written, result.removed), (1, 0));

    // Porcelain writes the same gitlink directly into porc's index.
    run_git(
        &porc,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{oid},member"),
        ],
    );

    assert_eq!(
        ls_files_stage(&prim),
        ls_files_stage(&porc),
        "primitive gitlink must match `git update-index --cacheinfo` (mode+oid+path)"
    );
    assert!(ls_files_stage(&prim).contains(&format!("160000 {oid} 0\tmember")));
}

#[test]
fn sync_gitlinks_reconciles_stale_and_is_idempotent() {
    let temp = TempDir::new("gitlink-reconcile");
    let backend = Git2Backend::new();
    let root = temp.path().join("root");
    backend.create_repo(&root).unwrap();
    backend.create_repo(&root.join("a")).unwrap();
    backend.create_repo(&root.join("b")).unwrap();
    let a = commit_file(&root.join("a"), "f", "a\n", "a", &[]).unwrap();
    let b = commit_file(&root.join("b"), "f", "b\n", "b", &[]).unwrap();

    let result = backend.sync_gitlinks(&root, &[("a", &a), ("b", &b)]).unwrap();
    assert_eq!((result.written, result.removed), (2, 0));
    assert!(ls_files_stage(&root).contains(&format!("160000 {a} 0\ta")));
    assert!(ls_files_stage(&root).contains(&format!("160000 {b} 0\tb")));

    // Idempotent: the same desired set rewrites in place, removes nothing.
    let result = backend.sync_gitlinks(&root, &[("a", &a), ("b", &b)]).unwrap();
    assert_eq!((result.written, result.removed), (2, 0));

    // Dropping `b` from the desired set removes only its gitlink.
    let result = backend.sync_gitlinks(&root, &[("a", &a)]).unwrap();
    assert_eq!((result.written, result.removed), (1, 1));
    let staged = ls_files_stage(&root);
    assert!(staged.contains(&format!("160000 {a} 0\ta")));
    assert!(!staged.contains("\tb"));
}

#[test]
fn sync_gitlinks_leaves_real_tracked_files_untouched() {
    let temp = TempDir::new("gitlink-guard");
    let backend = Git2Backend::new();
    let root = temp.path().join("root");
    backend.create_repo(&root).unwrap();
    // A real tracked file in the root index — must survive gitlink reconciliation.
    std::fs::write(root.join("keep.txt"), "keep\n").unwrap();
    backend.stage_paths(&root, &["keep.txt"]).unwrap();
    backend.create_repo(&root.join("m")).unwrap();
    let m = commit_file(&root.join("m"), "f", "m\n", "m", &[]).unwrap();

    backend.sync_gitlinks(&root, &[("m", &m)]).unwrap();
    // Reconcile to an empty desired set: the gitlink goes, the real file stays.
    let result = backend.sync_gitlinks(&root, &[]).unwrap();
    assert_eq!((result.written, result.removed), (0, 1));
    let staged = ls_files_stage(&root);
    assert!(staged.contains("\tkeep.txt"), "real tracked file must survive");
    assert!(!staged.contains("\tm"));
}

#[test]
fn sync_gitlinks_errors_on_non_repository() {
    let temp = TempDir::new("gitlink-nonrepo");
    let err = Git2Backend::new()
        .sync_gitlinks(temp.path(), &[])
        .expect_err("syncing gitlinks in a non-repository must fail");
    assert_eq!(err.code, ErrorCode::GitCommandFailed);
}

// §5.1 matrix: a gitlinked member is one opaque unit in root `git status`.

#[test]
fn root_status_reports_advanced_gitlink_member_as_one_modified_unit() {
    let temp = TempDir::new("gitlink-advanced");
    let backend = Git2Backend::new();
    let root = temp.path().join("root");
    backend.create_repo(&root).unwrap();
    backend.create_repo(&root.join("member")).unwrap();
    let x = commit_file(&root.join("member"), "f.txt", "x\n", "X", &[]).unwrap();

    // Project the gitlink and commit the root so HEAD records it — a clean baseline.
    backend.sync_gitlinks(&root, &[("member", &x)]).unwrap();
    run_git(&root, &["commit", "-m", "base"]);
    assert!(
        status_porcelain(&root).trim().is_empty(),
        "root is clean when the member is at the recorded oid"
    );

    // Advance the member past the recorded oid.
    let x_oid = git2::Oid::from_str(&x).unwrap();
    commit_file(&root.join("member"), "f.txt", "y\n", "Y", &[x_oid]).unwrap();

    // Root reports the member as ONE modified unit, never its internal files.
    let status = status_porcelain(&root);
    assert!(
        status.lines().any(|line| line.trim_end().ends_with("member")),
        "member reported as modified: {status:?}"
    );
    assert!(
        !status.contains("member/"),
        "member internals are not surfaced at the root: {status:?}"
    );
}

#[test]
fn root_status_does_not_surface_untracked_inside_gitlink_member() {
    let temp = TempDir::new("gitlink-opaque");
    let backend = Git2Backend::new();
    let root = temp.path().join("root");
    backend.create_repo(&root).unwrap();
    backend.create_repo(&root.join("member")).unwrap();
    let x = commit_file(&root.join("member"), "f.txt", "x\n", "X", &[]).unwrap();
    backend.sync_gitlinks(&root, &[("member", &x)]).unwrap();
    run_git(&root, &["commit", "-m", "base"]);

    // An untracked file inside the member must never appear as a root-level path.
    std::fs::write(root.join("member").join("secret.txt"), "shh\n").unwrap();
    let status = status_porcelain(&root);
    assert!(
        !status.contains("secret.txt"),
        "member internals must stay opaque to the root: {status:?}"
    );
}

// WS6: the CLI-backed `commit` primitive.

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
