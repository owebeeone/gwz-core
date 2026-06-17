    
    
    
    
    

    use crate::artifact::read_lock;
    use crate::git::Git2Backend;
    
    use crate::operation::NullSink;

    
use super::*;

#[test]
    pub(crate) fn pull_snapshot_rewrites_lock_after_success() {
        let temp = TempDir::new("pull-snapshot");
        let backend = Git2Backend::new();
        let fixture = materialize_snapshot_fixture(temp.path(), &backend);

        handle_pull_snapshot(
            &backend,
            temp.path(),
            crate::PullSnapshotRequest {
                meta: request_meta_with_workspace(),
                snapshot_id: "snap_first".to_owned(),
            },
            "op_pull_snapshot",
            &NullSink,
        )
        .unwrap();

        assert_eq!(
            read_lock(temp.path()).unwrap().members["mem_app"].commit,
            Some(fixture.first)
        );
    }

    