use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

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
    "catalog_bootstrap.ready_edge_root_flush",
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
const EXPECTED_KEY_COUNT: usize = 165;

/// Per-family activation map frozen by R2-D Step 0.1
/// (`dev-docs/GwzM5-8R2DInterfaceFreeze.md`; adopted plan §4-end and §9.4;
/// RemPlan §10 :1025-1038).
///
/// `Executed` means the family has injection sites and an executed
/// interruption/restart/convergence matrix on both target variants.
/// `Reserved(owner)` means the family is declared-reserved and has no
/// injection sites by design; the named package is the one that must add both
/// the injection sites and the matrix rows when it converts that family's
/// edges. Flipping a row to `Executed` is therefore a deliberate edit in the
/// converting package, reviewed with its evidence, and never a side effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FaultFamilyActivationV1 {
    Executed(&'static str),
    /// A family whose owning package converts only *part* of its edge set,
    /// because the family spans more than one package's edges.
    ///
    /// R2-D Step 2.3 is the first such case and the reason this variant exists.
    /// `managed_bootstrap.*` remains Phase 3's family in the frozen map, whose
    /// `managed_bootstrap.*` row now carries the dated Step-2.3 activation
    /// annotation recording this exact 8-of-30 flip
    /// (`GwzM5-8R2DInterfaceFreeze.md` §3.5). What moved is per *edge*: §4.3 rows
    /// E15 and E16 are Step 2.3's, and RemPlan §10's duty — "that family must
    /// gain injection sites and matrix rows in the same package that converts
    /// its edges" — binds per edge, not per family. Before this variant existed
    /// the fixture could only express all-or-nothing, so Step 2.3 would have had
    /// to either declare twenty-two unconverted keys executed or leave its own
    /// two edges unkeyed. Both are worse than naming the subset.
    ///
    /// The second field lists the activated keys by **stable-key suffix**. Every
    /// other key in the family stays reserved and is still asserted to have no
    /// injection site, so the guarantee the `Reserved` arm gives is preserved
    /// key-by-key rather than weakened family-wide.
    PartiallyExecuted(&'static str, &'static [&'static str]),
    Reserved(&'static str),
}

impl FaultFamilyActivationV1 {
    /// The suffixes this row declares activated. `Executed` activates the whole
    /// family, so it is never consulted per key.
    const fn activated_suffixes(self) -> &'static [&'static str] {
        match self {
            Self::PartiallyExecuted(_, suffixes) => suffixes,
            Self::Executed(_) | Self::Reserved(_) => &[],
        }
    }
}

/// The `managed_bootstrap.*` subset executed today, by the step that converted
/// each edge and landed its matrix.
///
/// **Step 2.3 — the four managed backend operations.** Exactly the boundaries
/// edge E15 (managed component install) and edge E16 (ownership-marker
/// retirement) cross, plus the two restart observations ConsumerCheckpoint §8
/// (:228-231) requires of them. Matrix: `namespace/tests_managed_matrix.rs`.
///
/// **Step 3.1b — the managed intent record's durable lifecycle** (freeze §4.3
/// row E17). The initial generation, every successor, every prior-generation
/// retirement, and the final retirement that is a row's completion record. Their
/// scratch boundaries have injection sites in
/// `capability/pre_catalog/provider/managed_mutation.rs` and their edge and
/// observation boundaries are announced from the same file, because the record's
/// moves themselves route through the Step-2.2 backend and the `namespace` owner
/// still holds no site. Matrix: `bootstrap/managed/tests_intent_matrix.rs`, on
/// both target variants, with thirteen repeated-boundary rows.
///
/// **Step 3.2 — the staged-component writer** (freeze §3.5's deferral record and
/// its Step-3.1b supersession clause). The five keys whose edges Step 3.1's
/// `stage_component` and `write_or_rewrite_marker` converted without sites: the
/// staging directory's creation, the ownership marker's create/write/flush, and
/// the two directory flushes that make the staged row durable. Sites in the same
/// provider file; matrix: `bootstrap/managed/tests_writer_matrix.rs`, on both
/// target variants, with one repeated-boundary row and four named
/// single-crossing exclusions.
///
/// **Still reserved: two.** `preflight` and `plan_complete`. Every edge-bearing
/// key of the family is now executed; these two name plan-level states, no step
/// has given them a boundary, and their disposition is a Phase 3 settle
/// determination, not this fixture's (Step-3.1b review [P3-1]). The test below
/// still proves both siteless, key by key.
const MANAGED_BOOTSTRAP_EXECUTED_KEYS: &[&str] = &[
    // Step 2.3 — edge E15 and its restart half.
    "parent_revalidate",
    "staging_directory_publish",
    "final_directory_reopen",
    "final_directory_reobserve",
    "component_reobserve",
    // Step 2.3 — edge E16 and its restart half.
    "marker_retire",
    "marker_retired_reobserve",
    "final_identity_reobserve",
    // Step 3.1b — the initial generation of the intent chain.
    "initial_intent_scratch_create",
    "initial_intent_scratch_write",
    "initial_intent_scratch_flush",
    "initial_intent_publish",
    "initial_intent_reobserve",
    // Step 3.1b — every successor generation.
    "successor_scratch_create",
    "successor_scratch_write",
    "successor_scratch_flush",
    "successor_scratch_reobserve",
    "successor_publish",
    "successor_reobserve",
    // Step 3.1b — the retirement of each superseded generation.
    "prior_generation_retire",
    "prior_generation_reobserve",
    // Step 3.1b — the final retirement, which is the row's completion record.
    "final_intent_retire",
    "final_intent_retired_reobserve",
    // Step 3.2 — the staged-component writer.
    "staging_directory_create",
    "ownership_marker_create",
    "ownership_marker_write",
    "ownership_marker_flush",
    "staging_directory_flush",
];

const FAULT_FAMILY_ACTIVATION: &[(&str, FaultFamilyActivationV1, usize)] = &[
    (
        "runtime",
        FaultFamilyActivationV1::Executed("R2-A/R2-B runtime bootstrap and catalog lease"),
        18,
    ),
    (
        "catalog_bootstrap",
        FaultFamilyActivationV1::Executed("R2-C2 physical first-catalog owner"),
        25,
    ),
    (
        "admission",
        FaultFamilyActivationV1::Executed("R2-D phase 1 step 1.3 (R2-C3 admission)"),
        19,
    ),
    // R2-D Phase 2 Step 2.1 converts every edge this family names: the bounded
    // and same-handle durable leaf observations of
    // `GwzM5-8R2DInterfaceFreeze.md` §4.3 rows E8-E11. All eleven keys have
    // injection sites in `capability/pre_catalog/provider/leaf_observation.rs`
    // and executed interruption/restart/convergence rows on both target
    // variants in `capability/pre_catalog/provider/tests_leaf_fault_matrix.rs`,
    // covering both sides of the two-sided proof — ten boundaries on the exact
    // arm and `missing_revalidate` plus the three shared boundaries on the
    // absence arm.
    (
        "durable_leaf",
        FaultFamilyActivationV1::Executed("R2-D phase 2 step 2.1 (leaf observer)"),
        11,
    ),
    // R2-D Phase 2 Step 2.2 converts every edge this family names. The eleven
    // keys are the boundaries of the three backend edges the frozen map assigns
    // it — E12 `publish_exact`, E13 `retire_exact` and E14 `barrier`
    // (`GwzM5-8R2DInterfaceFreeze.md` §4.3) — and all eleven have injection
    // sites in `capability/pre_catalog/provider/namespace_mutation.rs` plus
    // executed matrix rows on both target variants in
    // `namespace/tests_fault_matrix.rs`. Nothing is left reserved for Step 2.3:
    // the four managed operations are the same two physical edges plus a
    // managed observation, so they re-cross these same boundaries through the
    // same shared edge helper, and the observation boundaries they add belong
    // to `managed_bootstrap.*`.
    (
        "namespace",
        FaultFamilyActivationV1::Executed("R2-D phase 2 step 2.2 (namespace backend)"),
        11,
    ),
    // R2-D Phase 2 Step 2.4 converts every edge this family names, and the
    // family's own shape is the split: all thirteen keys are boundaries of a
    // *protocol record*, never of a payload. Four of them are the bounded parse
    // stages and live with the R1 parse owner
    // (`protocol/authority_record.rs`); eight are the record's durable install
    // and retirement, and one — `terminal_relation_validate` — is the single
    // join between the bounded record and the streamed source/goal proof; those
    // nine live with the binding owner
    // (`capability/pre_catalog/provider/authority_record_binding.rs`). The
    // payload side of the split has no `record.*` key by construction: its
    // boundaries are `durable_leaf.*`, executed by Step 2.1. Executed matrix
    // rows for all thirteen, on both target variants, are in
    // `capability/pre_catalog/provider/tests_authority_record_matrix.rs`.
    (
        "record",
        FaultFamilyActivationV1::Executed("R2-D phase 2 step 2.4 (authority record split)"),
        13,
    ),
    // R2-D Phase 2 Step 2.3 converted §4.3 rows E15 and E16; Phase 3 Step 3.1b
    // converted row E17, the intent record's own durable lifecycle; Phase 3 Step
    // 3.2 activated the staged-component writer keys whose edges Step 3.1 had
    // converted on the record. All twenty-eight keys named by
    // `MANAGED_BOOTSTRAP_EXECUTED_KEYS` have injection sites in
    // `capability/pre_catalog/provider/managed_mutation.rs` — the `namespace`
    // owner still holds none, as with `namespace.*` — and executed
    // interruption/restart/convergence rows on both target variants in
    // `namespace/tests_managed_matrix.rs` (eight),
    // `bootstrap/managed/tests_intent_matrix.rs` (fifteen) and
    // `bootstrap/managed/tests_writer_matrix.rs` (five). The remaining two are
    // `preflight` and `plan_complete`, whose disposition is a Phase 3 settle
    // determination; they are still proved siteless below.
    (
        "managed_bootstrap",
        FaultFamilyActivationV1::PartiallyExecuted(
            "R2-D phase 3 (managed-parent provider); rows E15/E16 by phase 2 step 2.3, \
             row E17 by phase 3 step 3.1b, the writer keys by phase 3 step 3.2",
            MANAGED_BOOTSTRAP_EXECUTED_KEYS,
        ),
        30,
    ),
    (
        "cleanup",
        FaultFamilyActivationV1::Reserved("R2-D phase 4 (legacy leaf edge conversion)"),
        11,
    ),
    (
        "barrier",
        FaultFamilyActivationV1::Reserved("R2-D phase 4 (Windows retirement closure)"),
        16,
    ),
    (
        "terminal",
        FaultFamilyActivationV1::Reserved("R2-D phase 4 (terminal retirement edges)"),
        11,
    ),
];

/// The complete set of production sources that hold `CheckedArtifactFaultKeyV1`
/// injection sites today — three holding `catalog_bootstrap.*` sites,
/// `admission_mutation.rs` holding all nineteen `admission.*` sites
/// converted by R2-D Phase 1 Step 1.3, and `namespace_mutation.rs` holding all
/// eleven `namespace.*` sites converted by R2-D Phase 2 Step 2.2, and
/// `leaf_observation.rs` holding all eleven `durable_leaf.*` sites converted by
/// R2-D Phase 2 Step 2.1, and the two sources holding the thirteen `record.*`
/// sites converted by R2-D Phase 2 Step 2.4 — `protocol/authority_record.rs`
/// for the four bounded-parse stages and
/// `authority_record_binding.rs` for the nine install/retire/join boundaries.
/// `admission/driver.rs` deliberately holds
/// none: it decides and never mutates (`admission/driver.rs:8-9`), so every
/// durable admission edge is announced from the owner-private mutation file.
/// `runtime.*` edges are executed through the separate
/// `bootstrap/runtime/fault.rs` mechanism, so they are executed without a key
/// reference here (`GwzM5-8R2C2OwnerInterface-ReviewState-2.md:160-169`).
///
/// Completeness of this list is pinned, not asserted:
/// `the_declared_injection_sources_are_every_production_source_holding_sites`
/// rescans the production tree, so an injection site added in an unregistered
/// file fails this fixture instead of silently escaping the reserved-family
/// scan, which reads only the sources declared here.
const FAULT_INJECTION_SOURCES: &[(&str, &str)] = &[
    (
        "capability/pre_catalog/provider/mutation.rs",
        include_str!("../capability/pre_catalog/provider/mutation.rs"),
    ),
    (
        "capability/pre_catalog/provider/directory_mutation.rs",
        include_str!("../capability/pre_catalog/provider/directory_mutation.rs"),
    ),
    (
        "capability/pre_catalog/provider/aggregate.rs",
        include_str!("../capability/pre_catalog/provider/aggregate.rs"),
    ),
    (
        "capability/pre_catalog/provider/admission_mutation.rs",
        include_str!("../capability/pre_catalog/provider/admission_mutation.rs"),
    ),
    // R2-D Phase 2 Step 2.1: all eleven `durable_leaf.*` sites. A leaf
    // observation is a read whose only durable edges are the leaf flush and the
    // scheduled namespace barrier, so its boundaries are announced from the
    // observer itself rather than from a mutation file.
    (
        "capability/pre_catalog/provider/leaf_observation.rs",
        include_str!("../capability/pre_catalog/provider/leaf_observation.rs"),
    ),
    // R2-D Phase 2 Step 2.2: all eleven `namespace.*` sites. The `namespace`
    // owner itself (`namespace/host.rs`) deliberately holds none — it validates
    // capabilities and never mutates, so every durable namespace edge is
    // announced from the owner-private provider mutation file, exactly as
    // `admission/driver.rs` defers to `admission_mutation.rs`.
    (
        "capability/pre_catalog/provider/namespace_mutation.rs",
        include_str!("../capability/pre_catalog/provider/namespace_mutation.rs"),
    ),
    // R2-D Phase 2 Step 2.3, Phase 3 Step 3.1b and Phase 3 Step 3.2: all
    // twenty-eight activated `managed_bootstrap.*` sites. Same rule as the row above, and it holds for
    // the restart boundary too: `component_reobserve` marks "a fresh process
    // chose the restart path", so the provider exposes a distinct restart entry
    // point rather than letting the `namespace` owner announce it. It holds for
    // Step 3.1b's intent lifecycle as well, whose record moves route through the
    // Step-2.2 backend: the scratch write and every post-edge observation happen
    // in this file, so `namespace/host.rs` still holds no injection site.
    (
        "capability/pre_catalog/provider/managed_mutation.rs",
        include_str!("../capability/pre_catalog/provider/managed_mutation.rs"),
    ),
    // R2-D Phase 2 Step 2.4: the four `record.*` *parse* sites. They sit with
    // the R1 bounded parse owner rather than with a mutation file because they
    // are the stages of a read whose only budget is the frozen record bound —
    // and keeping them here is what makes the parse/payload split visible in
    // the injection inventory itself.
    (
        "protocol/authority_record.rs",
        include_str!("../protocol/authority_record.rs"),
    ),
    // R2-D Phase 2 Step 2.4: the remaining nine `record.*` sites — the record's
    // durable install and retirement, plus the single `terminal_relation_validate`
    // join between the bounded record and the streamed source/goal proof.
    (
        "capability/pre_catalog/provider/authority_record_binding.rs",
        include_str!("../capability/pre_catalog/provider/authority_record_binding.rs"),
    ),
];

fn family_of(stable_key: &str) -> &str {
    stable_key
        .split_once('.')
        .expect("every stable fault key is family-qualified")
        .0
}

fn variant_prefix(family: &str) -> String {
    family
        .split('_')
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + characters.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[test]
fn every_fault_family_declares_its_owning_activation_package() {
    let declared = FAULT_FAMILY_ACTIVATION
        .iter()
        .map(|(family, _, _)| *family)
        .collect::<BTreeSet<_>>();
    let actual = EXPECTED_STABLE_KEYS
        .iter()
        .map(|key| family_of(key))
        .collect::<BTreeSet<_>>();

    assert_eq!(
        declared, actual,
        "a fault family gained or lost keys without declaring its owning R2-D package"
    );
    assert_eq!(
        declared.len(),
        FAULT_FAMILY_ACTIVATION.len(),
        "the activation map declares a family twice"
    );
    for (family, _, expected_keys) in FAULT_FAMILY_ACTIVATION {
        let actual_keys = EXPECTED_STABLE_KEYS
            .iter()
            .filter(|key| family_of(key) == *family)
            .count();
        assert_eq!(
            actual_keys, *expected_keys,
            "family {family} changed size without updating its activation row"
        );
    }
    assert_eq!(
        FAULT_FAMILY_ACTIVATION
            .iter()
            .map(|(_, _, keys)| keys)
            .sum::<usize>(),
        EXPECTED_KEY_COUNT,
        "the activation map does not cover the whole fault vocabulary"
    );
}

/// Whether `source` names exactly `variant` as a `CheckedArtifactFaultKeyV1`
/// reference.
///
/// The trailing-character check matters: variant names nest
/// (`ManagedBootstrapMarkerRetire` is a textual prefix of
/// `ManagedBootstrapMarkerRetiredReobserve`), so a bare `contains` would report a
/// reserved key as sited whenever an activated key extends its name. That would
/// turn the per-key guarantee below into a silent false positive.
fn names_variant(source: &str, variant: &str) -> bool {
    let needle = format!("CheckedArtifactFaultKeyV1::{variant}");
    source.match_indices(&needle).any(|(at, _)| {
        source[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !next.is_ascii_alphanumeric() && next != '_')
    })
}

fn sites_naming(variant: &str) -> Vec<&'static str> {
    FAULT_INJECTION_SOURCES
        .iter()
        .filter(|(_, source)| names_variant(source, variant))
        .map(|(relative, _)| *relative)
        .collect()
}

/// Every declared source naming *any* key of `family`.
///
/// This goes through the same boundary-aware matcher as the per-key scan. A bare
/// `contains` on the family prefix is safe only while no family's variant prefix
/// is a prefix of another's — true of today's ten, but unstated and unenforced,
/// so a future `record_wire` beside `record`, or `barrier_intent` beside
/// `barrier`, would silently weaken the `Reserved` arm's guarantee. Matching the
/// family's *keys* rather than its prefix removes the invariant instead of
/// relying on it.
fn family_sites(family: &str) -> Vec<&'static str> {
    let qualified = variant_prefix(family);
    let mut sites = EXPECTED_STABLE_KEYS
        .iter()
        .filter(|key| family_of(key) == family)
        .flat_map(|key| {
            let suffix = key
                .split_once('.')
                .expect("every stable fault key is family-qualified")
                .1;
            sites_naming(&format!("{qualified}{}", variant_prefix(suffix)))
        })
        .collect::<Vec<_>>();
    sites.sort_unstable();
    sites.dedup();
    sites
}

#[test]
fn reserved_fault_families_have_no_injection_sites_before_their_package() {
    for (family, activation, _) in FAULT_FAMILY_ACTIVATION {
        let sites = family_sites(family);
        match activation {
            FaultFamilyActivationV1::Reserved(owner) => assert!(
                sites.is_empty(),
                "reserved family {family} gained injection sites in {sites:?} but is still \
                 declared for {owner}; the converting package must flip its activation row \
                 and land its matrix rows in the same package (RemPlan §10)"
            ),
            FaultFamilyActivationV1::Executed(owner) => {
                if *family != "runtime" {
                    assert!(
                        !sites.is_empty(),
                        "family {family} is declared executed by {owner} but has no injection \
                         sites in the declared production sources"
                    );
                }
            }
            // The partial arm gives the same guarantee, one key at a time: each
            // activated key must have a site, and each key left reserved must
            // have none. RemPlan §10's duty is thereby discharged per edge,
            // which is the granularity at which the frozen map assigns edges.
            FaultFamilyActivationV1::PartiallyExecuted(owner, activated) => {
                let qualified = variant_prefix(family);
                for suffix in *activated {
                    let variant = format!("{qualified}{}", variant_prefix(suffix));
                    assert!(
                        !sites_naming(&variant).is_empty(),
                        "family {family} declares {family}.{suffix} activated by {owner} but it \
                         has no injection site in the declared production sources"
                    );
                }
                for key in EXPECTED_STABLE_KEYS
                    .iter()
                    .filter(|key| family_of(key) == *family)
                {
                    let suffix = key
                        .split_once('.')
                        .expect("every stable fault key is family-qualified")
                        .1;
                    if activated.contains(&suffix) {
                        continue;
                    }
                    let variant = format!("{qualified}{}", variant_prefix(suffix));
                    let reserved_sites = sites_naming(&variant);
                    assert!(
                        reserved_sites.is_empty(),
                        "{key} is still reserved for {owner} but gained injection sites in \
                         {reserved_sites:?}; the converting package must add it to the \
                         activated list and land its matrix row in the same package \
                         (RemPlan §10)"
                    );
                }
            }
        }
    }
}

/// Mirrors `production_rust_files` in
/// `scripts/checks/check_checked_artifact_boundaries.py:663-670`: a `.rs` file is
/// production unless it sits under a `tests`/`interface_tests` directory or its
/// file name starts with `tests`.
fn production_sources_holding_injection_sites(root: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory).expect("the crate source tree is readable");
        for entry in entries {
            let path = entry.expect("a source tree entry is readable").path();
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .expect("source tree names are UTF-8")
                .to_owned();
            if path.is_dir() {
                if name != "tests" && name != "interface_tests" {
                    pending.push(path);
                }
                continue;
            }
            if !name.ends_with(".rs") || name.starts_with("tests") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("a production source is readable");
            if source.contains("CheckedArtifactFaultKeyV1::") {
                found.insert(relative_slash_path(root, &path));
            }
        }
    }
    found
}

fn relative_slash_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("a scanned source lies under the scan root")
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .expect("source tree names are UTF-8")
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Completeness anchor for `FAULT_INJECTION_SOURCES`. Without it the
/// reserved-family scan is only as complete as the declared source list, so a
/// reserved family could gain an injection site in an unregistered production
/// file while its activation row still read `Reserved`.
///
/// `CheckedArtifactFaultKeyV1` is `pub(super)` on `checked_artifact::fault_v1`
/// (`fault_v1.rs:10`, `checked_artifact/mod.rs:51`), so no injection site can
/// exist outside `src/checked_artifact/` and scanning that subtree is exhaustive.
#[test]
fn the_declared_injection_sources_are_every_production_source_holding_sites() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/checked_artifact");
    let scanned = production_sources_holding_injection_sites(&root);
    let declared = FAULT_INJECTION_SOURCES
        .iter()
        .map(|(relative, _)| (*relative).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        scanned, declared,
        "the production injection-site inventory drifted: every production source that names \
         CheckedArtifactFaultKeyV1 must be declared in FAULT_INJECTION_SOURCES, or the \
         reserved-family scan stops covering it"
    );
}

/// The executed set is an explicit literal, not a projection of the activation
/// map, so flipping a family is a two-place deliberate edit made by the package
/// that lands the matrix — never a side effect.
///
/// R2-D Phase 1 Step 1.3 added `admission`, with its nineteen injection sites
/// and the executed matrix on both target variants
/// (`admission/tests_fault_matrix.rs`). R2-D Phase 2 Step 2.2 added
/// `namespace`, with its eleven injection sites and the executed matrix on both
/// target variants (`namespace/tests_fault_matrix.rs`). R2-D Phase 2 Step 2.4
/// added `record`, with its thirteen injection sites split across the bounded
/// parse owner and the binding owner, and the executed matrix on both target
/// variants (`capability/pre_catalog/provider/tests_authority_record_matrix.rs`).
/// Every remaining family stays reserved for the package the frozen map names
/// (`GwzM5-8R2DInterfaceFreeze.md` §3.5).
#[test]
fn only_the_families_with_executed_matrices_are_executed_today() {
    let executed = FAULT_FAMILY_ACTIVATION
        .iter()
        .filter(|(_, activation, _)| matches!(activation, FaultFamilyActivationV1::Executed(_)))
        .map(|(family, _, _)| *family)
        .collect::<BTreeSet<_>>();

    assert_eq!(
        executed,
        [
            "admission",
            "catalog_bootstrap",
            "durable_leaf",
            "namespace",
            "record",
            "runtime"
        ]
        .into_iter()
        .collect(),
        "a fault family changed activation state; the converting package owns that edit \
         together with its executed matrix evidence"
    );

    // Partially executed families are held to the same two-place rule, and to
    // their exact activated subset: `managed_bootstrap` carries E15/E16 from Step
    // 2.3 and E17 from Step 3.1b and nothing else, so widening that subset without
    // landing the matching matrix rows fails here rather than silently.
    let partial = FAULT_FAMILY_ACTIVATION
        .iter()
        .filter(|(_, activation, _)| {
            matches!(activation, FaultFamilyActivationV1::PartiallyExecuted(..))
        })
        .map(|(family, activation, _)| (*family, activation.activated_suffixes()))
        .collect::<Vec<_>>();

    assert_eq!(
        partial,
        vec![("managed_bootstrap", MANAGED_BOOTSTRAP_EXECUTED_KEYS)],
        "a fault family changed partial activation state; the converting package owns that \
         edit together with its executed matrix evidence"
    );
    assert_eq!(
        MANAGED_BOOTSTRAP_EXECUTED_KEYS.len(),
        28,
        "the managed_bootstrap activated subset changed size; 8 are Step 2.3's E15/E16 \
         boundaries, 15 are Step 3.1b's E17 intent-record lifecycle, and 5 are Step 3.2's \
         staged-component writer"
    );
}

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
