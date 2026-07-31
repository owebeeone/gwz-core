use super::*;

#[test]
fn v0_candidate_publication_prefixes_are_restartable_at_every_mutation() {
    use crate::workspace_ops::merge::{
        CandidatePublicationMutation, fail_next_candidate_publication_after,
    };

    for mutation in [
        CandidatePublicationMutation::Marker,
        CandidatePublicationMutation::Lock,
        CandidatePublicationMutation::Boundary,
        CandidatePublicationMutation::Staging,
    ] {
        let temp = TempDir::new(&format!("v0-publication-prefix-{mutation:?}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture = init_one_member_workspace(
            temp.path(),
            &backend,
            &format!("v0-publication-prefix-{mutation:?}"),
        );
        feature_commit(
            &backend,
            &temp.path().join("remote"),
            "README.md",
            "source\n",
        );
        let staged_before = backend.status(temp.path()).unwrap().staged;

        fail_next_candidate_publication_after(mutation);
        let error = handle_merge(
            &backend,
            temp.path(),
            request(false),
            format!("op_v0_publication_prefix_{mutation:?}"),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);

        let record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
        assert_eq!(record.state, OperationState::Finalizing);
        let publication = record.publication.as_ref().unwrap();
        assert_eq!(publication.step, PublicationStep::PublishingCandidate);
        let candidate = publication.candidate.as_ref().unwrap();

        assert_eq!(
            fs::read_to_string(crate::artifact::marker_path(
                temp.path(),
                &candidate.marker_id,
            ))
            .unwrap(),
            candidate.marker_yaml
        );
        assert_eq!(
            fs::read_to_string(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
            if mutation == CandidatePublicationMutation::Marker {
                candidate.baseline_lock_yaml.as_str()
            } else {
                candidate.lock_yaml.as_str()
            }
        );
        assert_eq!(
            fs::read_to_string(temp.path().join(".git/info/exclude")).unwrap_or_default(),
            if matches!(
                mutation,
                CandidatePublicationMutation::Marker | CandidatePublicationMutation::Lock
            ) {
                candidate.baseline_boundary_text.as_str()
            } else {
                candidate.boundary_text.as_str()
            }
        );

        let status = backend.status(temp.path()).unwrap();
        if mutation == CandidatePublicationMutation::Staging {
            assert_eq!(status.unstaged + status.untracked, 0, "{mutation:?}");
        } else if mutation == CandidatePublicationMutation::Boundary {
            assert!(status.staged >= staged_before, "{mutation:?}");
            assert!(status.unstaged + status.untracked >= 2, "{mutation:?}");
        }

        let completed = handle_merge(
            &backend,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
            format!("op_v0_publication_resume_{mutation:?}"),
        )
        .unwrap();
        assert_eq!(completed.state, crate::MergeOperationState::Completed);
        assert!(!completed.open);
        assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
    }
}
