    
    
    
    
    

    
    use crate::git::{Git2Backend, GitBackend};
    use crate::model::ErrorCode;
    use crate::operation::NullSink;

    
use super::*;

#[test]
    pub(crate) fn missing_snapshot_or_tag_fails_before_mutation() {
        let temp = TempDir::new("missing-target");
        let backend = Git2Backend::new();
        let fixture = materialize_snapshot_fixture(temp.path(), &backend);

        assert_eq!(
            handle_materialize(
                &backend,
                temp.path(),
                materialize_named_request(crate::MaterializeTargetKind::Snapshot, "missing"),
                "op_materialize",
                &NullSink,
            )
            .unwrap_err()
            .code,
            ErrorCode::SnapshotNotFound
        );
        assert_eq!(
            handle_materialize(
                &backend,
                temp.path(),
                materialize_named_request(crate::MaterializeTargetKind::Tag, "missing"),
                "op_materialize",
                &NullSink,
            )
            .unwrap_err()
            .code,
            ErrorCode::TagNotFound
        );
        assert_eq!(
            backend.head(&temp.path().join("repos/app")).unwrap().commit,
            Some(fixture.second)
        );
    }

    