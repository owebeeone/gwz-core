use super::*;

#[test]
fn physical_parent_enumeration_rejects_maximum_plus_one_entries() {
    let fixture = Fixture::new();
    for index in 0..=MAX_CATALOG_PARENT_ENTRIES_V1 {
        fs::write(
            fixture
                .private_parent()
                .join(format!("ordinary-{index:04}")),
            [],
        )
        .unwrap();
    }

    assert!(observe(&fixture.root).is_err());
    assert!(!fixture.private_parent().join("catalog-final").exists());
}

#[test]
fn physical_parent_observation_recognizes_one_dynamic_scratch_attempt() {
    let fixture = Fixture::new();
    let (target, historical) = {
        let runtime = try_acquire_workspace_runtime(&fixture.root)
            .unwrap()
            .expect("workspace runtime lease");
        let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
        let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
            panic!("workspace parent must produce a ready permit");
        };
        let (_, target, historical) = permit.digests();
        (target, historical)
    };
    let name = CatalogScratchNameV1::new(
        target,
        historical,
        CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([4; 32]).unwrap(),
    );
    fs::write(
        fixture
            .private_parent()
            .join(std::str::from_utf8(name.as_bytes()).unwrap()),
        [],
    )
    .unwrap();

    let observation = observe(&fixture.root).unwrap();
    assert_eq!(observation.raw_roles.enumeration.entry_count(), 2);
    assert_eq!(observation.raw_roles.enumeration.recognized_count(), 1);
    assert_eq!(observation.raw_roles.enumeration.scratch_candidates(), 1);
    assert_eq!(observation.raw_roles.rows.len(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn physical_parent_observation_charges_non_utf8_ordinary_names_losslessly() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let fixture = Fixture::new();
    fs::write(
        fixture
            .private_parent()
            .join(OsStr::from_bytes(b"foreign-\xff")),
        [],
    )
    .unwrap();

    let observation = observe(&fixture.root).unwrap();
    assert_eq!(observation.raw_roles.enumeration.entry_count(), 1);
    assert_eq!(observation.raw_roles.enumeration.encoded_name_bytes(), 9);
    assert_eq!(observation.raw_roles.enumeration.recognized_count(), 0);
    assert!(observation.raw_roles.rows.is_empty());
}

#[test]
fn malformed_scratch_family_entry_is_read_only_ambiguity() {
    let fixture = Fixture::new();
    fs::write(
        fixture
            .private_parent()
            .join("checked-artifacts-catalog-bootstrap-v1.scratch.malformed"),
        [],
    )
    .unwrap();

    assert!(observe(&fixture.root).is_err());
    assert_eq!(
        fs::read(
            fixture
                .private_parent()
                .join("checked-artifacts-catalog-bootstrap-v1.scratch.malformed")
        )
        .unwrap(),
        []
    );
}

// Fold-mode semantics with an existence guard, not APFS specifics: the same
// scenario applies to default case-insensitive NTFS parents (R2-F map G-3).
#[cfg(any(target_os = "macos", windows))]
#[test]
fn physical_case_fold_parent_rejects_non_ascii_equivalent_fixed_and_scratch_names() {
    let fixed = Fixture::new();
    let fixed_alias = "checked-artifact\u{017f}";
    fs::write(fixed.private_parent().join(fixed_alias), b"fixed-alias\n").unwrap();
    if fixed.private_parent().join("catalog-final").exists() {
        assert!(observe(&fixed.root).is_err());
        assert_eq!(
            fs::read(fixed.private_parent().join(fixed_alias)).unwrap(),
            b"fixed-alias\n"
        );
    }

    let scratch_fixture = Fixture::new();
    let (target, historical) = {
        let runtime = try_acquire_workspace_runtime(&scratch_fixture.root)
            .unwrap()
            .expect("workspace runtime lease");
        let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
        let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
            panic!("workspace parent must produce a ready permit");
        };
        let (_, target, historical) = permit.digests();
        (target, historical)
    };
    let canonical = CatalogScratchNameV1::new(
        target,
        historical,
        CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([11; 32]).unwrap(),
    );
    let canonical = std::str::from_utf8(canonical.as_bytes()).unwrap();
    let scratch_alias = canonical.replacen('s', "\u{017f}", 1);
    fs::write(
        scratch_fixture.private_parent().join(&scratch_alias),
        b"scratch-alias\n",
    )
    .unwrap();
    if scratch_fixture.private_parent().join(canonical).exists() {
        assert!(observe(&scratch_fixture.root).is_err());
        assert_eq!(
            fs::read(scratch_fixture.private_parent().join(scratch_alias)).unwrap(),
            b"scratch-alias\n"
        );
    }
}
