//! Compile-boundary pins for the five seams frozen by R2-D Step 0.1.
//!
//! Controlling text: `dev-docs/GwzM5-8R2DInterfaceFreeze.md`;
//! `GwzM5-8R2D-Plan.md` §4 Step 0.1/0.2 and §9.3;
//! `GwzM5-8R2CCatalogBootstrapAmendment.md` §8.10 (no production signature
//! accepts raw lease bytes, token bytes, raw role rows, expected records,
//! bootstrap callbacks, or synthetic observations) and §8.13 (direct raw
//! provider renames are structurally rejected).
//!
//! These tests pass at landing. They are freeze pins, not red tests: each one
//! fails only if a later package widens or silently reshapes a frozen seam.

/// Seam 2 (adopted plan §9.3). The four managed backend operations are
/// required methods; a re-added default body would restore a silently
/// unavailable managed post-observation in a production provider.
#[test]
fn managed_backend_operations_are_required_not_defaulted() {
    let backend = include_str!("../namespace/backend.rs");

    assert!(
        backend.contains("trait RawNamespaceBackend"),
        "the raw namespace backend seam moved out of namespace/backend.rs"
    );
    for declaration in [
        ") -> Result<ManagedInstallObservationV1, CheckedFsError>;",
        ") -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError>;",
    ] {
        assert_eq!(
            backend.matches(declaration).count(),
            2,
            "a managed backend operation lost its required declaration: {declaration}"
        );
    }
    for defaulted in [
        ") -> Result<ManagedInstallObservationV1, CheckedFsError> {",
        ") -> Result<ManagedMarkerRetirementObservationV1, CheckedFsError> {",
    ] {
        assert!(
            !backend.contains(defaulted),
            "a managed backend operation regained a default body: {defaulted}"
        );
    }
    assert!(
        !backend.contains("managed_operation_unavailable"),
        "the unavailable-managed-operation sentinel returned to the production trait"
    );
    for required in [
        "fn install_managed_component(",
        "fn observe_installed_managed_component(",
        "fn retire_managed_marker(",
        "fn observe_retired_managed_marker(",
    ] {
        assert!(
            backend.contains(required),
            "the frozen managed backend surface lost {required}"
        );
    }
}

/// Every `RawNamespaceBackend` implementation states all four managed
/// operations explicitly. The always-compiled production-shaped provider and
/// both test backends are the complete implementation set today.
#[test]
fn every_namespace_backend_implementation_states_the_managed_operations() {
    let implementations = [
        (
            "namespace/provider_compile.rs",
            include_str!("../namespace/provider_compile.rs"),
        ),
        (
            "namespace/test_support.rs",
            include_str!("../namespace/test_support.rs"),
        ),
    ];
    let expected_impl_count = [
        ("namespace/provider_compile.rs", 1),
        ("namespace/test_support.rs", 2),
    ];

    for (relative, source) in implementations {
        let expected = expected_impl_count
            .iter()
            .find(|(name, _)| *name == relative)
            .map(|(_, count)| *count)
            .expect("implementation inventory is declared");
        assert_eq!(
            source.matches("RawNamespaceBackend for").count(),
            expected,
            "the backend implementation inventory changed in {relative}"
        );
        for operation in [
            "fn install_managed_component(",
            "fn observe_installed_managed_component(",
            "fn retire_managed_marker(",
            "fn observe_retired_managed_marker(",
        ] {
            assert_eq!(
                source.matches(operation).count(),
                expected,
                "{relative} does not state {operation} for every backend implementation"
            );
        }
    }
}

/// Seam 1. The physical admission driver is constructible only from the opaque
/// retained catalog and returns only the opaque admitted-action handoff
/// (amendment §7: "without returning raw handles or mutation capability to
/// callers"; §8.10).
#[test]
fn admission_owner_consumes_only_the_opaque_retained_catalog() {
    let admission = include_str!("../admission/mod.rs");

    for required in [
        "struct ActionAdmissionOwnerV1",
        "fn from_retained_catalog(",
        "catalog: OpaqueRetainedCatalogV1<'lease>",
        "fn resume_or_admit(",
        "expected: &ActionCapacityReservationV1",
        ") -> Result<AdmittedActionV1, CheckedFsError>",
    ] {
        assert!(
            admission.contains(required),
            "the frozen admission driver seam lost {required}"
        );
    }
    for forbidden in [
        "cap_std",
        "&Path",
        "dyn ",
        "pub(crate)",
        "RetainedDirectory",
        "ProviderBinding",
        "raw_roles",
        "bootstrap: &",
        "token:",
    ] {
        assert!(
            !admission.contains(forbidden),
            "the admission driver seam accepts forbidden caller authority: {forbidden}"
        );
    }
    assert_eq!(
        admission.matches("pub(in crate::checked_artifact)").count(),
        3,
        "the admission owner exposes an unfrozen item"
    );
}

/// Seam 3. The `LeafObserver` provider contract Phase 2.1 implements: bounded
/// observation with a caller-stated budget, and one retained handle across
/// exact proof, flush, namespace barrier, and exact reobservation
/// (ConsumerCheckpoint §8 :232-237).
#[test]
fn leaf_observer_provider_seam_is_frozen() {
    let leaf = include_str!("../leaf.rs");

    for required in [
        "trait LeafObserver",
        "fn observe_bounded(",
        "max_bytes: usize",
        "fn observe_durable<Content, Protocol>(",
        "expected: DurableLeafExpectation<'_, Content>",
        "namespace: &mut Protocol",
        "barrier_ordinal: Protocol::BarrierOrdinal",
        ") -> Result<DurableLeafProof<Self::Identity>, CheckedFsError>",
        "enum DurableLeafProof",
        "MissingDurable",
    ] {
        assert!(
            leaf.contains(required),
            "the frozen leaf observer seam lost {required}"
        );
    }
    for forbidden in ["read_to_end", "fn observe_unbounded", "pub(crate)"] {
        assert!(
            !leaf.contains(forbidden),
            "the leaf observer seam gained an unbounded or public route: {forbidden}"
        );
    }
}

/// Seam 4. The `ManagedParentBootstrap` provider contract Phase 3.1
/// implements. Writers receive the retained-parents result, never a path
/// string (ConsumerCheckpoint §9 :264-266).
#[test]
fn managed_parent_bootstrap_provider_seam_is_frozen() {
    let owner = include_str!("../bootstrap/managed/owner.rs");

    for required in [
        "trait ManagedParentBootstrap",
        "type RetainedParents;",
        "fn provider_instance_id(&self) -> [u8; 32];",
        "fn observe_preflight(",
        "fn revalidate_plan(",
        "fn execute_bound(",
        "plan: &BoundManagedParentPlanV1",
        ") -> Result<Self::RetainedParents, CheckedFsError>;",
    ] {
        assert!(
            owner.contains(required),
            "the frozen managed-parent provider seam lost {required}"
        );
    }
    for forbidden in ["fn prepare_parent", "create_dir_all", "-> PathBuf"] {
        assert!(
            !owner.contains(forbidden),
            "the managed-parent provider seam gained ad hoc parent creation: {forbidden}"
        );
    }
}

/// The Track-P spike names the sealed publication primitive, so it must stay
/// test-only. A production module edge here would join the checker's
/// `publication_callers` set and break the six-move seam rule
/// (amendment §8.13).
#[test]
fn the_track_p_publication_spike_is_test_only() {
    let provider = include_str!("../capability/pre_catalog/provider.rs");
    let spike = include_str!("../capability/pre_catalog/provider/tests_admission_spike.rs");

    assert!(
        provider.contains("#[cfg(test)]\nmod tests_admission_spike;"),
        "the admission publication spike is no longer a test-only module"
    );
    assert!(
        spike.contains("publish_verified_no_replace("),
        "the admission spike no longer exercises the sealed publication primitive"
    );
    for forbidden in [
        "rename_relative",
        "open_rename_source",
        "rename_open_source",
    ] {
        assert!(
            !spike.contains(forbidden),
            "the admission spike bypassed the sealed primitive with a raw rename: {forbidden}"
        );
    }
}
