#[cfg(test)]
use super::*;

pub(crate) fn workspace_path(member_path: &str, repo_path: &str) -> String {
    if repo_path.is_empty() {
        member_path.to_owned()
    } else {
        format!("{member_path}/{repo_path}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::artifact::{
        ArtifactSourceKind, LockArtifact, ManifestArtifact, ManifestMember, RemoteArtifact,
        ResolvedMemberArtifact, WorkspaceHeader, write_lock, write_manifest,
    };
    use crate::git::{Git2Backend, GitBackend};
    use crate::model::ErrorCode;

    use super::*;

    #[test]
    fn status_on_empty_workspace_succeeds() {
        let temp = TempDir::new("empty");
        write_manifest(temp.path(), &manifest(vec![])).unwrap();

        let response = handle_status(
            &Git2Backend::new(),
            temp.path(),
            status_request(None),
            "op_status",
        )
        .unwrap();

        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        assert!(response.response.members.is_empty());
    }

    #[test]
    fn status_on_clean_member_reports_git_status_and_lock_match() {
        let temp = TempDir::new("clean");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repos/app");
        backend.create_repo(&repo_path).unwrap();
        let commit = commit_file(&repo_path, "README.md", "clean", "initial", &[]).unwrap();
        write_manifest(
            temp.path(),
            &manifest(vec![member("mem_app", "repos/app", true)]),
        )
        .unwrap();
        write_lock(
            temp.path(),
            &lock("mem_app", "repos/app", Some(commit.clone()), false),
        )
        .unwrap();

        let response =
            handle_status(&backend, &repo_path, status_request(None), "op_status").unwrap();
        let member = response.response.members.single();
        let git_status = member.git_status.as_ref().unwrap();

        assert_eq!(member.member_id, "mem_app");
        assert_eq!(member.status, crate::MemberStatus::Ok);
        assert_eq!(member.lock_match, Some(crate::LockMatch::Matches));
        assert_eq!(git_status.head, Some(commit));
        assert_eq!(git_status.branch, Some("main".to_owned()));
        assert!(!git_status.dirty);
    }

    #[test]
    fn status_on_dirty_member_reports_dirty_counts_and_lock_difference() {
        let temp = TempDir::new("dirty");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repos/app");
        backend.create_repo(&repo_path).unwrap();
        let commit = commit_file(&repo_path, "README.md", "clean", "initial", &[]).unwrap();
        fs::write(repo_path.join("README.md"), "dirty").unwrap();
        fs::write(repo_path.join("new.txt"), "new").unwrap();
        write_manifest(
            temp.path(),
            &manifest(vec![member("mem_app", "repos/app", true)]),
        )
        .unwrap();
        write_lock(
            temp.path(),
            &lock("mem_app", "repos/app", Some(commit), false),
        )
        .unwrap();

        let response =
            handle_status(&backend, temp.path(), status_request(None), "op_status").unwrap();
        let git_status = response
            .response
            .members
            .single()
            .git_status
            .clone()
            .unwrap();

        assert!(git_status.dirty);
        // F5: a dirty member surfaces in the aggregate (no longer masquerades as Ok).
        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Dirty
        );
        assert_eq!(git_status.unstaged, 1);
        assert_eq!(git_status.untracked, 1);
        assert_eq!(
            response.response.members.single().lock_match,
            Some(crate::LockMatch::Differs)
        );
    }

    #[test]
    fn status_on_unmaterialized_member_reports_missing_not_failure() {
        // Right after a bare `git clone` of a workspace root, members are
        // declared in gwz.conf but their working trees were never cloned. That
        // is an expected, recoverable state, not a git failure.
        let temp = TempDir::new("unmaterialized");
        let backend = Git2Backend::new();
        let commit = "0".repeat(40);
        write_manifest(
            temp.path(),
            &manifest(vec![member("mem_app", "repos/app", true)]),
        )
        .unwrap();
        write_lock(
            temp.path(),
            &lock("mem_app", "repos/app", Some(commit.clone()), false),
        )
        .unwrap();

        let mut request = status_request(None);
        request.mode = Some(crate::StatusMode::Combined);
        let response = handle_status(&backend, temp.path(), request, "op_status").unwrap();
        let member = response.response.members.single();

        assert_eq!(member.status, crate::MemberStatus::Noop);
        assert!(member.error.is_none());
        assert!(member.git_status.is_none());
        assert_eq!(member.lock_match, Some(crate::LockMatch::Missing));
        let state = member.state.as_ref().expect("member state present");
        assert!(!state.materialized);
        assert_eq!(state.commit.as_deref(), Some(commit.as_str()));
        assert_eq!(state.branch.as_deref(), Some("main"));
        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        // The absent member must not appear as a phantom branch group.
        let workspace_status = response.workspace_git_status.as_ref().unwrap();
        assert!(workspace_status.branch_groups.is_empty());
    }

    #[test]
    fn combined_status_reports_workspace_file_changes_and_branches() {
        let temp = TempDir::new("combined");
        let backend = Git2Backend::new();
        backend.create_repo(temp.path()).unwrap();
        let repo_path = temp.path().join("repos/app");
        backend.create_repo(&repo_path).unwrap();
        let commit = commit_file(&repo_path, "README.md", "clean", "initial", &[]).unwrap();
        fs::write(repo_path.join("README.md"), "dirty").unwrap();
        fs::write(repo_path.join("new.txt"), "new").unwrap();
        write_manifest(
            temp.path(),
            &manifest(vec![member("mem_app", "repos/app", true)]),
        )
        .unwrap();
        write_lock(
            temp.path(),
            &lock("mem_app", "repos/app", Some(commit), false),
        )
        .unwrap();
        let mut request = status_request(None);
        request.mode = Some(crate::StatusMode::Combined);
        request.include_file_changes = Some(true);
        request.include_branch_summary = Some(true);
        request.path_style = Some(crate::StatusPathStyle::WorkspaceRelative);

        let response = handle_status(&backend, temp.path(), request, "op_status").unwrap();
        let workspace_status = response.workspace_git_status.as_ref().unwrap();

        assert!(!workspace_status.clean);
        let root_status = workspace_status.root_status.as_ref().unwrap();
        assert_eq!(root_status.branch, Some("main".to_owned()));
        assert!(root_status.dirty);
        assert!(!workspace_status.root_file_changes.is_empty());
        assert!(workspace_status.root_file_changes.iter().any(|change| {
            change.repo_path == "gwz.conf/gwz.yml"
                && change.workspace_path == "gwz.conf/gwz.yml"
                && change.worktree_status == "?"
        }));
        assert_eq!(workspace_status.file_changes.len(), 2);
        assert!(workspace_status.file_changes.iter().any(|change| {
            change.member_id == "mem_app"
                && change.repo_path == "README.md"
                && change.workspace_path == "repos/app/README.md"
                && change.worktree_status == "M"
        }));
        assert!(workspace_status.file_changes.iter().any(|change| {
            change.member_id == "mem_app"
                && change.repo_path == "new.txt"
                && change.workspace_path == "repos/app/new.txt"
                && change.worktree_status == "?"
        }));
        assert_eq!(workspace_status.branches.len(), 1);
        assert_eq!(workspace_status.branches[0].label, "main");
        assert_eq!(workspace_status.branch_groups.len(), 1);
        assert!(workspace_status.branch_differences.is_empty());
    }

    #[test]
    fn combined_status_hides_ignored_root_files() {
        let temp = TempDir::new("ignored-root");
        let backend = Git2Backend::new();
        backend.create_repo(temp.path()).unwrap();
        write_manifest(temp.path(), &manifest(vec![])).unwrap();
        fs::write(temp.path().join(".gitignore"), "ignored/\n").unwrap();
        commit_all(temp.path(), "workspace metadata and ignores").unwrap();
        fs::create_dir_all(temp.path().join("ignored")).unwrap();
        fs::write(temp.path().join("ignored/cache.txt"), "cache").unwrap();

        let mut request = status_request(None);
        request.mode = Some(crate::StatusMode::Combined);
        request.include_file_changes = Some(true);
        request.path_style = Some(crate::StatusPathStyle::WorkspaceRelative);

        let response = handle_status(&backend, temp.path(), request, "op_status").unwrap();
        let workspace_status = response.workspace_git_status.as_ref().unwrap();

        assert!(workspace_status.clean);
        assert!(workspace_status.root_file_changes.is_empty());
        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
    }

    #[test]
    fn unknown_inactive_and_duplicate_selection_behave_before_member_work() {
        let temp = TempDir::new("selection");
        write_manifest(
            temp.path(),
            &manifest(vec![
                member("mem_active", "repos/active", true),
                member("mem_inactive", "repos/inactive", false),
            ]),
        )
        .unwrap();
        let backend = Git2Backend::new();

        assert_eq!(
            handle_status(
                &backend,
                temp.path(),
                status_request(Some(selection(false, &["mem_missing"], &[]))),
                "op_status",
            )
            .unwrap_err()
            .code,
            ErrorCode::MemberNotFound
        );
        assert_eq!(
            handle_status(
                &backend,
                temp.path(),
                status_request(Some(selection(false, &["mem_inactive"], &[]))),
                "op_status",
            )
            .unwrap_err()
            .code,
            ErrorCode::MemberInactive
        );
        let duplicate = handle_status(
            &backend,
            temp.path(),
            status_request(Some(selection(false, &["mem_active"], &["repos/active"]))),
            "op_status",
        )
        .unwrap();
        assert_eq!(duplicate.response.members.len(), 1);
        assert_eq!(duplicate.response.members[0].member_id, "mem_active");
    }

    fn status_request(selection: Option<crate::Selection>) -> crate::StatusRequest {
        crate::StatusRequest {
            meta: crate::RequestMeta {
                request_id: "req_status".to_owned(),
                schema_version: "gwz.protocol/v0".to_owned(),
                selection,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn selection(all: bool, member_ids: &[&str], paths: &[&str]) -> crate::Selection {
        crate::Selection {
            all: Some(all),
            member_ids: member_ids.iter().map(|value| (*value).to_owned()).collect(),
            paths: paths.iter().map(|value| (*value).to_owned()).collect(),
            targets: Vec::new(),
            exclude_targets: Vec::new(),
        }
    }

    fn manifest(members: Vec<ManifestMember>) -> ManifestArtifact {
        ManifestArtifact {
            schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: WorkspaceHeader {
                id: "ws_status".to_owned(),
            },
            members,
        }
    }

    fn member(id: &str, path: &str, active: bool) -> ManifestMember {
        ManifestMember {
            id: id.to_owned(),
            path: path.to_owned(),
            source_kind: ArtifactSourceKind::Git,
            source_id: "src_status".to_owned(),
            active,
            desired: None,
            remotes: vec![RemoteArtifact {
                name: "origin".to_owned(),
                url: "file:///tmp/origin.git".to_owned(),
                fetch: true,
                push: true,
            }],
        }
    }

    fn lock(member_id: &str, path: &str, commit: Option<String>, dirty: bool) -> LockArtifact {
        LockArtifact {
            schema: crate::artifact::LOCK_SCHEMA.to_owned(),
            workspace_id: "ws_status".to_owned(),
            manifest_schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            members: BTreeMap::from([(
                member_id.to_owned(),
                ResolvedMemberArtifact {
                    path: path.to_owned(),
                    source_id: Some("src_status".to_owned()),
                    source_kind: ArtifactSourceKind::Git,
                    commit,
                    branch: Some("main".to_owned()),
                    detached: Some(false),
                    upstream: None,
                    dirty: Some(dirty),
                    materialized: Some(true),
                },
            )]),
        }
    }

    fn commit_file(
        repo_path: &Path,
        relative_path: &str,
        content: &str,
        message: &str,
        parents: &[git2::Oid],
    ) -> Result<String, git2::Error> {
        fs::write(repo_path.join(relative_path), content).unwrap();
        let repo = git2::Repository::open(repo_path)?;
        let mut index = repo.index()?;
        index.add_path(Path::new(relative_path))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid")?;
        let parent_commits = parents
            .iter()
            .map(|id| repo.find_commit(*id))
            .collect::<Result<Vec<_>, _>>()?;
        let parent_refs = parent_commits.iter().collect::<Vec<_>>();
        Ok(repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent_refs,
            )?
            .to_string())
    }

    fn commit_all(repo_path: &Path, message: &str) -> Result<String, git2::Error> {
        let repo = git2::Repository::open(repo_path)?;
        let mut index = repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid")?;
        let parent_ids = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .into_iter()
            .collect::<Vec<_>>();
        let parent_commits = parent_ids
            .iter()
            .map(|id| repo.find_commit(*id))
            .collect::<Result<Vec<_>, _>>()?;
        let parent_refs = parent_commits.iter().collect::<Vec<_>>();
        Ok(repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent_refs,
            )?
            .to_string())
    }

    trait Single<T> {
        fn single(&self) -> &T;
    }

    impl<T> Single<T> for Vec<T> {
        fn single(&self) -> &T {
            assert_eq!(self.len(), 1);
            &self[0]
        }
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gwz-core-status-{prefix}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
