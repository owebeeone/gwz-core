use super::super::bootstrap::{
    ManagedParentBootstrap, ManagedParentBootstrapOwnerV1, ManagedParentBootstrapRequest,
    ManagedParentObservationV1, ManagedParentPlanV1,
};
use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, PathComponentMode, PreCatalogRootKindV1,
};
use super::super::coordinator::{
    CheckedActionOperationV1, CheckedActionOwnerV1, CheckedActionRequestV1, CheckedLeafFactV1,
    CoordinatorScheduleDecisionV1, derive_new_reservation, synthetic_action_preimage,
    synthetic_leaf_request, synthetic_owner_preimage, synthetic_record_owner_v0,
    synthetic_record_owner_v1,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn component(value: &[u8]) -> AsciiComponent {
    AsciiComponent::parse(value).unwrap()
}

fn merge_start() -> CheckedActionOwnerV1 {
    CheckedActionOwnerV1::for_merge_start("ws_1").unwrap()
}

fn leaf(
    operation: CheckedActionOperationV1,
    expected: CheckedLeafFactV1,
    goal: CheckedLeafFactV1,
) -> CheckedActionRequestV1 {
    synthetic_leaf_request(
        &merge_start(),
        operation,
        PreCatalogRootKindV1::Workspace,
        vec![component(b"gwz.conf"), component(b"lock")],
        expected,
        goal,
        0,
    )
    .unwrap()
}

#[test]
fn literal_owner_and_action_preimages_and_digests_are_pinned() {
    let start = merge_start();
    let v0 = synthetic_record_owner_v0("ws_1", "merge_1", "op_1", b"record-v0").unwrap();
    let v1 = synthetic_record_owner_v1("ws_1", "merge_1", "op_1", b"record-v1").unwrap();
    let action = leaf(
        CheckedActionOperationV1::Replace,
        CheckedLeafFactV1::Missing,
        CheckedLeafFactV1::Exact {
            length: 3,
            sha256: [5; 32],
        },
    );

    assert_eq!(
        hex(&synthetic_owner_preimage(&start)),
        "67777a2d636865636b65642d6f776e65722d7631000100000200000000000000000477735f3101000000000000000100"
    );
    assert_eq!(
        hex(&start.request_owner_binding().bytes()),
        "745cfb9cfe3834127da6b8a186dd5b07c2e76d1aff6e1424f308d62acfc48f28"
    );
    assert_eq!(
        hex(&synthetic_owner_preimage(&v0)),
        "67777a2d636865636b65642d6f776e65722d7631000101000400000000000000000477735f310100000000000000076d657267655f310200000000000000046f705f31030000000000000020b6667259d89663a42bc0ab9cc725854a0273e0de0ffe7443188a626c107cef1c"
    );
    assert_eq!(
        hex(&v0.request_owner_binding().bytes()),
        "785994aaa8bc695524792fdb81625dfeda99bb3476ec7ba27f0a7944f0a0d89a"
    );
    assert_eq!(
        hex(&synthetic_owner_preimage(&v1)),
        "67777a2d636865636b65642d6f776e65722d7631000102000400000000000000000477735f310100000000000000076d657267655f310200000000000000046f705f310300000000000000208b3f4ec0eda8520800ec2fc91c5a60d707b3c27a9d6755d061dbf9d8344e3e62"
    );
    assert_eq!(
        hex(&v1.request_owner_binding().bytes()),
        "bae304989b3c450d7542a989fe08560ee6907ce4d385c733f0153bc238d3e067"
    );
    assert_eq!(
        hex(&synthetic_action_preimage(&action)),
        "67777a2d636865636b65642d616374696f6e2d76310001000007000000000000000020745cfb9cfe3834127da6b8a186dd5b07c2e76d1aff6e1424f308d62acfc48f280100000000000000010102000000000000000100030000000000000013010002000867777a2e636f6e6600046c6f636b04000000000000000100050000000000000029010000000000000003050505050505050505050505050505050505050505050505050505050505050506000000000000000100"
    );
    assert_eq!(
        hex(&action.action_digest().bytes()),
        "0df28431be38c3ebbc8726a21403fecafe54360f549381c70481841e1298bf2b"
    );
}

#[test]
fn independent_action_digest_matrix_pins_every_encoding_axis() {
    let owner = merge_start();
    let exact_a = CheckedLeafFactV1::Exact {
        length: 7,
        sha256: [3; 32],
    };
    let exact_b = CheckedLeafFactV1::Exact {
        length: 6,
        sha256: [2; 32],
    };
    let leaf = |operation, root_kind, path: Vec<AsciiComponent>, expected, goal, purpose_mask| {
        synthetic_leaf_request(
            &owner,
            operation,
            root_kind,
            path,
            expected,
            goal,
            purpose_mask,
        )
        .unwrap()
    };
    let parent_request = ManagedParentBootstrapRequest::for_merge_start();
    let parent = CheckedActionRequestV1::for_managed_parents(&owner, &parent_request).unwrap();
    assert_eq!(
        hex(&synthetic_action_preimage(&parent)),
        "67777a2d636865636b65642d616374696f6e2d76310001000007000000000000000020745cfb9cfe3834127da6b8a186dd5b07c2e76d1aff6e1424f308d62acfc48f28010000000000000001030200000000000000010003000000000000000100040000000000000001000500000000000000010006000000000000000105"
    );

    let boundary_path = (0..15)
        .map(|_| component(&[b'a'; 255]))
        .chain(std::iter::once(component(&[b'b'; 236])))
        .collect();
    let cases = [
        (
            leaf(
                CheckedActionOperationV1::Observe,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                CheckedLeafFactV1::Missing,
                CheckedLeafFactV1::Missing,
                0,
            ),
            "ad82c4d0af7e4bcf066dcd1e822619a732cfdb14d562f08a770d51454deab03f",
        ),
        (
            leaf(
                CheckedActionOperationV1::Observe,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                exact_a,
                exact_a,
                0,
            ),
            "1394882c2f54e4fa13a016c7b330243e9d292cf64e3162c94e96a5167f1d001b",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                CheckedLeafFactV1::Missing,
                exact_a,
                0,
            ),
            "7c16542e3096b2e8c8a35a8fb835c6d3077925b6f1c172da52981c9dc60e2bba",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                exact_b,
                exact_a,
                0,
            ),
            "8402b79d48dee089c8ff2264106ad1eee43250429419df08e1f1b7bda7cf1592",
        ),
        (
            leaf(
                CheckedActionOperationV1::Remove,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                exact_a,
                CheckedLeafFactV1::Missing,
                0,
            ),
            "8a45906cfd94a0da333367ec736fcff95e15b1a2671a830188bdf3fd17eef6e5",
        ),
        (
            parent,
            "dbd5e9711ece97b70e50fb1027e1df9405ab19c89b8c5c0a4de63959d8f3631b",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::GitDirectory,
                vec![component(b"a")],
                CheckedLeafFactV1::Missing,
                exact_a,
                0,
            ),
            "b3adb8f1bb43d6f952eba8a2555f91faafc4e3d269d5367ff30cdfbaf4ae2ecb",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                CheckedLeafFactV1::Missing,
                exact_a,
                1,
            ),
            "f7c2b4cd374a6c9d16a3acaeabb3af4d317efe25927d400b6955eace5e6f6420",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                CheckedLeafFactV1::Missing,
                exact_a,
                2,
            ),
            "aaaa413e6b86ddc979da35a91955f58a97ab56f6d797edb59c4b9f85448cd153",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                CheckedLeafFactV1::Missing,
                exact_a,
                4,
            ),
            "e3a2a980d170fc18d88359d3a27b4b9597abb0893f4bbd27f40138605fecab14",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                CheckedLeafFactV1::Missing,
                exact_a,
                8,
            ),
            "f573ba0d714490fba0102f85e0eeef7ffe3b30ba5c72c576760d4a54a142ede3",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                CheckedLeafFactV1::Missing,
                exact_a,
                5,
            ),
            "333cb5a9d30577aabc3b4f12997f854e5701a9ca129c3390a60ee159ffbe66b6",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a")],
                CheckedLeafFactV1::Missing,
                exact_a,
                12,
            ),
            "2e71a4a3066d1e1c1fa0b4d80f30fb59096977cb1c93ed5bc35c2c7a180c94d0",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"a"), component(b"bc")],
                CheckedLeafFactV1::Missing,
                exact_a,
                0,
            ),
            "9a47c56a6da8bcaa3cd4cc05d2308aac943eb80afcc94a755e156eda50944350",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![component(b"ab"), component(b"c")],
                CheckedLeafFactV1::Missing,
                exact_a,
                0,
            ),
            "10753df53aa29576b69e8dd4217d2ee1feb0687e3ad98a68dcd02abcfbfacc95",
        ),
        (
            leaf(
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                boundary_path,
                CheckedLeafFactV1::Missing,
                exact_a,
                0,
            ),
            "73ed184527a194ae0b672fad241638e21297d595f49675f371f563b93d9a09f1",
        ),
    ];
    for (request, expected) in cases {
        assert_eq!(hex(&request.action_digest().bytes()), expected);
    }
}

#[test]
fn every_owner_field_and_action_axis_changes_the_binding() {
    let base = synthetic_record_owner_v0("ws_1", "merge_1", "op_1", b"record")
        .unwrap()
        .request_owner_binding();
    for changed in [
        synthetic_record_owner_v0("ws_2", "merge_1", "op_1", b"record").unwrap(),
        synthetic_record_owner_v0("ws_1", "merge_2", "op_1", b"record").unwrap(),
        synthetic_record_owner_v0("ws_1", "merge_1", "op_2", b"record").unwrap(),
        synthetic_record_owner_v0("ws_1", "merge_1", "op_1", b"record-2").unwrap(),
        synthetic_record_owner_v1("ws_1", "merge_1", "op_1", b"record").unwrap(),
    ] {
        assert_ne!(base, changed.request_owner_binding());
    }

    let expected = CheckedLeafFactV1::Exact {
        length: 3,
        sha256: [4; 32],
    };
    let goal = CheckedLeafFactV1::Exact {
        length: 4,
        sha256: [5; 32],
    };
    let base = synthetic_leaf_request(
        &merge_start(),
        CheckedActionOperationV1::Replace,
        PreCatalogRootKindV1::Workspace,
        vec![component(b"one")],
        expected,
        goal,
        0,
    )
    .unwrap()
    .action_digest();
    for changed in [
        synthetic_leaf_request(
            &CheckedActionOwnerV1::for_merge_start("ws_2").unwrap(),
            CheckedActionOperationV1::Replace,
            PreCatalogRootKindV1::Workspace,
            vec![component(b"one")],
            expected,
            goal,
            0,
        )
        .unwrap(),
        synthetic_leaf_request(
            &merge_start(),
            CheckedActionOperationV1::Replace,
            PreCatalogRootKindV1::GitDirectory,
            vec![component(b"one")],
            expected,
            goal,
            0,
        )
        .unwrap(),
        synthetic_leaf_request(
            &merge_start(),
            CheckedActionOperationV1::Replace,
            PreCatalogRootKindV1::Workspace,
            vec![component(b"two")],
            expected,
            goal,
            0,
        )
        .unwrap(),
        synthetic_leaf_request(
            &merge_start(),
            CheckedActionOperationV1::Replace,
            PreCatalogRootKindV1::Workspace,
            vec![component(b"one")],
            CheckedLeafFactV1::Missing,
            goal,
            0,
        )
        .unwrap(),
        synthetic_leaf_request(
            &merge_start(),
            CheckedActionOperationV1::Replace,
            PreCatalogRootKindV1::Workspace,
            vec![component(b"one")],
            expected,
            CheckedLeafFactV1::Exact {
                length: 5,
                sha256: [5; 32],
            },
            0,
        )
        .unwrap(),
        synthetic_leaf_request(
            &merge_start(),
            CheckedActionOperationV1::Replace,
            PreCatalogRootKindV1::Workspace,
            vec![component(b"one")],
            expected,
            goal,
            1,
        )
        .unwrap(),
    ] {
        assert_ne!(base, changed.action_digest());
    }
}

#[test]
fn owner_identity_accepts_only_bounded_durable_record_ids() {
    assert!(CheckedActionOwnerV1::for_merge_start("workspace").is_err());
    assert!(CheckedActionOwnerV1::for_merge_start("ws_").is_err());
    assert!(CheckedActionOwnerV1::for_merge_start(&format!("ws_{}", "a".repeat(253))).is_err());
    assert!(synthetic_record_owner_v0("ws_1", "../merge", "op_1", b"record").is_err());
    assert!(synthetic_record_owner_v0("ws_1", "merge_1", "operation", b"record").is_err());
    assert!(synthetic_record_owner_v0("ws_1", &"a".repeat(256), "op_1", b"record").is_err());
}

#[test]
fn checked_action_identity_rejects_a_path_beyond_the_shared_bound() {
    let oversized = (0..17).map(|_| component(&[b'a'; 255])).collect();
    let error = synthetic_leaf_request(
        &merge_start(),
        CheckedActionOperationV1::Replace,
        PreCatalogRootKindV1::Workspace,
        oversized,
        CheckedLeafFactV1::Missing,
        CheckedLeafFactV1::Exact {
            length: 1,
            sha256: [1; 32],
        },
        0,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CheckedFsError::Ambiguous { detail, .. }
            if detail == "checked leaf path identity exceeds the 4 KiB bound"
    ));
}

#[test]
fn schedule_table_is_coordinator_owned_and_exact() {
    let missing = CheckedLeafFactV1::Missing;
    let first = CheckedLeafFactV1::Exact {
        length: 3,
        sha256: [4; 32],
    };
    let second = CheckedLeafFactV1::Exact {
        length: 4,
        sha256: [5; 32],
    };
    let observe = leaf(CheckedActionOperationV1::Observe, first, first);
    assert_eq!(
        derive_new_reservation(&observe, None).unwrap(),
        CoordinatorScheduleDecisionV1::ProofOnly
    );

    for (request, expected_mask, expected_schedule, expected_reservation) in [
        (
            leaf(CheckedActionOperationV1::Replace, missing, first),
            0b110,
            "feb2f553ddf800568c142d6f90e3f736ddacc94e507e5b72854030f7c2cc138e",
            "6394e2256ce60020a9fbbbb1a29896c03d715d7b3f4aa8f7423c92f22e048b2e",
        ),
        (
            leaf(CheckedActionOperationV1::Replace, first, second),
            0b111,
            "a03e27e5eb51166011d19faf77a8897e8d4c97111201d91d6165f9591acfc38c",
            "dfc9c0586fcf1221b4aeb1fcb39a8b4c4f17dfb6d79302e71091ba94bc1dff64",
        ),
        (
            leaf(CheckedActionOperationV1::Remove, first, missing),
            0b101,
            "58ed14e7e5a55e0c9455d0fda27092eb14a035ffdbd7b1d0deedaa99a97f4674",
            "cd0551b9f555c77043e3d624f906b2c9e8512e3427e23182434a76bd10690b1e",
        ),
    ] {
        let CoordinatorScheduleDecisionV1::Reserve(reservation) =
            derive_new_reservation(&request, None).unwrap()
        else {
            panic!("mutation unexpectedly became proof-only");
        };
        assert_eq!(reservation.schedule().barrier_count(), 64);
        assert_eq!(
            hex(&reservation.schedule().digest().bytes()),
            expected_schedule
        );
        assert_eq!(
            hex(&reservation.record_digest().bytes()),
            expected_reservation
        );
        assert_eq!(
            reservation.schedule().cleanup_aliases().mask(),
            expected_mask
        );
        assert_eq!(reservation.schedule().bootstrap_rows().len(), 0);
    }

    let exact = leaf(CheckedActionOperationV1::Replace, first, first);
    assert_eq!(
        derive_new_reservation(&exact, None).unwrap(),
        CoordinatorScheduleDecisionV1::ProofOnly
    );
}

#[test]
fn parent_only_identity_requires_an_immutable_plan_before_reservation() {
    let parent_request = ManagedParentBootstrapRequest::for_merge_start();
    let request =
        CheckedActionRequestV1::for_managed_parents(&merge_start(), &parent_request).unwrap();
    assert!(derive_new_reservation(&request, None).is_err());

    struct ExistingParent;
    impl ManagedParentBootstrap for ExistingParent {
        type RetainedParents = ();

        fn provider_instance_id(&self) -> [u8; 32] {
            [7; 32]
        }

        fn observe_preflight(
            &self,
            request: &ManagedParentBootstrapRequest,
        ) -> Result<Vec<ManagedParentObservationV1>, CheckedFsError> {
            request
                .specs()
                .iter()
                .map(|spec| {
                    let components = spec
                        .components()
                        .iter()
                        .cloned()
                        .map(|component| {
                            CanonicalComponent::new(component, PathComponentMode::Sensitive)
                        })
                        .collect();
                    ManagedParentObservationV1::new(
                        spec.purpose(),
                        spec.components().len(),
                        DurableObjectIdentityV1::linux_ext4([8; 16], 1, vec![8]).unwrap(),
                        PathComponentMode::Sensitive,
                        CanonicalPathIdentityV1::new(components).unwrap(),
                    )
                })
                .collect()
        }

        fn revalidate_plan(&self, _plan: &ManagedParentPlanV1) -> Result<bool, CheckedFsError> {
            Ok(true)
        }

        fn execute_bound(
            &self,
            _plan: &super::super::bootstrap::BoundManagedParentPlanV1,
        ) -> Result<Self::RetainedParents, CheckedFsError> {
            Ok(())
        }
    }

    let plan = ManagedParentBootstrapOwnerV1::new(&ExistingParent)
        .preflight(
            &parent_request,
            request.action_digest(),
            request.owner_binding(),
        )
        .unwrap();
    assert!(plan.is_proof_only());
    assert_eq!(
        derive_new_reservation(&request, Some(&plan)).unwrap(),
        CoordinatorScheduleDecisionV1::ProofOnly
    );
    let other = CheckedActionRequestV1::for_managed_parents(
        &CheckedActionOwnerV1::for_merge_start("ws_2").unwrap(),
        &parent_request,
    )
    .unwrap();
    assert!(derive_new_reservation(&other, Some(&plan)).is_err());
}
