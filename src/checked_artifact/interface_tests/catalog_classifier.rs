use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, DurableCatalogTargetDigestV1,
    DurableObjectIdentityV1, DurablePathV1, HistoricalCollisionDigestV1, PathComponentMode,
    PreCatalogRootKindV1, SupportedFilesystemProfile,
};
use super::super::catalog::{
    CatalogAggregateFactsV1, CatalogAttemptBindingV1, CatalogDirectoryFactV1, CatalogNativeNameV1,
    CatalogParentGrammarV1, CatalogRecordFactV1, CatalogScratchNameV1, classify_catalog_attempt,
};
use super::super::protocol::{
    CatalogBootstrapOwnershipTokenV1, CatalogBootstrapRecoveryDecisionV1,
};

fn identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
}

fn path(byte: u8) -> DurablePathV1 {
    let live = CanonicalPathIdentityV1::new(vec![
        CanonicalComponent::try_bound(
            AsciiComponent::parse(&[byte]).unwrap(),
            PathComponentMode::Sensitive,
            identity(byte),
            vec![byte; 16],
            vec![byte; 16],
        )
        .unwrap(),
    ])
    .unwrap();
    DurablePathV1::from_live(&live).unwrap()
}

fn binding() -> CatalogAttemptBindingV1 {
    CatalogAttemptBindingV1::synthetic_for_test(
        PreCatalogRootKindV1::Workspace,
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
        DurableCatalogTargetDigestV1::owner_issue([2; 32]),
        HistoricalCollisionDigestV1::owner_issue([3; 32]),
        identity(1),
        path(1),
    )
}

fn scratch() -> CatalogScratchNameV1 {
    CatalogScratchNameV1::new(
        DurableCatalogTargetDigestV1::owner_issue([2; 32]),
        HistoricalCollisionDigestV1::owner_issue([3; 32]),
        CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([4; 32]).unwrap(),
    )
}

#[test]
fn scratch_name_is_exact_canonical_and_restart_complete() {
    let name = scratch();
    assert_eq!(name.as_bytes().len(), 241);
    assert_eq!(CatalogScratchNameV1::parse(name.as_bytes()).unwrap(), name);
    assert_eq!(name.durable_target_digest().bytes(), [2; 32]);
    assert_eq!(name.historical_collision_digest().bytes(), [3; 32]);
    assert_eq!(name.ownership_token().as_bytes(), &[4; 32]);

    for malformed in [
        name.as_bytes()[..240].to_vec(),
        name.as_bytes().to_ascii_uppercase(),
        [name.as_bytes(), b".extra"].concat(),
        {
            let mut value = name.as_bytes().to_vec();
            value[49] = b'g';
            value
        },
    ] {
        assert!(CatalogScratchNameV1::parse(&malformed).is_err());
    }
}

#[test]
fn bounded_parent_grammar_charges_every_name_before_classification() {
    let grammar = CatalogParentGrammarV1::new(PathComponentMode::Sensitive);
    let ordinary = (0..4_095).map(|index| {
        CatalogNativeNameV1::unix(format!("ordinary-{index:04}").into_bytes()).unwrap()
    });
    let names = ordinary.chain([CatalogNativeNameV1::unix(scratch().as_bytes().to_vec()).unwrap()]);
    let observed = grammar.classify(names).unwrap();
    assert_eq!(observed.entry_count(), 4_096);
    assert_eq!(observed.scratch_candidates(), 1);
    assert_eq!(observed.recognized_count(), 1);

    let overflow = (0..4_097).map(|index| {
        CatalogNativeNameV1::unix(format!("ordinary-{index:04}").into_bytes()).unwrap()
    });
    assert!(grammar.classify(overflow).is_err());
    assert!(
        CatalogNativeNameV1::unix(vec![b'x'; 256])
            .and_then(|name| grammar.classify([name]))
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_unix_names_are_lossless_budgeted_foreign_rows() {
    let grammar = CatalogParentGrammarV1::new(PathComponentMode::Sensitive);
    let observed = grammar
        .classify([
            CatalogNativeNameV1::unix(vec![0xff, b'x']).unwrap(),
            CatalogNativeNameV1::unix(b"locks".to_vec()).unwrap(),
        ])
        .unwrap();
    assert_eq!(observed.entry_count(), 2);
    assert_eq!(observed.recognized_count(), 0);
    assert_eq!(observed.encoded_name_bytes(), 7);
}

#[test]
fn native_windows_names_are_utf16_charged_and_ascii_classified() {
    let scratch_units = scratch()
        .as_bytes()
        .iter()
        .map(|byte| u16::from(*byte))
        .collect::<Vec<_>>();
    let observed = CatalogParentGrammarV1::new(PathComponentMode::Sensitive)
        .classify([
            CatalogNativeNameV1::windows(scratch_units).unwrap(),
            CatalogNativeNameV1::windows(vec![0x2603]).unwrap(),
        ])
        .unwrap();
    assert_eq!(observed.entry_count(), 2);
    assert_eq!(observed.encoded_name_bytes(), (241 + 1) * 2);
    assert_eq!(observed.scratch_candidates(), 1);
}

#[test]
fn non_ascii_native_names_fail_closed_only_on_case_fold_parents() {
    for name in [
        CatalogNativeNameV1::unix("\u{017f}".as_bytes().to_vec()).unwrap(),
        CatalogNativeNameV1::windows(vec![0x017f]).unwrap(),
        CatalogNativeNameV1::windows(vec![0x212a]).unwrap(),
    ] {
        assert!(
            CatalogParentGrammarV1::new(PathComponentMode::AsciiCaseFold)
                .classify([name.clone()])
                .is_err()
        );
        let observed = CatalogParentGrammarV1::new(PathComponentMode::Sensitive)
            .classify([name])
            .unwrap();
        assert_eq!(observed.entry_count(), 1);
        assert_eq!(observed.recognized_count(), 0);
    }
}

#[test]
fn malformed_duplicate_and_equivalent_reserved_names_are_ambiguous() {
    let exact = scratch();
    let malformed = CatalogNativeNameV1::unix(
        b"checked-artifacts-catalog-bootstrap-v1.scratch.not-a-digest".to_vec(),
    )
    .unwrap();
    assert!(
        CatalogParentGrammarV1::new(PathComponentMode::Sensitive)
            .classify([malformed])
            .is_err()
    );
    let candidate = CatalogNativeNameV1::unix(exact.as_bytes().to_vec()).unwrap();
    assert!(
        CatalogParentGrammarV1::new(PathComponentMode::Sensitive)
            .classify([candidate.clone(), candidate])
            .is_err()
    );
    let alias = CatalogNativeNameV1::unix(exact.as_bytes().to_ascii_uppercase()).unwrap();
    assert!(
        CatalogParentGrammarV1::new(PathComponentMode::AsciiCaseFold)
            .classify([alias])
            .is_err()
    );
}

#[test]
fn zero_and_every_partial_scratch_prefix_recover_the_observed_attempt() {
    let binding = binding();
    let name = scratch();
    let expected = binding.record_from_scratch(&name).unwrap();
    let bytes = expected.encode_canonical();
    for length in 0..bytes.len() {
        let aggregate = CatalogAggregateFactsV1::new(
            vec![CatalogRecordFactV1::scratch(
                name.clone(),
                bytes[..length].to_vec(),
            )],
            CatalogRecordFactV1::Missing,
            CatalogDirectoryFactV1::Missing,
            CatalogDirectoryFactV1::Missing,
            CatalogRecordFactV1::Missing,
        );
        let classified = classify_catalog_attempt(&binding, aggregate);
        assert_eq!(
            classified.decision(),
            CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch
        );
        assert_eq!(classified.expected_record(), Some(&expected));
    }
}

#[test]
fn recovery_uses_historical_name_values_but_rejects_current_target_drift() {
    let name = scratch();
    let original = binding();
    let expected = original.record_from_scratch(&name).unwrap();
    let changed_history = original
        .clone()
        .with_current_historical_for_test(HistoricalCollisionDigestV1::owner_issue([9; 32]));
    let aggregate = CatalogAggregateFactsV1::new(
        vec![CatalogRecordFactV1::scratch(
            name.clone(),
            expected.encode_canonical(),
        )],
        CatalogRecordFactV1::Missing,
        CatalogDirectoryFactV1::Missing,
        CatalogDirectoryFactV1::Missing,
        CatalogRecordFactV1::Missing,
    );
    let classified = classify_catalog_attempt(&changed_history, aggregate);
    assert_eq!(
        classified.decision(),
        CatalogBootstrapRecoveryDecisionV1::PublishActive
    );
    assert_eq!(
        classified
            .expected_record()
            .unwrap()
            .historical_collision_digest(),
        HistoricalCollisionDigestV1::owner_issue([3; 32])
    );

    let changed_target =
        original.with_target_for_test(DurableCatalogTargetDigestV1::owner_issue([8; 32]));
    let aggregate = CatalogAggregateFactsV1::new(
        vec![CatalogRecordFactV1::scratch(
            name,
            expected.encode_canonical(),
        )],
        CatalogRecordFactV1::Missing,
        CatalogDirectoryFactV1::Missing,
        CatalogDirectoryFactV1::Missing,
        CatalogRecordFactV1::Missing,
    );
    assert_eq!(
        classify_catalog_attempt(&changed_target, aggregate).decision(),
        CatalogBootstrapRecoveryDecisionV1::Ambiguous
    );
}

#[test]
fn aggregate_classifier_has_one_closed_edge_for_each_owned_state() {
    let binding = binding();
    let expected = binding.record_from_scratch(&scratch()).unwrap();
    let exact = || CatalogRecordFactV1::exact(expected.clone());
    let cases = [
        (
            CatalogAggregateFactsV1::new(
                vec![],
                CatalogRecordFactV1::Missing,
                CatalogDirectoryFactV1::Missing,
                CatalogDirectoryFactV1::Missing,
                CatalogRecordFactV1::Missing,
            ),
            CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch,
        ),
        (
            CatalogAggregateFactsV1::new(
                vec![],
                exact(),
                CatalogDirectoryFactV1::ActiveOwnedPrefix,
                CatalogDirectoryFactV1::Missing,
                CatalogRecordFactV1::Missing,
            ),
            CatalogBootstrapRecoveryDecisionV1::PrepareOrRewriteStaging,
        ),
        (
            CatalogAggregateFactsV1::new(
                vec![],
                exact(),
                CatalogDirectoryFactV1::ExactOwned,
                CatalogDirectoryFactV1::Missing,
                CatalogRecordFactV1::Missing,
            ),
            CatalogBootstrapRecoveryDecisionV1::PublishFinal,
        ),
        (
            CatalogAggregateFactsV1::new(
                vec![],
                exact(),
                CatalogDirectoryFactV1::Missing,
                CatalogDirectoryFactV1::ExactOwned,
                CatalogRecordFactV1::Missing,
            ),
            CatalogBootstrapRecoveryDecisionV1::RetireActive,
        ),
        (
            CatalogAggregateFactsV1::new(
                vec![],
                CatalogRecordFactV1::Missing,
                CatalogDirectoryFactV1::Missing,
                CatalogDirectoryFactV1::ExactOwned,
                exact(),
            ),
            CatalogBootstrapRecoveryDecisionV1::Complete,
        ),
    ];
    for (aggregate, expected_decision) in cases {
        assert_eq!(
            classify_catalog_attempt(&binding, aggregate).decision(),
            expected_decision
        );
    }
}

#[test]
fn every_unconsumed_or_unowned_reserved_fact_is_ambiguous() {
    let binding = binding();
    let expected = binding.record_from_scratch(&scratch()).unwrap();
    for aggregate in [
        CatalogAggregateFactsV1::new(
            vec![CatalogRecordFactV1::Other],
            CatalogRecordFactV1::Missing,
            CatalogDirectoryFactV1::Missing,
            CatalogDirectoryFactV1::Missing,
            CatalogRecordFactV1::Missing,
        ),
        CatalogAggregateFactsV1::new(
            vec![],
            CatalogRecordFactV1::exact(expected.clone()),
            CatalogDirectoryFactV1::ExactOwned,
            CatalogDirectoryFactV1::ExactOwned,
            CatalogRecordFactV1::Missing,
        ),
        CatalogAggregateFactsV1::new(
            vec![],
            CatalogRecordFactV1::exact(expected),
            CatalogDirectoryFactV1::Other,
            CatalogDirectoryFactV1::Missing,
            CatalogRecordFactV1::Missing,
        ),
    ] {
        assert_eq!(
            classify_catalog_attempt(&binding, aggregate).decision(),
            CatalogBootstrapRecoveryDecisionV1::Ambiguous
        );
    }
}

#[test]
fn pure_catalog_grammar_and_classifier_have_no_filesystem_writer() {
    let sources = [
        include_str!("../catalog.rs"),
        include_str!("../catalog/scratch.rs"),
        include_str!("../catalog/enumeration.rs"),
        include_str!("../catalog/classifier.rs"),
    ]
    .join("\n");
    for forbidden in [
        "std::fs::",
        "cap_std::fs",
        "OpenOptions",
        "File::create",
        ".write_all(",
        ".rename(",
        ".create_dir(",
    ] {
        assert!(
            !sources.contains(forbidden),
            "pure C1 catalog package gained a filesystem edge: {forbidden}"
        );
    }
}
