const SCHEMA: &str = include_str!("../../../protocol/checked_artifact.taut.py");

fn message(name: &str, next: &str) -> &'static str {
    let start = SCHEMA
        .find(&format!("Msg(\"{name}\""))
        .unwrap_or_else(|| panic!("missing {name} schema"));
    let end = SCHEMA[start..]
        .find(&format!("Msg(\"{next}\""))
        .map(|offset| start + offset)
        .unwrap_or(SCHEMA.len());
    &SCHEMA[start..end]
}

#[test]
fn durable_records_use_only_the_restart_stable_path_shape() {
    assert!(SCHEMA.contains("Msg(\"CheckedDurablePathComponentV1\""));
    assert!(SCHEMA.contains("Msg(\"CheckedDurablePathV1\""));
    assert!(!SCHEMA.contains("Ref.CheckedCanonicalPathIdentityV1"));

    for durable_message in [
        "CheckedAuthorityV1",
        "CheckedCatalogBootstrapV1",
        "CheckedBarrierIntentV1",
        "CheckedManagedBootstrapComponentV1",
        "CheckedManagedParentBootstrapIntentV1",
    ] {
        let start = SCHEMA.find(&format!("Msg(\"{durable_message}\"")).unwrap();
        let tail = &SCHEMA[start..];
        let end = tail.find("\n    Msg(").unwrap_or(tail.len());
        assert!(
            tail[..end].contains("Ref.CheckedDurablePathV1"),
            "{durable_message} must use the durable path shape"
        );
        assert!(!tail[..end].contains("Ref.CheckedCanonicalPathIdentityV1"));
    }
}

#[test]
fn catalog_bootstrap_schema_contains_only_restart_stable_bindings() {
    let catalog = message("CheckedCatalogBootstrapV1", "CheckedInfrastructureV1");
    for field in [
        "durable_target_digest=F(3, BYTES)",
        "historical_collision_digest=F(4, BYTES)",
        "retained_parent_identity=F(5, Ref.CheckedDurableObjectIdentityV1)",
        "retained_parent_path=F(6, Ref.CheckedDurablePathV1)",
        "staging_name=F(7, BYTES)",
        "final_name=F(8, BYTES)",
        "catalog_anchor_a_name=F(9, BYTES)",
        "catalog_anchor_b_name=F(10, BYTES)",
        "record_id=F(11, BYTES)",
        "bootstrap_ownership_token=F(12, BYTES)",
    ] {
        assert!(
            catalog.contains(field),
            "missing exact catalog field: {field}"
        );
    }
    for forbidden in [
        "invocation_identity",
        "rename_domain",
        "lease_binding",
        "collision_domain_digest",
    ] {
        assert!(
            !catalog.contains(forbidden),
            "catalog schema persists live-only field {forbidden}"
        );
    }
}
