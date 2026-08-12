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

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedCanonicalComponentV1 {
    pub original_ascii: Vec<u8>,
    pub parent_mode: CheckedPathComponentMode,
    pub canonical_ascii: Vec<u8>,
}
impl CheckedCanonicalComponentV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.original_ascii.clone())),
            (2, Cbor::Int(self.parent_mode.wire())),
            (3, Cbor::Bytes(self.canonical_ascii.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            original_ascii: c.try_get(1)?.try_bytes()?,
            parent_mode: CheckedPathComponentMode::from_wire(c.try_get(2)?.try_int()?)?,
            canonical_ascii: c.try_get(3)?.try_bytes()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CheckedCanonicalPathIdentityV1 {
    pub components: Vec<CheckedCanonicalComponentV1>,
}
impl CheckedCanonicalPathIdentityV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Array(self.components.iter().map(|x| x.to_cbor()).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            components: c.try_get(1)?.try_array()?.iter().map(|x| CheckedCanonicalComponentV1::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
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
}
impl CheckedActionScheduleV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Int(self.barrier_count)),
            (2, Cbor::Array(self.bootstraps.iter().map(|x| x.to_cbor()).collect())),
            (3, Cbor::Array(self.cleanup_aliases.iter().map(|x| Cbor::Int(x.wire())).collect())),
            (4, Cbor::Bytes(self.schedule_digest.clone())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            barrier_count: c.try_get(1)?.try_int()?,
            bootstraps: c.try_get(2)?.try_array()?.iter().map(|x| CheckedManagedBootstrapInputV1::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
            cleanup_aliases: c.try_get(3)?.try_array()?.iter().map(|x| Ok(CheckedCleanupAlias::from_wire(x.try_int()?)?)).collect::<Result<Vec<_>, DecodeError>>()?,
            schedule_digest: c.try_get(4)?.try_bytes()?,
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
    pub target_path_profile: CheckedCanonicalPathIdentityV1,
    pub reserved_target_leaf: Vec<u8>,
    pub intent_id: Vec<u8>,
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
            target_path_profile: CheckedCanonicalPathIdentityV1::from_cbor(c.try_get(9)?)?,
            reserved_target_leaf: c.try_get(10)?.try_bytes()?,
            intent_id: c.try_get(11)?.try_bytes()?,
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
}
impl CheckedCleanupWorklistV1 {
    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (1, Cbor::Bytes(self.action_digest.clone())),
            (2, Cbor::Bytes(self.request_owner_binding.clone())),
            (3, Cbor::Bytes(self.schedule_digest.clone())),
            (4, Cbor::Array(self.rows.iter().map(|x| x.to_cbor()).collect())),
        ])
    }
    pub fn from_cbor(c: &Cbor) -> Result<Self, DecodeError> {
        Ok(Self {
            action_digest: c.try_get(1)?.try_bytes()?,
            request_owner_binding: c.try_get(2)?.try_bytes()?,
            schedule_digest: c.try_get(3)?.try_bytes()?,
            rows: c.try_get(4)?.try_array()?.iter().map(|x| CheckedCleanupRowV1::from_cbor(x)).collect::<Result<Vec<_>, DecodeError>>()?,
        })
    }
}
