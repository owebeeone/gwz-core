use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, DurableCatalogTargetDigestV1,
    DurableObjectIdentityV1, FreshObservationDigestV1, HistoricalCollisionDigestV1,
    MissingParentObservationDigestV1, PathComponentMode,
};
use super::super::catalog_names::CatalogPrivateNameV1;

fn identity(value: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([value; 16], 1, vec![value]).unwrap()
}

fn path(root: u8, invocation: u8, domain: u8) -> CanonicalPathIdentityV1 {
    CanonicalPathIdentityV1::new(vec![
        CanonicalComponent::try_bound(
            AsciiComponent::parse(b".gwz").unwrap(),
            PathComponentMode::Sensitive,
            identity(root),
            vec![invocation],
            vec![domain],
        )
        .unwrap(),
        CanonicalComponent::try_bound(
            AsciiComponent::parse(CatalogPrivateNameV1::Final.leaf_bytes()).unwrap(),
            PathComponentMode::Sensitive,
            identity(root + 1),
            vec![invocation + 1],
            vec![domain],
        )
        .unwrap(),
    ])
    .unwrap()
}

#[test]
fn catalog_digest_domains_are_distinct_nonconvertible_types() {
    let fresh = FreshObservationDigestV1::owner_issue([1; 32]);
    let target = DurableCatalogTargetDigestV1::owner_issue([2; 32]);
    let historical = HistoricalCollisionDigestV1::owner_issue([3; 32]);
    let missing = MissingParentObservationDigestV1::owner_issue([4; 32]);

    assert_eq!(fresh.bytes(), [1; 32]);
    assert_eq!(target.bytes(), [2; 32]);
    assert_eq!(historical.bytes(), [3; 32]);
    assert_eq!(missing.bytes(), [4; 32]);
}

#[test]
fn catalog_preflight_surface_has_no_path_plus_lease_or_callback_seam() {
    let pre_catalog = include_str!("../capability/pre_catalog.rs");
    let bootstrap = include_str!("../bootstrap.rs");
    let lease = include_str!("../bootstrap/runtime/catalog_lease.rs");
    let target = include_str!("../bootstrap/runtime/catalog_lease/target.rs");
    let combined = format!("{pre_catalog}\n{bootstrap}\n{lease}\n{target}");

    for forbidden in [
        "CatalogBootstrapV1",
        "RevalidatedPreCatalogPermitV1",
        "lease_binding",
        "recover_or_create_workspace",
        "recover_or_create_git_directory",
        "bootstrap: &Bootstrap",
        "fn git_directory(",
        "fn canonical_target_path(&self)",
    ] {
        assert!(
            !combined.contains(forbidden),
            "forbidden provisional catalog interface remains: {forbidden}"
        );
    }
    for required in [
        "CatalogPreflightV1",
        "MissingGitPrivateParent(Box<MissingCatalogParentPermitV1",
        "Ready(Box<CatalogPermitV1",
        "CatalogMutationLeaseV1<'lease>",
        "CatalogLeaseTargetWitnessV1<'lease>",
        "begin_preflight",
        "CatalogLeaseSetV1",
        "CatalogLeaseTargetBatchV1",
        "repository_common_git_directory",
        "bound: provider::LeaseBoundPreCatalogObservationV1",
        "preflight_catalog_target(",
        "_attempt_binding: CatalogAttemptBindingV1",
        "revalidate_observation",
    ] {
        assert!(
            combined.contains(required),
            "missing C0 interface: {required}"
        );
    }
}

#[test]
fn catalog_batch_ordering_has_no_allocating_stable_sort() {
    let lease = include_str!("../bootstrap/runtime/catalog_lease.rs");
    assert!(!lease.contains(".sort_by("));
    assert_eq!(lease.matches(".sort_unstable_by(").count(), 2);
}

#[test]
fn catalog_publication_uses_one_source_associated_seam() {
    let mutation = include_str!("../capability/pre_catalog/provider/mutation.rs");
    let directory = include_str!("../capability/pre_catalog/provider/directory_mutation.rs");
    // R2-D Phase 1 Step 1.2 extends the caller inventory deliberately, per
    // `GwzM5-8R2D-Plan.md` §4 Step 1.2 and `GwzM5-8R2DInterfaceFreeze.md` §4.4
    // Class 1: admission edges E3 and E4 are the two added production sites,
    // and they publish through the same one primitive.
    let admission = include_str!("../capability/pre_catalog/provider/admission_mutation.rs");
    // R2-D Phase 2 Step 2.2 extends the caller inventory deliberately, per
    // `GwzM5-8R2D-Plan.md` §4 Step 2.2 and `GwzM5-8R2DInterfaceFreeze.md` §4.4
    // Class 1: backend edges E12 (`publish_exact`) and E13 (`retire_exact`) are
    // physically one no-replace move between two deterministic action slots, so
    // they add exactly one production site, in one added file, and they publish
    // through the same one primitive.
    let namespace = include_str!("../capability/pre_catalog/provider/namespace_mutation.rs");
    // R2-D Phase 2 Step 2.3 extends the caller inventory deliberately, per
    // `GwzM5-8R2D-Plan.md` §4 Step 2.3 and `GwzM5-8R2DInterfaceFreeze.md` §4.4
    // Class 1: managed edges E15 (component install) and E16 (ownership-marker
    // retirement) are two no-replace moves — one within the managed parent, one
    // out of the installed component into the action directory's scheduled
    // retirement row — so they add exactly two production sites, in one added
    // file, and they publish through the same one primitive. E15 carries the
    // managed source-interior arm; E16 carries no arm, because §4.3's E16
    // annotation conditions a destination arm on the marker retiring as a
    // directory and it retires as a regular file.
    let managed = include_str!("../capability/pre_catalog/provider/managed_mutation.rs");
    // R2-D Phase 2 Step 2.4 extends the caller inventory deliberately, per
    // `GwzM5-8R2D-Plan.md` §4 Step 2.4 and `GwzM5-8R2DInterfaceFreeze.md` §4.4:
    // the authority record's own durable lifecycle publishes it onto the active
    // slot and retires it onto the scheduled retired alias — two protocol-record
    // moves, in one added file, through the same one sealed primitive.
    let record = include_str!("../capability/pre_catalog/provider/authority_record_binding.rs");
    let publication = include_str!("../capability/pre_catalog/provider/publication.rs");
    let callers = format!("{mutation}\n{directory}\n{admission}\n{namespace}\n{managed}\n{record}");

    assert!(publication.contains("fn publish_verified_no_replace("));
    assert!(publication.contains("open_rename_source("));
    assert!(publication.contains("rename_open_source("));
    assert!(publication.contains("expected_identity"));
    assert_eq!(callers.matches("publish_verified_no_replace(").count(), 13);
    assert!(!callers.contains("platform::rename_relative"));
    assert!(!callers.contains("fn rename_no_replace("));
}

#[test]
fn catalog_owner_surface_is_sealed_and_lease_only() {
    let catalog = include_str!("../catalog.rs");
    let owner = include_str!("../catalog/bootstrap.rs");

    for required in [
        "mod bootstrap;",
        "pub(in crate::checked_artifact) use bootstrap::{",
        "CatalogOwnerV1",
        "OpaqueRetainedCatalogV1",
        "recover_or_create",
    ] {
        assert!(
            catalog.contains(required),
            "missing C2 catalog-owner interface: {required}"
        );
    }
    for forbidden in [
        "bootstrap: &Bootstrap",
        "expected: &CatalogBootstrapRecordV1",
        "token: CatalogBootstrapOwnershipTokenV1",
        "raw_roles:",
        "provider: impl",
    ] {
        assert!(
            !catalog.contains(forbidden),
            "catalog owner exposes forbidden caller authority: {forbidden}"
        );
    }

    for required in [
        "lease: CatalogMutationLeaseV1<'_>",
        ") -> Result<OpaqueRetainedCatalogV1<'_>, CheckedFsError>",
        "CatalogOwnerV1::recover_or_create(lease)",
        "CatalogPreflightV1::MissingGitPrivateParent",
        "CatalogPreflightV1::Ready",
        "CatalogOwnerStepV1::Retry",
        "CatalogOwnerStepV1::Complete",
    ] {
        assert!(
            owner.contains(required),
            "missing sealed C2 owner transition: {required}"
        );
    }
    for forbidden in [
        "dyn ",
        "Box<dyn",
        "provider: impl",
        "root: &Path",
        "raw_roles:",
        "expected: &CatalogBootstrapRecordV1",
        "bootstrap: &",
    ] {
        assert!(
            !owner.contains(forbidden),
            "sealed C2 owner exposes caller-supplied authority: {forbidden}"
        );
    }
}

#[test]
fn completed_catalog_capability_retains_the_target_and_exact_interior_handles() {
    let pre_catalog = include_str!("../capability/pre_catalog.rs");
    let completed = include_str!("../capability/pre_catalog/provider/completed.rs");
    let owner = include_str!("../catalog/bootstrap.rs");
    let permit = struct_body(pre_catalog, "CompletedCatalogPermitV1");
    for field in ["catalog_target", "retained_root", "completed"] {
        assert!(
            permit.contains(field),
            "completed permit is missing {field}"
        );
    }
    let retained = struct_body(completed, "RetainedCompletedCatalogV1");
    for field in [
        "final_directory",
        "catalog_format",
        "catalog_anchor",
        "roaming_anchor",
        "retired_actions",
        "retired_descriptor",
        "retired_bootstrap",
        "expected_bootstrap",
    ] {
        assert!(
            retained.contains(field),
            "retained completed catalog is missing {field}"
        );
    }
    for required in [
        "CatalogOwnerEdgeKindV1",
        "execute_owner_prepare_or_rewrite_staging",
        "execute_owner_publish_final",
        "execute_owner_retire_active",
        "execute_owner_complete",
    ] {
        assert!(owner.contains(required) || pre_catalog.contains(required));
    }
    assert!(!owner.contains("pub enum CatalogOwnerEdgeKindV1"));
    assert!(!completed.contains("pub fn handle"));
}

#[test]
fn ready_and_missing_parent_permits_have_disjoint_exact_authority_fields() {
    let source = include_str!("../capability/pre_catalog.rs");
    let ready = struct_body(source, "CatalogPermitV1");
    for field in [
        "_catalog_target",
        "_retained_root",
        "_raw_roles",
        "_fresh_observation_digest",
        "_durable_target_digest",
        "_historical_collision_digest",
    ] {
        assert!(ready.contains(field), "ready permit is missing {field}");
    }

    let missing = struct_body(source, "MissingCatalogParentPermitV1");
    for field in [
        "_catalog_target",
        "_retained_root",
        "_missing_parent_observation_digest",
    ] {
        assert!(
            missing.contains(field),
            "missing-parent permit is missing {field}"
        );
    }
    for forbidden in [
        "_raw_roles",
        "_fresh_observation_digest",
        "_durable_target_digest",
        "_historical_collision_digest",
    ] {
        assert!(
            !missing.contains(forbidden),
            "missing-parent permit gained ready authority: {forbidden}"
        );
    }
}

#[test]
fn every_live_path_component_retains_identity_mode_and_domain() {
    let value = path(1, 2, 3);
    assert_eq!(value.components().len(), 2);
    assert_eq!(
        value.components()[0].parent_durable_identity(),
        &identity(1)
    );
    assert_eq!(
        value.components()[1].parent_durable_identity(),
        &identity(2)
    );
    assert_eq!(value.components()[0].parent_invocation_identity(), [2]);
    assert_eq!(value.components()[1].parent_invocation_identity(), [3]);
    assert_eq!(value.components()[0].rename_domain(), [3]);
    assert_eq!(value.components()[1].rename_domain(), [3]);
}

fn struct_body<'a>(source: &'a str, name: &str) -> &'a str {
    source
        .split_once(&format!("struct {name}"))
        .unwrap()
        .1
        .split_once('}')
        .unwrap()
        .0
}
