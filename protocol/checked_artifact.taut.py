"""Private checked-artifact recovery-record schema.

These records are durable implementation protocol, not public GwzCore service
messages. Keeping them in a separate taut schema prevents internal recovery
state from becoming an accidental client API while retaining one authoritative,
generated canonical wire model.
"""

from taut.ir.dsl import BYTES, INT, Enum, F, List, Msg, Ref, schema


SCHEMA = schema(
    Enum("CheckedPathComponentMode",
         sensitive=0,
         ascii_case_fold=1),
    Enum("CheckedDurableIdentityKind",
         linux_ext4=0,
         mac=1,
         windows_ntfs=2),
    Enum("CheckedAdmissionState",
         idle=0,
         preparing=1),
    Enum("CheckedCleanupAlias",
         source=0,
         goal=1,
         authority=2),

    Msg("CheckedCanonicalComponentV1",
        original_ascii=F(1, BYTES),
        parent_mode=F(2, Ref.CheckedPathComponentMode),
        canonical_ascii=F(3, BYTES)),
    Msg("CheckedCanonicalPathIdentityV1",
        components=F(1, List(Ref.CheckedCanonicalComponentV1))),

    # Variant-specific fields are validated by the semantic adapter. Taut owns
    # the field tags and canonical encoding; Rust owns the supported-profile
    # invariants and exposes a closed enum to the rest of checked_artifact.
    Msg("CheckedDurableObjectIdentityV1",
        kind=F(1, Ref.CheckedDurableIdentityKind),
        linux_external_filesystem_uuid=F(2, BYTES, optional=True),
        linux_handle_type=F(3, INT, optional=True),
        linux_persistent_handle=F(4, BYTES, optional=True),
        mac_volume_uuid=F(5, BYTES, optional=True),
        mac_persistent_object_id=F(6, BYTES, optional=True),
        windows_volume_guid_utf16le=F(7, BYTES, optional=True),
        windows_file_id_128=F(8, BYTES, optional=True)),

    Msg("CheckedManagedBootstrapInputV1",
        spec_digest=F(1, BYTES),
        component_count=F(2, INT)),
    Msg("CheckedActionScheduleV1",
        barrier_count=F(1, INT),
        bootstraps=F(2, List(Ref.CheckedManagedBootstrapInputV1)),
        cleanup_aliases=F(3, List(Ref.CheckedCleanupAlias)),
        schedule_digest=F(4, BYTES)),
    Msg("CheckedActionCapacityReservationV1",
        action_digest=F(1, BYTES),
        request_owner_binding=F(2, BYTES),
        schedule=F(3, Ref.CheckedActionScheduleV1),
        record_digest=F(4, BYTES)),
    Msg("CheckedActionDirectoryAdmissionV1",
        state=F(1, Ref.CheckedAdmissionState),
        action_digest=F(2, BYTES, optional=True),
        request_owner_binding=F(3, BYTES, optional=True),
        capacity_schedule_sha256=F(4, BYTES, optional=True),
        staging_name=F(5, BYTES, optional=True),
        final_action_name=F(6, BYTES, optional=True),
        resident_reservation_sha256=F(7, BYTES, optional=True)),

    Msg("CheckedBarrierIntentV1",
        action_digest=F(1, BYTES),
        request_owner_binding=F(2, BYTES),
        schedule_digest=F(3, BYTES),
        ordinal=F(4, INT),
        catalog_anchor_identity=F(5, Ref.CheckedDurableObjectIdentityV1),
        private_home_parent_identity=F(6, Ref.CheckedDurableObjectIdentityV1),
        private_home_name=F(7, BYTES),
        target_parent_identity=F(8, Ref.CheckedDurableObjectIdentityV1),
        target_path_profile=F(9, Ref.CheckedCanonicalPathIdentityV1),
        reserved_target_leaf=F(10, BYTES),
        intent_id=F(11, BYTES)),

    Msg("CheckedDurableLeafFingerprintV1",
        identity=F(1, Ref.CheckedDurableObjectIdentityV1),
        # Fixed-width little-endian u64. Taut INT is signed i64 and must not
        # silently narrow valid filesystem lengths.
        length_u64le=F(2, BYTES),
        sha256=F(3, BYTES)),
    Msg("CheckedCleanupRowV1",
        alias=F(1, Ref.CheckedCleanupAlias),
        expected=F(2, Ref.CheckedDurableLeafFingerprintV1)),
    Msg("CheckedCleanupWorklistV1",
        action_digest=F(1, BYTES),
        request_owner_binding=F(2, BYTES),
        schedule_digest=F(3, BYTES),
        rows=F(4, List(Ref.CheckedCleanupRowV1))),
)
