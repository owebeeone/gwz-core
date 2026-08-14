use super::super::bootstrap::{
    BoundManagedParentPlanV1, ManagedParentBootstrap, ManagedParentBootstrapOwnerV1,
    ManagedParentBootstrapRequest, ManagedParentObservationV1, ManagedParentPlanV1,
    ManagedParentPurpose, SyntheticManagedParentAuthorityV1, synthetic_managed_parent_request,
};
use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, PathComponentMode, PreCatalogRootKindV1,
};
use super::super::coordinator::{
    CheckedActionOperationV1, CheckedActionOwnerV1, CheckedActionRequestV1, CheckedLeafFactV1,
    CheckedManagedActionV1, CoordinatorScheduleDecisionV1, derive_new_reservation,
    synthetic_leaf_request,
};
use crate::workspace_ops::RecordVersion;
use crate::workspace_ops::{
    MAX_CHECKED_OWNER_RECORD_BYTES, acquire_canonical_merge_locations, archived_fixture_for_test,
    observe_checked_archive_source_v0, observe_checked_archive_source_v0_leaves_for_test,
    observe_checked_archive_source_v1, observe_checked_owner_v0, observe_checked_owner_v1,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn v0_bytes(extra: &str) -> Vec<u8> {
    v0_bytes_with_state("executing", extra)
}

fn v0_bytes_with_state(state: &str, extra: &str) -> Vec<u8> {
    format!(
        "schema: gwz.merge-operation/v0\nrecord_schema_version: 0\nwriter_version: test\nworkspace_id: ws_test\nmerge_id: merge_1\noperation_id: op_1\nstate: {state}\nsource_ref: feature/x\ncreated_at: now\nbaseline: {{lock_sha256: lock, manifest_sha256: manifest}}\nselected_targets: []\nparticipants: {{}}\n{extra}"
    )
    .into_bytes()
}

fn archive_root(label: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "gwz-checked-archive-{label}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join(".gwz/merge")).unwrap();
    root
}

fn identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
}

struct Provider {
    retained: Vec<usize>,
}

impl ManagedParentBootstrap for Provider {
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
            .zip(&self.retained)
            .enumerate()
            .map(|(index, (spec, retained))| {
                let path = CanonicalPathIdentityV1::new(
                    spec.components()[..*retained]
                        .iter()
                        .cloned()
                        .map(|component| {
                            CanonicalComponent::new(component, PathComponentMode::Sensitive)
                        })
                        .collect(),
                )
                .unwrap();
                ManagedParentObservationV1::new(
                    spec.purpose(),
                    *retained,
                    identity(index as u8 + 1),
                    PathComponentMode::Sensitive,
                    path,
                )
            })
            .collect()
    }

    fn revalidate_plan(&self, _plan: &ManagedParentPlanV1) -> Result<bool, CheckedFsError> {
        Ok(true)
    }

    fn execute_bound(
        &self,
        _plan: &BoundManagedParentPlanV1,
    ) -> Result<Self::RetainedParents, CheckedFsError> {
        Ok(())
    }
}

#[test]
fn durable_owner_is_issued_only_from_one_bounded_decoded_record_observation() {
    let first_bytes = v0_bytes("");
    let second_bytes = v0_bytes("future_field: retained\n");
    let first = observe_checked_owner_v0(&first_bytes).unwrap();
    let second = observe_checked_owner_v0(&second_bytes).unwrap();
    let first_action = CheckedManagedActionV1::for_durable_merge(
        &first,
        &[ManagedParentPurpose::RootPreservationMarkers],
    )
    .unwrap();
    let second_action = CheckedManagedActionV1::for_durable_merge(
        &second,
        &[ManagedParentPurpose::RootPreservationMarkers],
    )
    .unwrap();

    assert_ne!(
        first_action.checked().owner_binding(),
        second_action.checked().owner_binding()
    );
    assert_ne!(
        first_action.checked().action_digest(),
        second_action.checked().action_digest()
    );
    assert!(observe_checked_owner_v0(&[]).is_err());
    assert!(observe_checked_owner_v0(&vec![0; MAX_CHECKED_OWNER_RECORD_BYTES + 1]).is_err());

    let v1_bytes = serde_yaml::to_string(&crate::workspace_ops::test_v1_record())
        .unwrap()
        .into_bytes();
    let v1 = observe_checked_owner_v1(&v1_bytes).unwrap();
    let v1_action = CheckedManagedActionV1::for_durable_merge(
        &v1,
        &[ManagedParentPurpose::PreservationBundles],
    )
    .unwrap();
    assert_ne!(
        first_action.checked().owner_binding(),
        v1_action.checked().owner_binding()
    );
}

#[test]
fn owner_class_and_managed_authority_are_one_sealed_decision() {
    let start = CheckedManagedActionV1::for_merge_start("ws_test").unwrap();
    assert_eq!(
        start
            .managed()
            .specs()
            .iter()
            .map(|spec| spec.purpose())
            .collect::<Vec<_>>(),
        [
            ManagedParentPurpose::MergeStore,
            ManagedParentPurpose::PreservationBundles,
        ]
    );

    let durable = ManagedParentBootstrapRequest::try_for_durable_merge(&[
        ManagedParentPurpose::RootPreservationMarkers,
    ])
    .unwrap();
    let start_owner = CheckedActionOwnerV1::for_merge_start("ws_test").unwrap();
    assert!(CheckedActionRequestV1::for_managed_parents(&start_owner, &durable).is_err());

    let record_bytes = v0_bytes("");
    let record = observe_checked_owner_v0(&record_bytes).unwrap();
    assert!(
        CheckedManagedActionV1::for_durable_merge(&record, &[ManagedParentPurpose::MergeStore],)
            .is_err()
    );
    let root = archive_root("terminal-source");
    let (terminal_bytes, terminal_id) = archived_fixture_for_test(RecordVersion::V0);
    std::fs::write(
        root.join(format!(".gwz/merge/{terminal_id}.yaml")),
        terminal_bytes,
    )
    .unwrap();
    let locations = acquire_canonical_merge_locations(&root, terminal_id).unwrap();
    let source = observe_checked_archive_source_v0(&locations).unwrap();
    let archive = CheckedManagedActionV1::for_archive(&source).unwrap();
    assert_eq!(
        archive.managed().specs()[0].purpose(),
        ManagedParentPurpose::MergeArchive
    );
    assert!(!root.join(".gwz/merge/done").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn archive_authority_requires_one_terminal_open_source_and_an_absent_destination() {
    for state in [
        "executing",
        "awaiting_resolution",
        "halted",
        "finalizing",
        "preserving",
        "rolling_back",
        "recovery_required",
    ] {
        let root = archive_root(&format!("open-{state}"));
        std::fs::write(
            root.join(".gwz/merge/merge_1.yaml"),
            v0_bytes_with_state(state, ""),
        )
        .unwrap();
        let locations = acquire_canonical_merge_locations(&root, "merge_1").unwrap();
        assert!(observe_checked_archive_source_v0(&locations).is_err());
        assert!(!root.join(".gwz/merge/done").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    for shape in ["both-absent", "destination-only", "both-present"] {
        let root = archive_root(shape);
        if shape != "both-absent" {
            std::fs::create_dir(root.join(".gwz/merge/done")).unwrap();
            std::fs::write(
                root.join(".gwz/merge/done/merge_1.yaml"),
                v0_bytes_with_state("completed", ""),
            )
            .unwrap();
        }
        if shape == "both-present" {
            std::fs::write(
                root.join(".gwz/merge/merge_1.yaml"),
                v0_bytes_with_state("completed", ""),
            )
            .unwrap();
        }
        let locations = acquire_canonical_merge_locations(&root, "merge_1").unwrap();
        assert!(observe_checked_archive_source_v0(&locations).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    let root = archive_root("both-present-different");
    std::fs::write(
        root.join(".gwz/merge/merge_1.yaml"),
        v0_bytes_with_state("completed", "source_extension: source\n"),
    )
    .unwrap();
    std::fs::create_dir(root.join(".gwz/merge/done")).unwrap();
    std::fs::write(
        root.join(".gwz/merge/done/merge_1.yaml"),
        v0_bytes_with_state("completed", "destination_extension: destination\n"),
    )
    .unwrap();
    let locations = acquire_canonical_merge_locations(&root, "merge_1").unwrap();
    assert!(observe_checked_archive_source_v0(&locations).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn archive_authority_rejects_a_terminal_label_with_a_contradictory_v0_envelope() {
    let root = archive_root("contradictory-terminal-v0");
    std::fs::write(
        root.join(".gwz/merge/merge_1.yaml"),
        v0_bytes_with_state("completed", ""),
    )
    .unwrap();
    let locations = acquire_canonical_merge_locations(&root, "merge_1").unwrap();

    assert!(observe_checked_archive_source_v0(&locations).is_err());
    assert!(!root.join(".gwz/merge/done").exists());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn archive_authority_rejects_an_archived_leaf_as_its_source_and_supports_terminal_v1() {
    let archived_root = archive_root("wrong-kind");
    std::fs::create_dir(archived_root.join(".gwz/merge/done")).unwrap();
    std::fs::write(
        archived_root.join(".gwz/merge/done/merge_1.yaml"),
        v0_bytes_with_state("completed", ""),
    )
    .unwrap();
    let archived = acquire_canonical_merge_locations(&archived_root, "merge_1").unwrap();
    let absent_root = archive_root("wrong-kind-absent");
    let absent = acquire_canonical_merge_locations(&absent_root, "merge_1").unwrap();
    assert!(
        observe_checked_archive_source_v0_leaves_for_test(archived.archived(), absent.archived(),)
            .is_err()
    );

    let v1_root = archive_root("terminal-v1");
    let (encoded, merge_id) = archived_fixture_for_test(RecordVersion::V1);
    std::fs::write(v1_root.join(format!(".gwz/merge/{merge_id}.yaml")), encoded).unwrap();
    let locations = acquire_canonical_merge_locations(&v1_root, merge_id).unwrap();
    let source = observe_checked_archive_source_v1(&locations).unwrap();
    assert!(CheckedManagedActionV1::for_archive(&source).is_ok());

    std::fs::remove_dir_all(archived_root).unwrap();
    std::fs::remove_dir_all(absent_root).unwrap();
    std::fs::remove_dir_all(v1_root).unwrap();
}

#[test]
fn same_owner_purpose_substitution_cannot_reach_a_reservation() {
    let owner = CheckedActionOwnerV1::for_merge_start("ws_test").unwrap();
    let request_a = synthetic_managed_parent_request(
        &[ManagedParentPurpose::PreservationBundles],
        SyntheticManagedParentAuthorityV1::Unrestricted,
    )
    .unwrap();
    let request_b = synthetic_managed_parent_request(
        &[ManagedParentPurpose::RootPreservationMarkers],
        SyntheticManagedParentAuthorityV1::Unrestricted,
    )
    .unwrap();
    let checked_a = CheckedActionRequestV1::for_managed_parents(&owner, &request_a).unwrap();
    let plan_b = ManagedParentBootstrapOwnerV1::new(&Provider { retained: vec![1] })
        .preflight(
            &request_b,
            checked_a.action_digest(),
            checked_a.owner_binding(),
        )
        .unwrap();
    assert!(derive_new_reservation(&checked_a, Some(&plan_b)).is_err());
}

struct Vector {
    label: &'static str,
    request: ManagedParentBootstrapRequest,
    retained: Vec<usize>,
    leaf: bool,
    schedule: &'static str,
    reservation: &'static str,
    rows: usize,
    components: usize,
    generations: usize,
    cleanup: u8,
}

#[test]
fn literal_managed_schedule_vectors_pin_every_promised_shape() {
    let unrestricted = |purposes: &[ManagedParentPurpose]| {
        synthetic_managed_parent_request(purposes, SyntheticManagedParentAuthorityV1::Unrestricted)
            .unwrap()
    };
    let vectors = [
        Vector {
            label: "one-purpose",
            request: unrestricted(&[ManagedParentPurpose::RootPreservationMarkers]),
            retained: vec![1],
            leaf: false,
            schedule: "dd5736ce49a9ea9a69317bdb72524e9d2b833100459adc8ae112b38f33173184",
            reservation: "b38800dcb1334cbe5bdc9b430bf08b4d953c85d05ebcc49fbc1a3051332bc4a5",
            rows: 1,
            components: 1,
            generations: 3,
            cleanup: 0,
        },
        Vector {
            label: "two-purpose",
            request: unrestricted(&[
                ManagedParentPurpose::PreservationBundles,
                ManagedParentPurpose::RootPreservationMarkers,
            ]),
            retained: vec![1, 1],
            leaf: false,
            schedule: "73271f31ff2e8927673d0249f89119c24fe75f15722107926cfd7b43108f32dd",
            reservation: "d52e42022791f629b4a778fe7a3a99b37a00cb4e3646d251da89e533e5b957a8",
            rows: 2,
            components: 3,
            generations: 8,
            cleanup: 0,
        },
        Vector {
            label: "four-purpose",
            request: unrestricted(ManagedParentPurpose::ALL),
            retained: vec![2, 2, 1, 1],
            leaf: false,
            schedule: "66c436021ae6dae9606d69f8b13b2a16ad40208c8acc17ee4f1e783a61aecf33",
            reservation: "af03dc1674b9bf96ee296c2c5ed9e375c9106b7b19c677869ed5e48dbc6fd749",
            rows: 3,
            components: 4,
            generations: 11,
            cleanup: 0,
        },
        Vector {
            label: "first-merge",
            request: ManagedParentBootstrapRequest::for_merge_start(),
            retained: vec![1, 1],
            leaf: false,
            schedule: "165f13ec951eba6c9f4d3cd24235edb30e45021c5c96902ee674711edf3b5255",
            reservation: "94e73c8214fb8032205fbaabc8252602c215a7dbcf67b541a6e5db3728baeba6",
            rows: 2,
            components: 3,
            generations: 8,
            cleanup: 0,
        },
        Vector {
            label: "partial-bootstrap",
            request: ManagedParentBootstrapRequest::for_merge_start(),
            retained: vec![2, 2],
            leaf: false,
            schedule: "d22ebc58e4c932dea7c413454735c7f39077a74fe9d6332465a934d9e00b496b",
            reservation: "5da5bbfda313245326fe22a992c603395e5bd6bd32c16e5540146741a8fbf800",
            rows: 1,
            components: 1,
            generations: 3,
            cleanup: 0,
        },
        Vector {
            label: "combined-parent-leaf",
            request: unrestricted(&[
                ManagedParentPurpose::MergeStore,
                ManagedParentPurpose::PreservationBundles,
            ]),
            retained: vec![1, 1],
            leaf: true,
            schedule: "d1a3b17c1c9618a779f8966478bf9548dca5c80078308cb5f7dca7c6f0015c45",
            reservation: "5cc308f0e4590c78ec05bcc4cf55b1d17fd437157859577e0e648ac3bba1045e",
            rows: 2,
            components: 3,
            generations: 8,
            cleanup: 0b110,
        },
    ];
    for vector in vectors {
        let owner = CheckedActionOwnerV1::for_merge_start("ws_test").unwrap();
        let checked = if vector.leaf {
            synthetic_leaf_request(
                &owner,
                CheckedActionOperationV1::Replace,
                PreCatalogRootKindV1::Workspace,
                vec![AsciiComponent::parse(b"gwz.conf").unwrap()],
                CheckedLeafFactV1::Missing,
                CheckedLeafFactV1::Exact {
                    length: 3,
                    sha256: [9; 32],
                },
                0b0101,
            )
            .unwrap()
        } else {
            CheckedActionRequestV1::for_managed_parents(&owner, &vector.request).unwrap()
        };
        let plan = ManagedParentBootstrapOwnerV1::new(&Provider {
            retained: vector.retained,
        })
        .preflight(
            &vector.request,
            checked.action_digest(),
            checked.owner_binding(),
        )
        .unwrap();
        let CoordinatorScheduleDecisionV1::Reserve(reservation) =
            derive_new_reservation(&checked, Some(&plan)).unwrap()
        else {
            panic!("{} unexpectedly became proof-only", vector.label);
        };
        assert_eq!(
            reservation.schedule().barrier_count(),
            64,
            "{}",
            vector.label
        );
        assert_eq!(reservation.schedule().bootstrap_rows().len(), vector.rows);
        assert_eq!(
            reservation
                .schedule()
                .bootstrap_rows()
                .iter()
                .map(|row| row.component_range().len())
                .sum::<usize>(),
            vector.components,
            "{}",
            vector.label
        );
        assert_eq!(
            reservation.schedule().cleanup_aliases().mask(),
            vector.cleanup
        );
        assert_eq!(
            reservation.schedule().generation_count(),
            vector.generations,
            "{} generation capacity",
            vector.label
        );
        for (ordinal, row) in reservation.schedule().bootstrap_rows().iter().enumerate() {
            assert_eq!(row.ordinal().index(), ordinal, "{} ordinal", vector.label);
            assert_eq!(
                row.generation_range().len(),
                row.component_range().len() * 2 + 1,
                "{} range",
                vector.label
            );
        }
        assert_eq!(
            hex(&reservation.schedule().digest().bytes()),
            vector.schedule,
            "{} schedule",
            vector.label
        );
        assert_eq!(
            hex(&reservation.record_digest().bytes()),
            vector.reservation,
            "{} reservation",
            vector.label
        );
    }
}

#[test]
fn prefixed_ids_reject_dot_and_dot_dot_suffixes() {
    for value in ["ws_.", "ws_.."] {
        assert!(CheckedManagedActionV1::for_merge_start(value).is_err());
    }
    for operation in ["op_.", "op_.."] {
        let bytes = String::from_utf8(v0_bytes(""))
            .unwrap()
            .replace("op_1", operation);
        assert!(
            CheckedManagedActionV1::for_durable_merge(
                &observe_checked_owner_v0(bytes.as_bytes()).unwrap(),
                &[ManagedParentPurpose::PreservationBundles],
            )
            .is_err()
        );
    }
}
