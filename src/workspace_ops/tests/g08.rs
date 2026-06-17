    
    use std::path::Path;
    
    
    

    
    use crate::git::{Git2Backend, GitBackend};
    
    

    
use super::*;

#[test]
    pub(crate) fn push_selected_member_to_local_bare_remote_succeeds() {
        let temp = TempDir::new("push-success");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let remote = temp.path().join("remote.git");
        init_bare_main(&remote);
        let repo_path = temp.path().join("repos/app");
        backend.create_repo(&repo_path).unwrap();
        backend
            .add_remote(&repo_path, "origin", remote.to_str().unwrap())
            .unwrap();
        let commit = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
        write_pull_fixture(
            temp.path(),
            vec![("mem_app", "repos/app", remote.to_str().unwrap(), &commit)],
        );

        let response =
            handle_push(&backend, temp.path(), push_request(None, None), "op_push").unwrap();

        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Ok
        );
        assert_eq!(
            response.response.members.single().status,
            crate::MemberStatus::Ok
        );
        assert_eq!(read_repo_ref(&remote, "refs/heads/main"), Some(commit));
    }

    #[test]
    pub(crate) fn push_honors_request_remote_and_refspec() {
        let temp = TempDir::new("push-refspec");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let remote = temp.path().join("publish.git");
        init_bare_main(&remote);
        let repo_path = temp.path().join("repos/app");
        backend.create_repo(&repo_path).unwrap();
        backend
            .add_remote(&repo_path, "publish", remote.to_str().unwrap())
            .unwrap();
        let commit = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
        write_pull_fixture(
            temp.path(),
            vec![("mem_app", "repos/app", remote.to_str().unwrap(), &commit)],
        );

        let response = handle_push(
            &backend,
            temp.path(),
            push_request_explicit(
                Some("publish"),
                Some("refs/heads/main:refs/heads/published"),
            ),
            "op_push",
        )
        .unwrap();

        assert_eq!(
            response.response.members.single().status,
            crate::MemberStatus::Ok
        );
        assert_eq!(read_repo_ref(&remote, "refs/heads/main"), None);
        assert_eq!(read_repo_ref(&remote, "refs/heads/published"), Some(commit));
    }

    #[test]
    pub(crate) fn push_local_only_member_without_remote_fails_or_skips_by_policy() {
        let temp = TempDir::new("push-local-only");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        handle_create_repo(
            &backend,
            temp.path(),
            create_repo_request("repos/app", None, None),
            "op_repo",
        )
        .unwrap();

        let failed =
            handle_push(&backend, temp.path(), push_request(None, None), "op_push").unwrap();
        assert_eq!(
            failed.response.meta.aggregate_status,
            crate::AggregateStatus::Rejected
        );
        assert_eq!(
            failed.response.members.single().status,
            crate::MemberStatus::Rejected
        );
        assert_eq!(
            failed
                .response
                .members
                .single()
                .error
                .as_ref()
                .unwrap()
                .code,
            crate::GwzErrorCode::MissingRemote
        );

        let skipped = handle_push(
            &backend,
            temp.path(),
            push_request(Some(crate::UnsupportedMemberBehavior::Skip), None),
            "op_push",
        )
        .unwrap();
        assert_eq!(
            skipped.response.meta.aggregate_status,
            crate::AggregateStatus::Noop
        );
        assert_eq!(
            skipped.response.members.single().status,
            crate::MemberStatus::Skipped
        );
    }

    #[test]
    pub(crate) fn push_remote_rejection_is_reported_per_member() {
        let temp = TempDir::new("push-reject");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
        let fixture = RemoteFixture::new("push-reject-source");
        let first = fixture.commit_and_push("README.md", "one", "initial", &backend);
        backend
            .clone_repo(fixture.remote_url(), &temp.path().join("repos/app"))
            .unwrap();
        let remote_second = fixture.commit_and_push("README.md", "two", "second", &backend);
        let first_oid = git2::Oid::from_str(&first).unwrap();
        let local = commit_file(
            &temp.path().join("repos/app"),
            "README.md",
            "local",
            "local",
            &[first_oid],
        )
        .unwrap();
        write_pull_fixture(
            temp.path(),
            vec![("mem_app", "repos/app", fixture.remote_url(), &local)],
        );

        let response =
            handle_push(&backend, temp.path(), push_request(None, None), "op_push").unwrap();

        assert_eq!(
            response.response.meta.aggregate_status,
            crate::AggregateStatus::Failed
        );
        let member = response.response.members.single();
        assert_eq!(member.status, crate::MemberStatus::Failed);
        assert_eq!(
            member.error.as_ref().unwrap().code,
            crate::GwzErrorCode::RemoteRejected
        );
        assert_eq!(
            read_repo_ref(Path::new(fixture.remote_url()), "refs/heads/main"),
            Some(remote_second)
        );
    }

    pub(crate) fn push_request(
        unsupported_member: Option<crate::UnsupportedMemberBehavior>,
        remote: Option<&str>,
    ) -> crate::PushRequest {
        crate::PushRequest {
            meta: crate::RequestMeta {
                policy: Some(crate::OperationPolicy {
                    unsupported_member,
                    remote: remote.map(ToOwned::to_owned),
                    ..Default::default()
                }),
                ..request_meta_with_workspace()
            },
            remote: None,
            refspec: None,
        }
    }

    pub(crate) fn push_request_explicit(remote: Option<&str>, refspec: Option<&str>) -> crate::PushRequest {
        crate::PushRequest {
            meta: request_meta_with_workspace(),
            remote: remote.map(ToOwned::to_owned),
            refspec: refspec.map(ToOwned::to_owned),
        }
    }

    pub(crate) fn read_repo_ref(repo_path: &Path, ref_name: &str) -> Option<String> {
        let repo = git2::Repository::open(repo_path).unwrap();
        repo.find_reference(ref_name)
            .ok()
            .and_then(|reference| reference.target())
            .map(|target| target.to_string())
    }

    pub(crate) fn init_bare_main(path: &Path) {
        let repo = git2::Repository::init_bare(path).unwrap();
        repo.set_head("refs/heads/main").unwrap();
    }

    impl RemoteFixture {
        pub(crate) fn new(prefix: &str) -> Self {
            let temp = TempDir::new(prefix);
            let source = temp.path().join("source");
            let remote = temp.path().join("remote.git");
            Git2Backend::new().create_repo(&source).unwrap();
            init_bare_main(&remote);
            Git2Backend::new()
                .add_remote(&source, "origin", remote.to_str().unwrap())
                .unwrap();
            Self {
                _temp: temp,
                source,
                remote,
            }
        }

        pub(crate) fn remote_url(&self) -> &str {
            self.remote.to_str().unwrap()
        }

        pub(crate) fn commit_and_push(
            &self,
            relative_path: &str,
            content: &str,
            message: &str,
            backend: &Git2Backend,
        ) -> String {
            let parent = backend
                .head(&self.source)
                .unwrap()
                .commit
                .and_then(|commit| git2::Oid::from_str(&commit).ok());
            let parents = parent.into_iter().collect::<Vec<_>>();
            let commit =
                commit_file(&self.source, relative_path, content, message, &parents).unwrap();
            backend
                .push(&self.source, "origin", "refs/heads/main:refs/heads/main")
                .unwrap();
            commit
        }
    }

    