use std::collections::BTreeMap;

use serde_yaml::Value;
use sha2::{Digest, Sha256};

use super::seed_open;
use crate::workspace_ops::merge::v1_lifecycle::checked::V1MutationLease;
use crate::workspace_ops::merge::v1_lifecycle::store::CheckedV1Store;
use crate::workspace_ops::merge::v1_lifecycle::tests::predecessor_matrix::{
    StoreEffectCase, capture_effect_cases,
};
use crate::workspace_ops::merge::v1_lifecycle::transition::{
    EFFECT_VARIANT_COUNT, RetiredContainer, prepared_for_store_matrix,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn every_transition_effect_commits_its_exact_unknown_manifest() {
    let cases = capture_effect_cases()
        .into_iter()
        .fold(BTreeMap::new(), |mut cases, case| {
            cases.entry(format!("{:?}", case.kind)).or_insert(case);
            cases
        });
    assert_eq!(cases.len(), EFFECT_VARIANT_COUNT);

    for (name, case) in cases {
        commit_case(&name, case);
    }
}

fn commit_case(name: &str, mut case: StoreEffectCase) {
    let root = TempDir::new_git(&format!("merge-v1-store-matrix-{name}"));
    let seed_accepted_lock_members = case
        .next
        .publication
        .as_ref()
        .and_then(|publication| publication.candidate.as_ref())
        .is_none();
    seed_open(&root, &case.old, |raw| {
        seed_all_live_containers(raw, seed_accepted_lock_members)
    });
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let store = CheckedV1Store::default();
    let current = store.load_open(&root.path, &case.old.merge_id).unwrap();
    carry_typed_extensions(current.record(), &mut case.next);
    let mut source = current.unknown_fields().clone();
    for retirement in case.effect.retired().unwrap() {
        if let RetiredContainer::ParticipantDrift {
            member_id,
            identity,
        } = retirement
        {
            source = source
                .after_participant_drift_retirement(&member_id, &drift_identity(&identity))
                .unwrap();
        }
    }
    let mut expected_raw = serde_yaml::to_value(&case.next).unwrap();
    let expected = source.apply_surviving(&mut expected_raw).unwrap();
    let rewrite = prepared_for_store_matrix(&current, case.next, case.effect).unwrap();
    let next = store.commit(&lease, &current, rewrite).unwrap();

    assert_eq!(next.raw(), &expected_raw, "raw unknown overlay for {name}");
    assert_eq!(
        next.unknown_fields(),
        &expected,
        "unknown manifest for {name}"
    );
}

fn drift_identity(
    identity: &crate::workspace_ops::merge::v1_lifecycle::authority::ParticipantDriftIdentity,
) -> crate::workspace_ops::merge::record_wire::SemanticIdentity {
    use crate::workspace_ops::merge::record_wire::{IdentityValue, SemanticIdentity};
    let optional = |value: &Option<String>| {
        value.as_ref().map_or(IdentityValue::Null, |value| {
            IdentityValue::String(value.clone())
        })
    };
    SemanticIdentity {
        kind: "participant_drift".into(),
        fields: vec![
            (
                "kind".into(),
                IdentityValue::String(
                    serde_yaml::to_value(identity.kind)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .into(),
                ),
            ),
            (
                "expected_branch".into(),
                optional(&identity.expected_branch),
            ),
            ("live_branch".into(), optional(&identity.live_branch)),
            ("expected_head".into(), optional(&identity.expected_head)),
            ("live_head".into(), optional(&identity.live_head)),
            (
                "expected_merge_head".into(),
                optional(&identity.expected_merge_head),
            ),
            (
                "live_merge_head".into(),
                optional(&identity.live_merge_head),
            ),
        ],
        occurrence: identity.occurrence,
    }
}

fn seed_all_live_containers(raw: &mut Value, seed_accepted_lock_members: bool) {
    let mut serial = 0;
    seed(raw, &mut serial);
    seed_child(raw, "baseline", &mut serial);
    for_each_map_value(raw, "participants", |participant| {
        seed(participant, &mut serial);
        seed_child(participant, "pending_action", &mut serial);
        if let Some(action) = child(participant, "pending_action")
            && let Some(spec) = child(action, "commit_spec")
        {
            seed(spec, &mut serial);
            seed_child(spec, "author", &mut serial);
            seed_child(spec, "committer", &mut serial);
        }
        seed_child(participant, "error", &mut serial);
        for_each_sequence_value(participant, "conflict_snapshot", |row| {
            seed(row, &mut serial)
        });
        for_each_sequence_value(participant, "preservation", |row| seed(row, &mut serial));
        for_each_sequence_value(participant, "drift", |row| seed(row, &mut serial));
    });
    for key in [
        "recovery_context",
        "pending_rollback",
        "pending_preservation",
        "accepted_workspace",
        "publication",
    ] {
        seed_child(raw, key, &mut serial);
    }
    if let Some(preservation) = child(raw, "pending_preservation") {
        seed_child(preservation, "owner", &mut serial);
        seed_child(preservation, "stash_object_id", &mut serial);
    }
    for_each_sequence_value(raw, "operation_drift", |row| seed(row, &mut serial));
    if let Some(accepted) = child(raw, "accepted_workspace") {
        if let Some(metadata) = child(accepted, "metadata_base") {
            seed(metadata, &mut serial);
            seed_child(metadata, "source", &mut serial);
        }
        seed_child(accepted, "lock", &mut serial);
        let mut lock_extensions = Vec::new();
        if let Some(audit) = child(accepted, "member_audit").and_then(Value::as_mapping_mut) {
            for (member_id, member) in audit {
                seed(member, &mut serial);
                seed_child(member, "integration", &mut serial);
                seed_child(member, "final_checkout", &mut serial);
                if seed_accepted_lock_members
                    && let Some(lock_member) = child(member, "lock_member")
                    && let Some((key, value)) = seed_named(lock_member, &mut serial)
                {
                    lock_extensions.push((member_id.as_str().unwrap().to_owned(), key, value));
                }
            }
        }
        sync_accepted_lock_extensions(accepted, &lock_extensions);
        if let Some(root) = child(accepted, "root") {
            seed(root, &mut serial);
            seed_child(root, "base", &mut serial);
            seed_child(root, "baseline_artifact_hashes", &mut serial);
        }
    }
    if let Some(publication) = child(raw, "publication") {
        seed_child(publication, "candidate", &mut serial);
        for_each_sequence_value(publication, "candidate_hashes", |row| {
            seed(row, &mut serial)
        });
        for_each_sequence_value(publication, "root_preservation", |row| {
            seed(row, &mut serial)
        });
    }
}

fn carry_typed_extensions(
    current: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    next: &mut crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
) {
    next.extensions.clone_from(&current.extensions);
    next.baseline
        .extensions
        .clone_from(&current.baseline.extensions);
    for (member_id, row) in &mut next.participants {
        let current_row = &current.participants[member_id];
        row.extensions.clone_from(&current_row.extensions);
        if let (Some(action), Some(current_action)) =
            (&mut row.pending_action, &current_row.pending_action)
        {
            action.extensions.clone_from(&current_action.extensions);
            if let (Some(spec), Some(current_spec)) =
                (&mut action.commit_spec, &current_action.commit_spec)
            {
                spec.extensions.clone_from(&current_spec.extensions);
                spec.author
                    .extensions
                    .clone_from(&current_spec.author.extensions);
                spec.committer
                    .extensions
                    .clone_from(&current_spec.committer.extensions);
            }
        }
    }
    if let (Some(publication), Some(current_publication)) =
        (&mut next.publication, &current.publication)
        && let (Some(candidate), Some(current_candidate)) =
            (&mut publication.candidate, &current_publication.candidate)
    {
        candidate
            .extensions
            .clone_from(&current_candidate.extensions);
    }
    if next.accepted_workspace.is_some() && current.accepted_workspace.is_some() {
        next.accepted_workspace
            .clone_from(&current.accepted_workspace);
    }
}

fn seed(value: &mut Value, serial: &mut usize) {
    let _ = seed_named(value, serial);
}

fn seed_named(value: &mut Value, serial: &mut usize) -> Option<(String, String)> {
    let mapping = value.as_mapping_mut()?;
    let key = format!("future_matrix_{serial}");
    let content = format!("value-{serial}");
    *serial += 1;
    mapping.insert(Value::String(key.clone()), Value::String(content.clone()));
    Some((key, content))
}

fn sync_accepted_lock_extensions(accepted: &mut Value, extensions: &[(String, String, String)]) {
    if extensions.is_empty() {
        return;
    }
    let lock = child(accepted, "lock").unwrap();
    let mut exact: Value =
        serde_yaml::from_str(child(lock, "exact_yaml").unwrap().as_str().unwrap()).unwrap();
    for (member_id, key, value) in extensions {
        exact["members"][member_id]
            .as_mapping_mut()
            .unwrap()
            .insert(Value::String(key.clone()), Value::String(value.clone()));
    }
    let exact = serde_yaml::to_string(&exact).unwrap();
    let sha256 = format!("{:x}", Sha256::digest(exact.as_bytes()));
    *child(lock, "exact_yaml").unwrap() = Value::String(exact);
    *child(lock, "sha256").unwrap() = Value::String(sha256);
}

fn seed_child(value: &mut Value, key: &str, serial: &mut usize) {
    if let Some(child) = child(value, key) {
        seed(child, serial);
    }
}

fn child<'a>(value: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    value
        .as_mapping_mut()?
        .get_mut(Value::String(key.into()))
        .filter(|value| !value.is_null())
}

fn for_each_map_value(value: &mut Value, key: &str, mut visit: impl FnMut(&mut Value)) {
    let Some(values) = child(value, key).and_then(Value::as_mapping_mut) else {
        return;
    };
    for value in values.values_mut() {
        visit(value);
    }
}

fn for_each_sequence_value(value: &mut Value, key: &str, mut visit: impl FnMut(&mut Value)) {
    let Some(values) = child(value, key).and_then(Value::as_sequence_mut) else {
        return;
    };
    for value in values {
        visit(value);
    }
}
