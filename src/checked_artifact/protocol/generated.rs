// GENERATED native Rust types + codec — do not edit.
#![allow(dead_code)]
use crate::cbor::{Cbor, DecodeError};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CheckedPathComponentMode {
    #[default] Sensitive,
    AsciiCaseFold,
}
impl CheckedPathComponentMode {
    pub fn wire(self) -> i64 { match self {
        Self::Sensitive => 0,
        Self::AsciiCaseFold => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Sensitive,
        1 => Self::AsciiCaseFold,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "CheckedPathComponentMode", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CheckedDurableIdentityKind {
    #[default] LinuxExt4,
    Mac,
    WindowsNtfs,
}
impl CheckedDurableIdentityKind {
    pub fn wire(self) -> i64 { match self {
        Self::LinuxExt4 => 0,
        Self::Mac => 1,
        Self::WindowsNtfs => 2,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::LinuxExt4,
        1 => Self::Mac,
        2 => Self::WindowsNtfs,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "CheckedDurableIdentityKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CheckedAdmissionState {
    #[default] Idle,
    Preparing,
}
impl CheckedAdmissionState {
    pub fn wire(self) -> i64 { match self {
        Self::Idle => 0,
        Self::Preparing => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Idle,
        1 => Self::Preparing,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "CheckedAdmissionState", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CheckedCleanupAlias {
    #[default] Source,
    Goal,
    Authority,
}
impl CheckedCleanupAlias {
    pub fn wire(self) -> i64 { match self {
        Self::Source => 0,
        Self::Goal => 1,
        Self::Authority => 2,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Source,
        1 => Self::Goal,
        2 => Self::Authority,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "CheckedCleanupAlias", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CheckedCatalogRootKind {
    #[default] Workspace,
    GitDirectory,
}
impl CheckedCatalogRootKind {
    pub fn wire(self) -> i64 { match self {
        Self::Workspace => 0,
        Self::GitDirectory => 1,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::Workspace,
        1 => Self::GitDirectory,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "CheckedCatalogRootKind", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CheckedManagedBootstrapPhase {
    #[default] InstallComponents,
    RetireMarkers,
    Complete,
}
impl CheckedManagedBootstrapPhase {
    pub fn wire(self) -> i64 { match self {
        Self::InstallComponents => 0,
        Self::RetireMarkers => 1,
        Self::Complete => 2,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::InstallComponents,
        1 => Self::RetireMarkers,
        2 => Self::Complete,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "CheckedManagedBootstrapPhase", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CheckedFilesystemProfile {
    #[default] LinuxExt4FsIocGetfsuuidV1,
    MacPersistentObjectIdV1,
    WindowsNtfsFileId128V1,
}
impl CheckedFilesystemProfile {
    pub fn wire(self) -> i64 { match self {
        Self::LinuxExt4FsIocGetfsuuidV1 => 0,
        Self::MacPersistentObjectIdV1 => 1,
        Self::WindowsNtfsFileId128V1 => 2,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::LinuxExt4FsIocGetfsuuidV1,
        1 => Self::MacPersistentObjectIdV1,
        2 => Self::WindowsNtfsFileId128V1,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "CheckedFilesystemProfile", value: v }),
    }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CheckedManagedParentPurpose {
    #[default] MergeStore,
    MergeArchive,
    PreservationBundles,
    RootPreservationMarkers,
}
impl CheckedManagedParentPurpose {
    pub fn wire(self) -> i64 { match self {
        Self::MergeStore => 0,
        Self::MergeArchive => 1,
        Self::PreservationBundles => 2,
        Self::RootPreservationMarkers => 3,
    } }
    pub fn from_wire(v: i64) -> Result<Self, DecodeError> { Ok(match v {
        0 => Self::MergeStore,
        1 => Self::MergeArchive,
        2 => Self::PreservationBundles,
        3 => Self::RootPreservationMarkers,
        _ => return Err(DecodeError::UnknownEnum { enum_name: "CheckedManagedParentPurpose", value: v }),
    }) }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedDurablePathComponentV1 {
    pub original_ascii: Vec<u8>,
    pub parent_mode: CheckedPathComponentMode,
    pub canonical_ascii: Vec<u8>,
    pub parent_durable_identity: CheckedDurableObjectIdentityV1,
}
impl CheckedDurablePathComponentV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.original_ascii.clone())),
            (2, Cbor::Int(self.parent_mode.wire())),
            (3, Cbor::Bytes(self.canonical_ascii.clone())),
            (4, self.parent_durable_identity.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            original_ascii: c.try_get(1)?.try_bytes()?,
            parent_mode: CheckedPathComponentMode::from_wire(c.try_get(2)?.try_int()?)?,
            canonical_ascii: c.try_get(3)?.try_bytes()?,
            parent_durable_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(4)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedDurablePathV1 {
    pub components: Vec<CheckedDurablePathComponentV1>,
}
impl CheckedDurablePathV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Array(self.components.iter().map(|x| x.to_cbor()).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            components: c.try_get(1)?.try_array()?.iter().map(|x| CheckedDurablePathComponentV1::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedDurableObjectIdentityV1 {
    pub kind: CheckedDurableIdentityKind,
    pub linux_external_filesystem_uuid: Option<Vec<u8>>,
    pub linux_handle_type: Option<i64>,
    pub linux_persistent_handle: Option<Vec<u8>>,
    pub mac_volume_uuid: Option<Vec<u8>>,
    pub mac_persistent_object_id: Option<Vec<u8>>,
    pub windows_volume_guid_utf16le: Option<Vec<u8>>,
    pub windows_file_id_128: Option<Vec<u8>>,
}
impl CheckedDurableObjectIdentityV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.kind.wire())),
            (2, match &self.linux_external_filesystem_uuid { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (3, match &self.linux_handle_type { Some(v) => Cbor::Int(*v), None => Cbor::Null }),
            (4, match &self.linux_persistent_handle { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (5, match &self.mac_volume_uuid { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (6, match &self.mac_persistent_object_id { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (7, match &self.windows_volume_guid_utf16le { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (8, match &self.windows_file_id_128 { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            kind: CheckedDurableIdentityKind::from_wire(c.try_get(1)?.try_int()?)?,
            linux_external_filesystem_uuid: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            linux_handle_type: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_int()?) } },
            linux_persistent_handle: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            mac_volume_uuid: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            mac_persistent_object_id: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            windows_volume_guid_utf16le: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            windows_file_id_128: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedManagedBootstrapInputV1 {
    pub spec_digest: Vec<u8>,
    pub component_count: i64,
}
impl CheckedManagedBootstrapInputV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.spec_digest.clone())),
            (2, Cbor::Int(self.component_count)),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            spec_digest: c.try_get(1)?.try_bytes()?,
            component_count: c.try_get(2)?.try_int()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedActionScheduleV1 {
    pub barrier_count: i64,
    pub bootstraps: Vec<CheckedManagedBootstrapInputV1>,
    pub cleanup_aliases: Vec<CheckedCleanupAlias>,
    pub schedule_digest: Vec<u8>,
    pub managed_plan_digest: Vec<u8>,
}
impl CheckedActionScheduleV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.barrier_count)),
            (2, Cbor::Array(self.bootstraps.iter().map(|x| x.to_cbor()).collect())),
            (3, Cbor::Array(self.cleanup_aliases.iter().map(|x| Cbor::Int(x.wire())).collect())),
            (4, Cbor::Bytes(self.schedule_digest.clone())),
            (5, Cbor::Bytes(self.managed_plan_digest.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            barrier_count: c.try_get(1)?.try_int()?,
            bootstraps: c.try_get(2)?.try_array()?.iter().map(|x| CheckedManagedBootstrapInputV1::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            cleanup_aliases: c.try_get(3)?.try_array()?.iter().map(|x| Ok(CheckedCleanupAlias::from_wire(x.try_int()?)?)).collect::<Result<Vec<_>, DecodeError>>()?,
            schedule_digest: c.try_get(4)?.try_bytes()?,
            managed_plan_digest: c.try_get(5)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedActionCapacityReservationV1 {
    pub action_digest: Vec<u8>,
    pub request_owner_binding: Vec<u8>,
    pub schedule: CheckedActionScheduleV1,
    pub record_digest: Vec<u8>,
}
impl CheckedActionCapacityReservationV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.action_digest.clone())),
            (2, Cbor::Bytes(self.request_owner_binding.clone())),
            (3, self.schedule.to_cbor()),
            (4, Cbor::Bytes(self.record_digest.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            action_digest: c.try_get(1)?.try_bytes()?,
            request_owner_binding: c.try_get(2)?.try_bytes()?,
            schedule: CheckedActionScheduleV1::from_cbor(c.try_get(3)?)?,
            record_digest: c.try_get(4)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedActionDirectoryAdmissionV1 {
    pub state: CheckedAdmissionState,
    pub action_digest: Option<Vec<u8>>,
    pub request_owner_binding: Option<Vec<u8>>,
    pub capacity_schedule_sha256: Option<Vec<u8>>,
    pub staging_name: Option<Vec<u8>>,
    pub final_action_name: Option<Vec<u8>>,
    pub resident_reservation_sha256: Option<Vec<u8>>,
    pub record_digest: Vec<u8>,
}
impl CheckedActionDirectoryAdmissionV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.state.wire())),
            (2, match &self.action_digest { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (3, match &self.request_owner_binding { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (4, match &self.capacity_schedule_sha256 { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (5, match &self.staging_name { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (6, match &self.final_action_name { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (7, match &self.resident_reservation_sha256 { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (8, Cbor::Bytes(self.record_digest.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            state: CheckedAdmissionState::from_wire(c.try_get(1)?.try_int()?)?,
            action_digest: { let v = c.try_get(2)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            request_owner_binding: { let v = c.try_get(3)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            capacity_schedule_sha256: { let v = c.try_get(4)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            staging_name: { let v = c.try_get(5)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            final_action_name: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            resident_reservation_sha256: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            record_digest: c.try_get(8)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedAuthorityV1 {
    pub action_digest: Vec<u8>,
    pub request_owner_binding: Vec<u8>,
    pub schedule_digest: Vec<u8>,
    pub reservation_digest: Vec<u8>,
    pub artifact_root: CheckedDurablePathV1,
    pub retained_parent_identity: CheckedDurableObjectIdentityV1,
    pub source: CheckedDurableLeafFingerprintV1,
    pub expected_sha256: Vec<u8>,
    pub goal_sha256: Vec<u8>,
    pub record_id: Vec<u8>,
}
impl CheckedAuthorityV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.action_digest.clone())),
            (2, Cbor::Bytes(self.request_owner_binding.clone())),
            (3, Cbor::Bytes(self.schedule_digest.clone())),
            (4, Cbor::Bytes(self.reservation_digest.clone())),
            (5, self.artifact_root.to_cbor()),
            (6, self.retained_parent_identity.to_cbor()),
            (7, self.source.to_cbor()),
            (8, Cbor::Bytes(self.expected_sha256.clone())),
            (9, Cbor::Bytes(self.goal_sha256.clone())),
            (10, Cbor::Bytes(self.record_id.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            action_digest: c.try_get(1)?.try_bytes()?,
            request_owner_binding: c.try_get(2)?.try_bytes()?,
            schedule_digest: c.try_get(3)?.try_bytes()?,
            reservation_digest: c.try_get(4)?.try_bytes()?,
            artifact_root: CheckedDurablePathV1::from_cbor(c.try_get(5)?)?,
            retained_parent_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(6)?)?,
            source: CheckedDurableLeafFingerprintV1::from_cbor(c.try_get(7)?)?,
            expected_sha256: c.try_get(8)?.try_bytes()?,
            goal_sha256: c.try_get(9)?.try_bytes()?,
            record_id: c.try_get(10)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedCatalogBootstrapV1 {
    pub root_kind: CheckedCatalogRootKind,
    pub support_profile: CheckedFilesystemProfile,
    pub durable_target_digest: Vec<u8>,
    pub historical_collision_digest: Vec<u8>,
    pub retained_parent_identity: CheckedDurableObjectIdentityV1,
    pub retained_parent_path: CheckedDurablePathV1,
    pub staging_name: Vec<u8>,
    pub final_name: Vec<u8>,
    pub catalog_anchor_a_name: Vec<u8>,
    pub catalog_anchor_b_name: Vec<u8>,
    pub record_id: Vec<u8>,
    pub bootstrap_ownership_token: Vec<u8>,
}
impl CheckedCatalogBootstrapV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.root_kind.wire())),
            (2, Cbor::Int(self.support_profile.wire())),
            (3, Cbor::Bytes(self.durable_target_digest.clone())),
            (4, Cbor::Bytes(self.historical_collision_digest.clone())),
            (5, self.retained_parent_identity.to_cbor()),
            (6, self.retained_parent_path.to_cbor()),
            (7, Cbor::Bytes(self.staging_name.clone())),
            (8, Cbor::Bytes(self.final_name.clone())),
            (9, Cbor::Bytes(self.catalog_anchor_a_name.clone())),
            (10, Cbor::Bytes(self.catalog_anchor_b_name.clone())),
            (11, Cbor::Bytes(self.record_id.clone())),
            (12, Cbor::Bytes(self.bootstrap_ownership_token.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            root_kind: CheckedCatalogRootKind::from_wire(c.try_get(1)?.try_int()?)?,
            support_profile: CheckedFilesystemProfile::from_wire(c.try_get(2)?.try_int()?)?,
            durable_target_digest: c.try_get(3)?.try_bytes()?,
            historical_collision_digest: c.try_get(4)?.try_bytes()?,
            retained_parent_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(5)?)?,
            retained_parent_path: CheckedDurablePathV1::from_cbor(c.try_get(6)?)?,
            staging_name: c.try_get(7)?.try_bytes()?,
            final_name: c.try_get(8)?.try_bytes()?,
            catalog_anchor_a_name: c.try_get(9)?.try_bytes()?,
            catalog_anchor_b_name: c.try_get(10)?.try_bytes()?,
            record_id: c.try_get(11)?.try_bytes()?,
            bootstrap_ownership_token: c.try_get(12)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedInfrastructureV1 {
    pub catalog_format: i64,
    pub catalog_root_identity: CheckedDurableObjectIdentityV1,
    pub catalog_anchor_identity: CheckedDurableObjectIdentityV1,
    pub roaming_anchor_identity: CheckedDurableObjectIdentityV1,
    pub retired_root_identity: CheckedDurableObjectIdentityV1,
    pub catalog_bootstrap_record_id: Vec<u8>,
    pub admission_active_name: Vec<u8>,
    pub admission_scratch_name: Vec<u8>,
    pub admission_staging_name: Vec<u8>,
    pub record_digest: Vec<u8>,
    pub bootstrap_ownership_token: Vec<u8>,
    pub staging_directory_identity: CheckedDurableObjectIdentityV1,
}
impl CheckedInfrastructureV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.catalog_format)),
            (2, self.catalog_root_identity.to_cbor()),
            (3, self.catalog_anchor_identity.to_cbor()),
            (4, self.roaming_anchor_identity.to_cbor()),
            (5, self.retired_root_identity.to_cbor()),
            (6, Cbor::Bytes(self.catalog_bootstrap_record_id.clone())),
            (7, Cbor::Bytes(self.admission_active_name.clone())),
            (8, Cbor::Bytes(self.admission_scratch_name.clone())),
            (9, Cbor::Bytes(self.admission_staging_name.clone())),
            (10, Cbor::Bytes(self.record_digest.clone())),
            (11, Cbor::Bytes(self.bootstrap_ownership_token.clone())),
            (12, self.staging_directory_identity.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            catalog_format: c.try_get(1)?.try_int()?,
            catalog_root_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(2)?)?,
            catalog_anchor_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(3)?)?,
            roaming_anchor_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(4)?)?,
            retired_root_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(5)?)?,
            catalog_bootstrap_record_id: c.try_get(6)?.try_bytes()?,
            admission_active_name: c.try_get(7)?.try_bytes()?,
            admission_scratch_name: c.try_get(8)?.try_bytes()?,
            admission_staging_name: c.try_get(9)?.try_bytes()?,
            record_digest: c.try_get(10)?.try_bytes()?,
            bootstrap_ownership_token: c.try_get(11)?.try_bytes()?,
            staging_directory_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(12)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedBarrierIntentV1 {
    pub action_digest: Vec<u8>,
    pub request_owner_binding: Vec<u8>,
    pub schedule_digest: Vec<u8>,
    pub ordinal: i64,
    pub catalog_anchor_identity: CheckedDurableObjectIdentityV1,
    pub private_home_parent_identity: CheckedDurableObjectIdentityV1,
    pub private_home_name: Vec<u8>,
    pub target_parent_identity: CheckedDurableObjectIdentityV1,
    pub target_path_profile: CheckedDurablePathV1,
    pub reserved_target_leaf: Vec<u8>,
    pub intent_id: Vec<u8>,
    pub reservation_digest: Vec<u8>,
}
impl CheckedBarrierIntentV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.action_digest.clone())),
            (2, Cbor::Bytes(self.request_owner_binding.clone())),
            (3, Cbor::Bytes(self.schedule_digest.clone())),
            (4, Cbor::Int(self.ordinal)),
            (5, self.catalog_anchor_identity.to_cbor()),
            (6, self.private_home_parent_identity.to_cbor()),
            (7, Cbor::Bytes(self.private_home_name.clone())),
            (8, self.target_parent_identity.to_cbor()),
            (9, self.target_path_profile.to_cbor()),
            (10, Cbor::Bytes(self.reserved_target_leaf.clone())),
            (11, Cbor::Bytes(self.intent_id.clone())),
            (12, Cbor::Bytes(self.reservation_digest.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            action_digest: c.try_get(1)?.try_bytes()?,
            request_owner_binding: c.try_get(2)?.try_bytes()?,
            schedule_digest: c.try_get(3)?.try_bytes()?,
            ordinal: c.try_get(4)?.try_int()?,
            catalog_anchor_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(5)?)?,
            private_home_parent_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(6)?)?,
            private_home_name: c.try_get(7)?.try_bytes()?,
            target_parent_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(8)?)?,
            target_path_profile: CheckedDurablePathV1::from_cbor(c.try_get(9)?)?,
            reserved_target_leaf: c.try_get(10)?.try_bytes()?,
            intent_id: c.try_get(11)?.try_bytes()?,
            reservation_digest: c.try_get(12)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedManagedBootstrapComponentV1 {
    pub component_ascii: Vec<u8>,
    pub staging_name: Vec<u8>,
    pub final_name: Vec<u8>,
    pub marker_name: Vec<u8>,
    pub global_component_ordinal: i64,
    pub ownership_marker_id: Option<Vec<u8>>,
    pub ownership_marker_intent_id: Option<Vec<u8>>,
    pub installed_identity: Option<CheckedDurableObjectIdentityV1>,
    pub installed_mode: Option<CheckedPathComponentMode>,
    pub installed_path: Option<CheckedDurablePathV1>,
    pub ownership_marker_object_identity: Option<CheckedDurableObjectIdentityV1>,
}
impl CheckedManagedBootstrapComponentV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.component_ascii.clone())),
            (2, Cbor::Bytes(self.staging_name.clone())),
            (3, Cbor::Bytes(self.final_name.clone())),
            (4, Cbor::Bytes(self.marker_name.clone())),
            (5, Cbor::Int(self.global_component_ordinal)),
            (6, match &self.ownership_marker_id { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (7, match &self.ownership_marker_intent_id { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (8, match &self.installed_identity { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (9, match &self.installed_mode { Some(v) => Cbor::Int(v.wire()), None => Cbor::Null }),
            (10, match &self.installed_path { Some(v) => v.to_cbor(), None => Cbor::Null }),
            (11, match &self.ownership_marker_object_identity { Some(v) => v.to_cbor(), None => Cbor::Null }),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            component_ascii: c.try_get(1)?.try_bytes()?,
            staging_name: c.try_get(2)?.try_bytes()?,
            final_name: c.try_get(3)?.try_bytes()?,
            marker_name: c.try_get(4)?.try_bytes()?,
            global_component_ordinal: c.try_get(5)?.try_int()?,
            ownership_marker_id: { let v = c.try_get(6)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            ownership_marker_intent_id: { let v = c.try_get(7)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            installed_identity: { let v = c.try_get(8)?; if v.is_null() { None } else { Some(CheckedDurableObjectIdentityV1::from_cbor(v)?) } },
            installed_mode: { let v = c.try_get(9)?; if v.is_null() { None } else { Some(CheckedPathComponentMode::from_wire(v.try_int()?)?) } },
            installed_path: { let v = c.try_get(10)?; if v.is_null() { None } else { Some(CheckedDurablePathV1::from_cbor(v)?) } },
            ownership_marker_object_identity: { let v = c.try_get(11)?; if v.is_null() { None } else { Some(CheckedDurableObjectIdentityV1::from_cbor(v)?) } },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedManagedParentBootstrapIntentV1 {
    pub action_digest: Vec<u8>,
    pub request_owner_binding: Vec<u8>,
    pub reservation_digest: Vec<u8>,
    pub schedule_digest: Vec<u8>,
    pub spec_digest: Vec<u8>,
    pub bootstrap_ordinal: i64,
    pub generation_ordinal: i64,
    pub generation_start: i64,
    pub component_start: i64,
    pub retained_parent_identity: CheckedDurableObjectIdentityV1,
    pub retained_parent_mode: CheckedPathComponentMode,
    pub retained_parent_path: CheckedDurablePathV1,
    pub components: Vec<CheckedManagedBootstrapComponentV1>,
    pub ownership_token: Vec<u8>,
    pub predecessor_intent_id: Option<Vec<u8>>,
    pub phase: CheckedManagedBootstrapPhase,
    pub cursor: i64,
    pub intent_id: Vec<u8>,
    pub purpose: CheckedManagedParentPurpose,
    pub managed_plan_digest: Vec<u8>,
}
impl CheckedManagedParentBootstrapIntentV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.action_digest.clone())),
            (2, Cbor::Bytes(self.request_owner_binding.clone())),
            (3, Cbor::Bytes(self.reservation_digest.clone())),
            (4, Cbor::Bytes(self.schedule_digest.clone())),
            (5, Cbor::Bytes(self.spec_digest.clone())),
            (6, Cbor::Int(self.bootstrap_ordinal)),
            (7, Cbor::Int(self.generation_ordinal)),
            (8, Cbor::Int(self.generation_start)),
            (9, Cbor::Int(self.component_start)),
            (10, self.retained_parent_identity.to_cbor()),
            (11, Cbor::Int(self.retained_parent_mode.wire())),
            (12, self.retained_parent_path.to_cbor()),
            (13, Cbor::Array(self.components.iter().map(|x| x.to_cbor()).collect())),
            (14, Cbor::Bytes(self.ownership_token.clone())),
            (15, match &self.predecessor_intent_id { Some(v) => Cbor::Bytes(v.clone()), None => Cbor::Null }),
            (16, Cbor::Int(self.phase.wire())),
            (17, Cbor::Int(self.cursor)),
            (18, Cbor::Bytes(self.intent_id.clone())),
            (19, Cbor::Int(self.purpose.wire())),
            (20, Cbor::Bytes(self.managed_plan_digest.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            action_digest: c.try_get(1)?.try_bytes()?,
            request_owner_binding: c.try_get(2)?.try_bytes()?,
            reservation_digest: c.try_get(3)?.try_bytes()?,
            schedule_digest: c.try_get(4)?.try_bytes()?,
            spec_digest: c.try_get(5)?.try_bytes()?,
            bootstrap_ordinal: c.try_get(6)?.try_int()?,
            generation_ordinal: c.try_get(7)?.try_int()?,
            generation_start: c.try_get(8)?.try_int()?,
            component_start: c.try_get(9)?.try_int()?,
            retained_parent_identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(10)?)?,
            retained_parent_mode: CheckedPathComponentMode::from_wire(c.try_get(11)?.try_int()?)?,
            retained_parent_path: CheckedDurablePathV1::from_cbor(c.try_get(12)?)?,
            components: c.try_get(13)?.try_array()?.iter().map(|x| CheckedManagedBootstrapComponentV1::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            ownership_token: c.try_get(14)?.try_bytes()?,
            predecessor_intent_id: { let v = c.try_get(15)?; if v.is_null() { None } else { Some(v.try_bytes()?) } },
            phase: CheckedManagedBootstrapPhase::from_wire(c.try_get(16)?.try_int()?)?,
            cursor: c.try_get(17)?.try_int()?,
            intent_id: c.try_get(18)?.try_bytes()?,
            purpose: CheckedManagedParentPurpose::from_wire(c.try_get(19)?.try_int()?)?,
            managed_plan_digest: c.try_get(20)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedOwnershipMarkerV1 {
    pub action_digest: Vec<u8>,
    pub request_owner_binding: Vec<u8>,
    pub schedule_digest: Vec<u8>,
    pub intent_id: Vec<u8>,
    pub bootstrap_ordinal: i64,
    pub local_component_ordinal: i64,
    pub global_component_ordinal: i64,
    pub component_ascii: Vec<u8>,
    pub staging_name: Vec<u8>,
    pub final_name: Vec<u8>,
    pub ownership_token: Vec<u8>,
    pub marker_id: Vec<u8>,
}
impl CheckedOwnershipMarkerV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.action_digest.clone())),
            (2, Cbor::Bytes(self.request_owner_binding.clone())),
            (3, Cbor::Bytes(self.schedule_digest.clone())),
            (4, Cbor::Bytes(self.intent_id.clone())),
            (5, Cbor::Int(self.bootstrap_ordinal)),
            (6, Cbor::Int(self.local_component_ordinal)),
            (7, Cbor::Int(self.global_component_ordinal)),
            (8, Cbor::Bytes(self.component_ascii.clone())),
            (9, Cbor::Bytes(self.staging_name.clone())),
            (10, Cbor::Bytes(self.final_name.clone())),
            (11, Cbor::Bytes(self.ownership_token.clone())),
            (12, Cbor::Bytes(self.marker_id.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            action_digest: c.try_get(1)?.try_bytes()?,
            request_owner_binding: c.try_get(2)?.try_bytes()?,
            schedule_digest: c.try_get(3)?.try_bytes()?,
            intent_id: c.try_get(4)?.try_bytes()?,
            bootstrap_ordinal: c.try_get(5)?.try_int()?,
            local_component_ordinal: c.try_get(6)?.try_int()?,
            global_component_ordinal: c.try_get(7)?.try_int()?,
            component_ascii: c.try_get(8)?.try_bytes()?,
            staging_name: c.try_get(9)?.try_bytes()?,
            final_name: c.try_get(10)?.try_bytes()?,
            ownership_token: c.try_get(11)?.try_bytes()?,
            marker_id: c.try_get(12)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedDurableLeafFingerprintV1 {
    pub identity: CheckedDurableObjectIdentityV1,
    pub length_u64le: Vec<u8>,
    pub sha256: Vec<u8>,
}
impl CheckedDurableLeafFingerprintV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, self.identity.to_cbor()),
            (2, Cbor::Bytes(self.length_u64le.clone())),
            (3, Cbor::Bytes(self.sha256.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            identity: CheckedDurableObjectIdentityV1::from_cbor(c.try_get(1)?)?,
            length_u64le: c.try_get(2)?.try_bytes()?,
            sha256: c.try_get(3)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedCleanupRowV1 {
    pub alias: CheckedCleanupAlias,
    pub expected: CheckedDurableLeafFingerprintV1,
}
impl CheckedCleanupRowV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.alias.wire())),
            (2, self.expected.to_cbor()),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            alias: CheckedCleanupAlias::from_wire(c.try_get(1)?.try_int()?)?,
            expected: CheckedDurableLeafFingerprintV1::from_cbor(c.try_get(2)?)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedCleanupWorklistV1 {
    pub action_digest: Vec<u8>,
    pub request_owner_binding: Vec<u8>,
    pub schedule_digest: Vec<u8>,
    pub rows: Vec<CheckedCleanupRowV1>,
    pub reservation_digest: Vec<u8>,
}
impl CheckedCleanupWorklistV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.action_digest.clone())),
            (2, Cbor::Bytes(self.request_owner_binding.clone())),
            (3, Cbor::Bytes(self.schedule_digest.clone())),
            (4, Cbor::Array(self.rows.iter().map(|x| x.to_cbor()).collect())),
            (5, Cbor::Bytes(self.reservation_digest.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            action_digest: c.try_get(1)?.try_bytes()?,
            request_owner_binding: c.try_get(2)?.try_bytes()?,
            schedule_digest: c.try_get(3)?.try_bytes()?,
            rows: c.try_get(4)?.try_array()?.iter().map(|x| CheckedCleanupRowV1::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            reservation_digest: c.try_get(5)?.try_bytes()?,
        })
    }
}
