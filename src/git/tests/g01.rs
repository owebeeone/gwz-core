    use std::fs;
    use std::path::{Path, PathBuf};
    

    use crate::model::ErrorCode;

    
use super::*;

#[test]
    pub(crate) fn creates_and_detects_ordinary_non_bare_repositories() {
        let temp = TempDir::new("create");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");

        let created = backend.create_repo(&repo_path).unwrap();

        assert_eq!(created.path, repo_path);
        assert!(backend.is_repository(&repo_path).unwrap());
        assert!(!backend.is_repository(&temp.path().join("missing")).unwrap());
        assert!(!git2::Repository::open(&repo_path).unwrap().is_bare());
    }

    #[test]
    pub(crate) fn stage_paths_matches_porcelain_git_add() {
        // Seed two identical repos; stage one via the primitive and one via
        // porcelain `git add`. The resulting index must be byte-identical —
        // pathspec scoping, recursive add, and `.gitignore` honoring all agree.
        let temp = TempDir::new("stage-parity");
        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        seed_stage_repo(&prim);
        seed_stage_repo(&porc);

        let result = Git2Backend::new()
            .stage_paths(&prim, &["tracked", ".gitignore"])
            .expect("primitive stage");
        // Only ".gitignore" is a top-level file; "tracked" is a directory.
        assert_eq!(result.staged, 1);

        run_git(&porc, &["add", "tracked", ".gitignore"]);

        assert_eq!(
            ls_files_stage(&prim),
            ls_files_stage(&porc),
            "primitive index must match `git add` porcelain (mode+oid+path)"
        );
        // Sanity: the gitignored, out-of-pathspec files are staged by neither.
        assert!(!ls_files_stage(&prim).contains("ignored/"));
        assert!(!ls_files_stage(&prim).contains("loose.txt"));
    }

    #[test]
    pub(crate) fn stage_paths_errors_on_non_repository() {
        let temp = TempDir::new("stage-nonrepo");
        let err = Git2Backend::new()
            .stage_paths(temp.path(), &[".gitignore"])
            .expect_err("staging a non-repository must fail");
        assert_eq!(err.code, ErrorCode::GitCommandFailed);
    }

    pub(crate) fn seed_stage_repo(root: &Path) {
        fs::create_dir_all(root.join("tracked")).unwrap();
        fs::write(root.join("tracked").join("a.txt"), "a\n").unwrap();
        fs::create_dir_all(root.join("ignored")).unwrap();
        fs::write(root.join("ignored").join("b.txt"), "b\n").unwrap();
        fs::write(root.join(".gitignore"), "/ignored/\n").unwrap();
        fs::write(root.join("loose.txt"), "loose\n").unwrap();
        Git2Backend::new().create_repo(root).unwrap();
    }

    pub(crate) fn run_git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=GWZ",
                "-c",
                "user.email=gwz@example.invalid",
            ])
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {args:?} failed");
    }

    pub(crate) fn ls_files_stage(root: &Path) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["ls-files", "--stage"])
            .output()
            .expect("spawn git ls-files");
        assert!(output.status.success(), "git ls-files failed");
        String::from_utf8(output.stdout).expect("ls-files utf8")
    }

    #[test]
    pub(crate) fn empty_repository_head_reports_unborn_branch_without_commit() {
        let temp = TempDir::new("empty-head");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");
        backend.create_repo(&repo_path).unwrap();

        let head = backend.head(&repo_path).unwrap();

        assert_eq!(head.branch, Some("main".to_owned()));
        assert_eq!(head.commit, None);
        assert!(!head.is_detached);
        assert_eq!(backend.read_ref(&repo_path, "HEAD").unwrap(), None);
    }

    #[test]
    pub(crate) fn reads_and_adds_remotes() {
        let temp = TempDir::new("remotes");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");
        backend.create_repo(&repo_path).unwrap();

        backend
            .add_remote(&repo_path, "origin", "file:///tmp/origin.git")
            .unwrap();

        let remotes = backend.remotes(&repo_path).unwrap();
        assert_eq!(
            remotes,
            vec![GitRemote {
                name: "origin".to_owned(),
                url: Some("file:///tmp/origin.git".to_owned()),
                push_url: None,
            }]
        );
    }

    #[test]
    pub(crate) fn reports_clean_untracked_unstaged_and_staged_status() {
        let temp = TempDir::new("status");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");
        backend.create_repo(&repo_path).unwrap();
        commit_file(&repo_path, "tracked.txt", "one", "initial", &[]).unwrap();

        assert_eq!(backend.status(&repo_path).unwrap(), GitStatus::clean());

        fs::write(repo_path.join("untracked.txt"), "new").unwrap();
        let status = backend.status(&repo_path).unwrap();
        assert!(status.is_dirty);
        assert_eq!(status.untracked, 1);
        fs::remove_file(repo_path.join("untracked.txt")).unwrap();

        fs::write(repo_path.join("tracked.txt"), "two").unwrap();
        let status = backend.status(&repo_path).unwrap();
        assert!(status.is_dirty);
        assert_eq!(status.unstaged, 1);
        assert_eq!(status.staged, 0);

        stage_path(&repo_path, "tracked.txt").unwrap();
        let status = backend.status(&repo_path).unwrap();
        assert!(status.is_dirty);
        assert_eq!(status.staged, 1);
        assert_eq!(status.unstaged, 0);
    }

    #[test]
    pub(crate) fn clones_local_repo_and_rejects_non_empty_targets_before_mutation() {
        let temp = TempDir::new("clone");
        let backend = Git2Backend::new();
        let source_path = temp.path().join("source");
        backend.create_repo(&source_path).unwrap();
        commit_file(&source_path, "README.md", "hello", "initial", &[]).unwrap();

        let clone_path = temp.path().join("clone");
        backend
            .clone_repo(source_path.to_str().unwrap(), &clone_path)
            .unwrap();
        assert!(backend.is_repository(&clone_path).unwrap());
        assert!(clone_path.join("README.md").is_file());

        let blocked_path = temp.path().join("blocked");
        fs::create_dir_all(&blocked_path).unwrap();
        fs::write(blocked_path.join("keep.txt"), "keep").unwrap();
        let err = backend
            .clone_repo(source_path.to_str().unwrap(), &blocked_path)
            .unwrap_err();

        assert_eq!(err.code, ErrorCode::PathCollision);
        assert!(blocked_path.join("keep.txt").is_file());
        assert!(!blocked_path.join(".git").exists());
    }

    #[test]
    pub(crate) fn pushes_fetches_fast_forwards_and_checks_out_commits() {
        let temp = TempDir::new("networkless");
        let backend = Git2Backend::new();
        let source_path = temp.path().join("source");
        let bare_path = temp.path().join("remote.git");
        let clone_path = temp.path().join("clone");
        backend.create_repo(&source_path).unwrap();
        init_bare_main(&bare_path);
        backend
            .add_remote(&source_path, "origin", bare_path.to_str().unwrap())
            .unwrap();

        let first = commit_file(&source_path, "README.md", "one", "initial", &[]).unwrap();
        backend
            .push(&source_path, "origin", "refs/heads/main:refs/heads/main")
            .unwrap();
        backend
            .clone_repo(bare_path.to_str().unwrap(), &clone_path)
            .unwrap();
        let cloned_head = backend.head(&clone_path).unwrap();
        assert_eq!(cloned_head.branch, Some("main".to_owned()));
        assert!(!cloned_head.is_detached);
        assert_eq!(cloned_head.commit, Some(first.clone()));
        assert_eq!(
            backend.read_ref(&clone_path, "HEAD").unwrap(),
            Some(first.clone())
        );

        let parent = git2::Repository::open(&source_path)
            .unwrap()
            .find_commit(git2::Oid::from_str(&first).unwrap())
            .unwrap()
            .id();
        let second =
            commit_file(&source_path, "dev-docs/new.md", "two", "second", &[parent]).unwrap();
        backend
            .push(&source_path, "origin", "refs/heads/main:refs/heads/main")
            .unwrap();

        backend.fetch(&clone_path, "origin").unwrap();
        backend
            .fast_forward(&clone_path, "main", "refs/remotes/origin/main")
            .unwrap();
        assert_eq!(backend.head(&clone_path).unwrap().commit, Some(second));
        assert_eq!(
            fs::read_to_string(clone_path.join("dev-docs/new.md")).unwrap(),
            "two"
        );
        assert!(!backend.status(&clone_path).unwrap().is_dirty);

        backend.checkout_commit(&clone_path, &first).unwrap();
        let head = backend.head(&clone_path).unwrap();
        assert!(head.is_detached);
        assert_eq!(head.commit, Some(first));
    }

    #[test]
    pub(crate) fn fast_forward_matches_porcelain_merge_ff_only_and_self_verifies() {
        // main@A behind feature@B (A is an ancestor of B): a clean fast-forward.
        let temp = TempDir::new("ff-parity");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        run_git(&base, &["branch", "feature"]);
        run_git(&base, &["checkout", "feature"]);
        let b = commit_file(&base, "f.txt", "b\n", "B", &[a_oid]).unwrap();
        run_git(&base, &["checkout", "main"]);

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);

        let result = backend
            .fast_forward(&prim, "main", "refs/heads/feature")
            .unwrap();
        assert!(result.updated);
        assert_eq!(result.commit.as_deref(), Some(b.as_str()));

        run_git(&porc, &["merge", "--ff-only", "feature"]);

        // Byte-identical end state vs porcelain: same HEAD, same tree, clean worktree.
        assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
        assert_eq!(rev_parse(&prim, "HEAD"), b);
        assert_eq!(
            rev_parse(&prim, "HEAD^{tree}"),
            rev_parse(&porc, "HEAD^{tree}")
        );
        assert!(status_porcelain(&prim).trim().is_empty());
        assert_eq!(fs::read_to_string(prim.join("f.txt")).unwrap(), "b\n");
    }

    #[test]
    pub(crate) fn fast_forward_rejects_divergent_history_without_moving_branch() {
        // main@D and feature@C both descend from A — not fast-forwardable.
        let temp = TempDir::new("ff-diverge");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        run_git(&base, &["branch", "feature"]);
        run_git(&base, &["checkout", "feature"]);
        commit_file(&base, "f.txt", "c\n", "C", &[a_oid]).unwrap();
        run_git(&base, &["checkout", "main"]);
        let d = commit_file(&base, "f.txt", "d\n", "D", &[a_oid]).unwrap();

        let err = backend
            .fast_forward(&base, "main", "refs/heads/feature")
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DivergedMember);
        // Porcelain agrees it is not fast-forwardable.
        assert!(!run_git_ok(&base, &["merge", "--ff-only", "feature"]));
        // Failed = nothing changed: main is still at D.
        assert_eq!(rev_parse(&base, "HEAD"), d);
    }

    #[test]
    pub(crate) fn checkout_commit_matches_porcelain_and_self_verifies() {
        // Detach onto an older commit A while B is current.
        let temp = TempDir::new("checkout-parity");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        commit_file(&base, "f.txt", "b\n", "B", &[a_oid]).unwrap();

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);

        let result = backend.checkout_commit(&prim, &a).unwrap();
        assert_eq!(result.commit.as_deref(), Some(a.as_str()));

        run_git(&porc, &["checkout", &a]);

        assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
        assert_eq!(rev_parse(&prim, "HEAD"), a);
        assert_eq!(
            rev_parse(&prim, "HEAD^{tree}"),
            rev_parse(&porc, "HEAD^{tree}")
        );
        assert!(status_porcelain(&prim).trim().is_empty());
        assert_eq!(fs::read_to_string(prim.join("f.txt")).unwrap(), "a\n");
        assert!(backend.head(&prim).unwrap().is_detached);
    }

    #[test]
    pub(crate) fn checkout_commit_rejects_unknown_commit_without_moving_head() {
        let temp = TempDir::new("checkout-missing");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let before = rev_parse(&base, "HEAD");

        let bogus = "0".repeat(40);
        let err = backend.checkout_commit(&base, &bogus).unwrap_err();
        assert_eq!(err.code, ErrorCode::GitCommandFailed);
        assert_eq!(rev_parse(&base, "HEAD"), before);
    }

    #[test]
    pub(crate) fn verify_checkout_state_accepts_match_and_rejects_mismatch() {
        // Direct test of the AD1 self-verify: HEAD is at B.
        let temp = TempDir::new("verify-state");
        let backend = Git2Backend::new();
        let repo = temp.path().join("repo");
        backend.create_repo(&repo).unwrap();
        let a = commit_file(&repo, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        let b = commit_file(&repo, "f.txt", "b\n", "B", &[a_oid]).unwrap();
        let b_oid = git2::Oid::from_str(&b).unwrap();

        assert!(verify_checkout_state(&repo, b_oid).is_ok());
        let err = verify_checkout_state(&repo, a_oid).unwrap_err();
        assert_eq!(err.code, ErrorCode::GitCommandFailed);
    }

    pub(crate) fn commit_file(
        repo_path: &Path,
        relative_path: &str,
        content: &str,
        message: &str,
        parents: &[git2::Oid],
    ) -> Result<String, git2::Error> {
        if let Some(parent) = Path::new(relative_path).parent() {
            fs::create_dir_all(repo_path.join(parent)).unwrap();
        }
        fs::write(repo_path.join(relative_path), content).unwrap();
        stage_path(repo_path, relative_path)?;

        let repo = git2::Repository::open(repo_path)?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid")?;
        let parent_commits = parents
            .iter()
            .map(|id| repo.find_commit(*id))
            .collect::<Result<Vec<_>, _>>()?;
        let parent_refs = parent_commits.iter().collect::<Vec<_>>();
        let oid = repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )?;
        Ok(oid.to_string())
    }

    pub(crate) fn stage_path(repo_path: &Path, relative_path: &str) -> Result<(), git2::Error> {
        let repo = git2::Repository::open(repo_path)?;
        let mut index = repo.index()?;
        index.add_path(Path::new(relative_path))?;
        index.write()
    }

    #[test]
    pub(crate) fn merge_upstream_matches_porcelain_merge_on_clean_diverge() {
        // main@D and feature@C diverge from A touching DIFFERENT files → clean 3-way merge.
        let temp = TempDir::new("merge-clean");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        run_git(&base, &["branch", "feature"]);
        run_git(&base, &["checkout", "feature"]);
        commit_file(&base, "feat.txt", "feature\n", "C", &[a_oid]).unwrap();
        run_git(&base, &["checkout", "main"]);
        commit_file(&base, "main.txt", "main\n", "D", &[a_oid]).unwrap();

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);

        let result = backend
            .merge_upstream(&prim, "main", "refs/heads/feature")
            .unwrap();
        assert!(result.is_clean());
        let merge_commit = result.commit.clone().unwrap();

        run_git(&porc, &["merge", "--no-edit", "feature"]);

        // Commit OIDs differ (signature/time), but the merged TREE must match porcelain,
        // the worktree is clean, and HEAD is a two-parent merge commit over feature.
        assert_eq!(
            rev_parse(&prim, "HEAD^{tree}"),
            rev_parse(&porc, "HEAD^{tree}")
        );
        assert!(status_porcelain(&prim).trim().is_empty());
        assert_eq!(rev_parse(&prim, "HEAD"), merge_commit);
        assert_eq!(
            rev_parse(&prim, "HEAD^2"),
            rev_parse(&prim, "refs/heads/feature")
        );
        assert_eq!(
            fs::read_to_string(prim.join("feat.txt")).unwrap(),
            "feature\n"
        );
        assert_eq!(fs::read_to_string(prim.join("main.txt")).unwrap(), "main\n");
    }

    #[test]
    pub(crate) fn merge_upstream_leaves_conflict_in_place_like_porcelain() {
        // main@D and feature@C both rewrite f.txt → a real merge conflict.
        let temp = TempDir::new("merge-conflict");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        run_git(&base, &["branch", "feature"]);
        run_git(&base, &["checkout", "feature"]);
        commit_file(&base, "f.txt", "feature\n", "C", &[a_oid]).unwrap();
        run_git(&base, &["checkout", "main"]);
        let d = commit_file(&base, "f.txt", "main\n", "D", &[a_oid]).unwrap();

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);

        let result = backend
            .merge_upstream(&prim, "main", "refs/heads/feature")
            .unwrap();
        // A conflict is reported, not errored: the path is named, HEAD has not moved.
        assert!(!result.is_clean());
        assert_eq!(result.conflicts, vec!["f.txt".to_owned()]);
        assert!(result.commit.is_none());
        assert_eq!(rev_parse(&prim, "HEAD"), d);
        // Faithful to porcelain: worktree is left mid-merge and `git merge --continue`-able.
        assert!(prim.join(".git/MERGE_HEAD").exists());
        assert!(!run_git_ok(&porc, &["merge", "--no-edit", "feature"]));
        assert_eq!(
            status_porcelain(&prim).trim(),
            status_porcelain(&porc).trim()
        );
    }

    #[test]
    pub(crate) fn rebase_onto_matches_porcelain_rebase_on_clean_diverge() {
        // main@D and feature@C diverge from A touching DIFFERENT files → clean replay.
        let temp = TempDir::new("rebase-clean");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        run_git(&base, &["branch", "feature"]);
        run_git(&base, &["checkout", "feature"]);
        let c = commit_file(&base, "feat.txt", "feature\n", "C", &[a_oid]).unwrap();
        run_git(&base, &["checkout", "main"]);
        commit_file(&base, "main.txt", "main\n", "D", &[a_oid]).unwrap();

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);

        let result = backend
            .rebase_onto(&prim, "main", "refs/heads/feature")
            .unwrap();
        assert!(result.is_clean());

        run_git(&porc, &["rebase", "feature"]);

        // Linear history replayed onto feature: same tree as porcelain, clean worktree,
        // HEAD reattached to main with the feature tip as its single parent.
        assert_eq!(
            rev_parse(&prim, "HEAD^{tree}"),
            rev_parse(&porc, "HEAD^{tree}")
        );
        assert!(status_porcelain(&prim).trim().is_empty());
        assert_eq!(rev_parse(&prim, "HEAD^"), c);
        assert_eq!(rev_parse(&prim, "HEAD"), result.commit.unwrap());
        let head = backend.head(&prim).unwrap();
        assert!(!head.is_detached);
        assert_eq!(head.branch.as_deref(), Some("main"));
    }

    #[test]
    pub(crate) fn rebase_onto_leaves_conflict_in_place_like_porcelain() {
        // main@D and feature@C both rewrite f.txt → the replay conflicts.
        let temp = TempDir::new("rebase-conflict");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        run_git(&base, &["branch", "feature"]);
        run_git(&base, &["checkout", "feature"]);
        commit_file(&base, "f.txt", "feature\n", "C", &[a_oid]).unwrap();
        run_git(&base, &["checkout", "main"]);
        commit_file(&base, "f.txt", "main\n", "D", &[a_oid]).unwrap();

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);

        let result = backend
            .rebase_onto(&prim, "main", "refs/heads/feature")
            .unwrap();
        assert!(!result.is_clean());
        assert_eq!(result.conflicts, vec!["f.txt".to_owned()]);
        assert!(result.commit.is_none());
        // Faithful to porcelain: the rebase is left in progress, `git rebase --continue`-able.
        assert!(prim.join(".git/rebase-merge").exists());
        assert!(!run_git_ok(&porc, &["rebase", "feature"]));
        assert!(porc.join(".git/rebase-merge").exists());
    }

    #[test]
    pub(crate) fn reset_hard_matches_porcelain_and_discards_local() {
        // main@D diverged from feature@C; reset --hard snaps main onto C, discarding D
        // AND any uncommitted changes.
        let temp = TempDir::new("reset-hard");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        run_git(&base, &["branch", "feature"]);
        run_git(&base, &["checkout", "feature"]);
        let c = commit_file(&base, "f.txt", "feature\n", "C", &[a_oid]).unwrap();
        run_git(&base, &["checkout", "main"]);
        commit_file(&base, "f.txt", "main\n", "D", &[a_oid]).unwrap();

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);
        // Dirty the primary worktree: reset --hard must discard this too.
        fs::write(prim.join("f.txt"), "uncommitted\n").unwrap();
        assert!(backend.status(&prim).unwrap().is_dirty);

        let result = backend
            .reset_hard(&prim, "main", "refs/heads/feature")
            .unwrap();
        assert!(result.updated);
        assert_eq!(result.commit.as_deref(), Some(c.as_str()));

        run_git(&porc, &["reset", "--hard", "feature"]);

        // Byte-identical end state vs porcelain: same HEAD at feature, same tree, clean.
        assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
        assert_eq!(rev_parse(&prim, "HEAD"), c);
        assert_eq!(
            rev_parse(&prim, "HEAD^{tree}"),
            rev_parse(&porc, "HEAD^{tree}")
        );
        assert!(status_porcelain(&prim).trim().is_empty());
        assert_eq!(fs::read_to_string(prim.join("f.txt")).unwrap(), "feature\n");
        let head = backend.head(&prim).unwrap();
        assert!(!head.is_detached);
        assert_eq!(head.branch.as_deref(), Some("main"));
    }

    #[test]
    pub(crate) fn checkout_branch_matches_porcelain_and_refuses_diverged_reset() {
        let temp = TempDir::new("checkout-branch");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        let b = commit_file(&base, "f.txt", "b\n", "B", &[a_oid]).unwrap();

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);

        // Create `feature` at the older commit A and check out onto it.
        let result = backend.checkout_branch(&prim, "feature", &a).unwrap();
        assert_eq!(result.commit.as_deref(), Some(a.as_str()));
        run_git(&porc, &["checkout", "-b", "feature", &a]);

        // Byte-identical end state vs porcelain: on `feature` at A, clean.
        assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
        assert_eq!(rev_parse(&prim, "HEAD"), a);
        assert_eq!(
            rev_parse(&prim, "HEAD^{tree}"),
            rev_parse(&porc, "HEAD^{tree}")
        );
        assert!(status_porcelain(&prim).trim().is_empty());
        let head = backend.head(&prim).unwrap();
        assert!(!head.is_detached);
        assert_eq!(head.branch.as_deref(), Some("feature"));
        // `main` is untouched at B — never silently reset.
        assert_eq!(rev_parse(&prim, "refs/heads/main"), b);

        // Refuse to move `main` (at B) back to A — that would orphan B.
        let err = backend.checkout_branch(&prim, "main", &a).unwrap_err();
        assert_eq!(err.code, ErrorCode::DivergedMember);
        assert_eq!(rev_parse(&prim, "refs/heads/main"), b);
    }

    pub(crate) fn init_bare_main(path: &Path) {
        let repo = git2::Repository::init_bare(path).unwrap();
        repo.set_head("refs/heads/main").unwrap();
    }

    #[test]
    pub(crate) fn ls_remote_lists_advertised_refs_matching_porcelain() {
        let temp = TempDir::new("ls-remote");
        let backend = Git2Backend::new();
        let source = temp.path().join("source");
        let bare = temp.path().join("remote.git");
        backend.create_repo(&source).unwrap();
        init_bare_main(&bare);
        backend
            .add_remote(&source, "origin", bare.to_str().unwrap())
            .unwrap();
        let first = commit_file(&source, "README.md", "one", "initial", &[]).unwrap();
        backend
            .push(&source, "origin", "refs/heads/main:refs/heads/main")
            .unwrap();
        run_git(&source, &["tag", "v1"]);
        backend
            .push(&source, "origin", "refs/tags/v1:refs/tags/v1")
            .unwrap();

        // Non-mutating: capture local refs, call ls_remote, confirm unchanged.
        let refs_before = all_local_refs(&source);
        let refs = backend.ls_remote(&source, "origin").unwrap();
        assert_eq!(
            all_local_refs(&source),
            refs_before,
            "ls_remote must not mutate local refs"
        );

        let mut got = refs
            .iter()
            .map(|r| format!("{} {}", r.target, r.name))
            .collect::<Vec<_>>();
        got.sort();
        // Same advertised ref set as porcelain `git ls-remote` (oid + name).
        assert_eq!(got, ls_remote_porcelain(&source, "origin"));
        // Sanity: main resolves to the pushed commit.
        assert!(
            refs.iter()
                .any(|r| r.name == "refs/heads/main" && r.target == first)
        );
    }

    #[test]
    pub(crate) fn ls_remote_rejects_missing_remote() {
        let temp = TempDir::new("ls-remote-missing");
        let backend = Git2Backend::new();
        let source = temp.path().join("source");
        backend.create_repo(&source).unwrap();
        let err = backend.ls_remote(&source, "origin").unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingRemote);
    }

    pub(crate) fn ls_remote_porcelain(repo: &Path, remote: &str) -> Vec<String> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["ls-remote", remote])
            .output()
            .expect("spawn git ls-remote");
        assert!(output.status.success(), "git ls-remote failed");
        let mut lines = String::from_utf8(output.stdout)
            .expect("ls-remote utf8")
            .lines()
            .map(|line| {
                let mut parts = line.split('\t');
                let oid = parts.next().unwrap_or_default();
                let name = parts.next().unwrap_or_default();
                format!("{oid} {name}")
            })
            .collect::<Vec<_>>();
        lines.sort();
        lines
    }

    pub(crate) fn all_local_refs(repo: &Path) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["for-each-ref", "--format=%(objectname) %(refname)"])
            .output()
            .expect("spawn git for-each-ref");
        assert!(output.status.success(), "git for-each-ref failed");
        let mut lines = String::from_utf8(output.stdout)
            .expect("for-each-ref utf8")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        lines.sort();
        lines.join("\n")
    }

    pub(crate) fn copy_repo(src: &Path, dst: &Path) {
        let status = std::process::Command::new("cp")
            .arg("-R")
            .arg(src)
            .arg(dst)
            .status()
            .expect("spawn cp");
        assert!(status.success(), "cp -R failed");
    }

    pub(crate) fn rev_parse(repo: &Path, rev: &str) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-parse", rev])
            .output()
            .expect("spawn git rev-parse");
        assert!(output.status.success(), "git rev-parse {rev} failed");
        String::from_utf8(output.stdout)
            .expect("rev-parse utf8")
            .trim()
            .to_owned()
    }

    pub(crate) fn status_porcelain(repo: &Path) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["status", "--porcelain"])
            .output()
            .expect("spawn git status");
        assert!(output.status.success(), "git status failed");
        String::from_utf8(output.stdout).expect("status utf8")
    }

    pub(crate) fn run_git_ok(root: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(["-c", "user.name=GWZ", "-c", "user.email=gwz@example.invalid"])
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("spawn git")
            .success()
    }

    pub(crate) struct TempDir {
        pub(crate) path: PathBuf,
    }

    