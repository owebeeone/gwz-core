use std::collections::BTreeMap;

use serde::Serialize;
use serde_yaml::Value;

use crate::git::{GitBackend, GitRepositoryState};
use crate::workspace_ops::merge::{
    MergeExecutionMode, MergeOperationRecord, MergeTargetKind, OperationState, ParticipantState,
    PublicationStep,
};

use super::*;

const REGISTRY: &str = include_str!("../../../../dev-docs/GwzM5-8I2CompatibilityPredicates.json");

fn value<T: Serialize>(input: T) -> Value {
    serde_yaml::to_value(input).unwrap()
}

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Mapping(
        fields
            .into_iter()
            .map(|(key, value)| (Value::String(key.to_owned()), value))
            .collect(),
    )
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value
        .as_mapping()
        .and_then(|mapping| mapping.get(Value::String(key.to_owned())))
        .unwrap_or_else(|| panic!("registry value is missing {key:?}"))
}

fn text_field<'a>(value: &'a Value, key: &str) -> &'a str {
    field(value, key)
        .as_str()
        .unwrap_or_else(|| panic!("registry field {key:?} is not text"))
}

fn publication_step(step: PublicationStep) -> &'static str {
    match step {
        PublicationStep::NotStarted => "not_started",
        PublicationStep::ValidatingResults => "validating_results",
        PublicationStep::PreparingCandidate => "preparing_candidate",
        PublicationStep::CommittingEvidence => "committing_evidence",
        PublicationStep::PublishingCandidate => "publishing_candidate",
        PublicationStep::VerifyingPublication => "verifying_publication",
        PublicationStep::Complete => "complete",
    }
}

fn normalize_descriptor<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> Value {
    assert_eq!(record.schema, "gwz.merge-operation/v0");
    assert_eq!(record.record_schema_version, 0);
    assert_eq!(record.mode, MergeExecutionMode::Normal);
    assert_eq!(record.state, OperationState::Finalizing);
    assert!(record.operation_drift.is_empty());
    assert_eq!(record.selected_targets.len(), 1);
    let selected = &record.selected_targets[0];
    assert_ne!(selected, "@root");
    assert_eq!(record.participants.len(), 1);
    let participant = &record.participants[selected];
    assert_eq!(participant.target_kind, MergeTargetKind::Member);
    assert!(participant.pending_action.is_none());
    assert!(participant.conflict_paths.is_empty());
    assert!(participant.conflict_snapshot.is_empty());
    assert!(participant.error.is_none());
    assert!(participant.preservation.is_empty());
    assert!(participant.drift.is_empty());

    let result = participant.resulting_commit.as_deref().unwrap();
    let (participant_state, result_relation) = match participant.state {
        ParticipantState::FastForwarded => {
            assert_ne!(result, participant.before_commit);
            assert_eq!(result, participant.source_commit);
            ("fast_forwarded", "changed_exact")
        }
        ParticipantState::UpToDate => {
            assert_eq!(result, participant.before_commit);
            ("up_to_date", "equals_before")
        }
        state => panic!("unlisted participant state in I2 fixture: {state:?}"),
    };
    let member_path = root.join(&participant.path);
    let member_head = backend.head(&member_path).unwrap();
    assert!(!member_head.is_detached);
    assert_eq!(
        member_head.branch.as_deref(),
        Some(participant.target_branch.as_str())
    );
    assert_eq!(member_head.commit.as_deref(), Some(result));
    assert_eq!(
        backend
            .read_ref(
                &member_path,
                &format!("refs/heads/{}", participant.target_branch),
            )
            .unwrap()
            .as_deref(),
        Some(result)
    );
    assert_eq!(
        backend.repository_state(&member_path).unwrap(),
        GitRepositoryState::Clean
    );
    assert!(!backend.status(&member_path).unwrap().is_dirty);

    let lock_yaml = record.baseline.lock_yaml.as_deref().unwrap();
    let manifest_yaml = record.baseline.manifest_yaml.as_deref().unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(lock_yaml.as_bytes())),
        record.baseline.lock_sha256
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(manifest_yaml.as_bytes())),
        record.baseline.manifest_sha256
    );
    assert!(record.baseline.lock_commit_sha256.is_none());
    assert!(record.baseline.manifest_commit_sha256.is_none());
    assert!(record.baseline.root_head.is_none());
    assert!(record.baseline.root_branch.is_some());
    let baseline_manifest = crate::artifact::ManifestArtifact::from_yaml(manifest_yaml).unwrap();
    let baseline_lock = crate::artifact::LockArtifact::from_yaml(lock_yaml).unwrap();
    assert_eq!(baseline_manifest.workspace.id, record.workspace_id);
    assert_eq!(baseline_lock.workspace_id, record.workspace_id);
    assert_eq!(baseline_manifest.members.len(), 1);
    assert_eq!(baseline_lock.members.len(), 1);
    let manifest_member = &baseline_manifest.members[0];
    let baseline_member = &baseline_lock.members[selected];
    assert_eq!(manifest_member.id, *selected);
    assert_eq!(manifest_member.path, participant.path);
    assert!(manifest_member.active);
    assert_eq!(baseline_member.path, participant.path);
    assert_eq!(
        baseline_member.source_id.as_ref(),
        Some(&manifest_member.source_id)
    );
    assert_eq!(baseline_member.source_kind, manifest_member.source_kind);
    assert_eq!(
        baseline_member.commit.as_deref(),
        Some(participant.before_commit.as_str())
    );

    let root_observation =
        crate::workspace_ops::merge::normalized_i2_root_observation(backend, root, record).unwrap();
    if root_observation != "prefix_boundary" {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(fs::read(root.join(crate::artifact::LOCK_PATH)).unwrap())
            ),
            record.baseline.lock_sha256
        );
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(fs::read(root.join(crate::workspace::WORKSPACE_MANIFEST)).unwrap())
            ),
            record.baseline.manifest_sha256
        );
    }

    let (publication_presence, step, candidate, composition, hashes) =
        if let Some(publication) = record.publication.as_ref() {
            assert!(publication.root_merge_commit.is_none());
            assert!(!publication.evidence_rolled_back);
            assert!(publication.root_preservation.is_empty());
            assert!(publication.preservation_prefix.is_none());
            let candidate = if let Some(candidate) = publication.candidate.as_ref() {
                crate::workspace_ops::merge::validate_candidate_for_i2_fixture(record).unwrap();
                assert_eq!(
                    Some(candidate.baseline_lock_yaml.as_str()),
                    record.baseline.lock_yaml.as_deref()
                );
                let candidate_lock =
                    crate::artifact::LockArtifact::from_yaml(&candidate.lock_yaml).unwrap();
                assert_eq!(candidate_lock.members.len(), 1);
                for target_id in &record.selected_targets {
                    let participant = &record.participants[target_id];
                    let baseline_member = &baseline_lock.members[target_id];
                    let candidate_member = &candidate_lock.members[target_id];
                    assert_eq!(baseline_member.path, participant.path);
                    assert_eq!(
                        baseline_member.commit.as_deref(),
                        Some(participant.before_commit.as_str())
                    );
                    assert_eq!(candidate_member.path, participant.path);
                    assert_eq!(candidate_member.source_id, baseline_member.source_id);
                    assert_eq!(candidate_member.source_kind, baseline_member.source_kind);
                    assert_eq!(candidate_member.upstream, baseline_member.upstream);
                    assert_eq!(
                        candidate_member.commit.as_deref(),
                        participant.resulting_commit.as_deref()
                    );
                    assert_eq!(
                        candidate_member.branch.as_deref(),
                        Some(participant.target_branch.as_str())
                    );
                    assert_eq!(candidate_member.detached, Some(false));
                    assert_eq!(candidate_member.dirty, Some(false));
                    assert_eq!(candidate_member.materialized, Some(true));
                }
                "complete_valid"
            } else {
                assert!(publication.candidate_lock_sha256.is_none());
                assert!(publication.candidate_marker_path.is_none());
                "absent"
            };
            let (composition, hashes) = match (
                publication.composition_commit.as_ref(),
                publication.composition_tree.as_ref(),
                publication.candidate_hashes.is_empty(),
            ) {
                (None, None, true) => ("absent", "empty"),
                (Some(_), Some(_), false) => ("complete_valid", "canonical_valid"),
                shape => panic!("partial composition fixture shape: {shape:?}"),
            };
            (
                "present",
                publication_step(publication.step),
                candidate,
                composition,
                hashes,
            )
        } else {
            ("absent", "absent", "absent", "absent", "empty")
        };

    object([
        ("location", value("open")),
        ("mode", value("normal")),
        (
            "operation",
            object([
                ("state", value("finalizing")),
                ("drift", value(Vec::<String>::new())),
            ]),
        ),
        (
            "selection",
            object([
                ("ordered_ids", value(["p0"])),
                ("root_selected", value(false)),
            ]),
        ),
        (
            "participants",
            value([object([
                ("id", value("p0")),
                ("path", value("selected_path")),
                ("target_kind", value("member")),
                ("target_branch", value("attached_live_branch")),
                ("state", value(participant_state)),
                ("result", value(result_relation)),
                (
                    "pending",
                    object([
                        ("kind", value("absent")),
                        ("expected", value("absent")),
                        ("commit_spec", value("absent")),
                    ]),
                ),
                ("conflict", value("absent")),
                ("error", value("absent")),
                ("preservation", value("absent")),
                ("drift", value(Vec::<String>::new())),
            ])]),
        ),
        (
            "baseline",
            object([
                ("lock_yaml", value("present_digest_valid")),
                ("manifest_yaml", value("present_digest_valid")),
                ("lock_commit_hash", value("absent")),
                ("manifest_commit_hash", value("absent")),
                ("root_checkout", value("unborn_attached")),
                ("root_commit_hash", value("absent")),
            ]),
        ),
        (
            "publication",
            object([
                ("presence", value(publication_presence)),
                ("step", value(step)),
                ("candidate", value(candidate)),
                ("composition", value(composition)),
                ("hashes", value(hashes)),
                ("root_merge", value("absent")),
                ("evidence_rolled_back", value(false)),
                ("root_preservation", value("absent")),
                ("preservation_prefix", value("absent")),
            ]),
        ),
        (
            "observation",
            object([
                (
                    "participants",
                    value([object([
                        ("id", value("p0")),
                        ("action", value("none")),
                        ("head", value("equals_result")),
                        ("target_ref", value("equals_result")),
                        ("index", value("clean")),
                        ("worktree", value("clean")),
                    ])]),
                ),
                ("root", value(root_observation)),
                ("preservation", value("none")),
                ("rollback", value("none")),
            ]),
        ),
    ])
}

fn write_json_string(value: &str, output: &mut String) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value < '\u{20}' => output.push_str(&format!("\\u{:04x}", value as u32)),
            value => output.push(value),
        }
    }
    output.push('"');
}

fn canonical_json(value: &Value, output: &mut String) {
    match value {
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => write_json_string(value, output),
        Value::Sequence(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                canonical_json(value, output);
            }
            output.push(']');
        }
        Value::Mapping(mapping) => {
            let sorted = mapping
                .iter()
                .map(|(key, value)| (key.as_str().unwrap(), value))
                .collect::<BTreeMap<_, _>>();
            output.push('{');
            for (index, (key, value)) in sorted.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_json_string(key, output);
                output.push(':');
                canonical_json(value, output);
            }
            output.push('}');
        }
        other => panic!("unsupported canonical JSON value: {other:?}"),
    }
}

#[test]
fn i2_whitelist_rejects_an_extra_metadata_base_member() {
    let temp = TempDir::new("i2-extra-metadata-member");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "i2-extra-metadata-member");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let store = FaultingMergeStore::new(FinalizationFault::AfterEnteringFinalizing);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_i2_extra_metadata_member",
    )
    .unwrap_err();
    let mut record = store.discover_open(temp.path()).unwrap().unwrap();
    let mut manifest = crate::artifact::ManifestArtifact::from_yaml(
        record.baseline.manifest_yaml.as_deref().unwrap(),
    )
    .unwrap();
    let mut manifest_member = manifest.members[0].clone();
    manifest_member.id = "mem_extra".to_owned();
    manifest_member.path = "extra".to_owned();
    manifest_member.source_id = "src_extra".to_owned();
    manifest.members.push(manifest_member);
    let manifest_yaml = manifest.to_yaml().unwrap();
    let mut lock =
        crate::artifact::LockArtifact::from_yaml(record.baseline.lock_yaml.as_deref().unwrap())
            .unwrap();
    let mut lock_member = lock.members.values().next().unwrap().clone();
    lock_member.path = "extra".to_owned();
    lock_member.source_id = Some("src_extra".to_owned());
    lock.members.insert("mem_extra".to_owned(), lock_member);
    let lock_yaml = lock.to_yaml().unwrap();
    record.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest_yaml.as_bytes()));
    record.baseline.lock_sha256 = format!("{:x}", Sha256::digest(lock_yaml.as_bytes()));
    record.baseline.manifest_yaml = Some(manifest_yaml.clone());
    record.baseline.lock_yaml = Some(lock_yaml.clone());
    fs::write(
        temp.path().join(crate::workspace::WORKSPACE_MANIFEST),
        manifest_yaml,
    )
    .unwrap();
    fs::write(temp.path().join(crate::artifact::LOCK_PATH), lock_yaml).unwrap();

    let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        normalize_descriptor(&backend, temp.path(), &record)
    }));

    assert!(rejected.is_err());
}

pub(super) fn assert_i2_compatibility_fixture<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    case_id: &str,
    subcase: &str,
    rule_id: &str,
) {
    let registry: Value = serde_yaml::from_str(REGISTRY).unwrap();
    let corpus = field(&registry, "fixture_corpus").as_sequence().unwrap();
    let binding = corpus
        .iter()
        .find(|row| text_field(row, "case_id") == case_id)
        .unwrap_or_else(|| panic!("fixture corpus is missing {case_id:?}"));
    assert_eq!(text_field(binding, "subcase"), subcase);
    assert_eq!(text_field(binding, "rule"), rule_id);

    let rules = field(&registry, "migration_whitelist")
        .as_sequence()
        .unwrap();
    let rule = rules
        .iter()
        .find(|row| text_field(row, "id") == rule_id)
        .unwrap_or_else(|| panic!("migration whitelist is missing {rule_id:?}"));
    let descriptor = normalize_descriptor(backend, root, record);
    assert_eq!(&descriptor, field(rule, "descriptor"), "{case_id}");
    let mut canonical = String::new();
    canonical_json(&descriptor, &mut canonical);
    assert_eq!(
        format!("{:x}", Sha256::digest(canonical.as_bytes())),
        text_field(rule, "descriptor_sha256"),
        "{case_id}"
    );
    assert_eq!(
        crate::workspace_ops::merge::finalization_next_action_for_i2(record).unwrap(),
        text_field(field(rule, "classification"), "next_action"),
        "{case_id}"
    );
}

pub(super) fn assert_i2_valid_unlisted_fixture(
    record: &MergeOperationRecord,
    case_id: &str,
    subcase: &str,
) {
    let registry: Value = serde_yaml::from_str(REGISTRY).unwrap();
    let corpus = field(&registry, "valid_unlisted_corpus")
        .as_sequence()
        .unwrap();
    let binding = corpus
        .iter()
        .find(|row| text_field(row, "case_id") == case_id)
        .unwrap_or_else(|| panic!("valid-unlisted corpus is missing {case_id:?}"));
    assert_eq!(text_field(binding, "subcase"), subcase);
    assert_eq!(
        text_field(binding, "operation_state"),
        operation_state(record.state)
    );

    let rules = field(&registry, "migration_whitelist")
        .as_sequence()
        .unwrap();
    assert!(rules.iter().all(|rule| {
        text_field(field(rule, "descriptor"), "location") == "open"
            && text_field(field(field(rule, "descriptor"), "operation"), "state") == "finalizing"
    }));
    assert_ne!(record.state, OperationState::Finalizing);
}

fn operation_state(state: OperationState) -> &'static str {
    match state {
        OperationState::Completed => "completed",
        OperationState::Aborted => "aborted",
        OperationState::RecoveryRequired => "recovery_required",
        OperationState::Preserving => "preserving",
        OperationState::RollingBack => "rolling_back",
        other => panic!("state {other:?} is not in the I2 valid-unlisted corpus"),
    }
}

#[test]
fn i2_runtime_binding_inventories_equal_the_registry() {
    let registry: Value = serde_yaml::from_str(REGISTRY).unwrap();
    let migration = field(&registry, "fixture_corpus")
        .as_sequence()
        .unwrap()
        .iter()
        .map(|row| {
            (
                text_field(row, "case_id"),
                text_field(row, "subcase"),
                text_field(row, "rule"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        migration,
        vec![
            (
                "changed/finalizing-before-publication-record",
                "finalizing_before_publication_record",
                "finalizing-before-publication-record"
            ),
            (
                "changed/validating-before-candidate",
                "validating_before_candidate",
                "validating-before-candidate"
            ),
            (
                "changed/candidate-persisted",
                "candidate_persisted_before_evidence",
                "candidate-persisted-before-evidence"
            ),
            (
                "changed/evidence-unrecorded",
                "evidence_created_before_recording",
                "evidence-created-before-recording"
            ),
            (
                "changed/evidence-recorded",
                "evidence_recorded_before_publication",
                "evidence-recorded-before-publication"
            ),
            (
                "changed/prefix-boundary",
                "candidate_published_before_recording",
                "candidate-published-before-recording"
            ),
            (
                "unchanged/no-publication-finalizing",
                "single",
                "no-publication-complete-before-terminal"
            ),
        ]
    );
    let unlisted = field(&registry, "valid_unlisted_corpus")
        .as_sequence()
        .unwrap()
        .iter()
        .map(|row| {
            (
                text_field(row, "case_id"),
                text_field(row, "subcase"),
                text_field(row, "operation_state"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        unlisted,
        vec![
            ("terminal/completed", "single", "completed"),
            ("terminal/aborted", "single", "aborted"),
            ("recovery/candidate", "candidate", "recovery_required"),
            (
                "recovery/no-publication",
                "no_publication",
                "recovery_required"
            ),
            ("preserving/stash", "single", "preserving"),
            ("rollback/participant", "0", "rolling_back"),
        ]
    );
}
