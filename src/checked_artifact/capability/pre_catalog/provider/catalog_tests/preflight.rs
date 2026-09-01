use super::*;

#[test]
fn lease_derived_ready_preflight_issues_three_noninterchangeable_digests() {
    let fixture = Fixture::new();
    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("workspace parent must produce a ready permit");
    };

    let (fresh, target, historical) = permit.digests();
    assert_ne!(fresh.bytes(), [0; 32]);
    assert_ne!(target.bytes(), [0; 32]);
    assert_ne!(historical.bytes(), [0; 32]);
    permit.revalidate_target_binding().unwrap();
}

#[test]
fn scratch_recovery_keeps_historical_digest_across_unrelated_index_change() {
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
    let scratch = CatalogScratchNameV1::new(
        target,
        historical,
        CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([7; 32]).unwrap(),
    );
    fs::write(
        fixture
            .private_parent()
            .join(std::str::from_utf8(scratch.as_bytes()).unwrap()),
        [],
    )
    .unwrap();
    let repository = git2::Repository::open(&fixture.root).unwrap();
    let id = repository.blob(b"unrelated\n").unwrap();
    let mut index = repository.index().unwrap();
    index
        .add(&git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            file_size: 0,
            id,
            flags: 0,
            flags_extended: 0,
            path: b"ordinary.txt".to_vec(),
        })
        .unwrap();
    index.write().unwrap();

    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("scratch recovery must produce a ready permit");
    };
    let (_, recovered_target, recovered_historical) = permit.digests();
    assert_eq!(recovered_target, target);
    assert_eq!(recovered_historical, historical);
}

#[test]
fn absent_git_private_parent_issues_only_the_disjoint_missing_parent_permit() {
    let fixture = Fixture::new();
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(&fixture.root);
    let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
    let leases = CatalogLeaseSetV1::try_acquire(batch)
        .unwrap()
        .expect("Git target lease");
    let witness = leases.leases().next().unwrap().begin_preflight().unwrap();

    let CatalogPreflightV1::MissingGitPrivateParent(permit) =
        preflight_catalog_target(witness).unwrap()
    else {
        panic!("missing Git parent must not issue a ready permit");
    };
    assert_ne!(permit.observation_digest().bytes(), [0; 32]);
    permit.revalidate_target_binding().unwrap();
    assert!(!fixture.repo().commondir().join("gwz").exists());
}

#[test]
fn exact_active_record_is_the_recovery_attempt_source() {
    let fixture = Fixture::new();
    let record = fresh_record(&fixture, 11);
    fs::write(
        fixture
            .private_parent()
            .join("checked-artifacts-catalog-bootstrap-v1.active"),
        record.encode_canonical(),
    )
    .unwrap();

    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("active recovery must produce a ready permit");
    };
    let (_, target, historical) = permit.digests();
    assert_eq!(target, record.durable_target_digest());
    assert_eq!(historical, record.historical_collision_digest());
}

#[test]
fn exact_retired_record_is_the_completed_reopen_attempt_source() {
    let fixture = Fixture::new();
    let record = fresh_record(&fixture, 12);
    let final_directory = fixture.private_parent().join("catalog-final");
    fs::create_dir(&final_directory).unwrap();
    fs::write(
        final_directory.join(InfrastructureSlotV1::CatalogBootstrapRetired.name()),
        record.encode_canonical(),
    )
    .unwrap();
    let repository = git2::Repository::open(&fixture.root).unwrap();
    let id = repository.blob(b"later index state\n").unwrap();
    let mut index = repository.index().unwrap();
    index
        .add(&git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            file_size: 0,
            id,
            flags: 0,
            flags_extended: 0,
            path: b"later.txt".to_vec(),
        })
        .unwrap();
    index.write().unwrap();

    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("completed recovery must produce a ready permit");
    };
    let (_, target, historical) = permit.digests();
    assert_eq!(target, record.durable_target_digest());
    assert_eq!(historical, record.historical_collision_digest());
}

#[test]
fn conflicting_recovery_attempt_sources_are_read_only_ambiguity() {
    let fixture = Fixture::new();
    let record = fresh_record(&fixture, 13);
    fs::write(
        fixture
            .private_parent()
            .join("checked-artifacts-catalog-bootstrap-v1.active"),
        record.encode_canonical(),
    )
    .unwrap();
    let scratch = CatalogScratchNameV1::new(
        record.durable_target_digest(),
        record.historical_collision_digest(),
        CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([14; 32]).unwrap(),
    );
    fs::write(
        fixture
            .private_parent()
            .join(std::str::from_utf8(scratch.as_bytes()).unwrap()),
        [],
    )
    .unwrap();

    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    assert!(preflight_catalog_target(witness).is_err());
}

#[test]
fn wrong_kind_scratch_candidate_is_read_only_ambiguity() {
    let fixture = Fixture::new();
    let record = fresh_record(&fixture, 15);
    let scratch = CatalogScratchNameV1::new(
        record.durable_target_digest(),
        record.historical_collision_digest(),
        record.bootstrap_ownership_token(),
    );
    fs::create_dir(
        fixture
            .private_parent()
            .join(std::str::from_utf8(scratch.as_bytes()).unwrap()),
    )
    .unwrap();

    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    assert!(preflight_catalog_target(witness).is_err());
}

#[test]
fn ready_permit_revalidation_rejects_namespace_drift() {
    let fixture = Fixture::new();
    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("workspace parent must produce a ready permit");
    };
    fs::write(fixture.private_parent().join("ordinary-after-permit"), []).unwrap();

    assert!(permit.revalidate_observation().is_err());
}

#[test]
fn missing_parent_permit_revalidation_rejects_parent_appearance() {
    let fixture = Fixture::new();
    let request = CatalogLeaseTargetRequestV1::repository_common_git_directory(&fixture.root);
    let batch = CatalogLeaseTargetBatchV1::try_new([request]).unwrap();
    let leases = CatalogLeaseSetV1::try_acquire(batch)
        .unwrap()
        .expect("Git target lease");
    let witness = leases.leases().next().unwrap().begin_preflight().unwrap();
    let CatalogPreflightV1::MissingGitPrivateParent(permit) =
        preflight_catalog_target(witness).unwrap()
    else {
        panic!("missing Git parent must issue its disjoint permit");
    };
    fs::create_dir(fixture.repo().commondir().join("gwz")).unwrap();

    assert!(permit.revalidate_observation().is_err());
}

#[test]
fn ready_permit_classifies_fresh_scratch_and_active_outer_states() {
    let fixture = Fixture::new();
    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("workspace parent must produce a ready permit");
    };
    assert_eq!(
        permit.classify_observed().decision(),
        CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch
    );
    let record = permit.record_for_test(
        CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([21; 32]).unwrap(),
    );
    let scratch = CatalogScratchNameV1::new(
        record.durable_target_digest(),
        record.historical_collision_digest(),
        record.bootstrap_ownership_token(),
    );
    drop(permit);
    drop(runtime);

    let scratch_path = fixture
        .private_parent()
        .join(std::str::from_utf8(scratch.as_bytes()).unwrap());
    fs::write(&scratch_path, []).unwrap();
    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("scratch attempt must produce a ready permit");
    };
    assert_eq!(
        permit.classify_observed().decision(),
        CatalogBootstrapRecoveryDecisionV1::WriteOrRewriteScratch
    );
    drop(permit);
    drop(runtime);

    fs::write(&scratch_path, record.encode_canonical()).unwrap();
    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("exact scratch must produce a ready permit");
    };
    assert_eq!(
        permit.classify_observed().decision(),
        CatalogBootstrapRecoveryDecisionV1::PublishActive
    );
    drop(permit);
    drop(runtime);

    fs::rename(
        &scratch_path,
        fixture
            .private_parent()
            .join("checked-artifacts-catalog-bootstrap-v1.active"),
    )
    .unwrap();
    let runtime = try_acquire_workspace_runtime(&fixture.root)
        .unwrap()
        .expect("workspace runtime lease");
    let witness = runtime.catalog_mutation_lease().begin_preflight().unwrap();
    let CatalogPreflightV1::Ready(permit) = preflight_catalog_target(witness).unwrap() else {
        panic!("active attempt must produce a ready permit");
    };
    assert_eq!(
        permit.classify_observed().decision(),
        CatalogBootstrapRecoveryDecisionV1::PrepareOrRewriteStaging
    );
}
