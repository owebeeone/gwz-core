    
    
    
    
    

    
    use crate::git::{Git2Backend, GitBackend};
    use crate::model::ErrorCode;
    

    
use super::*;

#[test]
    pub(crate) fn pull_head_divergence_blocks_all_selected_members_before_branch_mutation() {
        let temp = TempDir::new("pull-atomic");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        let good = RemoteFixture::new("pull-good");
        let good_first = good.commit_and_push("README.md", "one", "initial", &backend);
        backend
            .clone_repo(good.remote_url(), &temp.path().join("repos/good"))
            .unwrap();
        let good_second = good.commit_and_push("README.md", "two", "second", &backend);

        let bad = RemoteFixture::new("pull-bad");
        let bad_first = bad.commit_and_push("README.md", "one", "initial", &backend);
        backend
            .clone_repo(bad.remote_url(), &temp.path().join("repos/bad"))
            .unwrap();
        let bad_parent = git2::Oid::from_str(&bad_first).unwrap();
        let bad_local = commit_file(
            &temp.path().join("repos/bad"),
            "README.md",
            "local",
            "local",
            &[bad_parent],
        )
        .unwrap();
        bad.commit_and_push("README.md", "remote", "remote", &backend);

        write_pull_fixture(
            temp.path(),
            vec![
                ("mem_good", "repos/good", good.remote_url(), &good_first),
                ("mem_bad", "repos/bad", bad.remote_url(), &bad_local),
            ],
        );

        let err =
            handle_pull_head(&backend, temp.path(), pull_head_request(), "op_pull").unwrap_err();

        assert_eq!(err.code, ErrorCode::DivergedMember);
        assert_eq!(
            backend
                .head(&temp.path().join("repos/good"))
                .unwrap()
                .commit,
            Some(good_first)
        );
        assert_ne!(
            backend
                .head(&temp.path().join("repos/good"))
                .unwrap()
                .commit,
            Some(good_second)
        );
        assert_eq!(
            backend.head(&temp.path().join("repos/bad")).unwrap().commit,
            Some(bad_local)
        );
    }

    