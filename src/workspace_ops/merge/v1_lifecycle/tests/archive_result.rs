use sha2::{Digest, Sha256};

use super::super::archive::validated_result_for_test;
use crate::workspace_ops::merge::model::archive_projection::ArchiveSourceVersion;
use crate::workspace_ops::merge::model::v1::RecordVersion;
use crate::workspace_ops::merge::record_wire::archived_fixture_for_test;

#[test]
fn archived_result_requires_one_r3_decoding_of_the_same_v0_or_v1_bytes() {
    for version in [RecordVersion::V0, RecordVersion::V1] {
        let (bytes, merge_id) = archived_fixture_for_test(version);
        let result = validated_result_for_test(bytes.clone(), merge_id).unwrap();

        assert_eq!(result.source_version(), version);
        assert_eq!(result.destination_bytes(), bytes);
        assert_eq!(
            result.destination_sha256(),
            <[u8; 32]>::from(Sha256::digest(&bytes))
        );
        assert_eq!(
            result.projection().source_version,
            match version {
                RecordVersion::V0 => ArchiveSourceVersion::V0,
                RecordVersion::V1 => ArchiveSourceVersion::V1,
            }
        );
        assert!(result.cleanup().backup_refs().is_empty());
        assert!(!result.cleanup().has_stash_evidence());
    }
}
