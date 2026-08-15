use std::collections::{BTreeMap, BTreeSet};

use super::super::fault_v1::CheckedArtifactFaultKeyV1;

const EXPECTED_STABLE_KEYS: &[&str] = &[
    "runtime.git_dir_retain",
    "runtime.workspace_retain",
    "runtime.bootstrap_guard_open_or_create",
    "runtime.bootstrap_guard_lock_acquire",
    "runtime.bootstrap_guard_reobserve",
    "runtime.gwz_directory_create",
    "runtime.gwz_directory_reobserve",
    "runtime.locks_directory_create",
    "runtime.locks_directory_reobserve",
    "runtime.lease_file_open_or_create",
    "runtime.lease_file_reobserve_before_lock",
    "runtime.lease_lock_acquire",
    "runtime.lease_reobserve_after_lock",
    "runtime.bootstrap_guard_release",
    "runtime.lease_release",
    "runtime.path_walk",
    "runtime.collision_scan",
    "runtime.capability_proof",
    "catalog_bootstrap.git_parent_create",
    "catalog_bootstrap.git_parent_reobserve",
    "catalog_bootstrap.scratch_create",
    "catalog_bootstrap.scratch_write",
    "catalog_bootstrap.scratch_flush",
    "catalog_bootstrap.scratch_root_flush",
    "catalog_bootstrap.active_publish",
    "catalog_bootstrap.active_reobserve",
    "catalog_bootstrap.staging_create",
    "catalog_bootstrap.infrastructure_populate",
    "catalog_bootstrap.infrastructure_flush",
    "catalog_bootstrap.anchor_scratch_create",
    "catalog_bootstrap.anchor_scratch_flush",
    "catalog_bootstrap.anchor_publish",
    "catalog_bootstrap.anchor_reobserve",
    "catalog_bootstrap.anchor_home_a_exercise",
    "catalog_bootstrap.anchor_home_b_exercise",
    "catalog_bootstrap.staging_flush",
    "catalog_bootstrap.final_publish",
    "catalog_bootstrap.final_reopen",
    "catalog_bootstrap.final_reobserve",
    "catalog_bootstrap.active_retire",
    "catalog_bootstrap.retired_reobserve",
    "catalog_bootstrap.catalog_enumerate",
    "admission.occupancy_observe",
    "admission.capacity_check",
    "admission.preparing_scratch_create",
    "admission.preparing_scratch_write",
    "admission.preparing_scratch_flush",
    "admission.preparing_publish",
    "admission.preparing_reobserve",
    "admission.staging_create",
    "admission.reservation_create",
    "admission.reservation_write",
    "admission.reservation_flush",
    "admission.staging_flush",
    "admission.final_publish",
    "admission.final_reobserve",
    "admission.idle_scratch_create",
    "admission.idle_scratch_write",
    "admission.idle_scratch_flush",
    "admission.idle_publish",
    "admission.idle_reobserve",
    "record.bounded_read",
    "record.decode",
    "record.canonical_reencode",
    "record.binding_validate",
    "record.scratch_create",
    "record.scratch_write",
    "record.scratch_flush",
    "record.active_publish",
    "record.active_reobserve",
    "record.retirement_reserve",
    "record.retire_exact",
    "record.retired_reobserve",
    "record.terminal_relation_validate",
    "namespace.source_retain",
    "namespace.destination_reserve",
    "namespace.pre_publish_reobserve",
    "namespace.publish_no_replace",
    "namespace.published_reobserve",
    "namespace.retirement_reserve",
    "namespace.pre_retire_reobserve",
    "namespace.retire_exact",
    "namespace.retired_reobserve",
    "namespace.parent_barrier",
    "namespace.parent_revalidate",
    "durable_leaf.first_open",
    "durable_leaf.first_identity",
    "durable_leaf.first_content",
    "durable_leaf.file_flush",
    "durable_leaf.namespace_barrier",
    "durable_leaf.parent_revalidate",
    "durable_leaf.name_revalidate",
    "durable_leaf.handle_revalidate",
    "durable_leaf.length_revalidate",
    "durable_leaf.content_revalidate",
    "durable_leaf.missing_revalidate",
    "barrier.intent_scratch_create",
    "barrier.intent_scratch_write",
    "barrier.intent_scratch_flush",
    "barrier.intent_publish",
    "barrier.intent_reobserve",
    "barrier.anchor_outbound",
    "barrier.anchor_outbound_reobserve",
    "barrier.target_barrier",
    "barrier.target_reobserve",
    "barrier.anchor_return",
    "barrier.anchor_return_reobserve",
    "barrier.target_alias_retire",
    "barrier.target_alias_reobserve",
    "barrier.intent_retire",
    "barrier.intent_retired_reobserve",
    "barrier.completion_reobserve",
    "managed_bootstrap.preflight",
    "managed_bootstrap.initial_intent_scratch_create",
    "managed_bootstrap.initial_intent_scratch_write",
    "managed_bootstrap.initial_intent_scratch_flush",
    "managed_bootstrap.initial_intent_publish",
    "managed_bootstrap.initial_intent_reobserve",
    "managed_bootstrap.component_reobserve",
    "managed_bootstrap.staging_directory_create",
    "managed_bootstrap.ownership_marker_create",
    "managed_bootstrap.ownership_marker_write",
    "managed_bootstrap.ownership_marker_flush",
    "managed_bootstrap.staging_directory_flush",
    "managed_bootstrap.staging_directory_publish",
    "managed_bootstrap.final_directory_reopen",
    "managed_bootstrap.final_directory_reobserve",
    "managed_bootstrap.successor_scratch_create",
    "managed_bootstrap.successor_scratch_write",
    "managed_bootstrap.successor_scratch_flush",
    "managed_bootstrap.successor_scratch_reobserve",
    "managed_bootstrap.prior_generation_retire",
    "managed_bootstrap.prior_generation_reobserve",
    "managed_bootstrap.successor_publish",
    "managed_bootstrap.successor_reobserve",
    "managed_bootstrap.marker_retire",
    "managed_bootstrap.marker_retired_reobserve",
    "managed_bootstrap.final_identity_reobserve",
    "managed_bootstrap.final_intent_retire",
    "managed_bootstrap.final_intent_retired_reobserve",
    "managed_bootstrap.parent_revalidate",
    "managed_bootstrap.plan_complete",
    "cleanup.worklist_scratch_create",
    "cleanup.worklist_scratch_write",
    "cleanup.worklist_scratch_flush",
    "cleanup.worklist_publish",
    "cleanup.worklist_reobserve",
    "cleanup.source_reobserve",
    "cleanup.destination_reobserve",
    "cleanup.alias_retire",
    "cleanup.retired_alias_reobserve",
    "cleanup.row_complete",
    "cleanup.completion_reobserve",
    "terminal.authority_reobserve",
    "terminal.payload_reobserve",
    "terminal.cleanup_reobserve",
    "terminal.reservation_reobserve",
    "terminal.directory_flush",
    "terminal.retired_slot_reserve",
    "terminal.action_directory_retire",
    "terminal.retired_directory_reobserve",
    "terminal.catalog_barrier",
    "terminal.terminal_revalidate",
    "terminal.authority_release",
];
const EXPECTED_KEY_COUNT: usize = 164;

#[derive(Debug, Eq, PartialEq)]
struct KeySetMismatch {
    actual_duplicates: Vec<String>,
    expected_duplicates: Vec<String>,
    missing: Vec<String>,
    extra: Vec<String>,
}

fn duplicates<'a>(keys: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut counts = BTreeMap::new();
    for key in keys {
        *counts.entry(key).or_insert(0_usize) += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(key, _)| key.to_owned())
        .collect()
}

fn compare_key_sets(actual: &[String], expected: &[&str]) -> Result<(), KeySetMismatch> {
    let actual_set = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_set = expected.iter().copied().collect::<BTreeSet<_>>();
    let mismatch = KeySetMismatch {
        actual_duplicates: duplicates(actual.iter().map(String::as_str)),
        expected_duplicates: duplicates(expected.iter().copied()),
        missing: expected_set
            .difference(&actual_set)
            .map(|key| (*key).to_owned())
            .collect(),
        extra: actual_set
            .difference(&expected_set)
            .map(|key| (*key).to_owned())
            .collect(),
    };
    if mismatch.actual_duplicates.is_empty()
        && mismatch.expected_duplicates.is_empty()
        && mismatch.missing.is_empty()
        && mismatch.extra.is_empty()
        && actual.len() == expected.len()
    {
        Ok(())
    } else {
        Err(mismatch)
    }
}

#[test]
fn fault_vocabulary_exactly_matches_the_independent_stable_key_fixture() {
    let actual = CheckedArtifactFaultKeyV1::all()
        .iter()
        .map(CheckedArtifactFaultKeyV1::stable_key)
        .collect::<Vec<_>>();

    compare_key_sets(&actual, EXPECTED_STABLE_KEYS).unwrap();
    assert_eq!(EXPECTED_STABLE_KEYS.len(), EXPECTED_KEY_COUNT);
    assert_eq!(actual.len(), EXPECTED_KEY_COUNT);
}

#[test]
fn fault_fixture_comparison_reports_missing_extra_and_duplicate_keys() {
    assert_eq!(
        compare_key_sets(
            &["axis.present".to_owned(), "axis.extra".to_owned()],
            &["axis.present", "axis.missing"],
        ),
        Err(KeySetMismatch {
            actual_duplicates: Vec::new(),
            expected_duplicates: Vec::new(),
            missing: vec!["axis.missing".to_owned()],
            extra: vec!["axis.extra".to_owned()],
        })
    );
    assert_eq!(
        compare_key_sets(
            &["axis.present".to_owned(), "axis.present".to_owned()],
            &["axis.present"],
        )
        .unwrap_err()
        .actual_duplicates,
        vec!["axis.present".to_owned()]
    );
    assert_eq!(
        compare_key_sets(
            &["axis.present".to_owned()],
            &["axis.present", "axis.present"]
        )
        .unwrap_err()
        .expected_duplicates,
        vec!["axis.present".to_owned()]
    );
}
