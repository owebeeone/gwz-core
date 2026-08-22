use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::super::authority::{
    CandidatePayload, EvidencePayload, PreservationCursorPosition, PreservationCursorPrefix,
    PreservationPayload, VerifiedPreservationCursorPrefix,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::{PreservationTransition, V1Transition, prepare};
use crate::artifact::{
    ArtifactSourceKind, CreatedByArtifact, LOCK_SCHEMA, LockArtifact, MARKER_SCHEMA,
    ManifestArtifact, MarkerArtifact, MarkerRootArtifact, ResolvedMemberArtifact, WORKSPACE_SCHEMA,
};
use crate::workspace_ops::merge::model::v1::{
    AcceptedAttachedCheckoutV1, AcceptedIntegrationRefV1, AcceptedLockMemberV1, AcceptedLockV1,
    AcceptedMetadataBaseV1, AcceptedMetadataSourceV1, AcceptedRootBaseV1, AcceptedWorkspaceV1,
    GitObjectAlgorithmV1, GitObjectIdV1, MemberAcceptanceV1, MergeOperationRecordV1,
    PendingPreservationActionV1, PreservationOwnerV1, PreservationRefResetPhaseV1,
    PreservationStashPhaseV1, RootArtifactHashesV1, RootPublicationInputV1,
};
use crate::workspace_ops::merge::{
    MergeTargetKind, OperationState, ParticipantState, PendingMergeAction, PendingMergeActionKind,
    PendingMergeExpectedResult, PreservationEvidence, PublicationCandidate,
    PublicationCandidateHash, PublicationProgress, PublicationStep,
};
use crate::workspace_ops::tests::TempDir;

pub(in crate::workspace_ops::merge::v1_lifecycle) fn align_baseline_lock(
    record: &mut MergeOperationRecordV1,
) {
    let lock = accepted_lock_yaml(&record.workspace_id, &oid('a'));
    record.baseline.lock_sha256 = digest(&lock);
    record.baseline.lock_yaml = Some(lock);
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn accepted_workspace(
    current: &StoredV1Record,
) -> AcceptedWorkspaceV1 {
    let record = current.record();
    let selected_root = record
        .selected_targets
        .iter()
        .any(|target| target == "@root")
        .then(|| record.participants.get("@root"))
        .flatten();
    let manifest =
        ManifestArtifact::from_yaml(record.baseline.manifest_yaml.as_deref().unwrap()).unwrap();
    let mut lock = LockArtifact::from_yaml(record.baseline.lock_yaml.as_deref().unwrap()).unwrap();
    let mut member_audit = BTreeMap::new();
    for member in manifest.members.iter().filter(|member| member.active) {
        let selected = record
            .selected_targets
            .iter()
            .any(|target| target == &member.id);
        let participant = record.participants.get(&member.id);
        let mut row = lock.members.remove(&member.id).unwrap_or_default();
        row.path = member.path.clone();
        row.source_id = Some(member.source_id.clone());
        row.source_kind = member.source_kind;
        if selected {
            let participant = participant.unwrap();
            let result = participant
                .resulting_commit
                .as_deref()
                .unwrap_or(&participant.before_commit)
                .to_owned();
            row.commit = Some(result.clone());
            row.branch = Some(participant.target_branch.clone());
            row.detached = Some(false);
            row.dirty = Some(false);
            row.materialized = Some(true);
            let accepted_row = accepted_lock_member(&row);
            member_audit.insert(
                member.id.clone(),
                MemberAcceptanceV1::Selected {
                    integration: AcceptedIntegrationRefV1 {
                        branch: participant.target_branch.clone(),
                        before_commit: participant.before_commit.clone(),
                        resulting_commit: result.clone(),
                    },
                    final_checkout: AcceptedAttachedCheckoutV1 {
                        branch: participant.target_branch.clone(),
                        commit: result,
                    },
                    lock_member: accepted_row,
                },
            );
        } else {
            member_audit.insert(
                member.id.clone(),
                MemberAcceptanceV1::UnselectedPresent {
                    lock_member: accepted_lock_member(&row),
                },
            );
        }
        lock.members.insert(member.id.clone(), row);
    }
    let accepted_lock_yaml = lock.to_yaml().unwrap();
    AcceptedWorkspaceV1 {
        operation_baseline_lock_sha256: record.baseline.lock_sha256.clone(),
        metadata_base: AcceptedMetadataBaseV1 {
            source: selected_root.map_or(AcceptedMetadataSourceV1::OperationBaseline, |root| {
                AcceptedMetadataSourceV1::SelectedRootResult {
                    commit: root.resulting_commit.clone().unwrap(),
                }
            }),
            manifest_exact_yaml: record.baseline.manifest_yaml.clone().unwrap(),
            manifest_sha256: record.baseline.manifest_sha256.clone(),
            lock_exact_yaml: record.baseline.lock_yaml.clone().unwrap(),
            lock_sha256: record.baseline.lock_sha256.clone(),
        },
        lock: AcceptedLockV1 {
            sha256: digest(&accepted_lock_yaml),
            exact_yaml: accepted_lock_yaml,
        },
        member_audit,
        root: RootPublicationInputV1 {
            base: AcceptedRootBaseV1::BornAttached {
                commit: selected_root.map_or_else(
                    || record.baseline.root_head.clone().unwrap(),
                    |root| root.resulting_commit.clone().unwrap(),
                ),
                symbolic_branch: selected_root.map_or_else(
                    || record.baseline.root_branch.clone().unwrap(),
                    |root| root.target_branch.clone(),
                ),
            },
            publication_branch: Some(
                selected_root.map_or_else(|| "main".into(), |root| root.target_branch.clone()),
            ),
            baseline_artifact_hashes: RootArtifactHashesV1 {
                lock_worktree_sha256: record.baseline.lock_sha256.clone(),
                manifest_worktree_sha256: record.baseline.manifest_sha256.clone(),
                lock_commit_sha256: record.baseline.lock_commit_sha256.clone(),
                manifest_commit_sha256: record.baseline.manifest_commit_sha256.clone(),
            },
        },
    }
}

fn accepted_lock_member(row: &ResolvedMemberArtifact) -> AcceptedLockMemberV1 {
    AcceptedLockMemberV1 {
        path: row.path.clone(),
        source_id: row.source_id.clone().unwrap(),
        source_kind: row.source_kind,
        commit: row.commit.clone(),
        branch: row.branch.clone(),
        detached: row.detached,
        upstream: row.upstream.clone(),
        dirty: row.dirty,
        materialized: row.materialized,
        extensions: BTreeMap::new(),
    }
}

fn accepted_lock_yaml(workspace_id: &str, commit: &str) -> String {
    LockArtifact {
        schema: LOCK_SCHEMA.into(),
        workspace_id: workspace_id.into(),
        manifest_schema: WORKSPACE_SCHEMA.into(),
        members: BTreeMap::from([(
            "mem_a".into(),
            ResolvedMemberArtifact {
                path: "members/a".into(),
                source_id: Some("src_a".into()),
                source_kind: ArtifactSourceKind::Git,
                commit: Some(commit.into()),
                branch: Some("main".into()),
                detached: Some(false),
                upstream: None,
                dirty: Some(false),
                materialized: Some(true),
            },
        )]),
    }
    .to_yaml()
    .unwrap()
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn candidate_payload(
    current: &StoredV1Record,
) -> CandidatePayload {
    let record = current.record();
    let accepted = accepted_workspace(current);
    let mut marker_record = record.clone();
    marker_record.accepted_workspace = Some(accepted.clone());
    let marker_id = "01987b0c-2f75-7c4a-9a32-8fd22f7d7c91";
    let root_before = match &accepted.root.base {
        AcceptedRootBaseV1::BornAttached { commit, .. }
        | AcceptedRootBaseV1::BornDetached { commit } => Some(commit.clone()),
        AcceptedRootBaseV1::UnbornAttached { .. } => None,
    };
    let mut committed_targets = record
        .selected_targets
        .iter()
        .filter(|target| {
            record.participants[*target]
                .resulting_commit
                .as_deref()
                .is_some_and(|result| result != record.participants[*target].before_commit)
        })
        .cloned()
        .collect::<Vec<_>>();
    if !committed_targets.iter().any(|target| target == "@root") {
        committed_targets.push("@root".into());
    }
    let marker = MarkerArtifact {
        schema: MARKER_SCHEMA.into(),
        gwz_commit_id: marker_id.into(),
        workspace_id: record.workspace_id.clone(),
        origin_url_hash: None,
        created_at: record.created_at.clone(),
        created_by: CreatedByArtifact {
            actor_id: "agent_test".into(),
        },
        root: MarkerRootArtifact {
            path: ".".into(),
            before_commit: root_before,
            branch: Some("main".into()),
        },
        selected_targets: record.selected_targets.clone(),
        committed_targets,
        members: LockArtifact::from_yaml(&accepted.lock.exact_yaml)
            .unwrap()
            .members,
        merge: Some(
            crate::workspace_ops::merge::marker::marker_merge_from_v1_acceptance(&marker_record)
                .unwrap(),
        ),
    }
    .to_yaml()
    .unwrap();
    let baseline_boundary = String::new();
    let boundary = String::new();
    CandidatePayload {
        candidate: PublicationCandidate {
            marker_id: marker_id.into(),
            root_branch: "main".into(),
            actor_id: "agent_test".into(),
            baseline_lock_yaml: accepted.metadata_base.lock_exact_yaml,
            lock_yaml: accepted.lock.exact_yaml.clone(),
            marker_sha256: digest(&marker),
            marker_yaml: marker,
            baseline_boundary_sha256: digest(&baseline_boundary),
            baseline_boundary_text: baseline_boundary,
            boundary_sha256: digest(&boundary),
            boundary_text: boundary,
            extensions: BTreeMap::new(),
        },
        marker_path: format!("gwz.conf/markers/{marker_id}.yaml"),
        lock_sha256: digest(&accepted.lock.exact_yaml),
    }
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn evidence_payload(
    current: &StoredV1Record,
) -> EvidencePayload {
    let publication = current.record().publication.as_ref().unwrap();
    let candidate = publication.candidate.as_ref().unwrap();
    let mut candidate_hashes = vec![
        PublicationCandidateHash {
            path: crate::artifact::LOCK_PATH.into(),
            sha256: digest(&candidate.lock_yaml),
        },
        PublicationCandidateHash {
            path: publication.candidate_marker_path.clone().unwrap(),
            sha256: digest(&candidate.marker_yaml),
        },
    ];
    candidate_hashes.sort_by(|left, right| left.path.cmp(&right.path));
    EvidencePayload {
        composition_commit: oid('e'),
        composition_tree: oid('f'),
        root_merge_commit: None,
        candidate_hashes,
    }
}

pub(super) fn preserving_record() -> MergeOperationRecordV1 {
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::Preserving;
    model.preservation_publication_handoff =
        Some(crate::workspace_ops::merge::model::v1::PreservationPublicationHandoffV1::NoCandidate);
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::UpToDate;
    row.resulting_commit = Some(row.before_commit.clone());
    model
}

pub(super) fn preservation_owner() -> PreservationOwnerV1 {
    PreservationOwnerV1::Participant {
        member_id: "mem_a".into(),
    }
}

pub(super) fn backup_action() -> PendingPreservationActionV1 {
    PendingPreservationActionV1::BackupRef {
        owner: preservation_owner(),
        name: "refs/gwz/merge/merge_1/mem_a/head".into(),
        target_commit: oid('a'),
    }
}

pub(super) fn stash_action(phase: PreservationStashPhaseV1) -> PendingPreservationActionV1 {
    let ids = !matches!(
        phase,
        PreservationStashPhaseV1::NormalizeParent
            | PreservationStashPhaseV1::NormalizeMarker
            | PreservationStashPhaseV1::NormalizeLock
            | PreservationStashPhaseV1::NormalizeIndex
            | PreservationStashPhaseV1::CreateStash
    );
    PendingPreservationActionV1::Stash {
        owner: preservation_owner(),
        phase,
        stash_id: ids.then(|| "stash_merge_1".into()),
        stash_object_id: ids.then(|| GitObjectIdV1 {
            algorithm: GitObjectAlgorithmV1::Sha1,
            digest_hex: oid('b'),
        }),
        message: "gwz:stash_merge_1: merge preservation".into(),
        head_commit: oid('a'),
        preimage_sha256: "1".repeat(64),
        root_publication_handoff: None,
    }
}

pub(super) fn reset_action(phase: PreservationRefResetPhaseV1) -> PendingPreservationActionV1 {
    PendingPreservationActionV1::ResetAttachedRef {
        owner: preservation_owner(),
        branch: "main".into(),
        expected_commit: oid('a'),
        restore_commit: oid('a'),
        phase,
        root_publication_handoff: None,
    }
}

pub(super) fn preservation_evidence(with_stash: bool) -> PreservationEvidence {
    PreservationEvidence {
        backup_ref: Some("refs/gwz/merge/merge_1/mem_a/head".into()),
        backup_commit: Some(oid('a')),
        stash_id: with_stash.then(|| "stash_merge_1".into()),
        stash_object_id: with_stash.then(|| oid('b')),
        noop_commit: None,
        reset_commit: None,
    }
}

/// The marker-bearing row a reset retirement writes per
/// `GwzM5-8DurableCursorAmendment.md` §3.1 edge 1. This fixture owner carries a
/// stash pair, so its artifact pass is already decode-evidenced (§3.2) and no
/// `noop_commit` backfill fires — the retirement adds `reset_commit` alone,
/// yielding the §2.2-legal `B+S+R`.
pub(super) fn reset_completion_evidence() -> PreservationEvidence {
    PreservationEvidence {
        reset_commit: Some(oid('a')),
        ..preservation_evidence(true)
    }
}

pub(super) fn preservation_payload(
    observed_position: PreservationCursorPosition,
    pending: Option<PendingPreservationActionV1>,
    evidence: Option<PreservationEvidence>,
) -> PreservationPayload {
    PreservationPayload {
        owner: preservation_owner(),
        observed_position,
        pending,
        evidence,
        publication_prefix: None,
    }
}

pub(super) fn preservation_prefix(
    current: &StoredV1Record,
    position: PreservationCursorPosition,
) -> VerifiedPreservationCursorPrefix {
    VerifiedPreservationCursorPrefix::for_test(
        current,
        "mem_a",
        "preservation_cursor",
        "prefix_verified",
        PreservationCursorPrefix {
            owner: preservation_owner(),
            position,
        },
    )
    .unwrap()
}

pub(super) fn apply_preservation(
    root: &TempDir,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    transition: PreservationTransition,
) -> StoredV1Record {
    let rewrite = prepare(
        lease,
        current,
        V1Transition::Preservation(Box::new(transition)),
    )
    .unwrap();
    StoredV1Record::for_test(&root.path, rewrite.next().clone()).unwrap()
}

pub(super) fn evidence_rollback_record(root: &TempDir) -> MergeOperationRecordV1 {
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::RollingBack;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::FastForwarded;
    row.resulting_commit = Some(oid('d'));
    align_baseline_lock(&mut model);
    let seed = StoredV1Record::for_test(&root.path, model.clone()).unwrap();
    let accepted = accepted_workspace(&seed);
    let payload = candidate_payload(&seed);
    let mut hashes = vec![
        PublicationCandidateHash {
            path: crate::artifact::LOCK_PATH.into(),
            sha256: digest(&payload.candidate.lock_yaml),
        },
        PublicationCandidateHash {
            path: payload.marker_path.clone(),
            sha256: digest(&payload.candidate.marker_yaml),
        },
    ];
    hashes.sort_by(|left, right| left.path.cmp(&right.path));
    model.accepted_workspace = Some(accepted);
    model.publication = Some(PublicationProgress {
        step: PublicationStep::CommittingEvidence,
        candidate_lock_sha256: Some(payload.lock_sha256),
        candidate_marker_path: Some(payload.marker_path),
        root_merge_commit: None,
        composition_commit: Some(oid('e')),
        composition_tree: Some(oid('f')),
        candidate_hashes: hashes,
        candidate: Some(payload.candidate),
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    });
    model
}

pub(super) fn selected_root_rollback_record() -> MergeOperationRecordV1 {
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::RollingBack;
    let mut root = model.participants["mem_a"].clone();
    root.path = ".".into();
    root.target_kind = MergeTargetKind::Root;
    root.before_commit = model.baseline.root_head.clone().unwrap();
    root.state = ParticipantState::Aborted;
    model.selected_targets = vec!["@root".into()];
    model.participants.clear();
    model.participants.insert("@root".into(), root);
    model.baseline.lock_commit_sha256 = Some("4".repeat(64));
    model.baseline.manifest_commit_sha256 = Some("5".repeat(64));
    model
}

pub(super) fn up_to_date_action() -> PendingMergeAction {
    let row = &crate::workspace_ops::merge::model::v1::test_record().participants["mem_a"];
    PendingMergeAction {
        kind: PendingMergeActionKind::VerifyUpToDate,
        target_branch: row.target_branch.clone(),
        before_commit: row.before_commit.clone(),
        source_commit: row.source_commit.clone(),
        commit_message: row.commit_message.clone(),
        expected_result: Some(PendingMergeExpectedResult::Unchanged),
        commit_spec: None,
        extensions: BTreeMap::new(),
    }
}

pub(super) fn oid(value: char) -> String {
    value.to_string().repeat(40)
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
