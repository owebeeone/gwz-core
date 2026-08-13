use std::collections::BTreeSet;
use std::io::Cursor;

use super::super::capability::read_canonical_path_identity;
use super::super::protocol::generated;
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDirectoryAdmissionV1, BarrierIntentV1,
    BoundedCanonicalRecordV1, CatalogBootstrapRecordV1, CheckedAuthorityRecordV1,
    CleanupWorklistV1, InfrastructureRecordV1, ManagedParentBootstrapIntentV1, OwnershipMarkerV1,
    read_bounded_record,
};

const FIXTURE: &str = include_str!("../../../protocol/checked-artifact-semantic-v1/vectors.txt");
const GENERATED_SHAPE_CORPUS: &str =
    include_str!("../../../protocol/checked_artifact-corpus/golden.json");
const REGEN_SCRIPT: &str = include_str!("../../../protocol/regen.py");

struct Vector<'a> {
    name: &'a str,
    kind: &'a str,
    coverage: &'a str,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct Coverage {
    record_kinds: BTreeSet<String>,
    identity_kinds: BTreeSet<i64>,
    path_modes: BTreeSet<i64>,
    admission_states: BTreeSet<i64>,
    cleanup_aliases: BTreeSet<i64>,
    catalog_root_kinds: BTreeSet<i64>,
    filesystem_profiles: BTreeSet<i64>,
    managed_phases: BTreeSet<i64>,
    managed_purposes: BTreeSet<i64>,
    schedule_layouts: BTreeSet<(usize, usize)>,
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert!(!value.is_empty() && value.len().is_multiple_of(2));
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |byte| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                _ => panic!("semantic fixture must use lowercase hexadecimal"),
            };
            (nibble(pair[0]) << 4) | nibble(pair[1])
        })
        .collect()
}

fn vectors() -> Vec<Vector<'static>> {
    assert!(FIXTURE.starts_with(
        "# checked-artifact-semantic-v1\n\
# provenance=independent-hand-authored-semantic-recipes;generator=none;regen-owned=false\n"
    ));
    FIXTURE
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let mut fields = line.split('|');
            let vector = Vector {
                name: fields.next().unwrap(),
                kind: fields.next().unwrap(),
                coverage: fields.next().unwrap(),
                bytes: decode_hex(fields.next().unwrap()),
            };
            assert!(fields.next().is_none(), "unexpected fixture field");
            vector
        })
        .collect()
}

fn bounded_roundtrip<T: BoundedCanonicalRecordV1>(bytes: &[u8]) {
    let value = read_bounded_record::<T>(Cursor::new(bytes)).unwrap();
    assert_eq!(value.encode_record().unwrap(), bytes);
}

fn identity_kind(coverage: &mut Coverage, value: &generated::CheckedDurableObjectIdentityV1) {
    coverage.identity_kinds.insert(value.kind.wire());
}

fn path_coverage(coverage: &mut Coverage, value: &generated::CheckedCanonicalPathIdentityV1) {
    for component in &value.components {
        coverage.path_modes.insert(component.parent_mode.wire());
        identity_kind(coverage, &component.parent_durable_identity);
    }
}

fn inspect_vector(vector: &Vector<'_>, coverage: &mut Coverage) {
    let cbor = crate::cbor::try_decode(&vector.bytes).unwrap();
    coverage.record_kinds.insert(vector.kind.to_owned());
    match vector.kind {
        "canonical_path_identity" => {
            let value = read_canonical_path_identity(Cursor::new(&vector.bytes)).unwrap();
            assert_eq!(value.encode_canonical(), vector.bytes);
            path_coverage(
                coverage,
                &generated::CheckedCanonicalPathIdentityV1::from_cbor(&cbor).unwrap(),
            );
        }
        "capacity" => {
            bounded_roundtrip::<ActionCapacityReservationV1>(&vector.bytes);
            let value = generated::CheckedActionCapacityReservationV1::from_cbor(&cbor).unwrap();
            let component_count = value
                .schedule
                .bootstraps
                .iter()
                .map(|row| usize::try_from(row.component_count).unwrap())
                .sum();
            coverage
                .schedule_layouts
                .insert((value.schedule.bootstraps.len(), component_count));
            coverage.cleanup_aliases.extend(
                value
                    .schedule
                    .cleanup_aliases
                    .iter()
                    .map(|alias| alias.wire()),
            );
        }
        "admission" => {
            bounded_roundtrip::<ActionDirectoryAdmissionV1>(&vector.bytes);
            let value = generated::CheckedActionDirectoryAdmissionV1::from_cbor(&cbor).unwrap();
            coverage.admission_states.insert(value.state.wire());
        }
        "authority" => {
            bounded_roundtrip::<CheckedAuthorityRecordV1>(&vector.bytes);
            let value = generated::CheckedAuthorityV1::from_cbor(&cbor).unwrap();
            path_coverage(coverage, &value.artifact_root);
            identity_kind(coverage, &value.retained_parent_identity);
            identity_kind(coverage, &value.source.identity);
        }
        "catalog_bootstrap" => {
            bounded_roundtrip::<CatalogBootstrapRecordV1>(&vector.bytes);
            let value = generated::CheckedCatalogBootstrapV1::from_cbor(&cbor).unwrap();
            coverage.catalog_root_kinds.insert(value.root_kind.wire());
            coverage
                .filesystem_profiles
                .insert(value.support_profile.wire());
            identity_kind(coverage, &value.retained_parent_identity);
            path_coverage(coverage, &value.retained_parent_path);
        }
        "infrastructure" => {
            bounded_roundtrip::<InfrastructureRecordV1>(&vector.bytes);
            let value = generated::CheckedInfrastructureV1::from_cbor(&cbor).unwrap();
            for identity in [
                &value.catalog_root_identity,
                &value.catalog_anchor_identity,
                &value.roaming_anchor_identity,
                &value.retired_root_identity,
                &value.staging_directory_identity,
            ] {
                identity_kind(coverage, identity);
            }
        }
        "barrier_intent" => {
            bounded_roundtrip::<BarrierIntentV1>(&vector.bytes);
            let value = generated::CheckedBarrierIntentV1::from_cbor(&cbor).unwrap();
            for identity in [
                &value.catalog_anchor_identity,
                &value.private_home_parent_identity,
                &value.target_parent_identity,
            ] {
                identity_kind(coverage, identity);
            }
            path_coverage(coverage, &value.target_path_profile);
        }
        "bootstrap_intent" => {
            bounded_roundtrip::<ManagedParentBootstrapIntentV1>(&vector.bytes);
            let value = generated::CheckedManagedParentBootstrapIntentV1::from_cbor(&cbor).unwrap();
            coverage.managed_phases.insert(value.phase.wire());
            coverage.managed_purposes.insert(value.purpose.wire());
            identity_kind(coverage, &value.retained_parent_identity);
            path_coverage(coverage, &value.retained_parent_path);
            for component in &value.components {
                if let Some(identity) = &component.installed_identity {
                    identity_kind(coverage, identity);
                }
                if let Some(mode) = component.installed_mode {
                    coverage.path_modes.insert(mode.wire());
                }
                if let Some(path) = &component.installed_path {
                    path_coverage(coverage, path);
                }
                if let Some(identity) = &component.ownership_marker_object_identity {
                    identity_kind(coverage, identity);
                }
            }
        }
        "marker" => bounded_roundtrip::<OwnershipMarkerV1>(&vector.bytes),
        "cleanup_worklist" => {
            bounded_roundtrip::<CleanupWorklistV1>(&vector.bytes);
            let value = generated::CheckedCleanupWorklistV1::from_cbor(&cbor).unwrap();
            for row in &value.rows {
                coverage.cleanup_aliases.insert(row.alias.wire());
                identity_kind(coverage, &row.expected.identity);
            }
        }
        other => panic!("unknown semantic fixture kind {other}"),
    }
}

#[test]
fn independent_semantic_vectors_bounded_decode_and_reencode_exact_literals() {
    assert_ne!(FIXTURE, GENERATED_SHAPE_CORPUS);
    assert!(!REGEN_SCRIPT.contains("checked-artifact-semantic-v1"));

    let vectors = vectors();
    assert_eq!(vectors.len(), 26);
    let names = vectors
        .iter()
        .map(|vector| vector.name)
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), vectors.len());
    assert!(vectors.iter().all(|vector| !vector.coverage.is_empty()));

    let mut coverage = Coverage::default();
    for vector in &vectors {
        inspect_vector(vector, &mut coverage);
    }
    assert_eq!(
        coverage.record_kinds,
        [
            "admission",
            "authority",
            "barrier_intent",
            "bootstrap_intent",
            "capacity",
            "canonical_path_identity",
            "catalog_bootstrap",
            "cleanup_worklist",
            "infrastructure",
            "marker",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect()
    );
    assert_eq!(coverage.identity_kinds, [0, 1, 2].into_iter().collect());
    assert_eq!(coverage.path_modes, [0, 1].into_iter().collect());
    assert_eq!(coverage.admission_states, [0, 1].into_iter().collect());
    assert_eq!(coverage.cleanup_aliases, [0, 1, 2].into_iter().collect());
    assert_eq!(coverage.catalog_root_kinds, [0, 1].into_iter().collect());
    assert_eq!(
        coverage.filesystem_profiles,
        [0, 1, 2].into_iter().collect()
    );
    assert_eq!(coverage.managed_phases, [0, 1, 2].into_iter().collect());
    assert_eq!(
        coverage.managed_purposes,
        [0, 1, 2, 3].into_iter().collect()
    );
    assert_eq!(
        coverage.schedule_layouts,
        [(1, 8), (8, 8)].into_iter().collect()
    );
}
