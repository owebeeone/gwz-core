use std::collections::BTreeMap;

use serde_yaml::Value;

use crate::git::GitBackend;
use crate::workspace_ops::merge::{MergeExecutionMode, MergeOperationRecord, OperationState};

use super::*;

const REGISTRY: &str = include_str!("../../../../dev-docs/GwzM5-8I2CompatibilityPredicates.json");

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

fn normalize_descriptor<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
) -> crate::workspace_ops::merge::VerifiedV0Descriptor {
    crate::workspace_ops::merge::verified_v0_descriptor(backend, root, record).unwrap()
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

fn assert_adapter_guards<B: GitBackend>(backend: &B, root: &Path, record: &MergeOperationRecord) {
    let raw = serde_yaml::to_value(record).unwrap();
    let baseline_decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
        serde_yaml::to_string(record).unwrap().as_bytes(),
    )
    .unwrap();

    let mut no_ff = record.clone();
    no_ff.mode = MergeExecutionMode::NoFf;
    let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
        serde_yaml::to_string(&no_ff).unwrap().as_bytes(),
    )
    .unwrap();
    let error = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
        backend,
        root,
        &decoded,
        "r3-test-writer",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnsupportedLegacyMode);

    super::compatibility_v0_edges::assert_legacy_v0_compatibility_edges(backend, root, record);

    let member_drift = root
        .join(&record.participants[&record.selected_targets[0]].path)
        .join("i2-untracked-drift.txt");
    fs::write(&member_drift, "drift\n").unwrap();
    let error = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
        backend,
        root,
        &baseline_decoded,
        "r3-test-writer",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::AcceptanceInputDrift);
    fs::remove_file(member_drift).unwrap();

    for field_name in [
        "accepted_workspace",
        "recovery_context",
        "pending_rollback",
        "pending_preservation",
    ] {
        let mut colliding = raw.clone();
        colliding.as_mapping_mut().unwrap().insert(
            Value::String(field_name.to_owned()),
            Value::String("future-v0-value".to_owned()),
        );
        let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
            serde_yaml::to_string(&colliding).unwrap().as_bytes(),
        )
        .unwrap();
        let error = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decoded,
            "r3-test-writer",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::MergeRecordUnreadable, "{field_name}");
        assert!(error.message.contains("collides"), "{field_name}");
    }

    let mut future = raw;
    future.as_mapping_mut().unwrap().insert(
        Value::String("future_record".to_owned()),
        Value::String("retained".to_owned()),
    );
    future["baseline"].as_mapping_mut().unwrap().insert(
        Value::String("future_baseline".to_owned()),
        Value::Bool(true),
    );
    let selected = &record.selected_targets[0];
    future["participants"][selected]
        .as_mapping_mut()
        .unwrap()
        .insert(
            Value::String("future_participant".to_owned()),
            Value::Number(7.into()),
        );
    let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
        serde_yaml::to_string(&future).unwrap().as_bytes(),
    )
    .unwrap();
    match crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
        backend,
        root,
        &decoded,
        "r3-test-writer",
    )
    .unwrap()
    {
        crate::workspace_ops::merge::OpenV0Adaptation::Eligible {
            record,
            unknown_fields,
            ..
        } => {
            assert_eq!(unknown_fields.entries().len(), 3);
            assert_eq!(record.extensions["future_record"], "retained");
            assert_eq!(record.baseline.extensions["future_baseline"], true);
            assert_eq!(
                record.participants[selected].extensions["future_participant"],
                7
            );
        }
        crate::workspace_ops::merge::OpenV0Adaptation::ValidUnlisted => {
            panic!("eligible record with additive unknowns became unlisted")
        }
    }
}

fn assert_root_drift_taxonomy<B: GitBackend>(
    backend: &B,
    root: &Path,
    record: &MergeOperationRecord,
    rule_id: &str,
) {
    let manifest_path = root.join(crate::workspace::WORKSPACE_MANIFEST);
    let original = fs::read(&manifest_path).unwrap();
    fs::write(&manifest_path, b"root drift\n").unwrap();
    let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
        serde_yaml::to_string(record).unwrap().as_bytes(),
    )
    .unwrap();
    let error = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
        backend,
        root,
        &decoded,
        "r3-test-writer",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::AcceptanceInputDrift, "{rule_id}");
    fs::write(manifest_path, original).unwrap();

    if record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .is_some()
    {
        let lock_path = root.join(crate::artifact::LOCK_PATH);
        let original = fs::read(&lock_path).unwrap();
        fs::write(&lock_path, "illegal publication prefix\n").unwrap();
        let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
            serde_yaml::to_string(record).unwrap().as_bytes(),
        )
        .unwrap();
        let error = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decoded,
            "r3-test-writer",
        )
        .unwrap_err();
        assert_eq!(
            error.code,
            ErrorCode::PublicationPrefixMismatch,
            "{rule_id}"
        );
        fs::write(lock_path, original).unwrap();
    }

    if record
        .publication
        .as_ref()
        .and_then(|publication| publication.composition_commit.as_ref())
        .is_some()
    {
        let mut mismatched = record.clone();
        mismatched.publication.as_mut().unwrap().composition_commit = Some("f".repeat(40));
        let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
            serde_yaml::to_string(&mismatched).unwrap().as_bytes(),
        )
        .unwrap();
        let error = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decoded,
            "r3-test-writer",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::RecordedEvidenceDrift, "{rule_id}");
    } else if record
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .is_some()
    {
        let original_branch = backend.head(root).unwrap().branch.unwrap();
        super::compatibility_v0_edges::set_symbolic_head(root, "i2-ambiguous-root");
        let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
            serde_yaml::to_string(record).unwrap().as_bytes(),
        )
        .unwrap();
        let error = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decoded,
            "r3-test-writer",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::AmbiguousEvidenceCommit, "{rule_id}");
        super::compatibility_v0_edges::set_symbolic_head(root, &original_branch);
    }

    if rule_id == "candidate-persisted-before-evidence" {
        let mut corrupt = record.clone();
        corrupt
            .publication
            .as_mut()
            .unwrap()
            .candidate
            .as_mut()
            .unwrap()
            .lock_yaml
            .push_str("# corrupt\n");
        let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
            serde_yaml::to_string(&corrupt).unwrap().as_bytes(),
        )
        .unwrap();
        let error = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decoded,
            "r3-test-writer",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::CandidateIntegrityMismatch);
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
    if case_id == "changed/finalizing-before-publication-record" {
        assert_adapter_guards(backend, root, record);
    }
    assert_eq!(descriptor.value(), field(rule, "descriptor"), "{case_id}");
    let mut canonical = String::new();
    canonical_json(descriptor.value(), &mut canonical);
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
    super::compatibility_v0_edges::assert_exact_baseline_recovery(backend, root, record);
    assert_root_drift_taxonomy(backend, root, record, rule_id);
    let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
        serde_yaml::to_string(record).unwrap().as_bytes(),
    )
    .unwrap();
    match crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
        backend,
        root,
        &decoded,
        "r3-test-writer",
    )
    .unwrap()
    {
        crate::workspace_ops::merge::OpenV0Adaptation::Eligible {
            rule_id: adapted_rule,
            next_action,
            record: adapted,
            canonical,
            unknown_fields,
        } => {
            assert_eq!(adapted_rule, rule_id, "{case_id}");
            assert_eq!(
                next_action,
                text_field(field(rule, "classification"), "next_action"),
                "{case_id}"
            );
            assert!(adapted.accepted_workspace.is_some(), "{case_id}");
            let accepted = adapted.accepted_workspace.as_ref().unwrap();
            if let Some(candidate) = record
                .publication
                .as_ref()
                .and_then(|publication| publication.candidate.as_ref())
            {
                assert_eq!(accepted.lock.exact_yaml, candidate.lock_yaml, "{case_id}");
                assert_eq!(
                    accepted.lock.sha256,
                    format!("{:x}", Sha256::digest(candidate.lock_yaml.as_bytes())),
                    "{case_id}"
                );
            }
            assert_eq!(adapted.writer_version, "r3-test-writer", "{case_id}");
            assert_eq!(
                canonical.installed_kind(),
                crate::workspace_ops::merge::v1::CanonicalInstalledKind::V1,
                "{case_id}"
            );
            assert!(unknown_fields.entries().is_empty(), "{case_id}");
        }
        crate::workspace_ops::merge::OpenV0Adaptation::ValidUnlisted => {
            panic!("registered case {case_id} was not adapted")
        }
    }
}

pub(super) fn assert_i2_valid_unlisted_fixture<B: GitBackend>(
    backend: &B,
    root: &Path,
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
    let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
        serde_yaml::to_string(record).unwrap().as_bytes(),
    )
    .unwrap();
    assert_eq!(
        crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
            backend,
            root,
            &decoded,
            "r3-test-writer",
        )
        .unwrap(),
        crate::workspace_ops::merge::OpenV0Adaptation::ValidUnlisted,
        "{case_id}"
    );

    let mut malformed = record.clone();
    malformed
        .participants
        .remove(&malformed.selected_targets[0]);
    let decoded = crate::workspace_ops::merge::decode_v0_for_r3_tests(
        serde_yaml::to_string(&malformed).unwrap().as_bytes(),
    )
    .unwrap();
    let error = crate::workspace_ops::merge::adapt_open_v0_for_r3_tests(
        backend,
        root,
        &decoded,
        "r3-test-writer",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecordUnreadable, "{case_id}");
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
    let rules = field(&registry, "migration_whitelist")
        .as_sequence()
        .unwrap();
    let rule_ids = rules
        .iter()
        .map(|rule| text_field(rule, "id"))
        .collect::<std::collections::BTreeSet<_>>();
    let bound_rule_ids = migration
        .iter()
        .map(|(_, _, rule)| *rule)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(rule_ids, bound_rule_ids);
    assert_eq!(rules.len(), rule_ids.len());
    assert_eq!(migration.len(), bound_rule_ids.len());
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
