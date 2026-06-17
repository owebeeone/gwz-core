    use std::fs;
    
    
    
    

    use crate::artifact::read_lock;
    use crate::git::{Git2Backend, GitBackend};
    use crate::model::ErrorCode;
    

    
use super::*;

#[test]
    pub(crate) fn pull_head_dirty_member_blocks_all_selected_members_before_mutation() {
        let temp = TempDir::new("pull-dirty");
        let backend = Git2Backend::new();
        handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();

        let good = RemoteFixture::new("pull-dirty-good");
        let good_first = good.commit_and_push("README.md", "one", "initial", &backend);
        backend
            .clone_repo(good.remote_url(), &temp.path().join("repos/good"))
            .unwrap();
        let good_second = good.commit_and_push("README.md", "two", "second", &backend);

        let dirty = RemoteFixture::new("pull-dirty-bad");
        let dirty_first = dirty.commit_and_push("README.md", "one", "initial", &backend);
        backend
            .clone_repo(dirty.remote_url(), &temp.path().join("repos/dirty"))
            .unwrap();
        fs::write(temp.path().join("repos/dirty/README.md"), "dirty").unwrap();

        write_pull_fixture(
            temp.path(),
            vec![
                ("mem_good", "repos/good", good.remote_url(), &good_first),
                ("mem_dirty", "repos/dirty", dirty.remote_url(), &dirty_first),
            ],
        );
        let lock_before = read_lock(temp.path()).unwrap();

        let err =
            handle_pull_head(&backend, temp.path(), pull_head_request(), "op_pull").unwrap_err();

        assert_eq!(err.code, ErrorCode::DirtyMember);
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
        assert_eq!(read_lock(temp.path()).unwrap(), lock_before);
    }

    