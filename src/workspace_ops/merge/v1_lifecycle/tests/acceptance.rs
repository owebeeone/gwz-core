use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::artifact::{
    ArtifactSourceKind, LockArtifact, ManifestArtifact, ManifestMember, ResolvedMemberArtifact,
};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::acceptance::{
    V1AcceptanceMetadata, V1AcceptanceRecord, build_v1_acceptance, classify_frozen_v1_publication,
};
use crate::workspace_ops::merge::model::v1::{
    AcceptedMetadataSourceV1, AcceptedRootBaseV1, MemberAcceptanceV1, validate_v1_record,
};
use crate::workspace_ops::merge::{MergeTargetKind, OperationState, ParticipantState};

#[test]
fn builder_freezes_complete_selected_acceptance_and_publication_decision() {
    for changed in [false, true] {
        let mut record = acceptance_ready(changed);
        let built = build_v1_acceptance(
            V1AcceptanceRecord::V1(&record),
            V1AcceptanceMetadata::OperationBaseline,
        )
        .unwrap();
        assert_eq!(built.publication_required(), changed);
        assert!(matches!(
            built.accepted_workspace().member_audit["mem_a"],
            MemberAcceptanceV1::Selected { .. }
        ));
        record.accepted_workspace = Some(built.into_accepted_workspace());
        validate_v1_record(record.clone()).unwrap();
        assert_eq!(classify_frozen_v1_publication(&record).unwrap(), changed);
    }
}

#[test]
fn builder_accounts_for_unselected_present_and_intentionally_absent_members() {
    let mut record = acceptance_ready(true);
    let mut manifest =
        ManifestArtifact::from_yaml(record.baseline.manifest_yaml.as_ref().unwrap()).unwrap();
    manifest.members.extend([
        member("mem_b", "members/b", "src_b"),
        member("mem_c", "members/c", "src_c"),
    ]);
    let manifest_yaml = manifest.to_yaml().unwrap();
    record.baseline.manifest_sha256 = digest(&manifest_yaml);
    record.baseline.manifest_yaml = Some(manifest_yaml);
    let mut lock = LockArtifact::from_yaml(record.baseline.lock_yaml.as_ref().unwrap()).unwrap();
    lock.members.insert(
        "mem_b".into(),
        ResolvedMemberArtifact {
            path: "members/b".into(),
            source_id: Some("src_b".into()),
            source_kind: ArtifactSourceKind::Git,
            commit: Some("b".repeat(40)),
            branch: Some("main".into()),
            detached: Some(false),
            upstream: None,
            dirty: Some(false),
            materialized: Some(false),
        },
    );
    let lock_yaml = lock.to_yaml().unwrap();
    record.baseline.lock_sha256 = digest(&lock_yaml);
    record.baseline.lock_yaml = Some(lock_yaml);

    let built = build_v1_acceptance(
        V1AcceptanceRecord::V1(&record),
        V1AcceptanceMetadata::OperationBaseline,
    )
    .unwrap();
    assert!(matches!(
        built.accepted_workspace().member_audit["mem_b"],
        MemberAcceptanceV1::UnselectedPresent { .. }
    ));
    assert_eq!(
        built.accepted_workspace().member_audit["mem_c"],
        MemberAcceptanceV1::Absent
    );
    record.accepted_workspace = Some(built.into_accepted_workspace());
    validate_v1_record(record).unwrap();
}

#[test]
fn selected_root_metadata_uses_exact_result_bytes_and_baseline_identity_fallback() {
    let mut record = acceptance_ready(true);
    let mut root = record.participants["mem_a"].clone();
    root.path = ".".into();
    root.target_kind = MergeTargetKind::Root;
    root.before_commit = record.baseline.root_head.clone().unwrap();
    root.source_commit = "d".repeat(40);
    root.resulting_commit = Some("e".repeat(40));
    root.state = ParticipantState::FastForwarded;
    record.selected_targets.push("@root".into());
    record.participants.insert("@root".into(), root);
    record.baseline.lock_commit_sha256 = Some(record.baseline.lock_sha256.clone());
    record.baseline.manifest_commit_sha256 = Some(record.baseline.manifest_sha256.clone());

    let mut result_manifest =
        ManifestArtifact::from_yaml(record.baseline.manifest_yaml.as_ref().unwrap()).unwrap();
    result_manifest.members.clear();
    let result_manifest_yaml = result_manifest.to_yaml().unwrap();
    let result_lock_yaml = LockArtifact {
        schema: crate::artifact::LOCK_SCHEMA.into(),
        workspace_id: record.workspace_id.clone(),
        manifest_schema: crate::artifact::WORKSPACE_SCHEMA.into(),
        members: BTreeMap::new(),
    }
    .to_yaml()
    .unwrap();
    let built = build_v1_acceptance(
        V1AcceptanceRecord::V1(&record),
        V1AcceptanceMetadata::SelectedRootResult {
            commit: &"e".repeat(40),
            manifest_exact_yaml: &result_manifest_yaml,
            lock_exact_yaml: &result_lock_yaml,
        },
    )
    .unwrap();
    assert!(matches!(
        built.accepted_workspace().metadata_base.source,
        AcceptedMetadataSourceV1::SelectedRootResult { .. }
    ));
    assert_eq!(
        built.accepted_workspace().metadata_base.manifest_exact_yaml,
        result_manifest_yaml
    );
    assert!(matches!(
        built.accepted_workspace().root.base,
        AcceptedRootBaseV1::BornAttached { .. }
    ));
    record.accepted_workspace = Some(built.into_accepted_workspace());
    validate_v1_record(record.clone()).unwrap();
    assert!(classify_frozen_v1_publication(&record).unwrap());
}

#[test]
fn detached_root_is_accepted_only_for_a_frozen_no_publication_result() {
    let mut unchanged = acceptance_ready(false);
    unchanged.baseline.root_branch = None;
    let built = build_v1_acceptance(
        V1AcceptanceRecord::V1(&unchanged),
        V1AcceptanceMetadata::OperationBaseline,
    )
    .unwrap();
    assert!(!built.publication_required());
    assert!(matches!(
        built.accepted_workspace().root.base,
        AcceptedRootBaseV1::BornDetached { .. }
    ));
    unchanged.accepted_workspace = Some(built.into_accepted_workspace());
    validate_v1_record(unchanged).unwrap();

    let mut changed = acceptance_ready(true);
    changed.baseline.root_branch = None;
    assert_eq!(
        build_v1_acceptance(
            V1AcceptanceRecord::V1(&changed),
            V1AcceptanceMetadata::OperationBaseline,
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::AcceptanceInputDrift
    );
}

fn acceptance_ready(
    changed: bool,
) -> crate::workspace_ops::merge::model::v1::MergeOperationRecordV1 {
    let mut record = crate::workspace_ops::merge::model::v1::test_record();
    record.state = OperationState::Finalizing;
    let row = record.participants.get_mut("mem_a").unwrap();
    row.state = if changed {
        ParticipantState::FastForwarded
    } else {
        ParticipantState::UpToDate
    };
    row.resulting_commit = Some(if changed {
        "d".repeat(40)
    } else {
        row.before_commit.clone()
    });
    record
}

fn member(id: &str, path: &str, source_id: &str) -> ManifestMember {
    ManifestMember {
        id: id.into(),
        path: path.into(),
        source_kind: ArtifactSourceKind::Git,
        source_id: source_id.into(),
        active: true,
        desired: None,
        remotes: Vec::new(),
    }
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
